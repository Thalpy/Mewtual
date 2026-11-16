//! Actual MLS founding/private S0 and real record/page/manifest cryptography. Reservation
//! fixtures model ORIGINAL negative custody only, never physical Store fees, Saved or Current.
use super::*;
use crate::local_chat_scope::{ManagedChatScopeReservation, ManagedChatScopeRevocation};
use catcoms_mls::MlsDevice;
use catcoms_replication::chat::{
    cold_manifest::ManifestPage,
    cold_records::{HeaderRow, RecordBatch, RecordKind},
    owner_state::StateFrame,
};
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId, RequestCancellation};
use catcoms_storage::private_chat_page::open_private_chat_page;
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, Error, RngCore, SeedableRng};
use std::sync::{Arc, Weak};
use tokio::sync::watch;

type Node = ChannelSync<MemNetwork, ChaCha20Rng>;
const VAULT: [u8; 32] = [51; 32];
const NONCE: [u8; 32] = [52; 32];
const KEY: [u8; 32] = [53; 32];

struct Reservation {
    group: Vec<u8>,
    signer: [u8; 32],
    identity: Arc<()>,
    owner: Arc<watch::Sender<bool>>,
}
struct Revoker(Weak<watch::Sender<bool>>);
impl ManagedChatScopeRevocation for Revoker {
    fn revoke(&self) {
        if let Some(owner) = self.0.upgrade() {
            owner.send_replace(true);
        }
    }
}
impl ManagedChatScopeReservation for Reservation {
    fn group(&self) -> &[u8] {
        &self.group
    }
    fn signer(&self) -> &[u8; 32] {
        &self.signer
    }
    fn domain(&self) -> [u8; 32] {
        VAULT
    }
    fn nonce(&self) -> [u8; 32] {
        NONCE
    }
    fn owner_identity(&self) -> Arc<()> {
        self.identity.clone()
    }
    fn reservation_observer(&self) -> RequestCancellation {
        RequestCancellation::new(self.owner.subscribe(), None)
    }
    fn revocation(&self) -> Arc<dyn ManagedChatScopeRevocation> {
        Arc::new(Revoker(Arc::downgrade(&self.owner)))
    }
}
fn found(seed: u64) -> Node {
    ChannelSync::found(
        Hub::new().join(PeerId::from_u64(seed)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(seed),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap()
}
fn reserve(node: &mut Node) -> Reservation {
    let reservation = Reservation {
        group: node.group_id(),
        signer: node.my_public_key().try_into().unwrap(),
        identity: Arc::new(()),
        owner: Arc::new(watch::channel(false).0),
    };
    node.stage_managed_chat_scope(VAULT, NONCE).unwrap();
    node.attach_managed_chat_reservation(&reservation).unwrap();
    reservation
}
fn cached(node: &mut Node) -> FrozenFounder {
    let mut source = node
        .prepare_owner_v2_founder()
        .unwrap()
        .freeze(node)
        .unwrap();
    source
        .cache_private_snapshot(
            node,
            super::super::FounderSnapshotLimits::new(1024 * 1024, 512 * 1024, 512 * 1024).unwrap(),
        )
        .unwrap();
    source
}
struct CountingRng {
    inner: ChaCha20Rng,
    bytes: usize,
    before_first: Option<Box<dyn FnOnce()>>,
}
impl CountingRng {
    fn new() -> Self {
        Self {
            inner: ChaCha20Rng::from_seed([61; 32]),
            bytes: 0,
            before_first: None,
        }
    }
    fn enter(&mut self, bytes: usize) {
        if let Some(hook) = self.before_first.take() {
            hook();
        }
        self.bytes += bytes;
    }
}
impl RngCore for CountingRng {
    fn next_u32(&mut self) -> u32 {
        self.enter(4);
        self.inner.next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.enter(8);
        self.inner.next_u64()
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.enter(dest.len());
        self.inner.fill_bytes(dest);
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Error> {
        self.enter(dest.len());
        self.inner.try_fill_bytes(dest)
    }
}
impl CryptoRng for CountingRng {}

#[test]
fn actual_cached_founder_produces_genesis_records_and_manifest_with_original_signatures() {
    let mut node = found(0x4f01);
    let _reservation = reserve(&mut node);
    let source = cached(&mut node);
    let data = source.borrow_for_covering_save(&node).unwrap();
    let profile = SignedOwnerProfile::decode(data.profile).unwrap();
    let admission = AdmissionBody::decode(data.admission).unwrap();
    let original_header = data.header.to_vec();
    let private = source
        .borrow_private_snapshot_for_covering_save(&node)
        .unwrap();
    let original_snapshot = Zeroizing::new(private.snapshot.to_vec());
    let original_digest = private.digest;
    let mut archive = FounderArchivePreparation::from_cached_source(source, &node, VAULT).unwrap();
    let mut rng = CountingRng::new();
    let view = archive.prepare(&node, &KEY, &mut rng).unwrap();
    let record_bytes = open_private_chat_page(&KEY, view.records_id, view.records_frame).unwrap();
    let manifest_bytes =
        open_private_chat_page(&KEY, view.manifest_id, view.manifest_frame).unwrap();
    let operation = ArchiveOperation::Genesis {
        body: admission.id(),
    };
    let batch = RecordBatch::decode(
        &record_bytes,
        BatchBinding {
            vault: &VAULT,
            profile: &profile,
            role: PageRole::GenesisRecords,
            operation: &operation,
            append: None,
        },
    )
    .unwrap();
    assert_eq!(batch.row_count(), 3);
    assert_eq!(batch.group(), node.group_id_ref());
    let header_row = batch
        .rows()
        .find(|row| row.key.kind == RecordKind::Header)
        .unwrap();
    let header = HeaderRow::decode(header_row.payload).unwrap();
    assert_eq!(header.header().encode(), original_header);
    StateFrame::decode(header.image())
        .unwrap()
        .verify_genesis_binding(&profile, &admission, header.header())
        .unwrap();
    let manifest = ManifestPage::decode(&manifest_bytes).unwrap();
    assert_eq!(manifest.sequence(), 1);
    assert_eq!(manifest.previous_manifest(), None);
    assert_eq!(manifest.original_source_head(), None);
    assert_eq!(manifest.before_header(), None);
    assert_eq!(manifest.after_header(), header.header().id());
    assert_eq!(manifest.operation(), operation);
    assert_eq!(manifest.page_count(), 1);
    assert_eq!(
        manifest.page(0).unwrap(),
        PageDescriptor {
            role: PageRole::GenesisRecords,
            page: view.records_id,
            plaintext_bytes: record_bytes.len(),
            entries: 3
        }
    );
    assert_eq!(view.profile, profile.id());
    assert_eq!(view.header, header.header().id());
    assert_eq!(view.records_plaintext_bytes, record_bytes.len());
    assert_eq!(rng.bytes, 48, "exactly two original24-byte page nonces");
    let private = archive
        .borrow_private_snapshot_for_covering_save(&node)
        .unwrap();
    assert_eq!(private.snapshot, original_snapshot.as_slice());
    assert_eq!(private.digest, original_digest);
}

#[test]
fn exact_retries_reuse_same_frames_and_key_refusal_cannot_erase_the_original_package() {
    let mut node = found(0x4f02);
    let _reservation = reserve(&mut node);
    let source = cached(&mut node);
    let mut archive = FounderArchivePreparation::from_cached_source(source, &node, VAULT).unwrap();
    let mut rng = CountingRng::new();
    let view = archive.prepare(&node, &KEY, &mut rng).unwrap();
    let original = (
        view.records_frame.to_vec(),
        view.manifest_frame.to_vec(),
        view.records_id,
        view.manifest_id,
    );
    let pointers = (view.records_frame.as_ptr(), view.manifest_frame.as_ptr());
    assert!(archive.prepare(&node, &[54; 32], &mut rng).is_err());
    for _ in 0..3 {
        let view = archive.prepare(&node, &KEY, &mut rng).unwrap();
        assert_eq!(view.records_frame, original.0);
        assert_eq!(view.manifest_frame, original.1);
        assert_eq!(
            (view.records_id, view.manifest_id),
            (original.2, original.3)
        );
        assert_eq!(
            (view.records_frame.as_ptr(), view.manifest_frame.as_ptr()),
            pointers
        );
    }
    assert_eq!(rng.bytes, 48, "no randomness for refusal or exact retry");
    assert!(!format!("{archive:?}").contains("snapshot"));
}

#[test]
fn constructor_refusals_return_original_source_without_a_cached_false_positive_or_retarget() {
    let mut node = found(0x4f03);
    let _reservation = reserve(&mut node);
    let source = node
        .prepare_owner_v2_founder()
        .unwrap()
        .freeze(&node)
        .unwrap();
    let profile = source
        .borrow_for_covering_save(&node)
        .unwrap()
        .profile
        .to_vec();
    let refused = FounderArchivePreparation::from_cached_source(source, &node, VAULT).unwrap_err();
    assert_eq!(
        format!("{refused}"),
        "private archive source is unavailable"
    );
    let mut source = refused.into_original();
    assert_eq!(
        source.borrow_for_covering_save(&node).unwrap().profile,
        profile
    );
    source
        .cache_private_snapshot(
            &mut node,
            super::super::FounderSnapshotLimits::new(1024 * 1024, 512 * 1024, 512 * 1024).unwrap(),
        )
        .unwrap();
    let refused =
        FounderArchivePreparation::from_cached_source(source, &node, [99; 32]).unwrap_err();
    let source = refused.into_original();
    let archive = FounderArchivePreparation::from_cached_source(source, &node, VAULT).unwrap();
    assert!(
        archive.borrow_for_covering_save(&node).is_err(),
        "no prepared package exists yet"
    );

    let mut bare = found(0x4f04);
    bare.stage_managed_chat_scope(VAULT, NONCE).unwrap(); // Label alone: NO original reservation.
    let source = cached(&mut bare);
    assert!(FounderArchivePreparation::from_cached_source(source, &bare, VAULT).is_err());
}

#[test]
fn equal_restored_node_cannot_borrow_or_replace_the_original_archive_source() {
    let mut node = found(0x4f05);
    let _reservation = reserve(&mut node);
    let source = cached(&mut node);
    let snapshot = Zeroizing::new(
        source
            .borrow_private_snapshot_for_covering_save(&node)
            .unwrap()
            .snapshot
            .to_vec(),
    );
    let restored = ChannelSync::restore(
        &snapshot,
        Hub::new().join(PeerId::from_u64(0x4f06)),
        ChaCha20Rng::seed_from_u64(0x4f06),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    assert_eq!(restored.group_id(), node.group_id());
    assert_eq!(restored.my_public_key(), node.my_public_key());
    let mut archive = FounderArchivePreparation::from_cached_source(source, &node, VAULT).unwrap();
    let mut rng = CountingRng::new();
    let view = archive.prepare(&node, &KEY, &mut rng).unwrap();
    let original = view.manifest_frame.to_vec();
    assert!(archive.borrow_for_covering_save(&restored).is_err());
    assert!(archive
        .borrow_private_snapshot_for_covering_save(&restored)
        .is_err());
    assert!(archive.prepare(&restored, &KEY, &mut rng).is_err());
    assert_eq!(
        archive
            .prepare(&node, &KEY, &mut rng)
            .unwrap()
            .manifest_frame,
        original
    );
    assert_eq!(rng.bytes, 48);
}

#[test]
fn private_metadata_provider_and_original_negative_changes_refuse_before_new_crypto() {
    for which in 0..5 {
        let mut node = found(0x4f10 + which);
        let reservation = reserve(&mut node);
        let source = cached(&mut node);
        let mut archive =
            FounderArchivePreparation::from_cached_source(source, &node, VAULT).unwrap();
        let mut rng = CountingRng::new();
        archive.prepare(&node, &KEY, &mut rng).unwrap();
        let original = archive
            .package
            .as_ref()
            .unwrap()
            .manifest
            .as_bytes()
            .to_vec();
        let authenticator = node.group.epoch_authenticator_id();
        match which {
            0 => node.set_rendezvous_nodes(vec![("/ip4/192.0.2.1/tcp/1".into(), vec![1; 32])]),
            1 => {
                node.device
                    .key_package_for_invite(&node.group_id(), [1; 16])
                    .unwrap();
            }
            2 => {
                reservation.owner.send_replace(true);
            }
            3 => {
                node.take_fresh_chat_admission().unwrap();
            }
            _ => node.retire_managed_chat_scope(),
        }
        assert_eq!(node.group.epoch_authenticator_id(), authenticator);
        assert!(archive.borrow_for_covering_save(&node).is_err());
        assert!(archive.prepare(&node, &KEY, &mut rng).is_err());
        assert_eq!(rng.bytes, 48, "refusal must precede new key/RNG/AEAD work");
        assert_eq!(
            archive.package.as_ref().unwrap().manifest.as_bytes(),
            original
        );
    }
}

#[test]
fn cancellation_during_first_page_seal_returns_no_partial_package_and_cannot_be_renewed() {
    let mut node = found(0x4f20);
    let reservation = reserve(&mut node);
    let source = cached(&mut node);
    let mut archive = FounderArchivePreparation::from_cached_source(source, &node, VAULT).unwrap();
    let owner = reservation.owner.clone();
    let mut rng = CountingRng::new();
    rng.before_first = Some(Box::new(move || {
        owner.send_replace(true);
    }));
    assert!(archive.prepare(&node, &KEY, &mut rng).is_err());
    assert!(rng.bytes >= 24);
    assert!(
        archive.package.is_none(),
        "no partial original was handed off or stored"
    );
    let draws = rng.bytes;
    // No public production reset exists. This internal adversarial same-label replacement still
    // cannot renew the old original reservation; all observations retain their first identity.
    node.local_chat_scope = Default::default();
    let _replacement = reserve(&mut node);
    assert!(archive.prepare(&node, &KEY, &mut rng).is_err());
    assert_eq!(rng.bytes, draws);
}
