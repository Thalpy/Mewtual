//! Encrypted CRDT replication tests: live exchange, convergence under reorder,
//! de-duplication, snapshot catch-up for a late member (without old epoch keys),
//! and rejection of forged/tampered ops via the inner signature.

use automerge::transaction::Transactable;
use automerge::{AutoCommit, ReadDoc, ROOT};
use catcoms_mls::{InviteLedger, MlsDevice, ServerGroup};
use catcoms_replication::{EncryptedDoc, ReplError, SealedOp, SignedOp};
use catcoms_wire::DocType;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const CHANNEL: u128 = 1;

fn rng(seed: u64) -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(seed)
}

fn get_str(doc: &AutoCommit, key: &str) -> Option<String> {
    doc.get(ROOT, key)
        .unwrap()
        .map(|(v, _)| v.into_string().unwrap())
}

/// Alice founds a group and admits Bob via an invite; both end up at epoch 1.
fn two_members() -> (MlsDevice, ServerGroup, MlsDevice, ServerGroup) {
    let alice = MlsDevice::generate().unwrap();
    let mut ag = ServerGroup::create(&alice).unwrap();
    let bob = MlsDevice::generate().unwrap();
    let mut ledger = InviteLedger::new();
    let token = ag.mint_invite(&alice, [1u8; 16], 10_000, vec![]).unwrap();
    let kp = bob
        .key_package_for_invite(&ag.group_id(), token.invite_nonce)
        .unwrap();
    let welcome = ag
        .add_member_via_invite(&alice, kp, &token, &mut ledger, 1_000)
        .unwrap()
        .welcome;
    let bg = ServerGroup::join(&bob, &welcome).unwrap();
    (alice, ag, bob, bg)
}

#[test]
fn live_op_replicates_to_a_peer() {
    let (alice, ag, bob, bg) = two_members();
    let mut adoc = EncryptedDoc::new(DocType::Channel, CHANNEL, &alice.device_id());
    let mut bdoc = EncryptedDoc::new(DocType::Channel, CHANNEL, &bob.device_id());
    let mut r = rng(1);

    let op = adoc
        .edit(&alice, &ag, &mut r, |d| d.put(ROOT, "m1", "hello"))
        .unwrap();
    assert!(bdoc.ingest(&op, &bg, &bob).unwrap());
    assert_eq!(get_str(bdoc.doc(), "m1").as_deref(), Some("hello"));
}

#[test]
fn duplicate_ingest_is_idempotent() {
    let (alice, ag, bob, bg) = two_members();
    let mut adoc = EncryptedDoc::new(DocType::Channel, CHANNEL, &alice.device_id());
    let mut bdoc = EncryptedDoc::new(DocType::Channel, CHANNEL, &bob.device_id());
    let mut r = rng(1);

    let op = adoc
        .edit(&alice, &ag, &mut r, |d| d.put(ROOT, "k", "v"))
        .unwrap();
    assert!(bdoc.ingest(&op, &bg, &bob).unwrap());
    assert!(!bdoc.ingest(&op, &bg, &bob).unwrap()); // duplicate -> false
    assert_eq!(bdoc.op_count(), 1);
}

#[test]
fn out_of_order_ops_converge() {
    let (alice, ag, bob, bg) = two_members();
    let mut adoc = EncryptedDoc::new(DocType::Channel, CHANNEL, &alice.device_id());
    let mut bdoc = EncryptedDoc::new(DocType::Channel, CHANNEL, &bob.device_id());
    let mut r = rng(1);

    let op1 = adoc
        .edit(&alice, &ag, &mut r, |d| d.put(ROOT, "first", "1"))
        .unwrap();
    let op2 = adoc
        .edit(&alice, &ag, &mut r, |d| d.put(ROOT, "second", "2"))
        .unwrap();

    // Deliver the dependent op first; automerge buffers until its dep arrives.
    bdoc.ingest(&op2, &bg, &bob).unwrap();
    bdoc.ingest(&op1, &bg, &bob).unwrap();

    assert_eq!(get_str(bdoc.doc(), "first").as_deref(), Some("1"));
    assert_eq!(get_str(bdoc.doc(), "second").as_deref(), Some("2"));
}

#[test]
fn concurrent_edits_converge_both_ways() {
    let (alice, ag, bob, bg) = two_members();
    let mut adoc = EncryptedDoc::new(DocType::Channel, CHANNEL, &alice.device_id());
    let mut bdoc = EncryptedDoc::new(DocType::Channel, CHANNEL, &bob.device_id());
    let mut ra = rng(1);
    let mut rb = rng(2);

    let from_a = adoc
        .edit(&alice, &ag, &mut ra, |d| d.put(ROOT, "from_a", "a"))
        .unwrap();
    let from_b = bdoc
        .edit(&bob, &bg, &mut rb, |d| d.put(ROOT, "from_b", "b"))
        .unwrap();

    bdoc.ingest(&from_a, &bg, &bob).unwrap();
    adoc.ingest(&from_b, &ag, &alice).unwrap();

    for doc in [adoc.doc(), bdoc.doc()] {
        assert_eq!(get_str(doc, "from_a").as_deref(), Some("a"));
        assert_eq!(get_str(doc, "from_b").as_deref(), Some("b"));
    }
}

#[test]
fn late_member_catches_up_without_old_epoch_keys() {
    // Alice writes history BEFORE Bob exists (sealed under epoch 0)...
    let alice = MlsDevice::generate().unwrap();
    let mut ag = ServerGroup::create(&alice).unwrap();
    let mut adoc = EncryptedDoc::new(DocType::Channel, CHANNEL, &alice.device_id());
    let mut r = rng(1);
    adoc.edit(&alice, &ag, &mut r, |d| d.put(ROOT, "h1", "one"))
        .unwrap();
    adoc.edit(&alice, &ag, &mut r, |d| d.put(ROOT, "h2", "two"))
        .unwrap();
    assert_eq!(ag.epoch(), 0);

    // ...then Bob joins (advancing to epoch 1).
    let bob = MlsDevice::generate().unwrap();
    let mut ledger = InviteLedger::new();
    let token = ag.mint_invite(&alice, [9u8; 16], 10_000, vec![]).unwrap();
    let kp = bob
        .key_package_for_invite(&ag.group_id(), token.invite_nonce)
        .unwrap();
    let welcome = ag
        .add_member_via_invite(&alice, kp, &token, &mut ledger, 1_000)
        .unwrap()
        .welcome;
    let bg = ServerGroup::join(&bob, &welcome).unwrap();
    assert_eq!(ag.epoch(), 1);

    // Alice re-seals her log under the CURRENT epoch; Bob (who has only epoch-1
    // keys) catches up and converges.
    let bundle = adoc.export_catchup(&ag, &alice, &mut r).unwrap();
    let mut bdoc = EncryptedDoc::new(DocType::Channel, CHANNEL, &bob.device_id());
    assert_eq!(bdoc.import_catchup(&bundle, &bg, &bob).unwrap(), 2);

    assert_eq!(get_str(bdoc.doc(), "h1").as_deref(), Some("one"));
    assert_eq!(get_str(bdoc.doc(), "h2").as_deref(), Some("two"));
}

#[test]
fn tampered_op_is_rejected_by_the_inner_signature() {
    let (alice, ag, bob, bg) = two_members();
    let mut bdoc = EncryptedDoc::new(DocType::Channel, CHANNEL, &bob.device_id());
    let mut r = rng(1);

    // A malicious member holds the channel key, so they can seal anything; but
    // they cannot produce a valid inner signature for content they tamper.
    let mut forged = SignedOp::sign(&alice, DocType::Channel, CHANNEL, vec![1, 2, 3, 4]).unwrap();
    forged.signature[0] ^= 0xFF;
    let sealed = SealedOp::seal(&forged, &ag, &alice, &mut r).unwrap();

    let err = bdoc.ingest(&sealed, &bg, &bob).unwrap_err();
    assert!(matches!(err, ReplError::BadSignature));
}

#[test]
fn op_with_mismatched_author_is_rejected() {
    let alice = MlsDevice::generate().unwrap();
    let mut op = SignedOp::sign(&alice, DocType::Channel, CHANNEL, vec![9, 9, 9]).unwrap();
    // Claim a different author device than the embedded public key hashes to.
    op.author_device = catcoms_crypto::DeviceId::from_bytes([0u8; 32]);
    assert!(!op.verify());
}

#[test]
fn op_encode_decode_roundtrips() {
    let alice = MlsDevice::generate().unwrap();
    let op = SignedOp::sign(&alice, DocType::Wiki, 42, vec![1, 2, 3, 4, 5]).unwrap();
    let decoded = SignedOp::decode(&op.encode()).unwrap();
    assert_eq!(decoded, op);
    assert!(decoded.verify());
}
