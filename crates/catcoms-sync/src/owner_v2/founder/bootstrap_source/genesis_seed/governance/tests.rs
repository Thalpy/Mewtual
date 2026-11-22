//! Actual MLS candidate and Core math under explicit TEST host selection.
//! Synthetic native atomics are lifecycle fixtures, not command credentials.
//! Installation stays test-private until the real Root covering consumer exists.
use super::super::tests::ordinary_data::prepared_node_with_clock;
use super::*;
use crate::owner_v2::member::governance_fixture::governance;
use catcoms_mls::group::text_current_add::RetiredTextCurrentAdd;
use catcoms_mls::persist::text_current_add::{TextCurrentAddLimits, TextCurrentAddReservation};
use catcoms_replication::chat::join_bindings::{
    GenesisAnchorBindingData, InviteBindingSpecData, JoinPayloadKindData,
    JoinRequestBindingSpecData, SignedInviteBindingData, SignedJoinRequestBindingData,
};
use catcoms_replication::chat::owner_state::transition::ReplayBody;
use catcoms_replication::chat::V2HostSelectionExpectation;
use catcoms_rt::{
    ManualClock, MemNetwork, NativeChatSourceOwner, NativeChatUiBinding, NativeChatUiSessionOwner,
};
use rand_chacha::ChaCha20Rng;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::SeqCst};

type Node = PreparedV2FounderGovernanceNode<MemNetwork, ChaCha20Rng>;
const NOW: u64 = 1_800_000_000_000;
const RAW: usize = 2 * 1024 * 1024;

mod native_stage;
mod signed_request;
mod welcome;

fn prepared_node(
    id: u64,
) -> (
    UnboundV2GenesisNode<MemNetwork, ChaCha20Rng>,
    super::super::tests::Reservation,
    OpenedBootstrapSourceData,
    Arc<AtomicUsize>,
) {
    prepared_node_with_clock(id, Box::new(ManualClock::new(NOW)))
}

struct Native {
    _binding: NativeChatUiBinding,
    ui: NativeChatUiSessionOwner,
    source: NativeChatSourceOwner,
    generation: Arc<AtomicU64>,
    authorized: Arc<AtomicU64>,
}
impl Native {
    fn new() -> Self {
        let generation = Arc::new(AtomicU64::new(7));
        let authorized = Arc::new(AtomicU64::new(7));
        let binding = NativeChatUiBinding::from_trusted_host(
            generation.clone(),
            authorized.clone(),
            Arc::new(AtomicBool::new(false)),
            Arc::new(()),
        );
        let ui = binding.open_session(7, Arc::new(())).unwrap();
        let source =
            NativeChatSourceOwner::for_trusted_source(&ui.observe(), Arc::new(())).unwrap();
        Self {
            _binding: binding,
            ui,
            source,
            generation,
            authorized,
        }
    }
    fn cut(&self, purpose: NativeChatKeyPurpose) -> NativeChatPreparedKeyCut {
        self.source
            .prepare_cut(&self.ui.observe(), purpose)
            .unwrap()
    }
    fn reauthenticate(&mut self) {
        let generation = self.generation.load(SeqCst);
        self.authorized.store(generation, SeqCst);
        self.ui = self
            ._binding
            .open_session(generation, Arc::new(()))
            .unwrap();
    }
}

struct Fee(Arc<AtomicUsize>);
impl TextCurrentAddReservation for Fee {
    fn limits(&self) -> TextCurrentAddLimits {
        TextCurrentAddLimits::governed_text_v2()
    }
}
impl Drop for Fee {
    fn drop(&mut self) {
        self.0.fetch_sub(1, SeqCst);
    }
}
fn admission(count: &Arc<AtomicUsize>) -> TextCurrentAddAdmission {
    count.fetch_add(1, SeqCst);
    TextCurrentAddAdmission::new(Fee(count.clone())).unwrap()
}
struct Request {
    _peer: MlsDevice,
    frame: Vec<u8>,
    invite: SignedInviteBindingData,
    request: SignedJoinRequestBindingData,
}
impl Request {
    fn new(node: &Node, nonce: u8) -> Self {
        Self::with_package_times(node, nonce, NOW / 1000 - 60, NOW / 1000 + 60 * 24 * 60 * 60)
    }
    fn with_package_times(node: &Node, nonce: u8, not_before: u64, not_after: u64) -> Self {
        let peer = MlsDevice::generate().unwrap();
        let nonce = [nonce; 16];
        let kp = peer
            .key_package_for_invite_fixture(
                node.genesis_data().context.spec_data().group,
                nonce,
                not_before,
                not_after,
            )
            .unwrap();
        let reference = peer.key_package_reference(&kp).unwrap();
        let subject = peer.public_key_bytes().try_into().unwrap();
        let credential = *peer.key_package_identity(&kp).unwrap().credential_digest();
        let frame = catcoms_mls::serialize_key_package(&kp).unwrap();
        let d = node.genesis_data();
        let invite = SignedInviteBindingData::sign_using(
            InviteBindingSpecData {
                anchor: GenesisAnchorBindingData {
                    group: d.profile.spec().group.clone(),
                    founder: d.profile.spec().founder,
                    profile: *d.profile.id().as_bytes(),
                    admission: *d.admission.admission_id().as_bytes(),
                    tenure: *d.profile.tenure().as_bytes(),
                    header: *d.header.id().as_bytes(),
                    state: *d.header.spec().state.as_bytes(),
                },
                inviter: d.context.spec_data().signer,
                nonce,
                expires_at_ms: NOW + 60_000,
                policy: JoinPayloadKindData::Policy
                    .commitment_data(
                        &node
                            .node
                            .seed
                            .body
                            .metadata
                            .policy
                            .as_ref()
                            .unwrap()
                            .encode(),
                    )
                    .unwrap(),
                // Explicit public DATA fixture; Root route authorization is
                // deliberately absent from these library-only tests.
                public_routing_plan: JoinPayloadKindData::RoutingPlan
                    .commitment_data(b"test public plan")
                    .unwrap(),
            },
            |bytes| {
                node.node
                    .seed
                    .body
                    .device
                    .sign_retained_ed25519_bounded(
                        &d.context.spec_data().signer,
                        bytes,
                        catcoms_replication::chat::join_bindings::MAX_JOIN_BINDING_BYTES,
                    )
                    .map_err(|_| catcoms_replication::chat::ChatError::Dependency)
            },
        )
        .unwrap();
        let request = SignedJoinRequestBindingData::sign_using(
            JoinRequestBindingSpecData {
                invite: invite.id_data(),
                anchor: invite.spec_data().anchor.commitment_data().unwrap(),
                joiner: subject,
                request_token: [nonce[0]; 32],
                key_package_wire: JoinPayloadKindData::KeyPackageWire
                    .commitment_data(&frame)
                    .unwrap(),
                key_package_reference: reference,
                credential,
            },
            |bytes| {
                peer.sign(bytes)
                    .map_err(|_| catcoms_replication::chat::ChatError::Dependency)
            },
        )
        .unwrap();
        Self {
            _peer: peer,
            frame,
            invite,
            request,
        }
    }
    fn data(&self) -> V2FounderAddRequestData<'_> {
        V2FounderAddRequestData {
            invite: &self.invite,
            request: &self.request,
            key_package: &self.frame,
        }
    }
    fn sign_request(&self, spec: JoinRequestBindingSpecData) -> SignedJoinRequestBindingData {
        SignedJoinRequestBindingData::sign_using(spec, |bytes| {
            self._peer
                .sign(bytes)
                .map_err(|_| catcoms_replication::chat::ChatError::Dependency)
        })
        .unwrap()
    }
    fn change_invite(&mut self, node: &Node, change: impl FnOnce(&mut InviteBindingSpecData)) {
        let mut spec = self.invite.spec_data().clone();
        change(&mut spec);
        let signer = node.genesis_data().context.spec_data().signer;
        self.invite = SignedInviteBindingData::sign_using(spec, |bytes| {
            node.node
                .seed
                .body
                .device
                .sign_retained_ed25519_bounded(
                    &signer,
                    bytes,
                    catcoms_replication::chat::join_bindings::MAX_JOIN_BINDING_BYTES,
                )
                .map_err(|_| catcoms_replication::chat::ChatError::Dependency)
        })
        .unwrap();
        let mut spec = self.request.spec_data().clone();
        spec.invite = self.invite.id_data();
        spec.anchor = self.invite.spec_data().anchor.commitment_data().unwrap();
        self.request = self.sign_request(spec);
    }
}
fn begin(
    node: &mut Node,
    native: &Native,
    count: &Arc<AtomicUsize>,
    request: V2FounderAddRequestData<'_>,
) {
    node.prepare_add_data(
        request,
        native.source.observe(),
        admission(count),
        native.cut(NativeChatKeyPurpose::AddSdk),
        native.cut(NativeChatKeyPurpose::GoverningHeader),
        native.cut(NativeChatKeyPurpose::JoinWelcome),
    )
    .unwrap();
}

fn hook(cut: TestCut, f: impl FnOnce() + 'static) {
    TEST_CUT.with(|slot| *slot.borrow_mut() = Some((cut, Box::new(f))));
}

struct Retired {
    _sdk: RetiredTextCurrentAdd,
    // Retired old pair/output payloads must be gone before the original Ends.
    _pending: Box<PendingAdd>,
}

/// Test-only HOST selection. No production install API is exposed before the
/// real Root covering receipt, physical guard and cancellation worker exist.
fn install_for_test(
    node: &mut Node,
    verifier: &mut catcoms_replication::chat::V2GeneralPrefixVerifier,
) -> (Retired, catcoms_replication::chat::V2GeneralRetiredData) {
    node.require_sdk_current_data().unwrap();
    let p = node.pending.as_mut().unwrap();
    assert!(p.phase == Phase::Materialized);
    let result = p.cell.as_ref().unwrap().completed_result_data().unwrap();
    let header = p.header.as_ref().unwrap();
    let installed = V2InstalledFounderAddData {
        epoch: result.after_epoch_data(),
        authenticator: *result.after_authenticator_data(),
        header: header.id(),
        state: header.spec().state,
    };
    let mut candidate = Some(
        verifier
            .derive_successor(
                header,
                ReplayBody::Admission(p.body.as_ref().unwrap()),
                p.unsigned.as_ref().unwrap().image_data(),
            )
            .unwrap(),
    );
    let selected = V2HostSelectionExpectation::from_trusted_host(header.id(), header.spec().state);
    let s = &mut node.node.seed.body;
    // Stable fields only. SDK pair remains exclusively borrowed by preflight.
    let negative = V2FounderGovernanceNegative {
        source: s.source,
        context: &s.context,
        scope: &s.metadata.scope,
        parts: StableParts {
            warm: node.node._parts.body.warm.as_ref(),
            fresh: node.node._parts.body.fresh.as_ref(),
        },
    };
    let mls =
        PreparedTextCurrentAdd::prepare_install_into(&mut p.cell, &mut s.device, &mut s.group)
            .unwrap();
    negative.require_live_data().unwrap();
    // In production Root will cache this retired owner BEFORE the MLS swap.
    // No hook, Result, allocation or owning Drop follows Core success until
    // both the real installed SDK evidence and retired owners are retained.
    let core_retired = verifier
        .advance_host_selected(&selected, &mut candidate)
        .unwrap();
    let sdk_retired = mls.install();
    node.installed = Some(installed);
    let pending = node.pending.take().unwrap();
    (
        Retired {
            _sdk: sdk_retired,
            _pending: pending,
        },
        core_retired,
    )
}

#[test]
fn failed_sdk_constructor_is_sticky_without_replacing_original_fee_or_native_cut() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xe904);
    let mut node = original.prepare_governance_data().unwrap();
    let native = Native::new();
    let request = Request::new(&node, 34);
    let count = Arc::new(AtomicUsize::new(0));
    let mut wrong_spec = request.request.spec_data().clone();
    wrong_spec.key_package_reference = [88; 32];
    let wrong_request = request.sign_request(wrong_spec);
    let wrong = V2FounderAddRequestData {
        request: &wrong_request,
        ..request.data()
    };
    begin(&mut node, &native, &count, wrong);
    assert!(node.execute_add_data_native(wrong).is_err());
    let p = node.pending.as_ref().unwrap();
    assert!(p.phase == Phase::SdkEntered && p.cell.is_none() && p.admission.is_some());
    assert!(node.execute_add_data_native(request.data()).is_err());
    assert_eq!(node.node.seed.body.group.epoch(), 0);
    assert_eq!(count.load(SeqCst), 1);
    drop(node);
    assert_eq!(count.load(SeqCst), 0);
}

#[test]
fn cancellation_after_sdk_entry_finishes_same_finite_candidate_and_keeps_its_result() {
    for cut in [TestCut::SdkEntered, TestCut::SdkComplete] {
        let (original, _reservation, _opened, _old_fee) = prepared_node(0xe905);
        let mut node = original.prepare_governance_data().unwrap();
        let native = Native::new();
        let request = Request::new(&node, 35);
        let count = Arc::new(AtomicUsize::new(0));
        begin(&mut node, &native, &count, request.data());
        let generation = native.generation.clone();
        hook(cut, move || {
            generation.store(8, SeqCst);
        });
        assert!(node.execute_add_data_native(request.data()).is_err());
        assert_eq!(node.completed_add_data().unwrap().after_epoch_data(), 1);
        assert_eq!(node.node.seed.body.group.epoch(), 0);
        assert!(node.sign_admission_native().is_err());
        drop(node);
        assert_eq!(count.load(SeqCst), 0);
    }
}

#[test]
fn raw_header_survives_late_lock_and_exact_retry_never_reenters_key() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xe906);
    let mut node = original.prepare_governance_data().unwrap();
    let d = node.genesis_data();
    let g = governance(d.profile, d.admission, d.header, d.state);
    let native = Native::new();
    let request = Request::new(&node, 36);
    let count = Arc::new(AtomicUsize::new(0));
    begin(&mut node, &native, &count, request.data());
    node.execute_add_data_native(request.data()).unwrap();
    node.prepare_admission_data(g.verifier.replay_data())
        .unwrap();
    assert!(node.raw_admission_signature_data().is_err());
    let generation = native.generation.clone();
    hook(TestCut::RawCached, move || {
        generation.store(8, SeqCst);
    });
    node.sign_admission_native().unwrap();
    let raw = *node.raw_admission_signature_data().unwrap();
    assert!(node.signed_admission_data().is_err());
    // Would panic if this retry reached the original private key entry again.
    hook(TestCut::HeaderEntered, || {
        panic!("cached raw must bypass the key")
    });
    node.sign_admission_native().unwrap();
    TEST_CUT.with(|slot| {
        slot.borrow_mut().take();
    });
    assert_eq!(node.raw_admission_signature_data().unwrap(), &raw);
    node.materialize_admission_data().unwrap();
    let signed = node.signed_admission_data().unwrap();
    assert!(signed.verify());
    assert_eq!(signed.signature(), &raw);
    assert!(node.require_sdk_current_data().is_ok()); // Native lock is a distinct negative.
    assert!(node
        .pending
        .as_ref()
        .unwrap()
        .header_cut
        .as_ref()
        .unwrap()
        .is_cancelled());
    drop(node);
    assert_eq!(count.load(SeqCst), 0);
}

#[test]
fn panic_after_header_entry_does_not_repeat_key_or_lose_completed_sdk_result() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xe907);
    let mut node = original.prepare_governance_data().unwrap();
    let d = node.genesis_data();
    let g = governance(d.profile, d.admission, d.header, d.state);
    let native = Native::new();
    let request = Request::new(&node, 37);
    let count = Arc::new(AtomicUsize::new(0));
    begin(&mut node, &native, &count, request.data());
    node.execute_add_data_native(request.data()).unwrap();
    node.prepare_admission_data(g.verifier.replay_data())
        .unwrap();
    hook(TestCut::HeaderEntered, || {
        panic!("simulated interrupted signer entry")
    });
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        node.sign_admission_native()
    }))
    .is_err());
    assert!(node.sign_admission_native().is_err());
    assert!(node.raw_admission_signature_data().is_err());
    assert!(node.materialize_admission_data().is_err());
    assert_eq!(node.completed_add_data().unwrap().after_epoch_data(), 1);
    drop(node);
    assert_eq!(count.load(SeqCst), 0);
}

#[test]
fn same_node_verifier_and_original_admission_survive_two_real_adds() {
    use crate::owner_v2::member::governance_fixture::create_draft;
    let (original, _reservation, _opened, old_fee) = prepared_node(0xe908);
    let original_seed = original.seed.observe_original_seed_data();
    let mut node = original.prepare_governance_data().unwrap();
    let d = node.genesis_data();
    let original_context = d.context.clone();
    let original_admission = d.admission.admission_id();
    let mut g = governance(d.profile, d.admission, d.header, d.state);
    let native = Native::new();
    let count = Arc::new(AtomicUsize::new(0));
    let mut previous_transfer = None;
    for step in 1..=2 {
        let request = Request::new(&node, 40 + step);
        begin(&mut node, &native, &count, request.data());
        node.execute_add_data_native(request.data()).unwrap();
        let transfer = signed_request::require_joiner_routing(&node, &request);
        assert!(previous_transfer.as_ref() != Some(&transfer));
        previous_transfer = Some(transfer);
        node.prepare_admission_data(g.verifier.replay_data())
            .unwrap();
        node.sign_admission_native().unwrap();
        node.materialize_admission_data().unwrap();
        let (retired, core_retired) = install_for_test(&mut node, &mut g.verifier);
        assert_eq!(node.node.seed.body.group.epoch(), u64::from(step));
        node.require_sdk_current_data().unwrap();
        assert!(ordinary_create::source(&node.node).is_err());
        original_seed
            .require_same_seed_data(&node.node.seed)
            .unwrap();
        assert!(node.genesis_data().context == &original_context);
        assert_eq!(
            node.genesis_data().admission.admission_id(),
            original_admission
        );
        assert!(g.verifier.admission(original_admission).is_ok());
        drop(retired);
        drop(core_retired);
        assert_eq!(count.load(SeqCst), 1); // Live installed pair still pays its own candidate fee.
        assert_eq!(old_fee.load(SeqCst), 0);
        let mut inputs = crate::owner_v2::V2MemberDraftInputs::new();
        let drawn = node.draw_ordinary_draft_inputs(&mut inputs).unwrap();
        let signer = node.ordinary_member_signer_data().unwrap();
        let draft = create_draft(&g, &g.founder, signer, drawn, "ordinary after Add");
        let mut plan = g
            .verifier
            .prepare_ordinary_signing_plan(&draft, &g.founder, &[], &g.view)
            .unwrap();
        let observation = plan.observe_original();
        let guard = plan.admit_signing(&draft, &signer).unwrap();
        let mut cut = native.cut(NativeChatKeyPurpose::MemberAction);
        let mut ends = NativeChatKeyEnds::new();
        drop(
            node.sign_ordinary_admitted_native(
                &inputs,
                guard,
                &native.source.observe(),
                &mut cut,
                &mut ends,
            )
            .unwrap(),
        );
        let action = plan
            .complete_cached_raw64_data(&observation, &draft)
            .unwrap();
        assert!(action.verify());
        assert_eq!(action.signer(), &signer);
        drop(ends);
    }
    assert_eq!(g.verifier.replay_data().slots().len(), 3);
    drop(node);
    assert_eq!(count.load(SeqCst), 0);
}

#[test]
fn source_close_waits_for_actual_private_candidate_cleanup() {
    use std::future::Future;
    use std::task::{Context, Poll, Waker};
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xe909);
    let mut node = original.prepare_governance_data().unwrap();
    let native = Native::new();
    let request = Request::new(&node, 49);
    let count = Arc::new(AtomicUsize::new(0));
    begin(&mut node, &native, &count, request.data());
    node.execute_add_data_native(request.data()).unwrap();
    let close = native.source.observe_retirement();
    native.source.close();
    let mut drain = Box::pin(close.wait());
    let mut cx = Context::from_waker(Waker::noop());
    assert!(matches!(drain.as_mut().poll(&mut cx), Poll::Pending));
    assert!(node.completed_add_data().is_ok());
    assert_eq!(count.load(SeqCst), 1);
    drop(node);
    assert_eq!(count.load(SeqCst), 0);
    assert!(matches!(drain.as_mut().poll(&mut cx), Poll::Ready(())));
}

#[test]
fn cold_genesis_data_cannot_become_warm_governance_or_replace_the_original_node() {
    use rand_core::SeedableRng;
    let (original, _reservation, opened, count) = prepared_node(0xe90a);
    let mut seed = super::super::tests::seed(&opened, &count);
    // Exact genuine original scope attachment, but deliberately no retained
    // warm Fresh lifetime. An authenticated snapshot is still historical DATA.
    seed.prepare_scope(&_reservation).unwrap();
    let parts = V2GenesisRuntimeParts::cold_data(
        Arc::new(catcoms_rt::Hub::new().join(catcoms_rt::PeerId::from_u64(0xe90a))),
        ChaCha20Rng::seed_from_u64(0xe90a),
        Arc::new(catcoms_rt::ManualClock::new(NOW)),
    );
    let cold = seed.prepare_runtime(parts).unwrap().into_node();
    assert!(cold.prepare_governance_data().is_err());
    assert!(original.prepare_governance_data().is_ok());
}

#[test]
fn native_source_is_one_original_across_both_retained_cuts() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xe90b);
    let mut node = original.prepare_governance_data().unwrap();
    let a = Native::new();
    let b = Native::new();
    let count = Arc::new(AtomicUsize::new(0));
    let request = Request::new(&node, 51);
    for reversed in [false, true] {
        let (sdk, header) = if reversed { (&b, &a) } else { (&a, &b) };
        let refused = node
            .prepare_add_data(
                request.data(),
                a.source.observe(),
                admission(&count),
                sdk.cut(NativeChatKeyPurpose::AddSdk),
                header.cut(NativeChatKeyPurpose::GoverningHeader),
                a.cut(NativeChatKeyPurpose::JoinWelcome),
            )
            .unwrap_err();
        assert!(node.pending.is_none());
        assert_eq!(node.node.seed.body.group.epoch(), 0);
        assert_eq!(count.load(SeqCst), 1);
        drop(refused);
        assert_eq!(count.load(SeqCst), 0);
    }
    begin(&mut node, &a, &count, request.data()); // Genuine same-original control.
    node.execute_add_data_native(request.data()).unwrap();
    let d = node.genesis_data();
    let g = governance(d.profile, d.admission, d.header, d.state);
    node.prepare_admission_data(g.verifier.replay_data())
        .unwrap();
    a.source.close();
    let refused = node
        .prepare_add_data(
            request.data(),
            b.source.observe(),
            admission(&count),
            b.cut(NativeChatKeyPurpose::AddSdk),
            b.cut(NativeChatKeyPurpose::GoverningHeader),
            b.cut(NativeChatKeyPurpose::JoinWelcome),
        )
        .unwrap_err();
    drop(refused);
    assert!(node.sign_admission_native().is_err());
    assert!(node.pending.as_ref().unwrap().raw.is_none());
    assert_eq!(node.completed_add_data().unwrap().after_epoch_data(), 1);
    assert_eq!(count.load(SeqCst), 1);
    drop(node);
    assert_eq!(count.load(SeqCst), 0);
}

#[test]
fn lock_unlock_cannot_replace_the_original_add_cuts_or_revive_header_signing() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xe90c);
    let mut node = original.prepare_governance_data().unwrap();
    let d = node.genesis_data();
    let g = governance(d.profile, d.admission, d.header, d.state);
    let mut native = Native::new();
    let count = Arc::new(AtomicUsize::new(0));
    let request = Request::new(&node, 52);
    begin(&mut node, &native, &count, request.data());
    node.execute_add_data_native(request.data()).unwrap();
    node.prepare_admission_data(g.verifier.replay_data())
        .unwrap();
    native.generation.store(8, SeqCst);
    native.reauthenticate();
    let refused = node
        .prepare_add_data(
            request.data(),
            native.source.observe(),
            admission(&count),
            native.cut(NativeChatKeyPurpose::AddSdk),
            native.cut(NativeChatKeyPurpose::GoverningHeader),
            native.cut(NativeChatKeyPurpose::JoinWelcome),
        )
        .unwrap_err();
    drop(refused);
    assert!(node.sign_admission_native().is_err());
    assert!(node.pending.as_ref().unwrap().raw.is_none());
    assert_eq!(node.completed_add_data().unwrap().after_epoch_data(), 1);
    drop(node);
    assert_eq!(count.load(SeqCst), 0);
}

#[test]
fn explicit_time_package_fixture_still_refuses_the_genuine_closed_device() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xe90d);
    let node = original.prepare_governance_data().unwrap();
    let group = node.genesis_data().context.spec_data().group;
    assert!(matches!(
        node.node.seed.body.device.key_package_for_invite_fixture(
            group,
            [53; 16],
            NOW / 1000 - 60,
            NOW / 1000 + 60 * 24 * 60 * 60,
        ),
        Err(catcoms_mls::MlsError::Internal(_))
    ));
    let request = Request::new(&node, 53); // Unrestricted genuine peer control.
    assert!(!request.frame.is_empty());
    assert_eq!(node.node.seed.body.group.epoch(), 0);
    node.require_sdk_current_data().unwrap();
}

#[test]
fn cancelled_before_sdk_preparation_preserves_unspent_candidate_and_original_node() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xe901);
    let mut node = original.prepare_governance_data().unwrap();
    let native = Native::new();
    let request = Request::new(&node, 31);
    let count = Arc::new(AtomicUsize::new(0));
    begin(&mut node, &native, &count, request.data());
    native.generation.store(8, SeqCst);
    assert!(node.execute_add_data_native(request.data()).is_err());
    let p = node.pending.as_ref().unwrap();
    assert!(p.phase == Phase::Prepared && p.cell.is_none() && p.admission.is_some());
    assert_eq!(node.node.seed.body.group.epoch(), 0);
    assert!(node.completed_add_data().is_err());
    drop(node);
    assert_eq!(count.load(SeqCst), 0);
}

#[test]
fn real_completed_add_retry_keeps_exact_snapshot_and_refuses_changed_request() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xe902);
    let mut node = original.prepare_governance_data().unwrap();
    let native = Native::new();
    let request = Request::new(&node, 32);
    let count = Arc::new(AtomicUsize::new(0));
    begin(&mut node, &native, &count, request.data());
    node.execute_add_data_native(request.data()).unwrap();
    let snapshot = Zeroizing::new(
        node.completed_add_data()
            .unwrap()
            .secret_snapshot_data()
            .to_vec(),
    );
    let commit = node.completed_add_data().unwrap().commit_data().to_vec();
    node.execute_add_data_native(request.data()).unwrap();
    assert!(node.completed_add_data().unwrap().secret_snapshot_data() == snapshot.as_slice());
    assert!(node.completed_add_data().unwrap().commit_data() == commit);
    let mut wrong_spec = request.request.spec_data().clone();
    wrong_spec.key_package_reference = [88; 32];
    let wrong_request = request.sign_request(wrong_spec);
    let wrong = V2FounderAddRequestData {
        request: &wrong_request,
        ..request.data()
    };
    assert!(node.execute_add_data_native(wrong).is_err());
    assert_eq!(node.completed_add_data().unwrap().after_epoch_data(), 1);
    assert_eq!(node.node.seed.body.group.epoch(), 0);
    assert!(node.ordinary_member_signer_data().is_err());
    drop(node);
    assert_eq!(count.load(SeqCst), 0);
}

#[test]
fn late_cancel_preserves_exact_result_but_cannot_enter_governing_key() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xe903);
    let mut node = original.prepare_governance_data().unwrap();
    let d = node.genesis_data();
    let g = governance(d.profile, d.admission, d.header, d.state);
    let native = Native::new();
    let request = Request::new(&node, 33);
    let count = Arc::new(AtomicUsize::new(0));
    begin(&mut node, &native, &count, request.data());
    node.execute_add_data_native(request.data()).unwrap();
    node.prepare_admission_data(g.verifier.replay_data())
        .unwrap();
    native.generation.store(8, SeqCst);
    assert!(node.sign_admission_native().is_err());
    assert!(node.pending.as_ref().unwrap().raw.is_none());
    assert_eq!(node.completed_add_data().unwrap().after_epoch_data(), 1);
    assert!(node.materialize_admission_data().is_err());
    drop(node);
    assert_eq!(count.load(SeqCst), 0);
}
