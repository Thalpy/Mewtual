//! Per-server cache housekeeping: gather the planner's inputs from this server's own state, run
//! bounded eviction batches, and report a status the footer and settings page can show.
//!
//! The policy lives in [`crate::cache_plan`] and is pure. This module is the part that knows where
//! each input comes from and what makes an input untrustworthy. The governing rule is that **an
//! input that cannot be read stops eviction; it never reads as empty**:
//!
//! - no complete cache scan, no known limit, or an unreadable setting: nothing runs;
//! - the file index document missing or its projection failing: nothing runs;
//! - rows past the 256-row cap, or the profile document missing: the unclaimed class is skipped
//!   (`PlanInput::unclaimed_allowed`), because "named by nobody" is then unknowable;
//! - a Keep or Download inventory error, an unreadable saved plan, or unknown Studio protection:
//!   nothing runs, and unknown Studio protection asks the desktop's reference service for a scan.
//!
//! Deletes go through the server's own blob handle (the only one whose read epoch a delete must
//! retire) and `ProtectedBlobs`, which re-checks Studio protection under its mutex through the
//! unlink. At most [`MAX_DELETES_PER_TURN`] run per actor turn, and the plan is rebuilt before
//! every batch, so a pin, row or Keep change between turns is always seen before the next delete.

use std::collections::{BTreeSet, VecDeque};

use catcoms_rt::{CryptoRngCore, MeshTransport};
use catcoms_storage::{Cid, FileManifest};
use catcoms_wire::DocType;

use crate::cache_plan::{self, CachePlan, PlanInput, PlanRow, NEARLY_FULL_PERCENT};
use crate::{
    raw_file_index_row_count, read_file_rows, read_profile_records, Server, FILE_INDEX_DOC,
    MAX_FILE_ENTRIES, PROFILE_DOC,
};

/// A file read within this window counts as in use and is pinned.
pub const IN_USE_MS: u64 = 10 * 60 * 1000;
/// How many recently read files the in-use set remembers. When full, the oldest entry leaves,
/// which only makes that file evictable again; it never protects more than it should.
pub const MAX_IN_USE_FILES: usize = 64;
/// Minimum spacing of eviction passes (an explicit limit change bypasses it).
pub const PASS_INTERVAL_MS: u64 = 30_000;
/// A full evaluation (including Keep duplicates, which no cheap trigger can see) runs at least
/// this often while the server is running.
pub const EVALUATE_INTERVAL_MS: u64 = 60_000;
/// Deletes per actor turn, so commands and sync keep flowing during a large pass.
pub const MAX_DELETES_PER_TURN: usize = 64;
/// Status events are coalesced to at most one per this interval.
pub const STATUS_INTERVAL_MS: u64 = 1_000;
/// An actor asks for a Studio reference scan at most this often (the desktop service also
/// rate-limits and backs off on failure).
pub const STUDIO_REQUEST_INTERVAL_MS: u64 = 60_000;

/// The limit as this server's actor knows it. The desktop bridge loads or migrates the sealed
/// per-server setting and hands it over; until then nothing is evicted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CacheLimit {
    /// No setting handed over yet (before attach finished, or the in-memory fallback store).
    #[default]
    Pending,
    /// The sealed setting exists but could not be read. Eviction stays off until the user picks a
    /// limit again: losing data to a damaged setting is worse than a cache that grows a while.
    Unreadable,
    /// No limit: only completed-Keep duplicates are dropped.
    Unlimited,
    /// A limit in bytes, within `cache_plan::MIN_CACHE_LIMIT..=MAX_CACHE_LIMIT`.
    Bytes(u64),
}

impl CacheLimit {
    fn bytes(self) -> Option<Option<u64>> {
        match self {
            Self::Bytes(bytes) => Some(Some(bytes)),
            Self::Unlimited => Some(None),
            Self::Pending | Self::Unreadable => None,
        }
    }
}

/// Where a server's limit came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheSettingOrigin {
    /// Computed once from usage when the setting did not exist (`cache_plan::migrated_limit`).
    Migrated,
    /// Chosen by the user.
    User,
}

/// The sealed per-server, per-device cache setting (`servers/<id>.storage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheSetting {
    /// `None` = No limit.
    pub limit: Option<u64>,
    pub origin: CacheSettingOrigin,
}

/// Result of reading the sealed setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheSettingLoad {
    /// No record, or a record naming another group: migrate from current usage.
    Missing,
    /// A record that exists but cannot be read: keep eviction off.
    Unreadable,
    Found(CacheSetting),
}

const CACHE_SETTING_V1: u8 = 1;

impl CacheSetting {
    /// Version 1: version byte, group id (length-prefixed), limit tag (0 = No limit, 1 = bytes)
    /// and u64, origin byte (0 = migrated, 1 = user). The group binding means a record copied to
    /// another server number can never apply there.
    pub fn encode_for(&self, group: &[u8]) -> Vec<u8> {
        let mut e = catcoms_wire::Encoder::new();
        e.put_u8(CACHE_SETTING_V1);
        e.put_bytes(group).expect("group id fits");
        match self.limit {
            None => {
                e.put_u8(0);
            }
            Some(bytes) => {
                e.put_u8(1);
                e.put_u64(bytes);
            }
        }
        e.put_u8(match self.origin {
            CacheSettingOrigin::Migrated => 0,
            CacheSettingOrigin::User => 1,
        });
        e.finish()
    }

    /// Decode a version 1 record into `(group, setting)`. Anything else, including an out-of-range
    /// limit or trailing bytes, is `None` (unreadable).
    pub fn decode(bytes: &[u8]) -> Option<(Vec<u8>, Self)> {
        let mut d = catcoms_wire::Decoder::new(bytes);
        if d.get_u8().ok()? != CACHE_SETTING_V1 {
            return None;
        }
        let group = d.get_bytes().ok()?.to_vec();
        if group.is_empty() || group.len() > 256 {
            return None;
        }
        let limit = match d.get_u8().ok()? {
            0 => None,
            1 => Some(d.get_u64().ok()?),
            _ => return None,
        };
        let origin = match d.get_u8().ok()? {
            0 => CacheSettingOrigin::Migrated,
            1 => CacheSettingOrigin::User,
            _ => return None,
        };
        d.finish().ok()?;
        if !cache_plan::valid_limit(limit) {
            return None;
        }
        Some((group, Self { limit, origin }))
    }

    /// The limit as the actor holds it.
    pub fn as_limit(&self) -> CacheLimit {
        match self.limit {
            None => CacheLimit::Unlimited,
            Some(bytes) => CacheLimit::Bytes(bytes),
        }
    }
}

/// Why housekeeping is not deleting anything right now. Each maps to a short footer phrase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePause {
    /// No size-limited cache, no complete mount scan yet, or no limit handed over yet.
    NotReady,
    /// The last scan could not read every entry.
    ScanIncomplete,
    /// The sealed setting could not be read.
    SettingUnreadable,
    /// The file index document is missing or its projection failed.
    IndexUnreadable,
    /// A Keep or Download inventory or saved plan could not be read.
    PinsUnknown,
    /// Studio protection is unknown; a reference scan has been requested.
    StudioUnknown,
}

impl CachePause {
    /// Stable lowercase name for the desktop bridge and frontend.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotReady => "not-ready",
            Self::ScanIncomplete => "scan-incomplete",
            Self::SettingUnreadable => "setting-unreadable",
            Self::IndexUnreadable => "index-unreadable",
            Self::PinsUnknown => "pins-unknown",
            Self::StudioUnknown => "studio-unknown",
        }
    }
}

/// What the footer and settings page show. Counts are exactly what the planner would do now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheStatus {
    /// Sealed bytes in this server's cache on this device.
    pub usage_bytes: u64,
    /// `None` for No limit or while the limit is not known (see `paused`).
    pub limit_bytes: Option<u64>,
    /// True when the user chose No limit.
    pub unlimited: bool,
    /// Bytes held by pins (Keep in progress, Downloads, own uploads, Studio, files in use).
    pub pinned_bytes: u64,
    pub leaving_soon_files: usize,
    pub leaving_soon_bytes: u64,
    /// Usage above the 90% warning threshold.
    pub nearly_full: bool,
    /// Usage above the limit.
    pub over_limit: bool,
    /// Over the limit after the plan, so only pins hold it there.
    pub held_by_pins: bool,
    pub paused: Option<CachePause>,
}

#[derive(Debug, Clone)]
struct InUse {
    file: Cid,
    until: u64,
    chunks: BTreeSet<Cid>,
}

/// Housekeeping state owned by one [`Server`]. Derived and process-local: nothing is persisted
/// except the limit, which the desktop bridge stores in its own sealed record.
#[derive(Debug, Default)]
pub struct CacheRuntime {
    limit: CacheLimit,
    in_use: VecDeque<InUse>,
    /// Monotonic time the last pass finished (or the last evaluation found nothing to do).
    last_pass: Option<u64>,
    last_evaluation: Option<u64>,
    /// A pass is under way: keep taking batches each turn until the plan has nothing to delete.
    in_pass: bool,
    /// Run a pass at the next turn regardless of spacing (the user lowered the limit, or Studio
    /// protection was just refreshed).
    forced: bool,
    last_status: Option<CacheStatus>,
    last_status_event: Option<u64>,
    /// A changed status arrived inside the coalescing window and still has to be emitted.
    status_held: bool,
    last_studio_request: Option<u64>,
}

/// What one housekeeping turn wants the actor to do next.
#[derive(Debug, Default)]
pub struct CachePump {
    /// The next monotonic instant housekeeping wants a turn, if any.
    pub next_due: Option<u64>,
    /// A changed status to emit (already coalesced).
    pub status: Option<CacheStatus>,
    /// Ask the desktop's Studio reference service for a scan.
    pub studio_scan: bool,
    /// Blobs deleted this turn, and their sealed bytes.
    pub deleted: usize,
    pub freed: u64,
}

/// The planner's inputs, owned so they can be built under `&self` and used under `&mut self`.
struct Inputs {
    index: catcoms_storage::CacheIndex,
    rows: Vec<PlanRow>,
    profile_refs: BTreeSet<Cid>,
    pins: BTreeSet<Cid>,
    retained: BTreeSet<Cid>,
    limit: Option<u64>,
    unclaimed_allowed: bool,
}

impl Inputs {
    fn plan(&self) -> CachePlan {
        cache_plan::plan(PlanInput {
            index: &self.index,
            rows: &self.rows,
            profile_refs: &self.profile_refs,
            pins: &self.pins,
            retained: &self.retained,
            limit: self.limit,
            unclaimed_allowed: self.unclaimed_allowed,
        })
    }
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Hand over the limit loaded or migrated by the desktop bridge, or set by the user. A lower
    /// limit forces a pass at the next turn.
    pub fn set_cache_limit(&mut self, limit: CacheLimit) {
        self.cache.limit = limit;
        self.cache.forced = true;
    }

    pub fn cache_limit(&self) -> CacheLimit {
        self.cache.limit
    }

    /// Studio protection may have changed (the desktop's reference service finished a scan):
    /// replan at the next turn.
    pub fn cache_protection_changed(&mut self) {
        self.cache.forced = true;
    }

    /// Record that `chunks` of `file` are being read now, pinning them for [`IN_USE_MS`].
    pub(crate) fn note_cache_use(&mut self, file: Cid, chunks: impl IntoIterator<Item = Cid>) {
        let now = self.runtime_clock().monotonic_ms();
        let until = now.saturating_add(IN_USE_MS);
        let runtime = &mut self.cache;
        runtime
            .in_use
            .retain(|entry| entry.file != file && entry.until > now);
        if runtime.in_use.len() >= MAX_IN_USE_FILES {
            runtime.in_use.pop_front();
        }
        runtime.in_use.push_back(InUse {
            file,
            until,
            chunks: chunks.into_iter().collect(),
        });
    }

    /// Chunks of this device's own uploads: rows whose attestation verifies under this device's
    /// key. Only verified rows count, so nobody can exempt a file from this device's limit by
    /// writing this device's short fingerprint into a row.
    fn own_upload_chunks<'a>(
        &self,
        rows: impl IntoIterator<Item = &'a crate::FileEntry>,
    ) -> BTreeSet<Cid> {
        let me = hex::encode(self.device_id.as_bytes());
        rows.into_iter()
            .filter(|entry| entry.author_verified && entry.author_identity == me)
            .filter_map(|entry| FileManifest::decode_or_legacy(&entry.file_ref).ok())
            .flat_map(|manifest| manifest.chunks.into_iter().map(|c| c.ciphertext_cid))
            .collect()
    }

    /// Ciphertext CIDs of every chunk this device's own uploads reference, for Storage health:
    /// a missing own-upload chunk is damage (this device is the original source); any other
    /// absent chunk is simply not cached.
    pub(crate) fn own_upload_chunk_set(&self, files: &[crate::FileEntry]) -> BTreeSet<Cid> {
        self.own_upload_chunks(files)
    }

    /// Every pin set, as chunk addresses from each owner's own saved plans.
    fn cache_pins(
        &self,
        rows: &[(usize, crate::FileEntry)],
        now: u64,
    ) -> Result<BTreeSet<Cid>, CachePause> {
        let mut pins = BTreeSet::new();
        // Keep transfers that are not completed: every chunk of every saved variant. A completed
        // Keep is not a pin; a checked one replaces the cache duplicate instead (class 0).
        let kept = self.sync.kept_files();
        if kept.error.is_some() {
            return Err(CachePause::PinsUnknown);
        }
        for transfer in &kept.transfers {
            // Completed: class 0 handles it. Deleting: being released, so it protects nothing
            // and its saved plans are no longer readable (that must not pause eviction).
            if matches!(
                transfer.phase,
                catcoms_storage::kept::KeepPhase::Completed
                    | catcoms_storage::kept::KeepPhase::Deleting
            ) {
                continue;
            }
            let plans = self
                .sync
                .kept_saved_plans(&transfer.plan.cid)
                .ok_or(CachePause::PinsUnknown)?;
            pins.extend(
                plans
                    .iter()
                    .flat_map(|plan| plan.chunks.iter().map(|(cid, _)| *cid)),
            );
        }
        // Downloads, running or paused: every chunk of every saved variant.
        let downloads = self.sync.downloads().map_err(|_| CachePause::PinsUnknown)?;
        for download in &downloads {
            if download.state.phase == catcoms_storage::kept::KeepPhase::Deleting {
                continue;
            }
            let plans = self
                .sync
                .download_saved_plans(download.id)
                .map_err(|_| CachePause::PinsUnknown)?;
            pins.extend(
                plans
                    .iter()
                    .flat_map(|plan| plan.chunks.iter().map(|(cid, _)| *cid)),
            );
        }
        // Files in use right now.
        for entry in self.cache.in_use.iter().filter(|entry| entry.until > now) {
            pins.extend(entry.chunks.iter().copied());
        }
        // This device's own uploads.
        pins.extend(self.own_upload_chunks(rows.iter().map(|(_, entry)| entry)));
        // Studio references and live job holds.
        let studio = self
            .sync
            .blob_protected_cids()
            .ok_or(CachePause::StudioUnknown)?;
        pins.extend(studio);
        Ok(pins)
    }

    fn cache_inputs(&self, now: u64) -> Result<Inputs, CachePause> {
        let index = self.sync.blob_cache_index().ok_or(CachePause::NotReady)?;
        if !index.complete() {
            return Err(CachePause::ScanIncomplete);
        }
        let limit = match self.cache.limit {
            CacheLimit::Unreadable => return Err(CachePause::SettingUnreadable),
            other => other.bytes().ok_or(CachePause::NotReady)?,
        };
        let group = self.sync.group_id();
        let encrypted = self
            .sync
            .doc(DocType::FileIndex, FILE_INDEX_DOC)
            .ok_or(CachePause::IndexUnreadable)?;
        let doc = encrypted.doc();
        let file_rows = read_file_rows(doc, &group).map_err(|_| CachePause::IndexUnreadable)?;
        let mut unclaimed_allowed = raw_file_index_row_count(doc) <= MAX_FILE_ENTRIES;
        let rows = file_rows
            .iter()
            .filter_map(|(position, entry)| {
                let manifest = FileManifest::decode_or_legacy(&entry.file_ref).ok()?;
                Some(PlanRow {
                    position: *position,
                    file: manifest.plaintext_cid,
                    chunks: manifest.chunks.iter().map(|c| c.ciphertext_cid).collect(),
                })
            })
            .collect();
        let profile_refs = match self.sync.doc(DocType::Profile, PROFILE_DOC) {
            Some(profiles) => read_profile_records(profiles.doc())
                .values()
                .flat_map(|record| {
                    [
                        crate::parse_avatar_cid(&record.avatar_cid),
                        crate::parse_avatar_cid(&record.banner_cid),
                    ]
                })
                .flatten()
                .collect(),
            None => {
                unclaimed_allowed = false;
                BTreeSet::new()
            }
        };
        let pins = self.cache_pins(&file_rows, now)?;
        Ok(Inputs {
            index,
            rows,
            profile_refs,
            pins,
            retained: self.sync.blob_retained_chunks(),
            limit,
            unclaimed_allowed,
        })
    }

    fn status_from(&self, result: &Result<CachePlan, CachePause>) -> CacheStatus {
        let usage = self.sync.blob_cache_usage().map(|u| u.bytes).unwrap_or(0);
        match result {
            Ok(plan) => {
                let limit = plan.limit;
                let after = plan.usage_after();
                CacheStatus {
                    usage_bytes: plan.usage,
                    limit_bytes: limit,
                    unlimited: limit.is_none(),
                    pinned_bytes: plan.pinned_bytes,
                    leaving_soon_files: plan.leaving_soon_files,
                    leaving_soon_bytes: plan.leaving_soon_bytes,
                    nearly_full: limit.is_some_and(|limit| {
                        (plan.usage as u128) * 100 > (limit as u128) * NEARLY_FULL_PERCENT as u128
                    }),
                    over_limit: limit.is_some_and(|limit| plan.usage > limit),
                    held_by_pins: plan.over_after && limit.is_some_and(|limit| after > limit),
                    paused: None,
                }
            }
            Err(pause) => CacheStatus {
                usage_bytes: usage,
                limit_bytes: self.cache.limit.bytes().flatten(),
                unlimited: self.cache.limit == CacheLimit::Unlimited,
                paused: Some(*pause),
                ..CacheStatus::default()
            },
        }
    }

    /// The current status, recomputed from fresh inputs (bounded: 256 rows, no decryption).
    pub fn cache_status(&self) -> CacheStatus {
        let now = self.runtime_clock().monotonic_ms();
        let result = self.cache_inputs(now).map(|inputs| inputs.plan());
        self.status_from(&result)
    }

    /// One housekeeping turn. Cheap when nothing is due: a usage comparison and two clock reads.
    pub fn cache_pump(&mut self) -> CachePump {
        let now = self.runtime_clock().monotonic_ms();
        let mut pump = CachePump::default();
        let over = match (self.sync.blob_cache_usage(), self.cache.limit) {
            (Some(usage), CacheLimit::Bytes(limit)) => usage.bytes > limit,
            _ => false,
        };
        let pass_spaced = self
            .cache
            .last_pass
            .is_none_or(|last| now >= last.saturating_add(PASS_INTERVAL_MS));
        let evaluation_due = self
            .cache
            .last_evaluation
            .is_none_or(|last| now >= last.saturating_add(EVALUATE_INTERVAL_MS));
        let due =
            self.cache.in_pass || self.cache.forced || evaluation_due || (over && pass_spaced);
        if due {
            self.cache.forced = false;
            self.cache.last_evaluation = Some(now);
            self.run_batch(now, &mut pump);
        } else if self.cache.status_held {
            let status = self.cache_status();
            self.offer_status(now, status, &mut pump);
        }
        // Next wake: keep batching, or come back for the periodic evaluation, or when spacing
        // allows a pass for an over-limit cache.
        let held = self.cache.status_held.then(|| {
            self.cache
                .last_status_event
                .map_or(now, |last| last.saturating_add(STATUS_INTERVAL_MS))
        });
        pump.next_due = Some(if self.cache.in_pass {
            now
        } else {
            let evaluation = self
                .cache
                .last_evaluation
                .map_or(now, |last| last.saturating_add(EVALUATE_INTERVAL_MS));
            let next = if over {
                let spaced = self
                    .cache
                    .last_pass
                    .map_or(now, |last| last.saturating_add(PASS_INTERVAL_MS));
                evaluation.min(spaced)
            } else {
                evaluation
            };
            held.map_or(next, |held| next.min(held))
        });
        pump
    }

    fn run_batch(&mut self, now: u64, pump: &mut CachePump) {
        let mut result = self.cache_inputs(now).map(|inputs| (inputs.plan(), inputs));
        if let Ok((plan, _)) = &result {
            let wants_delete = !plan.duplicates.is_empty() || !plan.evict.is_empty();
            if wants_delete && !self.cache.in_pass {
                // Other handles (Studio, the desktop) write the same directory: refresh sizes
                // before the first delete of a pass, then plan again from the fresh index.
                if self.sync.blob_cache_scan(false).is_err() {
                    result = Err(CachePause::ScanIncomplete);
                } else {
                    result = self.cache_inputs(now).map(|inputs| (inputs.plan(), inputs));
                }
            }
        }
        match &result {
            Err(CachePause::StudioUnknown) => {
                let ask = self
                    .cache
                    .last_studio_request
                    .is_none_or(|last| now >= last.saturating_add(STUDIO_REQUEST_INTERVAL_MS));
                if ask {
                    self.cache.last_studio_request = Some(now);
                    pump.studio_scan = true;
                }
                self.cache.in_pass = false;
                // A paused attempt counts as a pass for spacing: otherwise an over-limit cache
                // that cannot evict would replan on every actor turn.
                self.cache.last_pass = Some(now);
            }
            Err(_) => {
                self.cache.in_pass = false;
                self.cache.last_pass = Some(now);
            }
            Ok((plan, inputs)) => {
                let batch: Vec<Cid> = plan
                    .duplicates
                    .iter()
                    .chain(plan.evict.iter())
                    .take(MAX_DELETES_PER_TURN)
                    .copied()
                    .collect();
                for cid in &batch {
                    let bytes = inputs.index.sizes().get(cid).copied().unwrap_or(0);
                    // Anything else (retained by Studio's guard, already gone, or refused for
                    // unknown protection or a third-party handle) is replanned next batch.
                    if let Ok(true) = self.sync.delete_blob(cid) {
                        pump.deleted += 1;
                        pump.freed = pump.freed.saturating_add(bytes);
                    }
                }
                let remaining = plan.duplicates.len() + plan.evict.len() > batch.len();
                // Stop the pass when nothing is left or nothing could be deleted this turn, so a
                // refused delete cannot spin the actor.
                self.cache.in_pass = remaining && pump.deleted > 0;
                if !self.cache.in_pass {
                    self.cache.last_pass = Some(now);
                }
            }
        }
        let plan_result = match result {
            // Report the state after this batch, not before it.
            Ok(_) if pump.deleted > 0 => self.cache_inputs(now).map(|inputs| inputs.plan()),
            Ok((plan, _)) => Ok(plan),
            Err(pause) => Err(pause),
        };
        let status = self.status_from(&plan_result);
        self.offer_status(now, status, pump);
    }

    /// Emit `status` if it changed, at most once per [`STATUS_INTERVAL_MS`]. A change inside the
    /// window is held and re-offered by [`cache_pump`](Self::cache_pump) once the window passes,
    /// so the footer is never left showing an older state.
    fn offer_status(&mut self, now: u64, status: CacheStatus, pump: &mut CachePump) {
        if self.cache.last_status.as_ref() == Some(&status) {
            self.cache.status_held = false;
            return;
        }
        let spaced = self
            .cache
            .last_status_event
            .is_none_or(|last| now >= last.saturating_add(STATUS_INTERVAL_MS));
        if spaced {
            self.cache.last_status = Some(status.clone());
            self.cache.last_status_event = Some(now);
            self.cache.status_held = false;
            pump.status = Some(status);
        } else {
            self.cache.status_held = true;
        }
    }
}

#[cfg(test)]
mod tests;
