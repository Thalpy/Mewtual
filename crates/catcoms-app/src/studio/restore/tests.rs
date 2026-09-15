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
        let mut state =
            types::StudioEpoch::new(&self.group, self.target, self.device.device_id()).unwrap();
        for (n, body) in bodies.into_iter().enumerate() {
            state
                .edit_or_reseal(
                    &self.device,
                    &self.group,
                    &mut ChaCha20Rng::seed_from_u64(91),
                    &domain(self.target, [(n + 1) as u8; 16], body),
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
        plan(now, old, &[], item, mode, self.device.device_id()).unwrap()
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
                f.device.device_id()
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
        f.device.device_id()
    )
    .is_err());
    let other = Fixture::new(false).projection(vec![]);
    assert!(plan(&a, &other, &[], title(&a), M::Copy, f.device.device_id()).is_err());
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
