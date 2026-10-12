//! A repaired seed pass is minted from a locally held repair, not a fresh owner proof, so every
//! binding a proof would have checked is checked here instead (design 5.6): the current owner
//! signed both the repair and its selected receipt, under this device's own authoring tenure, for
//! this exact target.
use super::*;
use crate::owner_tenure::OwnerTenure;
use catcoms_replication::ReceiptRepair;

fn signed(owner: &Node, bucket: u8, close: u8, signer: &MlsDevice) -> Receipt {
    let source = RegistryEpoch::new(&owner.group, bucket, owner.device.device_id()).unwrap();
    let seed = source
        .projection()
        .unwrap()
        .checkpoint([close; 32])
        .unwrap();
    Receipt::sign(
        registry_document(&owner.group.group_id(), bucket).unwrap(),
        0,
        [close; 32],
        seed.change_hash(),
        0,
        InheritedCheckpoint::EpochZero,
        signer,
    )
    .unwrap()
}

fn repair(selected: &Receipt, other: &Receipt, signer: &MlsDevice) -> ReceiptRepair {
    let mut hashes = [selected.hash(), other.hash()];
    hashes.sort();
    ReceiptRepair::sign_in_tenure(
        selected.document.clone(),
        selected.tenure_id,
        hashes,
        selected.hash(),
        1,
        0,
        signer,
    )
    .unwrap()
}

#[tokio::test]
async fn a_repaired_seed_pass_needs_the_owner_repair_its_receipt_and_authoring_tenure() {
    let (mut owner, mut client, _) = pair().await;
    let (receipt, seed) = seed(&owner, 4);
    let other = signed(&owner, 4, 8, &owner.device);
    let target = CheckpointTarget::Registry(4);
    let good = repair(&receipt, &other, &owner.device);

    // N16: a Welcome newcomer never observed the owner take office, so its tenure is Unknown.
    // Driving an installation from a repair is authoring (6.3), so even the exact repair holds
    // and the newcomer converges on the owner's fresh proof instead.
    assert_eq!(client.authoring_owner_tenure_start(), None);
    assert!(client
        .select_repaired_checkpoint(target, &good, &receipt)
        .is_err());
    // N42: an Imported start verifies but cannot author. It is the only state where the two
    // accessors differ, so this is what catches a swap to the verification one.
    let epoch = client.group.epoch();
    let mut v1 = catcoms_wire::Encoder::new();
    v1.put_u8(1);
    v1.put_u64(epoch);
    v1.put_bytes(owner.device.device_id().as_bytes()).unwrap();
    v1.put_bytes(0u64.to_be_bytes().as_ref()).unwrap();
    client.owner_tenure = OwnerTenure::decode(&v1.finish(), &client.group).unwrap();
    assert_eq!(client.verification_owner_tenure_start(), Some(0));
    assert_eq!(client.authoring_owner_tenure_start(), None);
    assert!(client
        .select_repaired_checkpoint(target, &good, &receipt)
        .is_err());
    // A peer that did observe it: the owner's own tenure tail, reopened against the same group.
    let observed = owner.owner_tenure.encode(&owner.group).unwrap();
    client.owner_tenure = OwnerTenure::decode(&observed, &client.group).unwrap();
    assert_eq!(client.authoring_owner_tenure_start(), Some(0));

    // Another bucket's target never accepts this document's repair.
    assert!(client
        .select_repaired_checkpoint(CheckpointTarget::Registry(5), &good, &receipt)
        .is_err());
    // The receipt offered must be the one the repair selected, not its losing sibling.
    assert!(client
        .select_repaired_checkpoint(target, &good, &other)
        .is_err());
    // A member's signature is not the owner's, on either object.
    let forged = repair(&receipt, &other, &client.device);
    assert!(client
        .select_repaired_checkpoint(target, &forged, &receipt)
        .is_err());
    let member_receipt = signed(&owner, 4, 9, &client.device);
    let naming_member = repair(&member_receipt, &other, &owner.device);
    assert!(client
        .select_repaired_checkpoint(target, &naming_member, &member_receipt)
        .is_err());
    // A repair claiming another issuer tenure is not this owner's current decision.
    let mut hashes = [receipt.hash(), other.hash()];
    hashes.sort();
    let other_tenure = ReceiptRepair::sign_in_tenure(
        receipt.document.clone(),
        receipt.tenure_id,
        hashes,
        receipt.hash(),
        1,
        1,
        &owner.device,
    )
    .unwrap();
    assert!(client
        .select_repaired_checkpoint(target, &other_tenure, &receipt)
        .is_err());

    // The exact repair mints a one-shot pass that carries the repair and fetches its seed.
    let mut pass = client
        .select_repaired_checkpoint(target, &good, &receipt)
        .unwrap();
    assert_eq!(pass.fault_repair(), Some(&good));
    assert_eq!(pass.selected_receipt(), &receipt);
    assert!(fetch(
        &mut owner,
        &mut client,
        &mut pass,
        Some(seed.bytes().to_vec())
    )
    .await
    .unwrap());
    client
        .with_registry_seed(&pass, |_, _, _, selected| {
            assert_eq!(selected.receipt, &receipt);
            assert_eq!(selected.checkpoint.bytes(), seed.bytes());
        })
        .unwrap();
    // A later selection for the same target supersedes it, exactly like a fresh proof would.
    let _current = client
        .select_repaired_checkpoint(target, &good, &receipt)
        .unwrap();
    assert!(client
        .with_registry_seed(&pass, |_, _, _, _| panic!("superseded"))
        .is_err());
    assert!(client.docs.is_empty());
}
