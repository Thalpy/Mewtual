//! One-shot tail pages keep the seed's original provenance and reservation. No separate mutable
//! epoch, cursor retry with a renewed deadline, or conversion into ordinary page authority.
use super::*;
use crate::registry_catchup::{CompletedStudioPage, PendingStudioPage, StudioPageQuery};
use catcoms_replication::epoch::MAX_EPOCH_OPERATIONS;
use catcoms_replication::registry_epoch::catchup::RegistryPageOutcome;
use catcoms_replication::studio::UnconfirmedStudioTailPreparation;
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct TailProgress {
    pages: usize,
    operations: usize,
    bytes: usize,
    cursor: Option<Vec<u8>>,
    pub(super) complete: bool,
}
pub struct PendingProvisionalStudioTail<T: MeshTransport> {
    page: PendingStudioPage<T>,
    seed: PreparedProvisionalStudioSeed,
}
pub struct CompletedProvisionalStudioTail {
    page: CompletedStudioPage,
    seed: PreparedProvisionalStudioSeed,
}
pub struct ProvisionalStudioTailPreparation {
    preparation: UnconfirmedStudioTailPreparation,
    tail: TailProgress,
    clock: Arc<dyn Clock + Send>,
    // Candidate graph and operation buffers must die before their retained capacity.
    hint: ProvisionalStudioHint,
}
impl<T: MeshTransport> PendingProvisionalStudioTail<T> {
    pub async fn fetch(self) -> CompletedProvisionalStudioTail {
        CompletedProvisionalStudioTail {
            page: self.page.fetch().await,
            seed: self.seed,
        }
    }
}
impl ProvisionalStudioTailPreparation {
    pub fn prepare(self) -> Result<PreparedProvisionalStudioSeed, SyncError> {
        if self.clock.monotonic_ms() >= self.hint.expires {
            return Err(SyncError::Unauthorized);
        }
        let seed = self.preparation.prepare()?;
        if self.clock.monotonic_ms() >= self.hint.expires {
            return Err(SyncError::Unauthorized);
        }
        Ok(PreparedProvisionalStudioSeed {
            seed,
            tail: self.tail,
            hint: self.hint,
        })
    }
}
impl<T: MeshTransport> fmt::Debug for PendingProvisionalStudioTail<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PendingProvisionalStudioTail { .. }")
    }
}
impl fmt::Debug for CompletedProvisionalStudioTail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CompletedProvisionalStudioTail { .. }")
    }
}
impl fmt::Debug for ProvisionalStudioTailPreparation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProvisionalStudioTailPreparation { .. }")
    }
}
impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    pub fn prepare_provisional_studio_tail(
        &mut self,
        seed: PreparedProvisionalStudioSeed,
    ) -> Result<PendingProvisionalStudioTail<T>, SyncError> {
        if !self.provisional_studio_hint_is_current(&seed.hint) || seed.tail.complete {
            return Err(SyncError::Unauthorized);
        }
        let page = self.prepare_unconfirmed_studio_page(
            &seed.hint.watch,
            seed.hint.head.peer(),
            StudioPageQuery {
                target: seed.hint.watch.target,
                doc_id: seed.seed.doc_id(),
                // The seed claim and starting frontier stay fixed across opaque cursors.
                heads: &[],
                seed: Some(seed.hint.head.receipt().seed_change_hash),
                cursor: seed.tail.cursor.as_deref(),
            },
            seed.hint._capacity.keepalive(),
            seed.hint.expires,
        )?;
        Ok(PendingProvisionalStudioTail { page, seed })
    }
    pub fn complete_provisional_studio_tail(
        &mut self,
        completed: CompletedProvisionalStudioTail,
    ) -> Result<Option<ProvisionalStudioTailPreparation>, SyncError> {
        if !self.provisional_studio_hint_is_current(&completed.seed.hint) {
            return Err(SyncError::Unauthorized);
        }
        let Some(RegistryPageOutcome::Page(page)) =
            self.complete_unconfirmed_studio_page(completed.page)?
        else {
            // Restart, absent service, checkpoint changes or removed-author history all require
            // a fresh discovery. No seed-only or partially consumed prefix escapes as a read.
            return Ok(None);
        };
        let mut tail = completed.seed.tail;
        tail.pages = tail.pages.saturating_add(1);
        tail.operations = tail.operations.saturating_add(page.operations.len());
        tail.bytes = tail.bytes.saturating_add(
            page.operations
                .iter()
                .map(|o| o.encode().len())
                .sum::<usize>(),
        );
        let next = page.next.map(|c| c.as_bytes().to_vec());
        if tail.pages > MAX_EPOCH_OPERATIONS + 1
            || tail.operations > MAX_EPOCH_OPERATIONS
            || tail.bytes > 16 * 1024 * 1024
            || (next.is_some() && (next == tail.cursor || page.operations.is_empty()))
        {
            return Err(SyncError::Malformed);
        }
        tail.complete = next.is_none();
        tail.cursor = next;
        Ok(Some(ProvisionalStudioTailPreparation {
            preparation: completed.seed.seed.prepare_tail(
                page.operations,
                &self.group,
                &self.device,
            )?,
            tail,
            clock: self.clock.clone(),
            hint: completed.seed.hint,
        }))
    }
}
