//! Bounded borrowed-row geometry DATA only. Real SDK-produced rows and source/fee/SIGNER
//! order are exercised by catcoms-mls; these synthetic fixed bytes grant no source authority.
use super::*;

fn geometry(facts: &mut shape::Facts, epoch: u64, own: u64) {
    let leaves = epoch as usize + 1;
    facts.leaves = leaves;
    facts.parents = leaves - 1;
    facts.secret_leaves = leaves;
    facts.secret_parents = leaves;
    facts.handshake = leaves;
    facts.application = leaves;
    facts.own_index = own;
    facts.secret_index = own;
    facts.secret_size = (2 * leaves - 1) as u64;
    facts.epoch = epoch;
}
fn bytes(maximum: u64, cursor: u64, epochs: &[u64]) -> Vec<u8> {
    let entries: Vec<_> = epochs
        .iter()
        .map(|epoch| serde_json::json!([epoch,{"secret":{"value":{"vec":vec![0;32]}}}]))
        .collect();
    serde_json::to_vec(&serde_json::json!({
        "max_number_of_secrets":maximum,"resumption_psk":entries,"cursor":cursor
    }))
    .unwrap()
}
fn scan(facts: &mut shape::Facts, row: &[u8]) -> Result<(), Error> {
    shape::check(
        Row::Resumption,
        row,
        &[9; 16],
        &[10; 32],
        facts,
        &mut account(100_000),
    )
}

#[test]
fn closed_resumption_geometry_binds_exact_epoch_prefix_cursor_and_zero_store() {
    for (epoch, own, maximum, cursor, epochs) in [
        (0, 0, 32, 1, vec![0]),
        (1, 0, 32, 2, vec![0, 1]),
        (1, 1, 0, 0, vec![]),
    ] {
        let mut facts = shape::Facts::default();
        geometry(&mut facts, epoch, own);
        scan(&mut facts, &bytes(maximum, cursor, &epochs)).unwrap();
        facts.finish().unwrap();
        facts.resumption_cursor = cursor + 1;
        assert!(facts.finish().is_err());
    }
    let mut facts = shape::Facts::default();
    geometry(&mut facts, 1, 0);
    scan(&mut facts, &bytes(32, 1, &[0])).unwrap();
    assert!(
        facts.finish().is_err(),
        "epoch1 requires BOTH original entries"
    );
    geometry(&mut facts, 0, 0);
    scan(&mut facts, &bytes(32, 2, &[0, 1])).unwrap();
    assert!(facts.finish().is_err(), "epoch0 cannot claim future epoch1");
}

#[test]
fn repeated_resumption_getters_recompute_all_facts_without_old_epoch_or_cursor_carry() {
    let mut facts = shape::Facts::default();
    geometry(&mut facts, 1, 0);
    scan(&mut facts, &bytes(32, 2, &[0, 1])).unwrap();
    assert_eq!(facts.resumption_epochs, [Some(0), Some(1)]);
    facts.finish().unwrap();
    // Change ONLY geometry, preserving every prior resumption fact until the actual next
    // scanner invocation. A row shortened to one entry must clear the old second epoch.
    geometry(&mut facts, 0, 0);
    scan(&mut facts, &bytes(32, 1, &[0])).unwrap();
    assert_eq!(facts.resumption_max, 32);
    assert_eq!(facts.resumption_cursor, 1);
    assert_eq!(facts.resumption_count, 1);
    assert_eq!(facts.resumption_epochs, [Some(0), None]);
    facts.finish().unwrap();
    geometry(&mut facts, 1, 1);
    scan(&mut facts, &bytes(0, 0, &[])).unwrap();
    assert_eq!(facts.resumption_max, 0);
    assert_eq!(facts.resumption_cursor, 0);
    assert_eq!(facts.resumption_count, 0);
    assert_eq!(facts.resumption_epochs, [None; 2]);
    facts.finish().unwrap();
}
