//! Genuine NativeRequired whole founder and member signature/preseal. UI atoms
//! and the cancellation fixture remain synthetic, not native authentication.
use super::super::network::tests::prepared_mesh_founder;
use super::super::tests::{command, inputs, policy, request, selected};
use super::*;
use catcoms_net::MeshService;
use catcoms_replication::chat::Body;
use rand_chacha::ChaCha20Rng;
mod admission;
mod governance;
mod history;

type Owner = NativeTextSources<MeshService, ChaCha20Rng>;
type Row = NativeTextRuntimeSelection<MeshService, ChaCha20Rng>;

thread_local! {
    static AFTER_PREPARE:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const {std::cell::RefCell::new(None)};
    static AFTER_COMMIT:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const {std::cell::RefCell::new(None)};
    static AFTER_HISTORY:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const {std::cell::RefCell::new(None)};
}
pub(super) fn after_prepare() {
    if let Some(hook) = AFTER_PREPARE.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}
pub(super) fn after_commit() {
    if let Some(hook) = AFTER_COMMIT.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}
pub(super) fn after_history() {
    if let Some(hook) = AFTER_HISTORY.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}
fn active(vault: &NativeTextVault, budget: &NativeUiInvocationBudget, seed: u64) -> (Owner, Row) {
    let owner = prepared_mesh_founder(vault, budget, seed);
    owner
        .prepare_creation(command(vault, budget), request(1, "Original"), inputs())
        .unwrap()
        .cover_founder_activation()
        .unwrap();
    let row = owner
        .prepare_creation(command(vault, budget), request(1, "Original"), inputs())
        .unwrap()
        .select_stored_runtime()
        .unwrap();
    (owner, row)
}
fn body(text: &str) -> Body {
    Body::Create {
        text: text.into(),
        reply_to: None,
    }
}
fn sign_request(body: &Body) -> V2MemberSignRequest<'_> {
    V2MemberSignRequest {
        token: [0xe7; 32],
        channel: 7,
        body,
        parents: &[],
        dependencies: &[],
    }
}
fn prepare(
    owner: &Owner,
    row: &Row,
    vault: &NativeTextVault,
    budget: &NativeUiInvocationBudget,
) -> NativeTextMemberJob<MeshService, ChaCha20Rng> {
    let credit = owner.policy.prepare_member().unwrap();
    owner
        .capture_member(row, command(vault, budget), credit)
        .finish()
        .unwrap()
}

#[tokio::test]
async fn native_runtime_member_prepare_uses_actual_native_signer_and_exact_retry_never_resigns() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let (owner, row) = active(&vault, &budget, 0xee01);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    #[cfg(windows)]
    let before=crate::store::PreparedV2TextHost::<MeshService,ChaCha20Rng>::native_member_sign_dispatches_for_test();
    let original = body("genuine native member text");
    assert_eq!(
        prepare(&owner, &row, &vault, &budget)
            .prepare_member_save(sign_request(&original), &session),
        Ok(V2MemberSaveOutcome::Ready)
    );
    #[cfg(windows)]
    assert_eq!(crate::store::PreparedV2TextHost::<MeshService,ChaCha20Rng>::native_member_sign_dispatches_for_test(),before+1);
    assert_eq!(
        prepare(&owner, &row, &vault, &budget)
            .prepare_member_save(sign_request(&original), &session),
        Ok(V2MemberSaveOutcome::Ready)
    );
    let changed = body("changed text under the same token");
    assert_eq!(
        prepare(&owner, &row, &vault, &budget)
            .prepare_member_save(sign_request(&changed), &session),
        Err(V2MemberSaveRefusal::Conflict)
    );
    #[cfg(windows)]
    assert_eq!(crate::store::PreparedV2TextHost::<MeshService,ChaCha20Rng>::native_member_sign_dispatches_for_test(),before+1);
    let entries = owner.entries.lock().unwrap();
    let state = entries[0].runtime.state.lock().unwrap();
    assert!(
        !state.running,
        "phase one returned the actual runtime before UI commit"
    );
    entries[0].owner.close(); // SC only; no new primitive after this test fence
    assert!(
        entries[0].owner.observe_retirement().is_drained(),
        "phase one released actual entered key Ends before a native commit await"
    );
}

#[tokio::test]
async fn native_runtime_member_late_cancellation_keeps_real_presealed_original_for_exact_retry() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let (owner, row) = active(&vault, &budget, 0xee07);
    let (sender, receiver) = tokio::sync::watch::channel(false);
    let old = RequestCancellation::new(receiver, None);
    #[cfg(windows)]
    let before=crate::store::PreparedV2TextHost::<MeshService,ChaCha20Rng>::native_member_sign_dispatches_for_test();
    AFTER_PREPARE.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(move || {
            sender.send_replace(true);
        }));
    });
    let original = body("prepared before cancellation");
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_member_save(sign_request(&original), &old),
        Err(V2MemberSaveRefusal::Cancelled)
    );
    assert!(AFTER_PREPARE.with(|slot| slot.borrow().is_none()));
    #[cfg(windows)]
    assert_eq!(crate::store::PreparedV2TextHost::<MeshService,ChaCha20Rng>::native_member_sign_dispatches_for_test(),before+1);
    let (_fresh_sender, fresh_receiver) = tokio::sync::watch::channel(false);
    let fresh = RequestCancellation::new(fresh_receiver, None);
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_member_save(sign_request(&original), &fresh),
        Ok(V2MemberSaveOutcome::Ready)
    );
    #[cfg(windows)]
    assert_eq!(crate::store::PreparedV2TextHost::<MeshService,ChaCha20Rng>::native_member_sign_dispatches_for_test(),before+1);
}

#[tokio::test]
async fn native_runtime_member_cancelled_prepare_does_not_enter_the_signer() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let (owner, row) = active(&vault, &budget, 0xee02);
    let (sender, receiver) = tokio::sync::watch::channel(false);
    sender.send_replace(true);
    let session = RequestCancellation::new(receiver, None);
    #[cfg(windows)]
    let before=crate::store::PreparedV2TextHost::<MeshService,ChaCha20Rng>::native_member_sign_dispatches_for_test();
    assert_eq!(
        prepare(&owner, &row, &vault, &budget)
            .prepare_member_save(sign_request(&body("cancelled")), &session),
        Err(V2MemberSaveRefusal::Cancelled)
    );
    #[cfg(windows)]
    assert_eq!(crate::store::PreparedV2TextHost::<MeshService,ChaCha20Rng>::native_member_sign_dispatches_for_test(),before);
}

#[tokio::test]
async fn native_runtime_member_capture_cannot_borrow_a_runtime_after_original_close() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let (owner, row) = active(&vault, &budget, 0xee03);
    let capture = owner.capture_member(
        &row,
        command(&vault, &budget),
        owner.policy.prepare_member().unwrap(),
    );
    let mut retirement = owner.prepare_retirement().unwrap();
    assert_eq!(
        retirement.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Pending
    );
    assert!(matches!(
        capture.finish(),
        Err(V2MemberSaveRefusal::Unavailable)
    ));
}

#[tokio::test]
async fn native_runtime_member_foreign_same_numeric_row_never_selects_the_other_issuer() {
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let (first, first_budget) = selected(first_dir.path());
    let (second, second_budget) = selected(second_dir.path());
    let (first_owner, first_row) = active(&first, &first_budget, 0xee04);
    let (_second_owner, second_row) = active(&second, &second_budget, 0xee05);
    assert_eq!(first_row.id_data(), second_row.id_data());
    let capture = first_owner.capture_member(
        &second_row,
        command(&first, &first_budget),
        first_owner.policy.prepare_member().unwrap(),
    );
    assert!(matches!(
        capture.finish(),
        Err(V2MemberSaveRefusal::Unavailable)
    ));
}

#[tokio::test]
async fn native_runtime_member_equal_limits_from_another_policy_cannot_supply_the_payment() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let (owner, row) = active(&vault, &budget, 0xee06);
    let foreign = policy();
    let charged = owner.policy.pool.charged_for_test();
    let capture = owner.capture_member(
        &row,
        command(&vault, &budget),
        foreign.prepare_member().unwrap(),
    );
    assert!(matches!(
        capture.finish(),
        Err(V2MemberSaveRefusal::Unavailable)
    ));
    assert_eq!(owner.policy.pool.charged_for_test(), charged);
    assert_eq!(foreign.pool.charged_for_test(), 0);
}
