//! Chat signed-history boundaries. Modern writer impersonation is a rejection contract.
//! The explicitly named `legacy_boundary_*` tests instead demonstrate remaining format limits;
//! passing those characterizations is NOT hostile-peer or moderator-audit acceptance.

use automerge::transaction::Transactable;
use automerge::{ActorId, AutoCommit, Change, ReadDoc, ROOT};
use catcoms_mls::{InviteLedger, MlsDevice, ServerGroup};
use catcoms_replication::{EncryptedDoc, ReplError, SealedOp, SignedOp};
use catcoms_wire::DocType;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const CHANNEL: u128 = 0xc4a7;

fn pair() -> (MlsDevice, ServerGroup, MlsDevice, ServerGroup) {
    let alice = MlsDevice::generate().unwrap();
    let bob = MlsDevice::generate().unwrap();
    let mut ag = ServerGroup::create(&alice).unwrap();
    let invite = ag.mint_invite(&alice, [1; 16], u64::MAX, vec![]).unwrap();
    let package = bob
        .key_package_for_invite(&ag.group_id(), invite.invite_nonce)
        .unwrap();
    let welcome = ag
        .add_member_via_invite(&alice, package, &invite, &mut InviteLedger::new(), 1_000)
        .unwrap()
        .welcome;
    let bg = ServerGroup::join(&bob, &welcome).unwrap();
    (alice, ag, bob, bg)
}

fn text(doc: &EncryptedDoc) -> String {
    doc.doc()
        .get(ROOT, "message")
        .unwrap()
        .unwrap()
        .0
        .into_string()
        .unwrap()
}

#[test]
fn forged_modern_writer_cannot_occupy_another_devices_sequence_in_either_arrival_order() {
    let (alice, ag, bob, bg) = pair();
    let mut rng = ChaCha20Rng::seed_from_u64(50);
    let mut genuine = EncryptedDoc::new(DocType::Channel, CHANNEL, &alice.device_id());
    let real = genuine
        .edit(&alice, &ag, &mut rng, |doc| {
            doc.put(ROOT, "message", "genuine")
        })
        .unwrap();
    let mut forged =
        AutoCommit::new().with_actor(ActorId::from(alice.device_id().as_bytes().to_vec()));
    forged.put(ROOT, "message", "forged").unwrap();
    forged.commit();
    let forged = SignedOp::sign(
        &bob,
        DocType::Channel,
        CHANNEL,
        forged.get_last_local_change().unwrap().raw_bytes().to_vec(),
    )
    .unwrap();
    assert!(
        forged.verify(),
        "the attack has a real member signature, not a broken signature"
    );
    let sealed = SealedOp::seal(&forged, &bg, &bob, &mut rng).unwrap();
    for forged_first in [false, true] {
        for via_catchup in [false, true] {
            let mut recipient = EncryptedDoc::new(DocType::Channel, CHANNEL, &bob.device_id());
            let order = if forged_first {
                [(&sealed, true), (&real, false)]
            } else {
                [(&real, false), (&sealed, true)]
            };
            for (operation, is_forged) in order {
                let result = if via_catchup {
                    recipient
                        .import_catchup(std::slice::from_ref(operation), &bg, &bob)
                        .map(|count| count != 0)
                } else {
                    recipient.ingest(operation, &bg, &bob)
                };
                if is_forged {
                    assert!(
                        matches!(result, Err(ReplError::ActorMismatch)),
                        "forged first={forged_first}, catchup={via_catchup}: {result:?}"
                    );
                } else {
                    assert!(result.unwrap());
                }
                // Each boundary may be a restart. A rejected envelope must leave no occupied
                // actor sequence or accepted-op hash behind in either persistence image.
                let saved = recipient.snapshot().unwrap();
                recipient = EncryptedDoc::restore_for_actor(&saved, &bob.device_id()).unwrap();
            }
            assert_eq!(recipient.op_count(), 1);
            assert_eq!(recipient.heads(), genuine.heads());
            assert_eq!(text(&recipient), "genuine");
            assert!(!recipient.ingest(&real, &bg, &bob).unwrap());
        }
    }
}

#[test]
fn authentic_pre_rebind_sixteen_byte_restore_actors_remain_relayable() {
    let (alice, ag, bob, bg) = pair();
    let mut rng = ChaCha20Rng::seed_from_u64(51);
    let mut before = EncryptedDoc::new(DocType::Channel, CHANNEL, &alice.device_id());
    before
        .edit(&alice, &ag, &mut rng, |doc| {
            doc.put(ROOT, "message", "before restart")
        })
        .unwrap();
    let saved = before.snapshot().unwrap();
    // This was the actual production restore path before actor rebinding was introduced.
    let mut old_client = EncryptedDoc::restore(&saved).unwrap();
    let historical = old_client
        .edit(&alice, &ag, &mut rng, |doc| {
            doc.put(ROOT, "message", "old restored writer")
        })
        .unwrap();
    let key = ag
        .channel_secret(&alice, DocType::Channel, CHANNEL)
        .unwrap();
    let inner = historical.open(&key).unwrap();
    let change = Change::from_bytes(inner.delta.clone()).unwrap();
    assert_eq!(change.actor_id().to_bytes().len(), 16);
    assert_ne!(change.actor_id().to_bytes(), alice.device_id().as_bytes());
    let mut recipient = EncryptedDoc::new(DocType::Channel, CHANNEL, &bob.device_id());
    let history = old_client.export_catchup(&ag, &alice, &mut rng).unwrap();
    assert_eq!(recipient.import_catchup(&history, &bg, &bob).unwrap(), 2);
    let saved = recipient.snapshot().unwrap();
    let recipient = EncryptedDoc::restore_for_actor(&saved, &bob.device_id()).unwrap();
    assert_eq!(text(&recipient), "old restored writer");
    let relayed = recipient.export_catchup(&bg, &bob, &mut rng).unwrap();
    assert!(relayed
        .iter()
        .any(|operation| operation.open(&key).unwrap() == inner));
}

#[test]
fn legacy_boundary_same_device_equivocation_is_not_an_automatic_merge() {
    let (alice, ag, bob, bg) = pair();
    let mut rng = ChaCha20Rng::seed_from_u64(52);
    let mut original = EncryptedDoc::new(DocType::Channel, CHANNEL, &alice.device_id());
    original
        .edit(&alice, &ag, &mut rng, |doc| {
            doc.put(ROOT, "message", "base")
        })
        .unwrap();
    let saved = original.snapshot().unwrap();
    let base = original.export_catchup(&ag, &alice, &mut rng).unwrap();
    let mut left = EncryptedDoc::restore_for_actor(&saved, &alice.device_id()).unwrap();
    let mut right = EncryptedDoc::restore_for_actor(&saved, &alice.device_id()).unwrap();
    let left = left
        .edit(&alice, &ag, &mut rng, |doc| {
            doc.put(ROOT, "message", "left device clone")
        })
        .unwrap();
    let right = right
        .edit(&alice, &ag, &mut rng, |doc| {
            doc.put(ROOT, "message", "right device clone")
        })
        .unwrap();
    let mut heads = Vec::new();
    for [first, second] in [[&left, &right], [&right, &left]] {
        let mut recipient = EncryptedDoc::new(DocType::Channel, CHANNEL, &bob.device_id());
        recipient.import_catchup(&base, &bg, &bob).unwrap();
        assert!(recipient.ingest(first, &bg, &bob).unwrap());
        assert!(
            matches!(
                recipient.ingest(second, &bg, &bob),
                Err(ReplError::Automerge(_))
            ),
            "same-device equivocation is still a documented rejection, not converged history"
        );
        heads.push(recipient.heads());
    }
    assert_ne!(
        heads[0], heads[1],
        "this characterization must stop claiming the gap if equivocation recovery is implemented"
    );
}

#[test]
fn legacy_boundary_signatures_do_not_attest_to_the_originating_group() {
    let (alice, ag, bob, bg) = pair();
    let other_group = ServerGroup::create(&bob).unwrap();
    assert_ne!(ag.group_id(), other_group.group_id());
    assert!(!other_group.contains_device(&alice.device_id()));
    let mut rng = ChaCha20Rng::seed_from_u64(53);
    let mut source = EncryptedDoc::new(DocType::Channel, CHANNEL, &alice.device_id());
    let sealed = source
        .edit(&alice, &ag, &mut rng, |doc| {
            doc.put(ROOT, "message", "signed in the first group")
        })
        .unwrap();
    let original_key = bg.channel_secret(&bob, DocType::Channel, CHANNEL).unwrap();
    let inner = sealed.open(&original_key).unwrap();
    // A dual-group member can rewrap the unchanged author signature. Legacy signatures bind
    // type/id/bytes, not the original group. Do not present this as proof of group-local tenure.
    let transplanted = SealedOp::seal(&inner, &other_group, &bob, &mut rng).unwrap();
    let mut recipient = EncryptedDoc::new(DocType::Channel, CHANNEL, &bob.device_id());
    assert!(recipient.ingest(&transplanted, &other_group, &bob).unwrap());
    assert_eq!(text(&recipient), "signed in the first group");
    let other_key = other_group
        .channel_secret(&bob, DocType::Channel, CHANNEL)
        .unwrap();
    assert_eq!(transplanted.open(&other_key).unwrap(), inner);
}
