use super::super::tests::{base, joined, next_in};
use super::*;

fn append_native(index: &mut Index, tag: u8, epoch: u64) {
    let Backend::Joined {
        mut birth,
        mut binding,
    } = joined(index)
    else {
        unreachable!()
    };
    birth.kind = BirthKind::NativeJoined;
    birth.invocation = [tag; 32];
    birth.request = [tag + 1; 32];
    birth.join_context = Some([tag + 2; 32]);
    binding.group = vec![4; 16];
    binding.source = [tag + 2; 32];
    binding.join_context = binding.source;
    binding.own_add = [tag + 3; 32];
    binding.initial_governing_tip = [tag + 4; 32];
    binding.initial_current_basis = [tag + 5; 32];
    binding.joined_epoch = epoch;
    index.last_issued_id += 1;
    index.rows.push(Row {
        id: index.last_issued_id,
        identity: [tag + 6; 32],
        is_dm: false,
        name: "original group".to_owned(),
        metadata_revision: 0,
        backend: Backend::Joined { birth, binding },
    });
}

fn rejoin(old: &Index, tag: u8, epoch: u64) -> (Index, RemovalData) {
    let mut candidate = next_in(old, true);
    let row = candidate.rows.last_mut().unwrap();
    let Backend::Joined { birth, binding } = &row.backend else {
        panic!("expected active native joined row");
    };
    let removal = RemovalData {
        row_id: row.id,
        row_identity: row.identity,
        own_admission: binding.own_add,
        header: [tag - 1; 32],
        epoch: epoch - 1,
    };
    row.backend = Backend::RetiredJoined {
        birth: birth.clone(),
        binding: binding.clone(),
        terminal: removal.header,
    };
    append_native(&mut candidate, tag, epoch);
    (candidate, removal)
}

#[test]
fn same_vault_rejoin_index_keeps_each_original_birth_and_only_the_latest_active() {
    let mut a1 = base();
    append_native(&mut a1, 70, 1);
    let frame = a1.parent.unwrap();
    assert!(a1.encoded_length().is_ok());

    let (a2, evict_a1) = rejoin(&a1, 80, 3);
    require_rejoin_successor(&a1, &a2, frame, &evict_a1).unwrap();
    assert!(a2.encoded_length().is_err());
    assert!(a1.successor_data(&a2, frame).is_err());
    let bytes = a2.encode_rejoin_data().unwrap();
    assert!(Index::decode_data(&bytes, a1.capability).is_err());
    assert!(Index::decode_rejoin_data(&a1.encode_data().unwrap(), a1.capability).is_err());
    let reopened = Index::decode_rejoin_data(&bytes, a1.capability).unwrap();
    assert!(reopened == a2);
    assert!(Index::decode_rejoin_data(&bytes[..bytes.len() - 1], a1.capability).is_err());
    require_rejoin_successor(&a1, &reopened, frame, &evict_a1).unwrap();
    assert!(matches!(
        a2.lookup_data([70; 32], [71; 32]).unwrap().unwrap().backend,
        Backend::RetiredJoined { .. }
    ));

    let (a3, evict_a2) = rejoin(&a2, 100, 6);
    require_rejoin_successor(&a2, &a3, frame, &evict_a2).unwrap();
    assert!(a2.rows[2] == a3.rows[2]);
    assert!(a3.rows[3].id < a3.rows[4].id);
    assert!(matches!(a3.rows[3].backend, Backend::RetiredJoined { .. }));
    assert!(matches!(a3.rows[4].backend, Backend::Joined { .. }));
}

#[test]
fn same_vault_rejoin_index_rejects_foreign_or_resurrected_memberships() {
    let mut a1 = base();
    append_native(&mut a1, 70, 1);
    let frame = a1.parent.unwrap();
    let (mut a2, mut proof) = rejoin(&a1, 80, 3);
    proof.header[0] ^= 1;
    assert!(require_rejoin_successor(&a1, &a2, frame, &proof).is_err());
    proof.header[0] ^= 1;
    proof.own_admission[0] ^= 1;
    assert!(require_rejoin_successor(&a1, &a2, frame, &proof).is_err());
    proof.own_admission[0] ^= 1;
    proof.epoch += 1;
    assert!(require_rejoin_successor(&a1, &a2, frame, &proof).is_err());
    proof.epoch -= 1;
    a2.rows[0].name.push('!');
    assert!(require_rejoin_successor(&a1, &a2, frame, &proof).is_err());
    a2.rows[0].name.pop();

    let Backend::Joined { binding, .. } = &mut a2.rows.last_mut().unwrap().backend else {
        unreachable!();
    };
    binding.pin[0] ^= 1;
    assert!(require_rejoin_successor(&a1, &a2, frame, &proof).is_err());
    let Backend::Joined { birth, binding } = &mut a2.rows.last_mut().unwrap().backend else {
        unreachable!();
    };
    binding.pin[0] ^= 1;
    birth.invocation = [70; 32];
    assert!(require_rejoin_successor(&a1, &a2, frame, &proof).is_err());
    a2.rows.pop();
    assert!(a2.rejoin_encoded_length().is_err());

    let mut two_active = next_in(&a1, true);
    append_native(&mut two_active, 80, 3);
    assert!(two_active.rejoin_encoded_length().is_err());
}
