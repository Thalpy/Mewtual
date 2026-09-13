//! One seed attempt consumes the hint. No retries or parallel buffers can be forked from it;
//! another attempt requires fresh discovery and a new provisional reservation.
use super::*;
use crate::registry_seed::transfer::{
    CompletedSeedTransfer, PendingSeedTransfer, SeedTransferContext,
};
use catcoms_replication::studio::{StudioProjection, UnconfirmedStudioSeed};
use zeroize::Zeroizing;
#[cfg(test)]
mod tests;

pub struct PendingProvisionalStudioSeed<T: MeshTransport> {
    hint: ProvisionalStudioHint,
    transfer: PendingSeedTransfer<T>,
}
pub struct CompletedProvisionalStudioSeed {
    hint: ProvisionalStudioHint,
    transfer: CompletedSeedTransfer,
}
/// Owns bounded decrypted bytes and their original capacity. Run prepare on the bounded cold
/// worker pool; dropping a join handle must not release that worker's separate parsing permit.
pub struct ProvisionalStudioSeedPreparation {
    raw: Zeroizing<Vec<u8>>,
    clock: Arc<dyn Clock + Send>,
    // Rust drops fields in declaration order. Free/zeroize bytes before returning capacity,
    // including when a cancelled cold worker unwinds or rejects an expired preparation.
    hint: ProvisionalStudioHint,
}
pub struct PreparedProvisionalStudioSeed {
    seed: UnconfirmedStudioSeed,
    hint: ProvisionalStudioHint,
}
/// Scoped, explicitly unconfirmed data. There is no owner capability or ordinary StudioView.
#[derive(Debug)]
pub struct ProvisionalStudioSeedUse<'a> {
    pub candidate: ProvisionalStudioHintUse<'a>,
    pub projection: &'a StudioProjection,
}
impl<T: MeshTransport> fmt::Debug for PendingProvisionalStudioSeed<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PendingProvisionalStudioSeed { .. }")
    }
}
macro_rules! redacted_debug {
    ($($name:ident),+) => {$(impl fmt::Debug for $name {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(concat!(stringify!($name), " { .. }"))
        }
    })+};
}
redacted_debug!(
    CompletedProvisionalStudioSeed,
    ProvisionalStudioSeedPreparation,
    PreparedProvisionalStudioSeed
);
impl<T: MeshTransport> PendingProvisionalStudioSeed<T> {
    pub async fn fetch(self) -> CompletedProvisionalStudioSeed {
        CompletedProvisionalStudioSeed {
            hint: self.hint,
            transfer: self.transfer.fetch().await,
        }
    }
}
impl ProvisionalStudioSeedPreparation {
    /// No sync/store borrow during expensive parsing. A finished value still needs the current
    /// watch/member/attempt/mount checks on use; a parse success alone cannot deliver a preview.
    pub fn prepare(self) -> Result<PreparedProvisionalStudioSeed, SyncError> {
        if self.clock.monotonic_ms() >= self.hint.expires {
            return Err(SyncError::Unauthorized);
        }
        let seed = UnconfirmedStudioSeed::parse(
            self.hint.watch.target,
            self.hint.head.receipt(),
            &self.raw,
        )?;
        if self.clock.monotonic_ms() >= self.hint.expires {
            return Err(SyncError::Unauthorized);
        }
        Ok(PreparedProvisionalStudioSeed {
            hint: self.hint,
            seed,
        })
    }
}
impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Pin the request to the authenticated hint provider, preserving its original 60s expiry.
    /// Preparing consumes the candidate even if the attempt fails or is never polled.
    pub fn prepare_provisional_studio_seed(
        &mut self,
        hint: ProvisionalStudioHint,
    ) -> Result<PendingProvisionalStudioSeed<T>, SyncError> {
        if !self.provisional_studio_hint_is_current(&hint) {
            return Err(SyncError::Unauthorized);
        }
        let now = self.clock.monotonic_ms();
        let receipt = hint.head.receipt();
        let target = CheckpointTarget::Studio(hint.watch.target);
        let query = ScopedQuery {
            target,
            doc_id: epoch_id(
                target.doc_type(),
                &receipt.document.logical_key,
                receipt
                    .closed_epoch
                    .checked_add(1)
                    .ok_or(SyncError::Malformed)?,
                &receipt.close_record_hash,
            ),
            hash: receipt.seed_change_hash,
        };
        let inner = encode_scoped_query(&query, &self.group.group_id())?;
        let outbound = self.reserve_seed_outbound()?;
        let (request, auth) = self.build_authed_request(target.seed_kind(), &inner)?;
        let expires = now
            .checked_add(REQUEST_MS)
            .ok_or(SyncError::Malformed)?
            .min(hint.expires);
        let transfer = PendingSeedTransfer {
            transport: self.transport.clone(),
            clock: self.clock.clone(),
            request,
            outbound,
            retained: hint._capacity.keepalive(),
            context: SeedTransferContext {
                instance: self.registry_instance(),
                query,
                peer: hint.head.peer(),
                provider: hint.head.provider(),
                requester: self.device.public_key_bytes(),
                group: self.group.group_id(),
                inner,
                auth,
                expires,
            },
        };
        Ok(PendingProvisionalStudioSeed { hint, transfer })
    }
    pub fn complete_provisional_studio_seed(
        &mut self,
        completed: CompletedProvisionalStudioSeed,
    ) -> Result<Option<ProvisionalStudioSeedPreparation>, SyncError> {
        if !self.provisional_studio_hint_is_current(&completed.hint) {
            return Err(SyncError::Unauthorized);
        }
        let Some(raw) = self.complete_seed_transfer(completed.transfer)? else {
            return Ok(None);
        };
        Ok(Some(ProvisionalStudioSeedPreparation {
            hint: completed.hint,
            raw,
            clock: self.clock.clone(),
        }))
    }
    pub fn with_provisional_studio_seed<O>(
        &self,
        prepared: &PreparedProvisionalStudioSeed,
        inspect: impl FnOnce(ProvisionalStudioSeedUse<'_>) -> O,
    ) -> Result<O, SyncError> {
        self.with_provisional_studio_hint(&prepared.hint, |candidate| {
            inspect(ProvisionalStudioSeedUse {
                candidate,
                projection: prepared.seed.projection(),
            })
        })
    }
}
