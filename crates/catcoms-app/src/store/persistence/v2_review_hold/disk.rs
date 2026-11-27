//! A hold uses a separate immutable record name and an exact replay stage.
//! Cold readers preserve uncertain bytes and never infer absence from them.
use super::*;
use crate::store::critical_namespaces::read_pool::{
    CatalogReadReservation, CriticalCatalogReadPool,
};
use crate::store::persistence::v2_member_outbox::fs::{
    bounded_entries, bounded_file_length, finish_exact_write, observe, open_exact_write,
    read_bounded_held, require_exact_held, require_same_name, HeldDirectory, Presence, WriteMode,
    MAX_FILE,
};
use catcoms_storage::kept::SessionId;
use catcoms_storage::VaultSessionGuard;
use std::{collections::BTreeSet, fs::File, path::Path};

const MAX_CASES: usize = 256;
const MAX_NAMES: usize = 2 * MAX_CASES;
const HOLD_SUFFIX: &str = ".held";
const STAGE_SUFFIX: &str = ".stage";
const _: () = assert!(MAX_FRAME <= MAX_FILE);

// Includes authenticated payload copies, signature verification and the
// bounded directory inventory, independently of the ingress frame charge.
pub(in crate::store::persistence) const WORK: usize =
    8 * MAX_FRAME + (MAX_NAMES + 1) * 1024 + 32 * 1024;

/// One exclusive disk owner from an authenticated ID10 mount. The scope
/// comes from its original selected host, never a renderer or network label.
pub(in crate::store::persistence) struct HoldDisk {
    directory: HeldDirectory,
    scope: HoldScope,
    pool: CriticalCatalogReadPool,
    key_binding: [u8; 32],
}

pub(in crate::store::persistence) struct HoldCandidate<'a> {
    pub(in crate::store::persistence) verifier: &'a V2GeneralPrefixVerifier,
    pub(in crate::store::persistence) evidence: &'a V2RemovedCreateEvidence,
    pub(in crate::store::persistence) original: &'a [u8],
}

impl HoldDisk {
    pub(in crate::store::persistence) fn from_held(
        directory: HeldDirectory,
        scope: SharedHistoryScope,
        key: &[u8; 32],
        pool: CriticalCatalogReadPool,
    ) -> Result<Self, AppError> {
        directory.require()?;
        let scope = HoldScope::new(scope);
        let key_binding = Self::key_binding(&scope, key);
        Ok(Self {
            directory,
            scope,
            pool,
            key_binding,
        })
    }

    fn key_binding(scope: &HoldScope, key: &[u8; 32]) -> [u8; 32] {
        blake3::derive_key(
            "catcoms/private-chat-v2-removed-create-disk/v1",
            &*scope.key(key),
        )
    }

    fn require(&self, key: &[u8; 32], fee: &CatalogReadReservation) -> Result<(), AppError> {
        fee.validate(&self.pool, WORK)?;
        if self.key_binding != Self::key_binding(&self.scope, key) {
            return Err(refused());
        }
        self.directory.require()
    }

    /// A caller checks current issuer/physical ownership and independently
    /// known history at every write phase. A failed known-history read is
    /// uncertainty and must refuse here. An already selected ordinary
    /// original cannot turn into an unproven case.
    pub(in crate::store::persistence) fn retain(
        &mut self,
        key: &[u8; 32],
        candidate: HoldCandidate<'_>,
        (session, fee): (&VaultSessionGuard, CatalogReadReservation),
        mut check_unproven: impl FnMut(HeldCase) -> Result<(), AppError>,
        rederive: impl FnOnce(&[u8]) -> Result<V2RemovedCreateEvidence, AppError>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<HeldCase, AppError> {
        let _lease = session.retain_for_physical_work();
        self.require(key, &fee)?;
        let HoldCandidate {
            verifier,
            evidence,
            original,
        } = candidate;
        verifier
            .require_current_removed_create(evidence)
            .map_err(|_| refused())?;
        evidence
            .require_original_data(original)
            .map_err(|_| refused())?;
        let case = evidence.case_data();
        let label = hex::encode(self.scope.file_label(key, case));
        let held = self.directory.child(&format!("{label}{HOLD_SUFFIX}"));
        let stage = self.directory.child(&format!("{label}{STAGE_SUFFIX}"));

        let mut require_unknown = |id| {
            verifier
                .require_current_removed_create(evidence)
                .map_err(|_| refused())?;
            check_unproven(id)
        };
        // A valid selected original remains authoritative for this case if
        // the staging file is separately damaged. For an incomplete final,
        // keep its native identity until verified Stage supplies the suffix.
        let destination = self.read_frame(&held)?;
        if let Some(saved) = destination.as_ref() {
            if let Ok(data) = self.scope.open(key, &saved.bytes) {
                if data.identity.id != case || data.identity.removal != evidence.removal_data() {
                    return Err(refused());
                }
                reprove(&data, verifier, rederive)?;
                require_unknown(data.identity)?;
                self.finish_selected(&held, destination.as_ref(), &saved.bytes)?;
                require_unknown(data.identity)?;
                return Ok(data.identity);
            }
        }
        if let Some(saved) = self.read_named(key, &stage, case)? {
            if saved.data.identity.removal != evidence.removal_data() {
                return Err(refused());
            }
            reprove(&saved.data, verifier, rederive)?;
            let (file, native) = open_exact_write(&stage, WriteMode::Existing)?;
            if native != saved.frame.native {
                return Err(refused());
            }
            let mut staged = (file, native);
            require_exact_held(&stage, &mut staged, &saved.frame.bytes)?;
            require_unknown(saved.data.identity)?;
            finish_exact_write(&stage, &mut staged, &saved.frame.bytes)?;
            self.directory.sync()?;
            return self.cover_selected(
                (&held, destination.as_ref()),
                (&stage, staged),
                saved.data.identity,
                &saved.frame.bytes,
                require_unknown,
            );
        }
        // A partial final without its authenticated source is ambiguous.
        if destination.is_some() {
            return Err(refused());
        }

        self.require_room()?;
        let (id, frame) = self.scope.prepare(key, verifier, evidence, original, rng)?;
        require_unknown(id)?;
        self.directory.require()?;
        let mut staged = open_exact_write(&stage, WriteMode::Create)?;
        finish_exact_write(&stage, &mut staged, &frame)?;
        self.directory.sync()?;
        self.cover_selected((&held, None), (&stage, staged), id, &frame, require_unknown)
    }

    fn cover_selected(
        &self,
        (held, original): (&Path, Option<&StoredFrame>),
        (stage, mut staged): (&Path, (File, SessionId)),
        identity: HeldCase,
        frame: &[u8],
        mut check_unproven: impl FnMut(HeldCase) -> Result<(), AppError>,
    ) -> Result<HeldCase, AppError> {
        self.directory.require()?;
        require_exact_held(stage, &mut staged, frame)?;
        check_unproven(identity)?;
        // Checking independent history can block on its file work. Rebind
        // the exact Stage before letting those bytes select the final name.
        self.directory.require()?;
        require_exact_held(stage, &mut staged, frame)?;
        self.finish_selected(held, original, frame)?;
        // Keep the authenticated stage as part of the two-file case. It
        // can complete this exact selected copy after an interrupted write.
        require_exact_held(stage, &mut staged, frame)?;
        check_unproven(identity)?;
        self.directory.require()?;
        Ok(identity)
    }

    fn finish_selected(
        &self,
        path: &Path,
        original: Option<&StoredFrame>,
        frame: &[u8],
    ) -> Result<(), AppError> {
        self.directory.require()?;
        let mut held = if let Some(original) = original {
            let mut held = open_exact_write(path, WriteMode::Existing)?;
            if held.1 != original.native || !frame.starts_with(&original.bytes) {
                return Err(refused());
            }
            require_same_name(path, &original.file, original.native)?;
            require_exact_held(path, &mut held, &original.bytes)?;
            held
        } else {
            // A name that appears after observation must refuse, even if
            // its bytes are an exact prefix. Only our own file may advance.
            open_exact_write(path, WriteMode::Create)?
        };
        finish_exact_write(path, &mut held, frame)?;
        self.directory.sync()?;
        require_exact_held(path, &mut held, frame)?;
        self.directory.require()
    }

    fn read_frame(&self, path: &Path) -> Result<Option<StoredFrame>, AppError> {
        self.directory.require()?;
        if observe(path)? == Presence::Absent {
            return Ok(None);
        }
        let (file, native, bytes) = read_bounded_held(path, MAX_FRAME, true)?;
        self.directory.require()?;
        Ok(Some(StoredFrame {
            file,
            native,
            bytes,
        }))
    }

    fn read_named(
        &self,
        key: &[u8; 32],
        path: &Path,
        expected: ReviewCaseId,
    ) -> Result<Option<NamedHold>, AppError> {
        let Some(frame) = self.read_frame(path)? else {
            return Ok(None);
        };
        let data = self.scope.open(key, &frame.bytes)?;
        if data.identity.id != expected {
            return Err(refused());
        }
        Ok(Some(NamedHold { frame, data }))
    }

    fn require_room(&self) -> Result<(), AppError> {
        let names = bounded_entries(&self.directory, MAX_NAMES)?;
        if names.len() > MAX_NAMES {
            return Err(refused());
        }
        let mut cases = BTreeSet::new();
        for name in &names {
            let name = name.to_str().ok_or_else(refused)?;
            let label = name
                .strip_suffix(HOLD_SUFFIX)
                .or_else(|| name.strip_suffix(STAGE_SUFFIX))
                .filter(|label| {
                    label.len() == 64
                        && label
                            .bytes()
                            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                })
                .ok_or_else(refused)?;
            bounded_file_length(&self.directory.child(name), MAX_FRAME)?;
            cases.insert(label);
        }
        if cases.len() >= MAX_CASES {
            return Err(refused());
        }
        self.directory.require()
    }
}

struct StoredFrame {
    file: File,
    native: SessionId,
    bytes: Zeroizing<Vec<u8>>,
}

struct NamedHold {
    frame: StoredFrame,
    data: OpenedHold,
}

fn reprove(
    data: &OpenedHold,
    verifier: &V2GeneralPrefixVerifier,
    rederive: impl FnOnce(&[u8]) -> Result<V2RemovedCreateEvidence, AppError>,
) -> Result<(), AppError> {
    let record = V2RemovedCreateRecord::decode(&data.plain).map_err(|_| refused())?;
    let evidence = rederive(record.original_bytes_data())?;
    verifier
        .require_current_removed_create(&evidence)
        .map_err(|_| refused())?;
    record.require_evidence(&evidence).map_err(|_| refused())
}

#[cfg(test)]
mod tests;
