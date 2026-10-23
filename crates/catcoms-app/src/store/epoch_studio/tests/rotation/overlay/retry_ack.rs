//! Exact retry acknowledges authenticated existing custody, never fresh close authority.
use super::*;
use crate::store::{FailError, WriteHooks};

#[test]
fn closing_retry_ack_flushes_unchanged_bytes_and_invalidates_accounting_even_on_failure() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    let basis = basis.fingerprint();
    let branch = request_branch(&f, &mut store, &close);
    let operation = f.title();
    save(&f, &mut store, &close, basis, operation.clone(), 300);
    let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
    let path = store.epoch_intent_path(&scope);
    let original = fs::read(&path).unwrap();
    let before_inventory = store.inventory_generation();
    let before_intents = store.intent_generation();
    let error = store
        .acknowledge_studio_closing_retry_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            basis,
            branch,
            &operation,
            &mut WriteHooks::fail_before_sync(FailError::NotDurable(
                "injected retry flush failure",
            )),
        )
        .unwrap_err();
    assert!(matches!(error, AppError::CommittedButNotDurable(_)));
    assert!(!Arc::ptr_eq(
        &before_inventory,
        &store.inventory_generation()
    ));
    assert!(!Arc::ptr_eq(&before_intents, &store.intent_generation()));
    assert_eq!(fs::read(&path).unwrap(), original);
    let saved = store
        .acknowledge_studio_closing_retry_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            basis,
            branch,
            &operation,
            &mut WriteHooks::MustNotWrite(
                "retry must only flush authenticated existing ciphertext",
            ),
        )
        .unwrap()
        .unwrap();
    assert_eq!(acknowledged(saved), (basis, 1));
    assert_eq!(fs::read(&path).unwrap(), original);
    let mut changed = operation.clone();
    changed.body = FlipnoteOp::SetHeader(FlipnoteHeader::Title("different".into()))
        .encode()
        .unwrap();
    refusal(
        store
            .acknowledge_studio_closing_retry(
                SERVER, &f.group, f.target, &f.device, basis, branch, &changed,
            )
            .unwrap_err(),
        StudioOverlaySaveRefusal::NonceConflict,
    );
    let mut fresh = operation.clone();
    fresh.nonce = [0x91; 16];
    assert!(store
        .acknowledge_studio_closing_retry(
            SERVER, &f.group, f.target, &f.device, basis, branch, &fresh
        )
        .unwrap()
        .is_none());
    let stranger = MlsDevice::generate().unwrap();
    assert!(store
        .acknowledge_studio_closing_retry(
            SERVER, &f.group, f.target, &stranger, basis, branch, &operation
        )
        .is_err());
    // Dispose through the actual accounted transaction, then classify the retained terminal.
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let metadata = state.handoff_metadata().unwrap();
    let request = crate::store::epoch_intents::disposal::StudioOverlayDisposalRequest {
        branch,
        content: metadata.branch_content(&state.ledger).unwrap(),
        accepted: 1,
        mode: crate::store::epoch_intents::disposal::StudioDisposalRequestMode::Discard(
            catcoms_replication::studio::StudioDiscardConfirmation::parse(
                catcoms_replication::studio::StudioDiscardConfirmation::TOKEN,
            )
            .unwrap(),
        ),
    };
    drop(state);
    let mut b = budget(&mut store, &f);
    let mut manifest = store
        .dispose_studio_overlay_with_io(
            SERVER,
            &f.logical,
            f.target,
            &f.group,
            &f.device,
            request,
            500,
            &mut rng(),
            &mut b.storage,
            &mut b.intents,
            &mut WriteHooks::None,
        )
        .unwrap();
    let disposed_bytes = fs::read(&path).unwrap();
    assert!(matches!(
        store
            .acknowledge_studio_closing_retry(
                SERVER, &f.group, f.target, &f.device, basis, branch, &operation,
            )
            .unwrap(),
        Some(catcoms_replication::studio::StudioOverlaySave::Disposed(_))
    ));
    assert_eq!(fs::read(&path).unwrap(), disposed_bytes);
    refusal(
        store
            .acknowledge_studio_closing_retry(
                SERVER, &f.group, f.target, &f.device, [0x94; 32], branch, &operation,
            )
            .unwrap_err(),
        StudioOverlaySaveRefusal::NonceConflict,
    );
    // The same terminal guard also handles an Unconfirmed disposal retained beside a newer
    // Closing generation. Current branch provenance cannot relabel this historic manifest.
    manifest.provenance = StudioOverlayProvenance::Unconfirmed {
        provider: f.device.device_id(),
        observed_mls_epoch: 0,
        observed_at_ms: 100,
    };
    refusal(
        crate::store::epoch_studio::overlay::check_closing_disposal_retry(&manifest, basis)
            .unwrap_err(),
        StudioOverlaySaveRefusal::OtherKindDraft,
    );
    let mut corrupted = original.clone();
    let last = corrupted.len() - 1;
    corrupted[last] ^= 1;
    fs::write(&path, corrupted).unwrap();
    assert!(
        store
            .acknowledge_studio_closing_retry(
                SERVER, &f.group, f.target, &f.device, basis, branch, &operation
            )
            .is_err(),
        "a corrupt acceptance cannot authorize an acknowledgement"
    );
}
