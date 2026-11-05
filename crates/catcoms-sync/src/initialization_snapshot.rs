//! Closed bounded encoding for a never-run private owner initialization candidate.
//!
//! This is deliberately narrower than ordinary snapshots: it refuses every retained graph,
//! receipt, invite consumption and publication obligation before any inner encoder runs. The
//! canonical Store reserves the explicit output/sealing buffers before calling it. It does not
//! create a membership or persistence capability and cannot replace the later cold-archive saver.
use super::*;

// The closed metadata invoice below is < 64 KiB: routing <= 364B; empty ledger 8B;
// empty inventories/framing < 128B; <=8 fixed finalization pairs < 1KiB; wrap key 36B;
// <=8 rendezvous entries each
// <=1024+128+8B (9280B); tenure <=256B; policy <=4101B; admin <=64*36+4B;
// scope <=512B; <=8 small owner facts/tail <33KiB. Reserve64KiB for all outer framing.
// New variable fields MUST enter this preflight/invoice before the shared encoder changes.
pub(super) const METADATA_FEE: usize = 64 * 1024;
pub(super) const MAX_INITIAL_RENDEZVOUS: usize = 8;
pub(super) const MAX_INITIAL_ADMINS: usize = 64;
pub(super) const MAX_INITIAL_FINALIZATIONS: usize = 8;
pub(super) const MAX_INITIAL_FINALIZATION_CAPACITY: usize = 16;

#[cfg(test)]
std::thread_local! {
    static ENCODING_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
pub(super) fn note_encoding() {
    ENCODING_ENTRIES.with(|entries| entries.set(entries.get() + 1));
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Encode exact existing snapshot framing under previously reserved explicit limits.
    ///
    /// Only an actual fresh factory source with privately staged Active scope is eligible.
    /// Refuse live/history-bearing state in O(1) before graph/ledger/cache encoding. Bounded
    /// metadata and provider preflights precede their allocations; the provider read guard
    /// covers both its checked lengths and actual output. The canonical reservation additionally
    /// pays four max-snapshot buffers, covering this output, bounded MLS output, SDK roster
    /// scratch (whose original credential/key material is in the checked provider snapshot),
    /// and fixed metadata. The reservation independently pays the provider's actual table backing
    /// and encoded payload allowance. The 64KiB metadata invoice is output framing, not the
    /// whole-memory fee. The exactly pinned provider exposes actual backing under the SAME guard;
    /// reported HashMap capacity is deliberately not used as the tombstone/iteration bound.
    /// The returned bytes confer no save,
    /// Current scope, trusted root or publication authority.
    pub fn snapshot_owner_initialization(
        &mut self,
        max_snapshot_bytes: usize,
        max_mls_bytes: usize,
        max_mls_backing_bytes: usize,
    ) -> Result<Zeroizing<Vec<u8>>, SyncError> {
        let bad = || SyncError::Malformed;
        let source = self.fresh_chat_admission().ok_or_else(bad)?;
        if source.group().is_empty()
            || source.group().len() > 256
            || !self.group.is_active()
            || !self.group.has_local_signer(source.signer())
            || self.device.public_key_bytes().as_slice() != source.signer()
            || !self.local_chat_scope.initialization_snapshot_allowed()
            || !self.durable_chat.initialization_empty()
            || !self.ledger.is_empty()
            || !self.docs.is_empty()
            || !self.commit_log.is_empty()
            || !self.peer_records.is_empty()
            || !self.direct_admit_results.is_empty()
            || self.member_finalization_pending.len() > MAX_INITIAL_FINALIZATIONS
            || self.member_finalization_pending.capacity() > MAX_INITIAL_FINALIZATION_CAPACITY
            || !self.outbox.is_empty()
            || !self.managed_chat_outbox.is_empty()
            || self.control_subscribed
            || !self.routing_subs.is_empty()
            || self.routing_secrets.len() > MAX_ROUTING_SECRETS
            || self.rendezvous_nodes.len() > MAX_INITIAL_RENDEZVOUS
            || self.admin_roster.len() > MAX_INITIAL_ADMINS
            || self
                .rendezvous_nodes
                .iter()
                .any(|(address, peer)| address.len() > 1024 || peer.len() > 128)
            || self
                .group_policy
                .as_ref()
                .is_some_and(|policy| policy.group_id() != source.group())
            || max_mls_bytes == 0
            || max_mls_bytes > u32::MAX as usize
            || max_mls_bytes
                .checked_add(METADATA_FEE)
                .is_none_or(|fee| fee > max_snapshot_bytes)
        {
            return Err(bad());
        }
        // No mutable provider access escapes from this synchronous call; the MLS encoder also
        // holds its actual interior read guard through preflight and its one output allocation.
        let mls = catcoms_mls::persist::snapshot_server_bounded(
            &self.device,
            &self.group,
            max_mls_bytes,
            max_mls_backing_bytes,
        )?;
        let capacity = mls.len().checked_add(METADATA_FEE).ok_or_else(bad)?;
        let bytes = self.snapshot_with_mls(&mls, Some(capacity))?;
        debug_assert!(
            bytes.len() <= max_snapshot_bytes,
            "initial metadata invoice drifted"
        );
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::{Hub, ManualClock};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    fn private() -> ChannelSync<catcoms_rt::MemNetwork, ChaCha20Rng> {
        let mut node = ChannelSync::found(
            Hub::new().join(PeerId::from_u64(0xb001)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(0xb001),
            Box::new(ManualClock::new(1_000)),
        )
        .unwrap();
        node.initialize_new_group_policy().unwrap();
        // Saved metadata only; this fixture never supplies a publication binding or claims
        // a successful canonical save. The actual Store consumer tests cover that barrier.
        node.stage_managed_chat_scope([1; 32], [2; 32]).unwrap();
        node
    }

    #[test]
    fn owner_initialization_bounded_snapshot_preserves_exact_legacy_bytes() {
        let mut node = private();
        let legacy = node.snapshot().unwrap();
        let mls = snapshot_server(&node.device, &node.group).unwrap();
        let bounded = node
            .snapshot_owner_initialization(mls.len() + METADATA_FEE, mls.len(), 512 * 1024)
            .unwrap();
        assert_eq!(bounded.as_slice(), legacy.as_slice());
        let restored = ChannelSync::restore(
            &bounded,
            Hub::new().join(PeerId::from_u64(0xb002)),
            ChaCha20Rng::seed_from_u64(0xb002),
            Box::new(ManualClock::new(1_000)),
        )
        .unwrap();
        assert!(restored.fresh_chat_admission().is_none());
        assert!(
            restored.durable_send_context().is_err(),
            "raw saved labels stay unbound"
        );
    }

    #[test]
    fn owner_initialization_refuses_tiny_budget_and_existing_history_before_encoding() {
        let mut node = private();
        let before = ENCODING_ENTRIES.with(std::cell::Cell::get);
        assert!(node.snapshot_owner_initialization(1, 1, 1).is_err());
        node.ledger.consume([3; 16]).unwrap();
        assert!(node
            .snapshot_owner_initialization(1024 * 1024, 512 * 1024, 512 * 1024)
            .is_err());
        assert_eq!(ENCODING_ENTRIES.with(std::cell::Cell::get), before);
        assert!(
            node.ledger.is_consumed(&[3; 16]),
            "refusal preserves history"
        );
    }

    #[tokio::test]
    async fn owner_initialization_refuses_real_open_document_and_routing_overflow() {
        let mut node = private();
        node.open_channel(DocType::Channel, 9).await.unwrap();
        let before = ENCODING_ENTRIES.with(std::cell::Cell::get);
        assert!(node
            .snapshot_owner_initialization(1024 * 1024, 512 * 1024, 512 * 1024)
            .is_err());
        assert_eq!(ENCODING_ENTRIES.with(std::cell::Cell::get), before);
        assert!(node.docs.contains_key(&(DocType::Channel, 9)));
        let mut empty = private();
        empty.rendezvous_nodes.push(("a".repeat(1025), vec![1]));
        assert!(empty
            .snapshot_owner_initialization(1024 * 1024, 512 * 1024, 512 * 1024)
            .is_err());
        assert_eq!(empty.rendezvous_nodes[0].0.len(), 1025);
    }
    #[tokio::test]
    async fn owner_initialization_preserves_actual_join_finalization_retry_on_restore() {
        let hub = Hub::new();
        let owner_peer = PeerId::from_u64(0xb003);
        let mut owner = ChannelSync::found(
            hub.join(owner_peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(0xb003),
            Box::new(ManualClock::new(1_000)),
        )
        .unwrap();
        owner.initialize_new_group_policy().unwrap();
        owner.subscribe_control().await.unwrap();
        let invite = owner.mint_invite([4; 16], 10_000, vec![]).unwrap();
        let member = MlsDevice::generate().unwrap();
        let transport = hub.join(PeerId::from_u64(0xb004));
        let (joined, tick) = tokio::join!(
            request_join(&transport, owner_peer, &member, &invite),
            owner.run_once(),
        );
        tick.unwrap();
        let (group, routing) = joined.unwrap();
        let mut node = ChannelSync::new_joined(
            transport,
            group,
            member,
            ChaCha20Rng::seed_from_u64(0xb004),
            Box::new(ManualClock::new(1_000)),
            routing,
        );
        // EXACT real App::Server::join retry bookkeeping, not a fabricated empty join state.
        node.note_candidate_peer(owner_peer);
        node.note_member_finalization_candidate(owner_peer, invite.inviter_device_id);
        assert_eq!(
            node.member_finalization_pending
                .get(&invite.inviter_device_id),
            Some(&owner_peer)
        );
        node.stage_managed_chat_scope([4; 32], [5; 32]).unwrap();
        let legacy = node.snapshot().unwrap();
        let bytes = node
            .snapshot_owner_initialization(1024 * 1024, 512 * 1024, 512 * 1024)
            .unwrap();
        assert_eq!(bytes.as_slice(), legacy.as_slice());
        let restored = ChannelSync::restore(
            &bytes,
            hub.join(PeerId::from_u64(0xb005)),
            ChaCha20Rng::seed_from_u64(0xb005),
            Box::new(ManualClock::new(1_000)),
        )
        .unwrap();
        assert_eq!(
            restored
                .member_finalization_pending
                .get(&invite.inviter_device_id),
            Some(&owner_peer)
        );
        assert!(restored.durable_send_context().is_err());
    }
}
