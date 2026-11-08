//! Separately named negative CURRENT text policy. It shares the original counted
//! decoder/lease fences and never enables writes, Source, membership or transport.
use super::*;
pub(super) mod shape;

#[derive(Clone, Copy)]
pub(super) enum Family {
    Initial,
    TextCurrent,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextCurrentRestoreLimits {
    pub max_typed_bytes: usize,
    pub max_work: usize,
}
#[must_use]
pub struct TextCurrentRestoreGuard {
    original: OwnerCheckpointRestoreGuard,
}
impl TextCurrentRestoreGuard {
    pub fn complete(self) -> Result<(), MemoryStorageError> {
        self.original.complete()
    }
}
impl Account {
    pub(super) fn check_shape(
        &mut self,
        row: Row,
        bytes: &[u8],
        group: &[u8; 16],
        signer: &[u8; 32],
        same: bool,
    ) -> Result<(), Error> {
        if let Some(mut facts) = self.text_facts {
            // Fixed array copies/comparison and ring arithmetic are real work too.
            self.work(4096)?;
            shape::check(row, bytes, group, signer, &mut facts, self)?;
            if same && Some(facts) != self.text_facts {
                return Err(Error);
            }
            self.text_facts = Some(facts);
        } else {
            let mut facts = self.facts;
            super::shape::check(row, bytes, group, signer, &mut facts, self)?;
            if same && facts != self.facts {
                return Err(Error);
            }
            self.facts = facts;
        }
        Ok(())
    }
    pub(super) fn finish_shape(&mut self) -> Result<(), Error> {
        if let Some(facts) = self.text_facts {
            self.work(4096)?;
            facts.finish()
        } else {
            self.facts.finish()
        }
    }
}
impl MemoryStorage {
    /// A distinct CLOSED exporter-text policy; caller-selected limits are negative
    /// budgets only. All current ratchet slots MUST be null, including handshake.
    pub fn begin_text_current_restore(
        &self,
        group: &[u8],
        signer: &[u8],
        limits: TextCurrentRestoreLimits,
    ) -> Result<TextCurrentRestoreGuard, MemoryStorageError> {
        self.begin_closed_restore(
            group,
            signer,
            OwnerCheckpointRestoreLimits {
                max_typed_bytes: limits.max_typed_bytes,
                max_work: limits.max_work,
            },
            Family::TextCurrent,
        )
        .map(|original| TextCurrentRestoreGuard { original })
    }
    pub(super) fn begin_closed_restore(
        &self,
        group: &[u8],
        signer: &[u8],
        limits: OwnerCheckpointRestoreLimits,
        family: Family,
    ) -> Result<OwnerCheckpointRestoreGuard, MemoryStorageError> {
        let group: [u8; 16] = group.try_into().map_err(|_| Error)?;
        let signer: [u8; 32] = signer.try_into().map_err(|_| Error)?;
        let mut account = Account::new(limits)?;
        if matches!(family, Family::TextCurrent) {
            account.text_facts = Some(shape::Facts::default());
        }
        account.bytes(
            std::mem::size_of::<Attempt>()
                .checked_add(2 * std::mem::size_of::<usize>())
                .and_then(|n| n.checked_add(SDK_FIXED_REQUESTS))
                .ok_or(Error)?,
        )?;
        // Explicit fixed setup-work ceiling: sixteen512-byte slot passes per key cover
        // zeroing, decimal emission, bounded key/header work and fixed custody moves through
        // Result/array/Attempt/Arc. Independent of input length and charged BEFORE that work.
        account.work(ROWS.len().checked_mul(512 * 16).ok_or(Error)?)?;
        // Fallible fixed-key construction has no heap. No SDK calls occur before installation.
        let mut built = ROWS.map(|row| Key::new(row, &group, &signer));
        let keys = [
            built[0].take_key()?,
            built[1].take_key()?,
            built[2].take_key()?,
            built[3].take_key()?,
            built[4].take_key()?,
            built[5].take_key()?,
            built[6].take_key()?,
            built[7].take_key()?,
            built[8].take_key()?,
            built[9].take_key()?,
            built[10].take_key()?,
            built[11].take_key()?,
            built[12].take_key()?,
        ];
        let attempt = Arc::new(Attempt {
            phase: AtomicU8::new(ACTIVE),
            account: Mutex::new(account),
            keys,
            group,
            signer,
        });
        let values = self.values.read().map_err(|_| Error)?;
        {
            let mut original = self.owner_restore.lock().map_err(|_| Error)?;
            if original.is_some() {
                return Err(Error.into());
            }
            *original = Some(attempt.clone());
        }
        let guard = OwnerCheckpointRestoreGuard {
            attempt: attempt.clone(),
            complete: false,
        };
        // Unsupported feature graphs still close THIS installed attempt. Returning before
        // installation would leave a legacy None fallback on the same caller-owned provider.
        if cfg!(any(
            feature = "extensions-draft",
            feature = "virtual-clients-draft",
            feature = "draft-ietf-mls-pq-ciphersuites"
        )) {
            return Err(Error.into());
        }
        {
            let mut account = attempt.account.lock().map_err(|_| Error)?;
            for row in ROWS {
                let key = attempt.keys[row as usize].bytes();
                account.lookup(values.allocation_size(), key.len())?;
                let bytes = match values.get(key) {
                    Some(bytes) => bytes.as_slice(),
                    None if row.empty_list() => b"[]",
                    None => return Err(Error.into()),
                };
                account.check_shape(row, bytes, &attempt.group, &attempt.signer, false)?;
            }
            account.finish_shape()?;
        }
        Ok(guard)
    }
}

#[cfg(test)]
mod tests;
