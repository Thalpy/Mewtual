//! Closed retained-transfer journal. `current` is authoritative; `next` is its exact successor,
//! never a backup. An invalid newer debit/deletion must not resurrect an older execution grant.
use super::*;
use catcoms_crypto::DeviceId;

pub(super) const RECORD_LIMIT: usize = 80 * 1024;
/// V4 deliberately has a separate reservation. One plan is at most 71,249 encoded bytes:
/// 64 KiB manifest, 128 (36-byte CID + 8-byte bound) entries and 81 fixed bytes. The
/// working mirror plus four candidates, five bitmaps and all journal fields fit below
/// 360 KiB including the 40-byte disk seal. Old formats retain their 80 KiB slot charge.
pub(super) const VARIANT_RECORD_LIMIT: usize = 400 * 1024;
const RECORD_V4: &[u8] = b"mewtual/kept-transfer/v4";
#[path = "variants.rs"]
mod variants;
use variants::Variants;
pub(super) use variants::VARIANT_CHUNK_LIMIT;
const RECORD_V2: &[u8] = b"mewtual/kept-transfer/v2";
pub(super) const RECORD_DOMAIN: &[u8] = b"mewtual/kept-transfer/v3";

/// Preserve the original canonical bytes when reading an older journal: predecessor hashes
/// cover that exact encoding. Only a successor may upgrade it; normalization on read would
/// break crash reconciliation of an old `current` and its already-written new `next`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RecordEncoding {
    V2,
    V3,
    V4,
}
/// A restart spends the complete already-issued slice. Process-monotonic timestamps never go
/// on disk and cannot turn into a fresh allowance when a different process mounts the vault.
pub const KEEP_EXECUTION_SLICE_MS: u64 = 15_000;
const REQUESTS: u32 = 131_072;
const SLICES: u16 = 120;

/// Actual MLS namespace and complete local device identity. A UI server number, fingerprint or
/// filesystem alias cannot authorize a retained transfer or supply a second quota namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepScope {
    pub group: Vec<u8>,
    pub device: DeviceId,
}
impl KeepScope {
    pub(super) fn validate(&self) -> Result<(), StorageError> {
        if self.group.is_empty() || self.group.len() > 256 {
            return Err(StorageError::Malformed);
        }
        Ok(())
    }
}

/// Durable ownership, distinct from this mount's `checked` verdict and temporary run lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepPhase {
    Resumable,
    Completed,
    Deleting,
}

/// Remaining native-owned allowance, not a caller-supplied timeout or a new grant on restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeepAllowance {
    pub requests: u32,
    pub pages: u32,
    pub bytes: u64,
    pub slices: u16,
}
impl KeepAllowance {
    pub(super) fn initial(plan: &KeepPlan) -> Self {
        Self {
            requests: REQUESTS,
            pages: REQUESTS,
            // Fixed search overhead permits bounded misses even for a very small file. Every
            // issued byte is debited before use; unused prepaid batch bytes are lost on crash.
            bytes: plan.chunks.iter().map(|(_, bound)| bound).sum::<u64>() * 3 + 16 * 1024 * 1024,
            slices: SLICES,
        }
    }
    pub(super) fn subtract(&mut self, grant: Self) -> Result<(), StorageError> {
        let remaining = Self {
            requests: self
                .requests
                .checked_sub(grant.requests)
                .ok_or(StorageError::KeepExhausted)?,
            pages: self
                .pages
                .checked_sub(grant.pages)
                .ok_or(StorageError::KeepExhausted)?,
            bytes: self
                .bytes
                .checked_sub(grant.bytes)
                .ok_or(StorageError::KeepExhausted)?,
            slices: self
                .slices
                .checked_sub(grant.slices)
                .ok_or(StorageError::KeepExhausted)?,
        };
        *self = remaining;
        Ok(())
    }
    fn at_most(self, other: Self) -> bool {
        self.requests <= other.requests
            && self.pages <= other.pages
            && self.bytes <= other.bytes
            && self.slices <= other.slices
    }
}

/// One explicit Retry compare-and-swap. Only exhaustion permits a fresh native-owned grant;
/// repeating the exact request recovers its receipt without replenishing consumed allowance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeepRetry {
    pub transfer: [u8; 32],
    pub expected_grant: u64,
    pub request: [u8; 16],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeepRetryReceipt {
    pub request: KeepRetry,
    pub granted: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Record {
    pub encoding: RecordEncoding,
    pub user_paused: bool,
    /// A deterministic manifest/content failure is retried only after explicit user action.
    pub blocked: bool,
    /// Changes only with explicit Pause/Resume, independent of continuous progress/debits.
    pub intent: u64,
    pub scope: KeepScope,
    pub transfer: [u8; 32],
    pub revision: u64,
    pub predecessor: [u8; 32],
    pub plan: KeepPlan,
    /// Original replicated manifest-set identity, separate from this exact encrypted variant.
    /// An unlisted saved copy retains local repair authority, not shared publication authority.
    pub listing: [u8; 32],
    pub phase: KeepPhase,
    pub verified: Vec<bool>,
    pub remaining: KeepAllowance,
    pub exhausted: bool,
    pub grant: u64,
    pub last_retry: Option<KeepRetryReceipt>,
    /// V2/V3 never infer alternatives from today's index. V4 owns its complete saved set.
    pub variants: Option<Variants>,
}
impl Record {
    pub(super) fn initial(
        scope: KeepScope,
        plan: KeepPlan,
        listing: [u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, StorageError> {
        scope.validate()?;
        plan.charge()?;
        let mut transfer = [0; 32];
        rng.fill_bytes(&mut transfer);
        Ok(Self {
            encoding: RecordEncoding::V3,
            user_paused: false,
            blocked: false,
            intent: 1,
            scope,
            transfer,
            revision: 1,
            predecessor: [0; 32],
            verified: vec![false; plan.chunks.len()],
            remaining: KeepAllowance::initial(&plan),
            plan,
            listing,
            phase: KeepPhase::Resumable,
            exhausted: false,
            grant: 1,
            last_retry: None,
            variants: None,
        })
    }
    pub(super) fn successor(&self) -> Result<Self, StorageError> {
        let mut next = self.clone();
        next.revision = self
            .revision
            .checked_add(1)
            .ok_or(StorageError::Malformed)?;
        next.predecessor = self.digest();
        if self.encoding != RecordEncoding::V4 {
            next.encoding = RecordEncoding::V3;
        }
        if self.encoding == RecordEncoding::V2 {
            next.intent = 1;
        }
        Ok(next)
    }
    pub(super) fn digest(&self) -> [u8; 32] {
        *blake3::hash(&self.encode()).as_bytes()
    }
    pub(super) fn validate_initial(&self) -> Result<(), StorageError> {
        self.validate()?;
        if self.revision != 1 || !self.initial_fields_valid() {
            return Err(StorageError::Malformed);
        }
        Ok(())
    }
    fn initial_fields_valid(&self) -> bool {
        self.predecessor == [0; 32]
            && match self.encoding {
                RecordEncoding::V2 => self.user_paused && self.intent == 0,
                RecordEncoding::V3 | RecordEncoding::V4 => !self.user_paused && self.intent == 1,
            }
            && !self.blocked
            && self.phase == KeepPhase::Resumable
            && self.grant == 1
            && !self.exhausted
            && !self.verified.iter().any(|bit| *bit)
            && self.remaining == self.initial_allowance()
            && self
                .variants
                .as_ref()
                .is_none_or(Variants::initial_fields_valid)
    }
    pub(super) fn validate(&self) -> Result<(), StorageError> {
        self.scope.validate()?;
        self.plan.charge()?;
        self.validate_variants()?;
        if self.revision == 0
            || match self.encoding {
                RecordEncoding::V2 => !self.user_paused || self.intent != 0 || self.blocked,
                RecordEncoding::V3 | RecordEncoding::V4 => {
                    self.intent == 0 || self.intent > self.revision
                }
            }
            || self.grant == 0
            || self.grant > self.revision
            || (self.revision == 1 && !self.initial_fields_valid())
            || self.verified.len() != self.plan.chunks.len()
            || !self.remaining.at_most(self.initial_allowance())
            || (self.encoding != RecordEncoding::V4
                && self.phase == KeepPhase::Completed
                && self.verified.iter().any(|bit| !*bit))
            || self.last_retry.is_some_and(|receipt| {
                receipt.request.transfer != self.transfer
                    || receipt.request.expected_grant.checked_add(1) != Some(receipt.granted)
                    || receipt.granted != self.grant
            })
            || (self.grant == 1) != self.last_retry.is_none()
        {
            return Err(StorageError::Malformed);
        }
        Ok(())
    }
    /// Recovery validates the transition as well as both independently authenticated records.
    /// Ordinary writes cannot refill allowance, change manifests, clear verified progress or
    /// leave Deleting. Retry is the sole refill edge and must carry the exact previous grant.
    pub(super) fn validate_successor(&self, next: &Self) -> Result<(), StorageError> {
        self.validate()?;
        next.validate()?;
        if next.revision
            != self
                .revision
                .checked_add(1)
                .ok_or(StorageError::Malformed)?
            || next.predecessor != self.digest()
            || (self.encoding != RecordEncoding::V2 && next.encoding != self.encoding)
            || (self.encoding == RecordEncoding::V2 && next.encoding == RecordEncoding::V4)
            || if self.encoding == RecordEncoding::V2 && next.encoding == RecordEncoding::V2 {
                // A process may upgrade while the old writer's exact successor is still in
                // `next`. Recover that old-format debit/progress/deletion before migrating;
                // neither legacy record had an explicit intent counter or auto-resume consent.
                next.intent != self.intent
            } else if next.user_paused != self.user_paused
                || (self.blocked && !next.blocked)
                || self.encoding == RecordEncoding::V2
            {
                next.intent != self.intent.checked_add(1).ok_or(StorageError::Malformed)?
            } else {
                next.intent != self.intent && Some(next.intent) != self.intent.checked_add(1)
            }
            || next.scope != self.scope
            || next.transfer != self.transfer
            || (self.variants.is_none() && next.plan != self.plan)
            || next.listing != self.listing
            || (self.variants.is_none()
                && self
                    .verified
                    .iter()
                    .zip(&next.verified)
                    .any(|(old, new)| *old && !*new))
            || (self.phase == KeepPhase::Deleting && next.phase != KeepPhase::Deleting)
            || (self.phase == KeepPhase::Completed && next.phase == KeepPhase::Resumable)
        {
            return Err(StorageError::Malformed);
        }
        self.validate_variant_successor(next)?;
        if next.grant == self.grant {
            if !next.remaining.at_most(self.remaining)
                || next.last_retry != self.last_retry
                || (self.exhausted && !next.exhausted)
            {
                return Err(StorageError::Malformed);
            }
        } else if !self.exhausted
            || self.phase == KeepPhase::Deleting
            || next.phase != self.phase
            || next.grant != self.grant.checked_add(1).ok_or(StorageError::Malformed)?
            || next.remaining != self.initial_allowance()
            || next.exhausted
            || next.verified != self.verified
            || next.variants != self.variants
            || next
                .last_retry
                .is_none_or(|receipt| receipt.request.expected_grant != self.grant)
        {
            return Err(StorageError::Malformed);
        }
        Ok(())
    }
    pub(super) fn retry(
        &self,
        request: KeepRetry,
    ) -> Result<(Self, KeepRetryReceipt), StorageError> {
        // This lookup precedes today's phase/exhaustion so a lost reply remains recoverable
        // after the issued grant has been used. It supplies metadata, never a new run lease.
        if let Some(receipt) = self.last_retry.filter(|receipt| receipt.request == request) {
            return Ok((self.clone(), receipt));
        }
        if request.transfer != self.transfer
            || request.expected_grant != self.grant
            || self.phase == KeepPhase::Deleting
            || !self.exhausted
        {
            return Err(StorageError::Malformed);
        }
        let mut next = self.successor()?;
        next.grant = self.grant.checked_add(1).ok_or(StorageError::Malformed)?;
        let receipt = KeepRetryReceipt {
            request,
            granted: next.grant,
        };
        next.last_retry = Some(receipt);
        next.remaining = next.initial_allowance();
        next.exhausted = false;
        // Retry changes only the execution allowance. It cannot undo explicit Pause; even
        // an exact lost-reply replay remains a metadata lookup with no intent side effect.
        self.validate_successor(&next)?;
        Ok((next, receipt))
    }
    pub(super) fn charge(&self) -> Result<u64, StorageError> {
        self.plan.charge()?;
        Ok(self
            .plans()
            .flat_map(|plan| &plan.chunks)
            .map(|(_, n)| n + 40)
            .sum::<u64>()
            + 2 * self.slot_limit() as u64)
    }
    pub(super) fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_bytes(match self.encoding {
            RecordEncoding::V2 => RECORD_V2,
            RecordEncoding::V3 => RECORD_DOMAIN,
            RecordEncoding::V4 => RECORD_V4,
        })
        .expect("fixed domain");
        e.put_bytes(&self.scope.group).expect("bounded group");
        e.put_bytes(self.scope.device.as_bytes())
            .expect("fixed device");
        e.put_bytes(&self.transfer).expect("fixed transfer");
        e.put_u64(self.revision);
        e.put_bytes(&self.predecessor).expect("fixed digest");
        e.put_bytes(&self.plan.encode()).expect("bounded plan");
        e.put_bytes(&self.listing).expect("fixed listing");
        e.put_u8(match self.phase {
            KeepPhase::Resumable => 1,
            KeepPhase::Completed => 2,
            KeepPhase::Deleting => 3,
        });
        e.put_bytes(
            &self
                .verified
                .iter()
                .map(|bit| u8::from(*bit))
                .collect::<Vec<_>>(),
        )
        .expect("bounded bitmap");
        e.put_u32(self.remaining.requests);
        e.put_u32(self.remaining.pages);
        e.put_u64(self.remaining.bytes);
        e.put_u16(self.remaining.slices);
        e.put_u8(u8::from(self.exhausted));
        e.put_u64(self.grant);
        e.put_u8(u8::from(self.last_retry.is_some()));
        if let Some(receipt) = self.last_retry {
            e.put_bytes(&receipt.request.request)
                .expect("fixed request");
            e.put_u64(receipt.request.expected_grant);
            e.put_u64(receipt.granted);
        }
        if self.encoding != RecordEncoding::V2 {
            e.put_u8(u8::from(self.user_paused));
            e.put_u64(self.intent);
            e.put_u8(u8::from(self.blocked));
        }
        if let Some(variants) = &self.variants {
            variants.encode(&mut e);
        }
        e.finish()
    }
    pub(super) fn decode(bytes: &[u8]) -> Result<Self, StorageError> {
        let bad = || StorageError::Malformed;
        if bytes.len() + 40 > VARIANT_RECORD_LIMIT {
            return Err(StorageError::BlobSizeLimit);
        }
        let mut d = Decoder::new(bytes);
        let encoding = match d.get_bytes().map_err(|_| bad())? {
            RECORD_V2 => RecordEncoding::V2,
            RECORD_DOMAIN => RecordEncoding::V3,
            RECORD_V4 => RecordEncoding::V4,
            _ => return Err(bad()),
        };
        if bytes.len() + 40
            > if encoding == RecordEncoding::V4 {
                VARIANT_RECORD_LIMIT
            } else {
                RECORD_LIMIT
            }
        {
            return Err(StorageError::BlobSizeLimit);
        }
        let group = d.get_bytes().map_err(|_| bad())?;
        if group.is_empty() || group.len() > 256 {
            return Err(bad());
        }
        let group = group.to_vec();
        let device = DeviceId::from_bytes(
            d.get_bytes()
                .map_err(|_| bad())?
                .try_into()
                .map_err(|_| bad())?,
        );
        let transfer = d
            .get_bytes()
            .map_err(|_| bad())?
            .try_into()
            .map_err(|_| bad())?;
        let revision = d.get_u64().map_err(|_| bad())?;
        let predecessor = d
            .get_bytes()
            .map_err(|_| bad())?
            .try_into()
            .map_err(|_| bad())?;
        let plan = KeepPlan::decode(d.get_bytes().map_err(|_| bad())?)?;
        let listing = d
            .get_bytes()
            .map_err(|_| bad())?
            .try_into()
            .map_err(|_| bad())?;
        let phase = match d.get_u8().map_err(|_| bad())? {
            1 => KeepPhase::Resumable,
            2 => KeepPhase::Completed,
            3 => KeepPhase::Deleting,
            _ => return Err(bad()),
        };
        let bitmap = d.get_bytes().map_err(|_| bad())?;
        if bitmap.len() != plan.chunks.len() || bitmap.iter().any(|bit| *bit > 1) {
            return Err(bad());
        }
        let verified = bitmap.iter().map(|bit| *bit == 1).collect();
        let remaining = KeepAllowance {
            requests: d.get_u32().map_err(|_| bad())?,
            pages: d.get_u32().map_err(|_| bad())?,
            bytes: d.get_u64().map_err(|_| bad())?,
            slices: d.get_u16().map_err(|_| bad())?,
        };
        let exhausted = match d.get_u8().map_err(|_| bad())? {
            0 => false,
            1 => true,
            _ => return Err(bad()),
        };
        let grant = d.get_u64().map_err(|_| bad())?;
        let last_retry = match d.get_u8().map_err(|_| bad())? {
            0 => None,
            1 => Some(KeepRetryReceipt {
                request: KeepRetry {
                    transfer,
                    request: d
                        .get_bytes()
                        .map_err(|_| bad())?
                        .try_into()
                        .map_err(|_| bad())?,
                    expected_grant: d.get_u64().map_err(|_| bad())?,
                },
                granted: d.get_u64().map_err(|_| bad())?,
            }),
            _ => return Err(bad()),
        };
        let user_paused = if encoding != RecordEncoding::V2 {
            match d.get_u8().map_err(|_| bad())? {
                0 => false,
                1 => true,
                _ => return Err(bad()),
            }
        } else {
            // The former manual-only format cannot distinguish a user Pause from a runtime
            // stop. Require one explicit Resume before enabling background execution.
            true
        };
        let intent = if encoding != RecordEncoding::V2 {
            d.get_u64().map_err(|_| bad())?
        } else {
            0
        };
        let blocked = if encoding != RecordEncoding::V2 {
            match d.get_u8().map_err(|_| bad())? {
                0 => false,
                1 => true,
                _ => return Err(bad()),
            }
        } else {
            false
        };
        let variants = if encoding == RecordEncoding::V4 {
            Some(Variants::decode(&mut d)?)
        } else {
            None
        };
        d.finish().map_err(|_| bad())?;
        let record = Self {
            encoding,
            blocked,
            user_paused,
            intent,
            scope: KeepScope { group, device },
            transfer,
            revision,
            predecessor,
            plan,
            listing,
            phase,
            verified,
            remaining,
            exhausted,
            grant,
            last_retry,
            variants,
        };
        record.validate()?;
        Ok(record)
    }
}
