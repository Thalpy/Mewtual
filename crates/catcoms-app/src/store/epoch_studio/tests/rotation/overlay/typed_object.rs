//! Typed Index dependencies at P2/H1/H5. Art and Score share a physical object namespace;
//! the validated Index operation chooses only the expected family, never source authority.
use super::*;
use catcoms_replication::studio::{ScoreHeader, ScoreOp, StudioOverlayEligibility as E};

const OBJECT: [u8; 16] = [15; 16];

fn index_branch(f: &Fixture, store: &mut ServerStore, kind: StudioKind) -> [u8; 32] {
    let (close, basis) = closing(f, store);
    let op = f.domain(
        IndexOp::PutObject {
            object: OBJECT,
            kind,
            title: "typed dependency".into(),
            created_by: f.device.device_id(),
            ts: 123,
            expiry: StudioExpiry::Never,
        }
        .encode()
        .unwrap(),
        55,
    );
    save(f, store, &close, basis.fingerprint(), op, 123);
    install(f, store, &close);
    basis.fingerprint()
}

fn create(f: &Fixture, store: &mut ServerStore, kind: StudioKind) -> StudioTarget {
    let target = crate::studio::object_target(f.target.channel(), OBJECT, kind);
    let logical = target.document(&f.group.group_id()).unwrap();
    let body = match kind {
        StudioKind::Flipnote => FlipnoteOp::SetHeader(FlipnoteHeader::Title("art source".into()))
            .encode()
            .unwrap(),
        StudioKind::Score => ScoreOp::SetHeader(ScoreHeader::Title("Score source".into()))
            .encode()
            .unwrap(),
    };
    let operation = DomainOp {
        doc_type: logical.doc_type,
        logical_key: logical.logical_key.clone(),
        nonce: [56; 16],
        body,
    };
    let mut b = budget(store, f);
    let (_, state) = store
        .edit_studio_epoch(
            SERVER,
            &f.group,
            target,
            epoch_zero_id(logical.doc_type, &logical.logical_key),
            &f.device,
            operation,
            123,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    store.retain_studio_source(&f.group, &f.device, state);
    target
}

fn start(
    f: &Fixture,
    store: &mut ServerStore,
    basis: [u8; 32],
) -> crate::store::StudioHandoffStart {
    let mut b = budget(store, f);
    store
        .start_studio_handoff_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            basis,
            Some(0),
            &mut rng(),
            &mut b,
            &mut WriteHooks::None,
        )
        .unwrap()
}

#[test]
fn studio_score_index_classification_and_capture_require_exact_family_cold_and_warm() {
    // Both directions matter: absent wrapper extension means art, never a fallback Score.
    for (claimed, held) in [
        (StudioKind::Score, StudioKind::Flipnote),
        (StudioKind::Flipnote, StudioKind::Score),
    ] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(false);
        let mut store = open(root.path());
        let basis = index_branch(&f, &mut store, claimed);
        let target = create(&f, &mut store, held);
        for cold in [false, true] {
            if cold {
                drop(store);
                store = open(root.path());
            } else {
                assert!(store.studio_source_is_warm(SERVER, &f.group, target, &f.device));
            }
            let before = canonical(&store);
            assert_eq!(
                store
                    .studio_overlay_eligibility(
                        SERVER,
                        &f.group,
                        f.target,
                        &f.device,
                        StudioOwnerTenure::Known(0)
                    )
                    .unwrap(),
                Some(E::Manual(StudioOverlayManualReason::ObjectMissing))
            );
            assert!(matches!(
                start(&f, &mut store, basis),
                crate::store::StudioHandoffStart::Refused(StudioOverlayManualReason::ObjectMissing)
            ));
            assert_eq!(
                canonical(&store),
                before,
                "wrong-kind claim wrote source state"
            );
            assert!(store
                .load_epoch_intents(SERVER, &f.logical)
                .unwrap()
                .overlay()
                .is_some());
        }
    }
}

#[test]
fn studio_score_index_capture_and_commit_transfer_only_while_typed_source_exists() {
    for disappeared in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(false);
        let mut store = open(root.path());
        let basis = index_branch(&f, &mut store, StudioKind::Score);
        let referenced = create(&f, &mut store, StudioKind::Score);
        assert_eq!(
            store
                .studio_overlay_eligibility(
                    SERVER,
                    &f.group,
                    f.target,
                    &f.device,
                    StudioOwnerTenure::Known(0)
                )
                .unwrap(),
            Some(E::Transferable)
        );
        let crate::store::StudioHandoffStart::Captured(capture) = start(&f, &mut store, basis)
        else {
            panic!("H1 refused a valid Score dependency")
        };
        let mut plan = capture.prepare().unwrap();
        assert!(plan
            .sign_slice(&f.device, &f.group, 0, false, usize::MAX, None)
            .unwrap()
            .complete());
        let commit = plan.assemble().unwrap();
        if disappeared {
            let document = referenced.document(&f.group.group_id()).unwrap();
            fs::remove_file(store.studio_epoch_path(&scope_bytes(SERVER, &document).unwrap()))
                .unwrap();
        }
        let before = canonical(&store);
        let mut b = budget(&mut store, &f);
        let result = store.commit_studio_handoff_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            commit,
            Some(0),
            &mut rng(),
            &mut b,
            &mut WriteHooks::None,
        );
        if disappeared {
            let error = result.err().expect("H5 published a dangling Score pointer");
            assert!(
                error.to_string().contains("unavailable Score"),
                "wrong refusal: {error}"
            );
            assert_eq!(canonical(&store), before, "refused H5 wrote source state");
            assert!(store
                .load_epoch_intents(SERVER, &f.logical)
                .unwrap()
                .overlay()
                .is_some());
        } else {
            result.expect("H5 refused a still-present typed Score");
            let StudioProjection::Index(p) = f.load(&store).unwrap().projection().unwrap() else {
                panic!()
            };
            assert!(matches!(
                p.objects[&OBJECT].creations[0].value.kind,
                StudioKind::Score
            ));
            assert!(!p.tombstones.contains_key(&OBJECT));
            drop(store);
            store = open(root.path());
            assert!(store
                .load_studio_epoch(SERVER, &f.group, referenced, &f.device)
                .unwrap()
                .is_some());
        }
    }
}
