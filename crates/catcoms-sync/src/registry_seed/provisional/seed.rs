//! One seed attempt consumes the hint. No retries or parallel buffers can be forked from it;
//! another attempt requires fresh discovery and a new provisional reservation.
use super::*;
use crate::registry_seed::transfer::{
    CompletedSeedTransfer, PendingSeedTransfer, SeedTransferContext,
};
use catcoms_replication::studio::{
    StudioProjection, StudioUnconfirmedOverlayBasis, UnconfirmedStudioSeed,
};
use zeroize::Zeroizing;
#[cfg(test)]
pub(in crate::registry_seed::provisional) mod tests;

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
    pub(super) seed: UnconfirmedStudioSeed,
    pub(super) tail: super::tail::TailProgress,
    pub(super) hint: ProvisionalStudioHint,
}
/// Scoped, explicitly unconfirmed data. There is no owner capability or ordinary StudioView.
///
/// `non_exhaustive` so only this crate builds one (design 8.1 (ii)). The fields are public for
/// reading, and without it any crate could assemble a view around bytes of its own choosing and
/// present it as scoped.
#[non_exhaustive]
pub struct ProvisionalStudioSeedUse<'a> {
    pub candidate: ProvisionalStudioHintUse<'a>,
    /// The merged preview: the seed plus any authenticated tail applied since.
    pub projection: &'a StudioProjection,
    /// The exact seed checkpoint bytes the preview was parsed from, NOT the merged view above
    /// (design 8.1 part 2). An unconfirmed draft is based on these, because a tail is never
    /// persisted. Reachable only through this callback, so every scope check the hint performs
    /// (mount, server, channel, watch, attempt, membership, provider identity, expiry) gates them.
    /// Unconfirmed bytes, not authority; a consumer that keeps them must re-parse them first.
    pub seed_bytes: &'a [u8],
}
impl fmt::Debug for ProvisionalStudioSeedUse<'_> {
    // The seed is private Studio content, and a derive would print `seed_bytes` as a byte list.
    // The projection types already redact their own Debug; this view shows only its epoch anyway,
    // so its redaction does not rest on theirs. Only the scope metadata, the epoch and a length are
    // shown, as every sibling type (`UnconfirmedStudioSeed`, `StudioOverlay`, `StudioDraftArchive`)
    // already does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProvisionalStudioSeedUse")
            .field("candidate", &self.candidate)
            .field("epoch", &self.projection.epoch())
            .field("seed_bytes", &self.seed_bytes.len())
            .finish_non_exhaustive()
    }
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
        // The sanctioned call site (design 8.1 (ii)): the bytes are the live transfer's own, and
        // the value is kept in this crate's private field, never handed out.
        #[allow(clippy::disallowed_methods)]
        let seed = UnconfirmedStudioSeed::parse_live_transfer(
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
            tail: super::tail::TailProgress::default(),
        })
    }
}
impl PreparedProvisionalStudioSeed {
    /// Unconfirmed data only; runtime delivery must separately fence current lifecycle.
    pub fn unconfirmed_projection(&self) -> &StudioProjection {
        self.seed.projection()
    }
    pub fn unconfirmed_doc_id(&self) -> u128 {
        self.seed.doc_id()
    }
    pub fn unconfirmed_is_unexpired(&self, now: u64) -> bool {
        now < self.hint.expires
    }

    /// A finite authenticated provider prefix has been checked, not proof of a current owner.
    pub fn tail_complete(&self) -> bool {
        self.tail.complete
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
                seed_bytes: prepared.seed.seed_bytes(),
            })
        })
    }

    /// Design 8.1: the one production site that mints an Unconfirmed overlay basis.
    ///
    /// Inside `with_provisional_studio_hint`, so every current-scope condition the hint enforces
    /// holds at the moment of the mint: the same registry instance, group and MLS epoch the hint
    /// was authenticated under, current membership for this device and the provider, the
    /// provider's proven endpoint identity, the attempt generation, watch currency and expiry. On
    /// top of those, the tail must be complete: a preview whose authenticated tail has not
    /// finished has not shown the whole current history, and a draft must not start beside a gap.
    /// An incomplete tail refuses as `Unauthorized`, like every other not-current condition.
    ///
    /// The author is this device; the provider comes from the hint, never the caller. The MLS
    /// epoch and **wall-clock** time are recorded as admission facts (`Clock::now_ms`, since they
    /// are persisted) and are not fingerprinted (design review (i)). Not checked here, because
    /// this crate cannot see the store: that the document has no installed source. The app
    /// checks that under custody before asking, and again at S3.
    pub fn mint_unconfirmed_overlay_basis(
        &self,
        prepared: &PreparedProvisionalStudioSeed,
    ) -> Result<StudioUnconfirmedOverlayBasis, SyncError> {
        if !prepared.tail_complete() {
            return Err(SyncError::Unauthorized);
        }
        let author = self.device.device_id();
        let observed_mls_epoch = self.group.epoch();
        let observed_at_ms = self.clock.now_ms();
        let basis = self.with_provisional_studio_hint(&prepared.hint, |candidate| {
            // The sanctioned call site (design 8.1 (ii)): the value it mints from exists only
            // inside this crate's private field, and the hint has just been re-checked.
            #[allow(clippy::disallowed_methods)]
            StudioUnconfirmedOverlayBasis::mint_from_live_preview(
                &prepared.seed,
                candidate.receipt,
                author,
                candidate.provider,
                observed_mls_epoch,
                observed_at_ms,
            )
        })??;
        Ok(basis)
    }
}
