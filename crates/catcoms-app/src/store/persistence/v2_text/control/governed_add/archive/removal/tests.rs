use super::*;
use catcoms_crypto::identity::DeviceKeypair;
use catcoms_replication::chat::{
    acceptance_tree::{Frontier, TreeContext},
    owner_bodies::RemovalSpec,
    owner_facts::OwnerFactId,
    owner_header::{HeaderKind, HeaderSpec, OwnerBodyId, OwnerTenureId},
    owner_state::{encode_state, CurrentAdmissionSlot, StateFrame, StateMeta},
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn governed_remove_archive_binds_the_exact_encrypted_predecessor_and_fetch() {
    let key = [11; 32];
    let mut rng = ChaCha20Rng::seed_from_u64(173);
    let parent = GovernedHeadData::after_first_archive(
        GovernedArchiveLabels {
            context: TextContextIdData::from_bytes([1; 32]),
            before_head: TextHeadIdData::from_bytes([2; 32]),
            before_archive: ArchiveHeadId::from_bytes([3; 32]),
            header: OwnerHeaderId::from_bytes([4; 32]),
            state: OwnerStateCommitment::from_bytes([5; 32]),
            epoch: 1,
            authenticator: [6; 32],
        },
        ArchiveHeadId::from_bytes([7; 32]),
        1,
    )
    .unwrap();
    let head = parent.seal_data(&key, &mut rng).unwrap();
    let alternative = parent.seal_data(&key, &mut rng).unwrap();
    let signer = DeviceKeypair::from_seed(&[21; 32]);
    let issuer = signer.verifying_key().to_bytes();
    let profile = OwnerFactId::from_bytes([22; 32]);
    let founder = TreeContext {
        profile,
        admission: OwnerFactId::from_bytes([23; 32]),
    };
    let target = TreeContext {
        profile,
        admission: OwnerFactId::from_bytes([24; 32]),
    };
    let group = b"original group";
    // The paid MLS path has its own genuine Commit regression. This record
    // fixture checks signed bindings and stored bytes at the codec boundary.
    let commit = b"exact encrypted Remove commit";
    let snapshot = b"private snapshot";
    let removal = RemovalBody::new(RemovalSpec {
        group: group.to_vec(),
        subject: [25; 32],
        commit: *blake3::hash(commit).as_bytes(),
        before_epoch: 1,
        after_epoch: 2,
        tip_header: parent.transition.header,
        frozen_tip: Frontier::empty(target).tip(),
    })
    .unwrap();
    let meta = StateMeta {
        group,
        profile,
        issuer,
        tenure: OwnerTenureId::from_bytes([27; 32]),
        parent: parent.transition.header,
        previous: parent.transition.state,
        sequence: 3,
        kind: HeaderKind::Removal,
        body: removal.id(),
        mls_epoch: 2,
    };
    let image = encode_state(
        meta,
        &[CurrentAdmissionSlot {
            admission: OwnerBodyId::from_bytes(*founder.admission.as_bytes()),
            subject: issuer,
            tip_body: OwnerBodyId::from_bytes([28; 32]),
            accepted_count: 0,
            accepted_root: founder.empty_hash(),
        }],
    )
    .unwrap();
    let header = SignedOwnerHeader::sign_using(
        HeaderSpec {
            group: group.to_vec(),
            profile,
            issuer,
            tenure: meta.tenure,
            parent: meta.parent,
            sequence: meta.sequence,
            kind: meta.kind,
            body: meta.body,
            state: StateFrame::decode(&image).unwrap().commitment(),
        },
        |bytes| Ok(signer.sign(bytes)),
    )
    .unwrap();
    let body = removal.encode();
    let row = encode_header_row(&header, &image).unwrap();
    let request = |token| {
        let mut e = Encoder::new();
        e.put_bytes(b"mewtual/chat/next-member-add/v2\0")
            .unwrap()
            .put_u16(2);
        e.put_bytes(parent.transition.header.as_bytes())
            .unwrap()
            .put_u64(1);
        e.put_bytes(&[token; 32]).unwrap();
        V2MemberAddRequestData::decode(&e.finish()).unwrap()
    };
    let original = request(31);
    let fetch = original.verifier_data();
    let inputs: [&[u8]; 7] = [&head, &row, &body, commit, snapshot, &fetch, &[41; 32]];
    let output = seal_fields(&key, inputs, &mut rng).unwrap();
    let selected = GovernedHeadData::open_governed_data(&key, &output.head).unwrap();
    assert!(selected.kind == GovernedHeadKind::Remove);
    assert!(GovernedHeadData::open_data(&key, &output.head).is_err());
    assert!(open_later_data(&key, &selected, &output.archive).is_err());
    let plain = open_data(&key, &selected, &output.archive).unwrap();
    let record = RemovalRecord::decode(&plain).unwrap();
    assert_eq!(record.parent_head, head.as_slice());
    assert_eq!(record.snapshot, snapshot);
    assert_eq!(record.commit, commit);
    assert_eq!(
        record.encode_member_transition(&original).unwrap(),
        V2MemberTransitionFrameData::encode_removal(&header, &removal, &image, commit).unwrap()
    );
    assert!(record.encode_member_transition(&request(32)).is_err());

    let mut replacement = inputs;
    replacement[0] = &alternative;
    let replacement = seal_fields(&key, replacement, &mut rng).unwrap();
    assert!(open_data(&key, &selected, &replacement.archive).is_err());
    let alternative_labels = GovernedArchiveLabels {
        before_head: TextHeadIdData::of_data(&alternative),
        ..selected.transition
    };
    let mismatched = GovernedHeadData::after_successor(
        &key,
        &alternative,
        alternative_labels,
        selected.archive,
        GovernedHeadKind::Remove,
    )
    .unwrap();
    assert!(open_data(&key, &mismatched, &output.archive).is_err());
    assert!(open_data(&key, &selected, &output.archive[..output.archive.len() - 1]).is_err());
    let mut broken = inputs;
    broken[3] = &commit[..commit.len() - 1];
    assert!(seal_fields(&key, broken, &mut rng).is_err());
}
