//! Real signatures over PUBLIC attestation/math fixtures. These tests establish no actual MLS
//! execution, trusted root/prefix, cold lookup, prepaid control or projection/native authority.
use super::*;
use crate::chat::acceptance_tree::TreeHash;
use crate::chat::owner_bodies::{AppendSpec, RemovalSpec};
use crate::chat::owner_header::{HeaderSpec, OwnerTenureId};
use crate::chat::owner_profile::{AdmissionSpec, ProfileSpec};
use catcoms_crypto::DeviceKeypair;

mod admission_prepare;
mod append_prepare;

const GROUP: &[u8] = b"body-transition-math-real-signatures";

#[test]
fn replay_debug_omits_slot_frontier_and_transition_payloads() {
    let (key, base) = genesis();
    assert_eq!(
        format!("{:?}", base.slots().next().unwrap()),
        "SlotData { .. }"
    );
    let (body, header, image) = add(&base, &key, [91; 32], 92);
    assert_eq!(
        format!("{:?}", ReplayBody::Admission(&body)),
        "ReplayBody::Admission { .. }"
    );
    let step = base
        .derive_successor(&header, ReplayBody::Admission(&body), &image)
        .unwrap();
    assert_eq!(format!("{step:?}"), "StepData { .. }");
}

fn sign(key: &DeviceKeypair, image: &[u8]) -> SignedOwnerHeader {
    let frame = StateFrame::decode(image).unwrap();
    let m = frame.meta();
    SignedOwnerHeader::sign_using(
        HeaderSpec {
            group: m.group.to_vec(),
            profile: m.profile,
            issuer: m.issuer,
            tenure: m.tenure,
            parent: m.parent,
            sequence: m.sequence,
            kind: m.kind,
            body: m.body,
            state: frame.commitment(),
        },
        |bytes| Ok(key.sign(bytes)),
    )
    .unwrap()
}
fn genesis() -> (DeviceKeypair, ReplayMath) {
    let key = DeviceKeypair::from_seed(&[8; 32]);
    let profile = SignedOwnerProfile::sign_using(
        ProfileSpec {
            group: GROUP.to_vec(),
            founder: key.verifying_key().to_bytes(),
            creation: [9; 32],
        },
        |bytes| Ok(key.sign(bytes)),
    )
    .unwrap();
    let body = AdmissionBody::new(AdmissionSpec {
        group: GROUP.to_vec(),
        profile: profile.id(),
        subject: profile.spec().founder,
        origin: AdmissionOrigin::Founder {
            creation: profile.spec().creation,
        },
    })
    .unwrap();
    let context = TreeContext {
        profile: profile.id(),
        admission: body.admission_id(),
    };
    let slot = CurrentAdmissionSlot {
        admission: body.id(),
        subject: profile.spec().founder,
        tip_body: body.id(),
        accepted_count: 0,
        accepted_root: context.empty_hash(),
    };
    let image = encode_state(
        StateMeta {
            group: GROUP,
            profile: profile.id(),
            issuer: profile.spec().founder,
            tenure: profile.tenure(),
            parent: profile.genesis_parent(),
            previous: virtual_genesis_state(&profile).unwrap(),
            sequence: 1,
            kind: HeaderKind::Admission,
            body: body.id(),
            mls_epoch: 0,
        },
        &[slot],
    )
    .unwrap();
    let header = sign(&key, &image);
    (
        key,
        ReplayMath::from_genesis(&profile, &body, &header, &image).unwrap(),
    )
}
fn received(
    base: &ReplayMath,
    key: &DeviceKeypair,
    kind: HeaderKind,
    body: OwnerBodyId,
    epoch: u64,
    mut slots: Vec<CurrentAdmissionSlot>,
) -> (SignedOwnerHeader, Vec<u8>) {
    slots.sort_by_key(|slot| slot.admission);
    let image = encode_state(
        StateMeta {
            group: GROUP,
            profile: base.profile.id(),
            issuer: base.profile.spec().founder,
            tenure: base.profile.tenure(),
            parent: base.header.id(),
            previous: base.header.spec().state,
            sequence: base.header.spec().sequence.checked_add(1).unwrap(),
            kind,
            body,
            mls_epoch: epoch,
        },
        &slots,
    )
    .unwrap();
    let header = sign(key, &image);
    (header, image)
}
fn add(
    base: &ReplayMath,
    key: &DeviceKeypair,
    subject: [u8; 32],
    marker: u8,
) -> (AdmissionBody, SignedOwnerHeader, Vec<u8>) {
    let body = AdmissionBody::new(AdmissionSpec {
        group: GROUP.to_vec(),
        profile: base.profile.id(),
        subject,
        origin: AdmissionOrigin::Add {
            commit: [marker; 32],
            before_epoch: base.epoch,
            after_epoch: base.epoch + 1,
            key_package: [marker.wrapping_add(1); 32],
            credential: [marker.wrapping_add(2); 32],
        },
    })
    .unwrap();
    let context = TreeContext {
        profile: base.profile.id(),
        admission: body.admission_id(),
    };
    let mut slots: Vec<_> = base.slots().map(|slot| slot.image).collect();
    slots.push(CurrentAdmissionSlot {
        admission: body.id(),
        subject,
        tip_body: body.id(),
        accepted_count: 0,
        accepted_root: context.empty_hash(),
    });
    let (header, image) = received(
        base,
        key,
        HeaderKind::Admission,
        body.id(),
        base.epoch + 1,
        slots,
    );
    (body, header, image)
}
fn append(
    base: &ReplayMath,
    key: &DeviceKeypair,
    admission: OwnerBodyId,
    marker: u8,
) -> (AppendBody, SignedOwnerHeader, Vec<u8>) {
    let old = base.slot(admission).unwrap();
    let leaves = vec![AcceptanceLeaf {
        channel: 7,
        acceptance: OwnerFactId::from_bytes([marker; 32]),
    }];
    let prepared = old.frontier.prepare_append(&leaves).unwrap();
    let body = AppendBody::new(AppendSpec {
        group: GROUP.to_vec(),
        previous_tip: old.tip_header,
        old_tip: prepared.old_tip(),
        new_tip: prepared.new_tip(),
        leaves,
    })
    .unwrap();
    let mut slots: Vec<_> = base.slots().map(|slot| slot.image).collect();
    let changed = slots
        .iter_mut()
        .find(|slot| slot.admission == admission)
        .unwrap();
    changed.tip_body = body.id();
    changed.accepted_count = prepared.new_tip().count;
    changed.accepted_root = prepared.new_tip().root;
    let (header, image) = received(
        base,
        key,
        HeaderKind::AcceptanceAppend,
        body.id(),
        base.epoch,
        slots,
    );
    (body, header, image)
}
fn remove(
    base: &ReplayMath,
    key: &DeviceKeypair,
    admission: OwnerBodyId,
) -> (RemovalBody, SignedOwnerHeader, Vec<u8>) {
    let old = base.slot(admission).unwrap();
    let body = RemovalBody::new(RemovalSpec {
        group: GROUP.to_vec(),
        subject: old.image.subject,
        commit: [71; 32],
        before_epoch: base.epoch,
        after_epoch: base.epoch + 1,
        tip_header: old.tip_header,
        frozen_tip: old.frontier.tip(),
    })
    .unwrap();
    let slots = base
        .slots()
        .filter(|slot| slot.image.admission != admission)
        .map(|slot| slot.image)
        .collect();
    let (header, image) = received(
        base,
        key,
        HeaderKind::Removal,
        body.id(),
        base.epoch + 1,
        slots,
    );
    (body, header, image)
}

#[test]
fn replay_math_interleaving_preserves_each_admissions_own_header_tip_and_frontier() {
    let (key, base) = genesis();
    let founder = base.slots().next().unwrap().image.admission;
    let founder_tip = base.slot(founder).unwrap().tip_header;
    let (a, ah, ai) = add(&base, &key, [21; 32], 21);
    let with_a = base
        .derive_successor(&ah, ReplayBody::Admission(&a), &ai)
        .unwrap()
        .next;
    let (b, bh, bi) = add(&with_a, &key, [22; 32], 22);
    let with_b = with_a
        .derive_successor(&bh, ReplayBody::Admission(&b), &bi)
        .unwrap()
        .next;
    assert_eq!(with_b.slot(a.id()).unwrap().tip_header, ah.id());
    let (aa, aah, aai) = append(&with_b, &key, a.id(), 30);
    let after_a = with_b
        .derive_successor(&aah, ReplayBody::Append(&aa), &aai)
        .unwrap();
    assert_eq!(after_a.append.as_ref().unwrap().created_nodes().len(), 1);
    assert_eq!(after_a.next.slot(b.id()).unwrap().tip_header, bh.id());
    assert_eq!(after_a.next.slot(founder).unwrap().tip_header, founder_tip);
    let (ba, bah, bai) = append(&after_a.next, &key, b.id(), 31);
    let after_b = after_a
        .next
        .derive_successor(&bah, ReplayBody::Append(&ba), &bai)
        .unwrap()
        .next;
    assert_eq!(after_b.slot(a.id()).unwrap().tip_header, aah.id());
    assert_eq!(after_b.slot(b.id()).unwrap().tip_header, bah.id());
    assert_eq!(after_b.slot(founder).unwrap().frontier.tip().count, 0);
    assert_eq!(
        StateFrame::decode(after_b.image())
            .unwrap()
            .meta()
            .mls_epoch,
        2
    );
    let (cut, ch, ci) = remove(&after_b, &key, a.id());
    assert_eq!(
        cut.spec().tip_header,
        aah.id(),
        "cut binds A's own tip, not B's later global header"
    );
    let mut wrong = cut.spec().clone();
    wrong.tip_header = bah.id();
    let wrong = RemovalBody::new(wrong).unwrap();
    let (wh, wi) = received(
        &after_b,
        &key,
        HeaderKind::Removal,
        wrong.id(),
        after_b.epoch + 1,
        after_b
            .slots()
            .filter(|slot| slot.image.admission != a.id())
            .map(|slot| slot.image)
            .collect(),
    );
    assert!(after_b
        .derive_successor(&wh, ReplayBody::Removal(&wrong), &wi)
        .is_err());
    let removed = after_b
        .derive_successor(&ch, ReplayBody::Removal(&cut), &ci)
        .unwrap();
    assert_eq!(removed.removal.as_ref().unwrap().tip_header, aah.id());
    assert_eq!(removed.next.slot(b.id()).unwrap().tip_header, bah.id());
    assert_eq!(
        base.header().spec().sequence,
        1,
        "immutable derivation never advances its base"
    );
    assert_eq!(base.slots().len(), 1);
}

#[test]
fn replay_math_requires_own_previous_header_and_rejects_changed_unchanged_rows_and_epoch() {
    let (key, base) = genesis();
    let (a, ah, ai) = add(&base, &key, [21; 32], 21);
    let with_a = base
        .derive_successor(&ah, ReplayBody::Admission(&a), &ai)
        .unwrap()
        .next;
    let (b, bh, bi) = add(&with_a, &key, [22; 32], 22);
    let with_b = with_a
        .derive_successor(&bh, ReplayBody::Admission(&b), &bi)
        .unwrap()
        .next;
    let (good, _, _) = append(&with_b, &key, a.id(), 30);
    let mut wrong = good.spec().clone();
    wrong.previous_tip = bh.id();
    let wrong = AppendBody::new(wrong).unwrap();
    let mut slots: Vec<_> = with_b.slots().map(|slot| slot.image).collect();
    let target = slots
        .iter_mut()
        .find(|slot| slot.admission == a.id())
        .unwrap();
    target.tip_body = wrong.id();
    target.accepted_count = wrong.spec().new_tip.count;
    target.accepted_root = wrong.spec().new_tip.root;
    let (header, image) = received(
        &with_b,
        &key,
        HeaderKind::AcceptanceAppend,
        wrong.id(),
        with_b.epoch,
        slots,
    );
    assert!(with_b
        .derive_successor(&header, ReplayBody::Append(&wrong), &image)
        .is_err());
    let (c, ch, ci) = add(&with_b, &key, [23; 32], 23);
    let parsed = StateFrame::decode(&ci).unwrap();
    let mut slots: Vec<_> = parsed.slots().collect();
    let changed = slots
        .iter_mut()
        .find(|slot| slot.admission == a.id())
        .unwrap();
    changed.tip_body = OwnerBodyId::from_bytes([99; 32]);
    changed.accepted_count = 1;
    changed.accepted_root = TreeHash::from_bytes([98; 32]);
    let bad = encode_state(parsed.meta(), &slots).unwrap();
    let signed = sign(&key, &bad);
    assert!(StateFrame::decode(&bad)
        .unwrap()
        .verify_successor_binding(&signed, &with_b.header)
        .is_ok());
    assert!(
        with_b
            .derive_successor(&signed, ReplayBody::Admission(&c), &bad)
            .is_err(),
        "whole expected state must reject tampered UNCHANGED row"
    );
    let mut meta = StateFrame::decode(&ci).unwrap().meta();
    meta.mls_epoch += 1;
    let wrong_epoch = encode_state(
        meta,
        &StateFrame::decode(&ci).unwrap().slots().collect::<Vec<_>>(),
    )
    .unwrap();
    let wrong_header = sign(&key, &wrong_epoch);
    assert!(with_b
        .derive_successor(&wrong_header, ReplayBody::Admission(&c), &wrong_epoch)
        .is_err());
    assert!(with_b
        .derive_successor(&ch, ReplayBody::Admission(&c), &ci)
        .is_ok());
    assert_eq!(with_b.slots().len(), 3);
}

#[test]
fn replay_math_cut_is_exact_and_same_key_rejoin_never_reopens_old_incarnation() {
    let (key, base) = genesis();
    let (a, ah, ai) = add(&base, &key, [21; 32], 21);
    let source = base
        .derive_successor(&ah, ReplayBody::Admission(&a), &ai)
        .unwrap()
        .next;
    let (aa, aah, aai) = append(&source, &key, a.id(), 31);
    let source = source
        .derive_successor(&aah, ReplayBody::Append(&aa), &aai)
        .unwrap()
        .next;
    let (cut, ch, ci) = remove(&source, &key, a.id());
    let removed = source
        .derive_successor(&ch, ReplayBody::Removal(&cut), &ci)
        .unwrap();
    let retained = removed.removal.as_ref().unwrap();
    assert_eq!(retained.tip_header, aah.id());
    assert_eq!(retained.tip.count, 1);
    assert!(removed.next.slot(a.id()).is_none());
    let (a2, a2h, a2i) = add(&removed.next, &key, [21; 32], 22);
    let rejoined = removed
        .next
        .derive_successor(&a2h, ReplayBody::Admission(&a2), &a2i)
        .unwrap()
        .next;
    assert_ne!(a.id(), a2.id());
    assert_eq!(rejoined.slot(a2.id()).unwrap().frontier.tip().count, 0);
    let (old, _, _) = append(&source, &key, a.id(), 32);
    let unchanged = rejoined.slots().map(|slot| slot.image).collect();
    let (oldh, oldi) = received(
        &rejoined,
        &key,
        HeaderKind::AcceptanceAppend,
        old.id(),
        rejoined.epoch,
        unchanged,
    );
    assert!(rejoined
        .derive_successor(&oldh, ReplayBody::Append(&old), &oldi)
        .is_err());
    let mut wrong = cut.spec().clone();
    wrong.before_epoch = rejoined.epoch;
    wrong.after_epoch = rejoined.epoch + 1;
    let wrong = RemovalBody::new(wrong).unwrap();
    let (oldh, oldi) = received(
        &rejoined,
        &key,
        HeaderKind::Removal,
        wrong.id(),
        rejoined.epoch + 1,
        rejoined.slots().map(|slot| slot.image).collect(),
    );
    assert!(rejoined
        .derive_successor(&oldh, ReplayBody::Removal(&wrong), &oldi)
        .is_err());
    assert!(rejoined.slot(a2.id()).is_some());
}

#[test]
fn replay_math_invalid_signature_foreign_scope_and_retry_never_mutate_or_poison_base() {
    let (key, base) = genesis();
    let (a, header, image) = add(&base, &key, [21; 32], 21);
    let mut invalid = header.encode();
    *invalid.last_mut().unwrap() ^= 1;
    let invalid = SignedOwnerHeader::decode(&invalid).unwrap();
    assert_eq!(invalid.id(), header.id());
    assert!(!invalid.verify());
    assert!(matches!(
        base.derive_successor(&invalid, ReplayBody::Admission(&a), &image),
        Err(ChatError::Signature)
    ));
    for field in 0..4 {
        let mut spec = header.spec().clone();
        match field {
            0 => spec.parent = OwnerHeaderId::from_bytes([77; 32]),
            1 => spec.tenure = OwnerTenureId::from_bytes([78; 32]),
            2 => spec.profile = OwnerFactId::from_bytes([79; 32]),
            _ => spec.group = b"foreign".to_vec(),
        }
        let foreign = SignedOwnerHeader::sign_using(spec, |bytes| Ok(key.sign(bytes))).unwrap();
        assert!(base
            .derive_successor(&foreign, ReplayBody::Admission(&a), &image)
            .is_err());
    }
    let one = base
        .derive_successor(&header, ReplayBody::Admission(&a), &image)
        .unwrap();
    let two = base
        .derive_successor(&header, ReplayBody::Admission(&a), &image)
        .unwrap();
    assert_eq!(one.next.image(), two.next.image());
    assert_eq!(base.header.spec().sequence, 1);
    assert_eq!(
        base.slots().len(),
        1,
        "this is pure same-base computation, not a durable exact-result cache"
    );
}

#[test]
fn replay_math_current_64_limit_is_not_a_lifetime_admission_limit() {
    let (key, mut source) = genesis();
    for id in 1..64 {
        let (a, h, i) = add(&source, &key, [id; 32], id);
        source = source
            .derive_successor(&h, ReplayBody::Admission(&a), &i)
            .unwrap()
            .next;
    }
    assert_eq!(source.slots().len(), 64);
    let full = AdmissionBody::new(AdmissionSpec {
        group: GROUP.to_vec(),
        profile: source.profile.id(),
        subject: [100; 32],
        origin: AdmissionOrigin::Add {
            commit: [100; 32],
            before_epoch: source.epoch,
            after_epoch: source.epoch + 1,
            key_package: [101; 32],
            credential: [102; 32],
        },
    })
    .unwrap();
    let (h, i) = received(
        &source,
        &key,
        HeaderKind::Admission,
        full.id(),
        source.epoch + 1,
        source.slots().map(|slot| slot.image).collect(),
    );
    assert!(matches!(
        source.derive_successor(&h, ReplayBody::Admission(&full), &i),
        Err(ChatError::Bound)
    ));
    for turn in 0..130 {
        let admission = source
            .slots()
            .find(|slot| slot.image.subject == [1; 32])
            .unwrap()
            .image
            .admission;
        let (cut, h, i) = remove(&source, &key, admission);
        source = source
            .derive_successor(&h, ReplayBody::Removal(&cut), &i)
            .unwrap()
            .next;
        let (a, h, i) = add(&source, &key, [1; 32], (turn % 200) as u8);
        source = source
            .derive_successor(&h, ReplayBody::Admission(&a), &i)
            .unwrap()
            .next;
        assert_eq!(source.slots().len(), 64);
        assert_eq!(source.slots.capacity(), MAX_SLOTS);
    }
    // The actual repeated slot/capacity checks above are dynamic; this named ABI invoice
    // relationship is fixed for the build and must hold before any test can execute.
    const { assert!(NAMED_LAYOUT_BYTES <= REPLAY_MATH_PRIVATE_INVOICE) };
}

#[test]
fn replay_math_current_issuer_removal_is_unsupported_even_with_signed_consistent_image() {
    let (key, source) = genesis();
    let issuer = source.slots().next().unwrap();
    let cut = RemovalBody::new(RemovalSpec {
        group: GROUP.to_vec(),
        subject: source.profile.spec().founder,
        commit: [80; 32],
        before_epoch: 0,
        after_epoch: 1,
        tip_header: issuer.tip_header,
        frozen_tip: issuer.frontier.tip(),
    })
    .unwrap();
    let (h, i) = received(
        &source,
        &key,
        HeaderKind::Removal,
        cut.id(),
        1,
        source.slots().map(|slot| slot.image).collect(),
    );
    assert!(matches!(
        source.derive_successor(&h, ReplayBody::Removal(&cut), &i),
        Err(ChatError::Authority)
    ));
    assert_eq!(source.slots().len(), 1);
}

#[test]
fn replay_math_review_and_signed_fork_candidates_never_create_selection_or_lookup_authority() {
    use crate::chat::owner_bodies::{AcceptanceRecord, AcceptanceSpec, ReviewDecision, ReviewSpec};
    use crate::chat::{ActionId, BasisId, GrantId};
    let (key, source) = genesis();
    let (a, ah, ai) = add(&source, &key, [21; 32], 21);
    let (b, bh, bi) = add(&source, &key, [22; 32], 22);
    assert_eq!(ah.spec().parent, bh.spec().parent);
    assert_eq!(ah.spec().sequence, bh.spec().sequence);
    assert_ne!(ah.id(), bh.id());
    assert!(source
        .derive_successor(&ah, ReplayBody::Admission(&b), &ai)
        .is_err());
    assert!(source
        .derive_successor(&ah, ReplayBody::Admission(&a), &ai)
        .is_ok());
    assert!(source
        .derive_successor(&bh, ReplayBody::Admission(&b), &bi)
        .is_ok());
    assert_eq!(
        source.header.spec().sequence,
        1,
        "both are mathematical candidates; no first-arrival winner or fork clearing occurs"
    );
    let slot = source.slots().next().unwrap();
    let record = AcceptanceRecord::new(AcceptanceSpec {
        group: GROUP.to_vec(),
        context: slot.frontier.tip().context,
        channel: 7,
        action: ActionId::from_bytes([10; 32]),
        subject: slot.image.subject,
        grant: GrantId::from_bytes([11; 32]),
        basis: BasisId::from_bytes([12; 32]),
        create: None,
    })
    .unwrap();
    let review = ReviewDecisionBody::new(ReviewSpec {
        target: record,
        intent: [13; 16],
        decision: ReviewDecision::Approve,
        previous: None,
    })
    .unwrap();
    let (h, i) = received(
        &source,
        &key,
        HeaderKind::ReviewDecision,
        review.id(),
        0,
        source.slots().map(|slot| slot.image).collect(),
    );
    assert!(
        matches!(
            source.derive_successor(&h, ReplayBody::Review(&review), &i),
            Err(ChatError::Dependency)
        ),
        "signature/binding cannot fabricate selected-case or already-accepted lookup absence"
    );
    assert_eq!(source.header.spec().sequence, 1);
}

#[test]
fn replay_math_sequence_boundary_is_checked_arithmetic_not_prepaid_control_space() {
    let (key, mut source) = genesis();
    // Internal arithmetic fixture ONLY: no public snapshot initializer or claimed complete
    // selected history makes this enormous prefix. Actual signing must prepay controls first.
    let mut meta = StateFrame::decode(&source.image).unwrap().meta();
    meta.sequence = u64::MAX - 1;
    source.image = encode_state(
        meta,
        &source.slots().map(|slot| slot.image).collect::<Vec<_>>(),
    )
    .unwrap();
    source.header = sign(&key, &source.image);
    let (a, h, i) = add(&source, &key, [21; 32], 21);
    let final_data = source
        .derive_successor(&h, ReplayBody::Admission(&a), &i)
        .unwrap()
        .next;
    assert_eq!(
        final_data.header.spec().sequence,
        u64::MAX,
        "mathematics alone does NOT certify an ordinary operation leaves mandatory control slots"
    );
    let mut impossible = final_data.header.spec().clone();
    impossible.parent = final_data.header.id();
    let impossible =
        SignedOwnerHeader::sign_using(impossible, |bytes| Ok(key.sign(bytes))).unwrap();
    assert!(matches!(
        final_data.derive_successor(&impossible, ReplayBody::Admission(&a), &[]),
        Err(ChatError::Dependency)
    ));
    assert_eq!(source.header.spec().sequence, u64::MAX - 1);
    let overflow = AdmissionBody::new(AdmissionSpec {
        group: GROUP.to_vec(),
        profile: source.profile.id(),
        subject: [22; 32],
        origin: AdmissionOrigin::Add {
            commit: [20; 32],
            before_epoch: u64::MAX,
            after_epoch: 0,
            key_package: [21; 32],
            credential: [22; 32],
        },
    });
    assert!(
        matches!(overflow, Err(ChatError::Dependency)),
        "typed epoch grammar rejects wrap before math"
    );
}

#[test]
fn replay_math_synthetic_full_count_cut_needs_no_new_leaf_capacity() {
    let (key, source) = genesis();
    let (a, h, i) = add(&source, &key, [21; 32], 21);
    let mut source = source
        .derive_successor(&h, ReplayBody::Admission(&a), &i)
        .unwrap()
        .next;
    // Fixed64-hash counter fixture, not a claim that this test produced2^64 accepted leaves.
    // The public API cannot import an arbitrary current snapshot/frontier. This pins the
    // independent arithmetic property: a frozen cut never needs a new acceptance leaf.
    let hashes = [TreeHash::from_bytes([23; 32]); 64];
    let mut root = hashes[0];
    for left in hashes.iter().skip(1) {
        let mut hash = blake3::Hasher::new();
        hash.update(b"mewtual/chat/acceptance-node/v2\0");
        hash.update(left.as_bytes());
        hash.update(root.as_bytes());
        root = TreeHash::from_bytes(*hash.finalize().as_bytes());
    }
    let index = source.find(a.id()).unwrap();
    let context = source.slots[index].frontier.tip().context;
    let tip = TreeTip {
        context,
        count: u64::MAX,
        root,
    };
    source.slots[index].frontier = Frontier::from_tip(tip, &hashes).unwrap();
    source.slots[index].image.accepted_count = u64::MAX;
    source.slots[index].image.accepted_root = root;
    source.slots[index].image.tip_body = OwnerBodyId::from_bytes([24; 32]);
    let mut meta = StateFrame::decode(&source.image).unwrap().meta();
    meta.kind = HeaderKind::AcceptanceAppend;
    meta.body = OwnerBodyId::from_bytes([24; 32]);
    meta.sequence = 1 << 59;
    source.image = encode_state(
        meta,
        &source.slots().map(|slot| slot.image).collect::<Vec<_>>(),
    )
    .unwrap();
    source.header = sign(&key, &source.image);
    source.slots[index].tip_header = source.header.id();
    assert!(source.slots[index]
        .frontier
        .prepare_append(&[AcceptanceLeaf {
            channel: 7,
            acceptance: OwnerFactId::from_bytes([25; 32])
        }])
        .is_err());
    let (cut, h, i) = remove(&source, &key, a.id());
    let cut = source
        .derive_successor(&h, ReplayBody::Removal(&cut), &i)
        .unwrap();
    assert_eq!(cut.removal.unwrap().tip.count, u64::MAX);
    assert!(cut.next.slot(a.id()).is_none());
    assert_eq!(cut.next.slots().len(), 1);
}

#[test]
fn replay_math_tree_consistency_does_not_claim_cross_batch_accepted_lookup() {
    let (key, source) = genesis();
    let admission = source.slots().next().unwrap().image.admission;
    let (body, h, i) = append(&source, &key, admission, 41);
    let once = source
        .derive_successor(&h, ReplayBody::Append(&body), &i)
        .unwrap()
        .next;
    let (same_leaf, h, i) = append(&once, &key, admission, 41);
    assert_eq!(body.spec().leaves, same_leaf.spec().leaves);
    let arithmetic = once
        .derive_successor(&h, ReplayBody::Append(&same_leaf), &i)
        .unwrap()
        .next;
    assert_eq!(arithmetic.slot(admission).unwrap().frontier.tip().count, 2);
    // Merkle arithmetic permits repeated values. This deliberate public DATA example is
    // NOT valid protocol idempotency: the real complete per-Admission/action lookup must
    // reject/replay the original before positive issuance. No token/verifier exists here.
}
