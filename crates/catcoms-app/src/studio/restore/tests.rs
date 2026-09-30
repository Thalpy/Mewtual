use super::*;
use catcoms_mls::{MlsDevice, ServerGroup};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::collections::BTreeMap;
use StudioRecoveryDisposition as D;
use StudioRecoveryItem as I;
use StudioRecoveryMode as M;

struct Fixture {
    device: MlsDevice,
    group: ServerGroup,
    target: StudioTarget,
}
impl Fixture {
    fn new(index: bool) -> Self {
        let device = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&device).unwrap();
        let target = if index {
            StudioTarget::Index { channel: [1; 16] }
        } else {
            StudioTarget::Flipnote {
                channel: [1; 16],
                object: [2; 16],
            }
        };
        Self {
            device,
            group,
            target,
        }
    }
    fn projection(&self, bodies: Vec<Vec<u8>>) -> StudioProjection {
        self.projection_of(self.target, bodies)
    }
    /// The same build against another target in the **same** group, for cross-document cases where
    /// only the logical key may differ.
    fn projection_of(&self, target: StudioTarget, bodies: Vec<Vec<u8>>) -> StudioProjection {
        let mut state =
            types::StudioEpoch::new(&self.group, target, self.device.device_id()).unwrap();
        for (n, body) in bodies.into_iter().enumerate() {
            state
                .edit_or_reseal(
                    &self.device,
                    &self.group,
                    &mut ChaCha20Rng::seed_from_u64(91),
                    &domain(target, [(n + 1) as u8; 16], body),
                    n as u64,
                )
                .unwrap();
        }
        state.projection().unwrap()
    }
    fn plan(
        &self,
        now: &StudioProjection,
        old: &StudioProjection,
        item: I,
        mode: M,
    ) -> StudioRecoveryPlan {
        plan(
            now,
            old,
            &[],
            item,
            mode,
            self.device.device_id(),
            PlanScope::SameDocument,
        )
        .unwrap()
    }
}
fn insert(id: u8, after: Option<u8>, cid: u8) -> Vec<u8> {
    FlipnoteOp::InsertFrame {
        frame: [id; 16],
        after: after.map(|n| [n; 16]),
        cid: [cid; 32],
        bytes: 20,
    }
    .encode()
    .unwrap()
}
fn frame(p: &StudioProjection, id: u8) -> I {
    let StudioProjection::Flipnote(p) = p else {
        panic!()
    };
    I::Frame {
        id: [id; 16],
        value: p.frames[&[id; 16]].pixels.selected.source.op_id,
    }
}
fn title(p: &StudioProjection) -> I {
    let StudioProjection::Flipnote(p) = p else {
        panic!()
    };
    I::Title {
        value: p.title.as_ref().unwrap().selected.source.op_id,
    }
}

#[test]
fn studio_restore_uses_selected_pixels_and_resolves_only_missing_anchor_to_end() {
    let f = Fixture::new(false);
    let old = f.projection(vec![
        insert(1, None, 1),
        insert(2, Some(1), 2),
        FlipnoteOp::ReplaceFrame {
            frame: [2; 16],
            cid: [9; 32],
            bytes: 21,
        }
        .encode()
        .unwrap(),
    ]);
    for (now, after) in [
        (f.projection(vec![insert(3, None, 3)]), Some([3; 16])),
        (
            f.projection(vec![insert(1, None, 1), insert(3, Some(1), 3)]),
            Some([1; 16]),
        ),
    ] {
        let result = f.plan(&now, &old, frame(&old, 2), M::Restore);
        assert_eq!(result.disposition, D::Ready);
        assert_eq!(
            FlipnoteOp::decode(result.body.as_ref().unwrap()).unwrap(),
            FlipnoteOp::InsertFrame {
                frame: [2; 16],
                after,
                cid: [9; 32],
                bytes: 21
            }
        );
        assert_eq!(result.original_author, Some(f.device.device_id()));
    }
}

#[test]
fn studio_restore_is_additive_copy_is_explicit_and_fork_deletions_never_auto_apply() {
    let f = Fixture::new(false);
    let old = f.projection(vec![
        insert(1, None, 1),
        FlipnoteOp::SetHeader(FlipnoteHeader::Title("old".into()))
            .encode()
            .unwrap(),
    ]);
    let now = f.projection(vec![
        insert(1, None, 8),
        FlipnoteOp::SetHeader(FlipnoteHeader::Title("current".into()))
            .encode()
            .unwrap(),
    ]);
    for item in [frame(&old, 1), title(&old)] {
        assert_eq!(
            f.plan(&now, &old, item, M::Restore).disposition,
            D::Conflict
        );
        assert_eq!(f.plan(&now, &old, item, M::Copy).disposition, D::Ready);
        assert_eq!(
            f.plan(&old, &old, item, M::Restore).disposition,
            D::Unchanged
        );
    }
    let deleted = f.projection(vec![
        insert(1, None, 1),
        FlipnoteOp::RemoveFrame { frame: [1; 16] }.encode().unwrap(),
    ]);
    let item = I::FrameDeletion { id: [1; 16] };
    assert_eq!(
        f.plan(&now, &deleted, item, M::Restore).disposition,
        D::Conflict
    );
    assert_eq!(f.plan(&now, &deleted, item, M::Copy).disposition, D::Ready);
    assert_eq!(
        f.plan(&deleted, &old, frame(&old, 1), M::Copy).disposition,
        D::Deleted
    );
    let saved = StudioRecovery::snapshot(
        &deleted,
        None,
        RecoveryReason::Excluded,
        [1; 32],
        &BTreeMap::new(),
    )
    .unwrap();
    let history = StudioRecovery::from_snapshot(&saved, old.document(), old.channel()).unwrap();
    let empty = f.projection(vec![]);
    for mode in [M::Restore, M::Copy] {
        assert_eq!(
            plan(
                &empty,
                &old,
                &[StudioRecovery::from_snapshot(&saved, old.document(), old.channel()).unwrap()],
                frame(&old, 1),
                mode,
                f.device.device_id(),
                PlanScope::SameDocument,
            )
            .unwrap()
            .disposition,
            D::Deleted
        );
    }
    assert!(history.projection().recovery_fingerprint().is_ok());
}

#[test]
fn studio_restore_index_uses_restorer_author_and_never_replaces_immutable_creation() {
    let f = Fixture::new(true);
    let birth = IndexOp::PutObject {
        object: [4; 16],
        kind: StudioKind::Flipnote,
        title: "old".into(),
        created_by: f.device.device_id(),
        ts: 123,
        expiry: StudioExpiry::Never,
    };
    let old = f.projection(vec![birth.encode().unwrap()]);
    let current = f.projection(vec![]);
    let restorer = crate::DeviceId::from_bytes([9; 32]);
    let result = plan(
        &current,
        &old,
        &[],
        I::Object { id: [4; 16] },
        M::Restore,
        restorer,
        PlanScope::SameDocument,
    )
    .unwrap();
    let IndexOp::PutObject { created_by, ts, .. } =
        IndexOp::decode(result.body.as_ref().unwrap()).unwrap()
    else {
        panic!()
    };
    assert_eq!((created_by, ts), (restorer, 123));
    assert_eq!(result.original_author, Some(f.device.device_id()));
    let edited = f.projection(vec![
        birth.encode().unwrap(),
        IndexOp::SetTitle {
            object: [4; 16],
            title: "new".into(),
        }
        .encode()
        .unwrap(),
    ]);
    assert_eq!(
        f.plan(&edited, &old, I::Object { id: [4; 16] }, M::Copy)
            .disposition,
        D::Conflict
    );
    let StudioProjection::Index(p) = &old else {
        panic!()
    };
    let item = I::ObjectTitle {
        id: [4; 16],
        value: p.objects[&[4; 16]].title.selected.source.op_id,
    };
    assert_eq!(
        f.plan(&edited, &old, item, M::Restore).disposition,
        D::Conflict
    );
    assert_eq!(f.plan(&edited, &old, item, M::Copy).disposition, D::Ready);
}

#[test]
fn studio_restore_preview_fingerprint_tracks_provenance_not_just_visible_content() {
    let f = Fixture::new(false);
    let body = FlipnoteOp::SetHeader(FlipnoteHeader::Title("same".into()))
        .encode()
        .unwrap();
    let a = f.projection(vec![body.clone()]);
    let b = f.projection(vec![body.clone(), body]);
    assert_ne!(
        a.recovery_fingerprint().unwrap(),
        b.recovery_fingerprint().unwrap()
    );
    assert!(plan(
        &a,
        &b,
        &[],
        I::Title { value: [99; 32] },
        M::Copy,
        f.device.device_id(),
        PlanScope::SameDocument,
    )
    .is_err());
    let other = Fixture::new(false).projection(vec![]);
    assert!(plan(
        &a,
        &other,
        &[],
        title(&a),
        M::Copy,
        f.device.device_id(),
        PlanScope::SameDocument,
    )
    .is_err());
}

/// **CrossDocument must refuse a foreign server**, and the refusal must be the scope check itself.
///
/// `channel()` is the 16-byte channel id and does not name a server, so two unrelated groups can
/// both hold channel `[1; 16]`: a relaxation written against the channel alone would admit a
/// foreign group's document as a copy source.
///
/// The foreign document is given a **real title of its own**, and the item names that title's op id.
/// A first version of this test used an empty foreign projection and asserted only `is_err()`; it
/// passed with the scope check deleted, because the arm was failing at "missing recovery title"
/// instead. Asserting the message is what makes the case reach the guard it names.
#[test]
fn studio_restore_cross_document_copy_refuses_a_foreign_server_at_the_scope_check() {
    let f = Fixture::new(false);
    let body = |t: &str| {
        FlipnoteOp::SetHeader(FlipnoteHeader::Title(t.into()))
            .encode()
            .unwrap()
    };
    let mine = f.projection(vec![body("mine")]);
    let theirs = Fixture::new(false).projection(vec![body("theirs")]);
    assert_ne!(mine.document().server_id, theirs.document().server_id);
    assert_eq!(
        mine.channel(),
        theirs.channel(),
        "the case only bites when the channel ids collide"
    );

    // Resolvable in `theirs`: without the scope check this plan would succeed.
    let item = title(&theirs);
    for scope in [PlanScope::SameDocument, PlanScope::CrossDocument] {
        let error = plan(
            &mine,
            &theirs,
            &[],
            item,
            M::Copy,
            f.device.device_id(),
            scope,
        )
        .expect_err("a copy must never reach outside its own server");
        assert!(
            error.to_string().contains("document scope differs"),
            "{scope:?} refused for the wrong reason: {error}"
        );
    }
}

#[test]
fn studio_restore_copy_count_predicate_matches_local_admission_even_for_a_playable_frame() {
    let f = Fixture::new(false);
    let old = f.projection(vec![insert(1, None, 1)]);
    let mut current = f.projection(vec![insert(1, None, 2)]);
    let StudioProjection::Flipnote(art) = &mut current else {
        panic!()
    };
    // Isolate the preview count predicate. The replication reader/admission integration suite
    // separately builds actual concurrent over-cap projections; no fake signed change is used
    // here or admitted by this test. The selected frame remains inside the playable prefix.
    art.timeline = (1u128..=1000).map(u128::to_be_bytes).collect();
    assert!(!art.over_cap.contains_key(&[1; 16]));
    assert_eq!(
        f.plan(&current, &old, frame(&old, 1), M::Copy).disposition,
        D::Full
    );
}

/// `source_ops` reports the ids the plan actually resolved, and nothing else.
///
/// The field exists because an earlier revision let the *caller* say what a proposal consumed,
/// which allowed a request to claim work it never read. Deriving it here is only worth anything if
/// it is derived from the resolved values, so this pins the ids to the exact registers the arms
/// read: one for a frame, and three for an object put, which resolves a creation, a selected title
/// and a selected expiry.
#[test]
fn studio_restore_source_ops_name_exactly_what_the_plan_resolved() {
    // The frame is INSERTED and then REPLACED, so the pixels register and the insertion record have
    // different op ids. With a single InsertFrame they coincide and "the pixels, not the insertion"
    // is not a distinguishable claim - the shape a review caught this test in.
    let f = Fixture::new(false);
    let old = f.projection(vec![
        insert(1, None, 1),
        FlipnoteOp::ReplaceFrame {
            frame: [1; 16],
            cid: [9; 32],
            bytes: 21,
        }
        .encode()
        .unwrap(),
    ]);
    let empty = f.projection(vec![]);
    let StudioProjection::Flipnote(p) = &old else {
        panic!()
    };
    let pixels = p.frames[&[1; 16]].pixels.selected.source.op_id;
    let insertion = p.frames[&[1; 16]].insertions.first().unwrap().source.op_id;
    assert_ne!(
        pixels, insertion,
        "the fixture must separate the two, or the assertion below is vacuous"
    );
    let ready = f.plan(&empty, &old, frame(&old, 1), M::Copy);
    assert_eq!(ready.disposition, D::Ready);
    assert_eq!(
        ready.source_ops,
        vec![pixels],
        "a frame copy consumes the pixels register it resolved, and not the insertion whose \
         position it merely read"
    );

    // A held plan consumed nothing, and must not report otherwise: carrying ids for a proposal that
    // will never be offered would report work read on behalf of a refusal.
    let unchanged = f.plan(&old, &old, frame(&old, 1), M::Copy);
    assert_eq!(unchanged.disposition, D::Unchanged);
    assert!(unchanged.source_ops.is_empty());

    // An object whose title and expiry were SEPARATELY set, so the three registers have three
    // distinct sources. With a bare PutObject they all fall back to the creating operation, and
    // replacing all three ids with the birth op survives - which is how a review found this test
    // proving nothing.
    let index = Fixture::new(true);
    let old = index.projection(vec![
        IndexOp::PutObject {
            object: [4; 16],
            kind: StudioKind::Flipnote,
            title: "old".into(),
            created_by: index.device.device_id(),
            ts: 123,
            expiry: StudioExpiry::Never,
        }
        .encode()
        .unwrap(),
        IndexOp::SetTitle {
            object: [4; 16],
            title: "retitled".into(),
        }
        .encode()
        .unwrap(),
        IndexOp::SetExpiry {
            object: [4; 16],
            expiry: StudioExpiry::Never,
        }
        .encode()
        .unwrap(),
    ]);
    let StudioProjection::Index(p) = &old else {
        panic!()
    };
    let entry = &p.objects[&[4; 16]];
    let expected = vec![
        entry.creations.first().unwrap().source.op_id,
        entry.title.selected.source.op_id,
        entry.expiry.selected.source.op_id,
    ];
    assert_eq!(
        expected
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3,
        "the fixture must give the three registers three distinct sources"
    );
    let ready = index.plan(
        &index.projection(vec![]),
        &old,
        I::Object { id: [4; 16] },
        M::Restore,
    );
    assert_eq!(ready.disposition, D::Ready);
    assert_eq!(
        ready.source_ops, expected,
        "an object put resolves three values, so it must report three ids"
    );

    // And when they are NOT separately set, all three registers fall back to the creating
    // operation and the plan must report that one id once. Reporting it three times would say the
    // proposal consumed three operations when it consumed one.
    let bare = index.projection(vec![IndexOp::PutObject {
        object: [5; 16],
        kind: StudioKind::Flipnote,
        title: "bare".into(),
        created_by: index.device.device_id(),
        ts: 123,
        expiry: StudioExpiry::Never,
    }
    .encode()
    .unwrap()]);
    let StudioProjection::Index(p) = &bare else {
        panic!()
    };
    let birth = p.objects[&[5; 16]].creations.first().unwrap().source.op_id;
    let ready = index.plan(
        &index.projection(vec![]),
        &bare,
        I::Object { id: [5; 16] },
        M::Restore,
    );
    assert_eq!(ready.disposition, D::Ready);
    assert_eq!(
        ready.source_ops,
        vec![birth],
        "three registers backed by one operation are one consumed operation"
    );
}

/// A cross-document copy within one server is permitted, and every destination check still applies.
///
/// Two Flipnotes in the same group differing only in logical key: exactly the case `CrossDocument`
/// exists for. The `SameDocument` refusal on the same inputs is asserted in the same run, so the
/// test cannot pass by making both scopes behave alike.
#[test]
fn studio_restore_cross_document_copy_is_permitted_only_within_one_server() {
    let f = Fixture::new(false);
    let source = f.projection(vec![insert(1, None, 1)]);
    let destination = f.projection_of(
        StudioTarget::Flipnote {
            channel: [1; 16],
            object: [8; 16],
        },
        vec![],
    );
    assert_ne!(source.document(), destination.document());
    assert_eq!(
        source.document().server_id,
        destination.document().server_id
    );

    let item = frame(&source, 1);
    assert!(
        plan(
            &destination,
            &source,
            &[],
            item,
            M::Copy,
            f.device.device_id(),
            PlanScope::SameDocument,
        )
        .is_err(),
        "recovery must never span two documents"
    );
    let ready = plan(
        &destination,
        &source,
        &[],
        item,
        M::Copy,
        f.device.device_id(),
        PlanScope::CrossDocument,
    )
    .expect("a copy between two Flipnotes of one server is the primary cross-document case");
    assert_eq!(ready.disposition, D::Ready);

    // Restore mode is still refused for an absent frame whose chosen value is not the selected one,
    // and capacity is still the destination's: the relaxation drops the logical key and nothing
    // else. A full destination refuses even though the source is tiny.
    let mut full = destination.clone();
    let StudioProjection::Flipnote(art) = &mut full else {
        panic!()
    };
    art.timeline = (1u128..=1000).map(u128::to_be_bytes).collect();
    assert_eq!(
        plan(
            &full,
            &source,
            &[],
            item,
            M::Copy,
            f.device.device_id(),
            PlanScope::CrossDocument,
        )
        .unwrap()
        .disposition,
        D::Full,
        "capacity is a property of where the work is going"
    );
}
