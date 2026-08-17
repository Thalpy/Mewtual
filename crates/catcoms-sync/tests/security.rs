//! Phase 7; **consolidated security suite**.
//!
//! The Mewtual security contract, in one auditable place. Most adversarial properties
//! are enforced (and unit-tested) inside the crate that owns them; this file (a) maps
//! the threat model to where each property is proven, and (b) adds the **cross-cutting,
//! end-to-end** adversarial scenarios that no single crate's unit tests cover.
//!
//! ## Threat model → where it is proven
//!
//! | Property (defends against) | Proven in |
//! |---|---|
//! | **Invite is single-use + device-bound** (leaked/replayed invite) | `sync::spent_invite_cannot_be_reused_for_a_second_join`; `mls::invite` ledger tests |
//! | **Only the named inviter admits** (group-substitution / forged Welcome) | `sync::joining_via_a_non_inviter_node_is_rejected` |
//! | **Single-committer admission** (un-authorized admit) | `sync::a_non_committer_cannot_admit` |
//! | **Catch-up is members-only** (Sybil-C1 request side) | `sync::catch_up_is_refused_to_a_non_member` |
//! | **Catch-up *responses* are member-signed + anti-replayed** (Sybil-C1 response side) | `sync::a_commit_catchup_response_verifies_only_from_a_current_member_bound_to_the_request`; `an_authed_request_roundtrips_with_its_nonce_and_epoch` |
//! | **Forged membership commits are rejected at apply** | `sync::control_commit_with_a_forged_committer_label_is_rejected` |
//! | **Topics/namespaces are member-only + rotate on removal** (A1; routing forward-secrecy) | `sync::topics_are_keyed_by_the_routing_secret_and_bound_to_the_label`, `routing_namespace_rotates_only_on_member_removal`; **+ `a_removed_member_is_excluded_from_the_rotated_namespace` here** |
//! | **Pre-dial membership tag** (Sybil/colluding-rendezvous injection) | `sync::a_membership_tag_binds_the_secret_label_and_peer` |
//! | **Member PEX is members-only + can't forge addresses** | `sync::pex_*` (ingest gate, non-member rejection, rate limit) |
//! | **DiscoveryPolicy: junk-last, budgeted, no auto-dial** | `discovery::*`; `net::a_node_registers_via_a_circuit_and_another_discovers_it` (no auto-dial) |
//! | **Eclipse detector is advisory; never gates** (H3 weaponization) | **`an_eclipse_caution_never_gates_a_removal` here** |
//! | **Address cache: tamper-detected on load** | `discovery::a_tampered_row_is_rejected_on_load` |
//! | **Rendezvous addrs validated** (circuit / dup-PeerId misconfig) | `net::rendezvous_address_validation_rejects_circuits_and_duplicates` |
//!
//! The two tests below are the cross-layer scenarios that span crates and so live here.

use std::sync::Arc;

use catcoms_crypto::DeviceId;
use catcoms_discovery::{EclipseConfig, EclipseDetector, EclipseLevel, EclipseObservation};
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use catcoms_sync::{request_join, ChannelSync};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

type Member = ChannelSync<MemNetwork, ChaCha20Rng>;

/// Build a converged `n`-member group over one in-memory hub, every member subscribed
/// to the control topic. `members[0]` is the founder (the designated committer);
/// returns the members and their device ids, aligned by index.
async fn build_members(n: u64) -> (Arc<Hub>, Vec<Member>, Vec<DeviceId>) {
    assert!(n >= 1);
    let hub = Hub::new();
    let founder = MlsDevice::generate().unwrap();
    let founder_id = founder.device_id();
    let fgroup = ServerGroup::create(&founder).unwrap();
    let fpeer = PeerId::from_u64(1);
    let mut founder_sync = ChannelSync::new(
        hub.join(fpeer),
        fgroup,
        founder,
        ChaCha20Rng::seed_from_u64(1),
        Box::new(ManualClock::new(1_000)),
    );
    founder_sync.subscribe_control().await.unwrap();
    let mut members = Vec::new();
    let mut ids = vec![founder_id];
    for i in 2..=n {
        let dev = MlsDevice::generate().unwrap();
        ids.push(dev.device_id());
        let invite = founder_sync
            .mint_invite([i as u8; 16], u64::MAX, vec![])
            .unwrap();
        let net = hub.join(PeerId::from_u64(i));
        let (joined, _) = tokio::join!(
            request_join(&net, fpeer, &dev, &invite),
            founder_sync.run_once(),
        );
        let (group, routing) = joined.unwrap();
        let mut sync = ChannelSync::new_joined(
            net,
            group,
            dev,
            ChaCha20Rng::seed_from_u64(i),
            Box::new(ManualClock::new(1_000)),
            routing,
        );
        sync.subscribe_control().await.unwrap();
        members.push(sync);
    }
    members.insert(0, founder_sync);
    (hub, members, ids)
}

/// **H3; the eclipse detector can never be weaponized to block a legitimate removal.**
/// It is *advisory only*: it returns an `EclipseLevel` and nothing in the membership
/// path consults it. So even while the detector is raising CAUTION (sustained isolation
/// signs), the designated committer's removal still commits; an attacker who can drive
/// the detector's inputs gains no power to keep a member in the group.
#[tokio::test]
async fn an_eclipse_caution_never_gates_a_removal() {
    let (_hub, mut members, ids) = build_members(3).await;

    // Drive an advisory detector all the way to CAUTION (big roster, no reach, one root).
    let mut detector = EclipseDetector::new(EclipseConfig::default());
    let clock = ManualClock::new(0);
    let suspect = EclipseObservation {
        roster_size: 20,
        reachable_devices: 1,
        trust_roots: 1,
    };
    assert_eq!(detector.observe(suspect, &clock), EclipseLevel::Ok); // held under grace
    clock.advance_ms(30_000);
    assert_eq!(
        detector.observe(suspect, &clock),
        EclipseLevel::Caution,
        "the detector is raising CAUTION"
    );

    // …and the founder (designated committer) removes Carol regardless. There is no
    // code path from the advisory level to the membership commit.
    let carol_id = ids[2];
    let alice = &mut members[0];
    assert_eq!(alice.member_count(), 3);
    alice.request_remove(&carol_id).await.unwrap();
    assert_eq!(
        alice.member_count(),
        2,
        "the removal committed under CAUTION"
    );
    assert!(!alice.contains_member(&carol_id), "Carol is gone");
}

/// **Routing-metadata forward secrecy; a removed member is excluded from the rotated
/// rendezvous namespace.** On removal the routing label `L` advances and `ns_secret_L`
/// is the *post-removal* epoch secret the removed member can never export. So the
/// remaining members converge on the new namespace while the removed member is stuck on
/// the old one; it cannot discover (or be discovered by) the group going forward.
#[tokio::test]
async fn a_removed_member_is_excluded_from_the_rotated_namespace() {
    // members[0]=Alice (founder/committer), [1]=Bob (joined 2nd), [2]=Carol (joined
    // last → full roster, current). We remove Bob; Carol (current) rotates with the
    // group, Bob (removed) is frozen out.
    let (_hub, mut members, ids) = build_members(3).await;
    let rz = b"a-rendezvous-peer-id";

    // The removed member's namespace before the removal (label 0).
    let removed_ns_before = members[1].rendezvous_namespaces(rz)[0].clone();
    assert_eq!(members[1].routing_label(), 0);

    // The founder removes Bob; the founder rotates to label 1 immediately.
    let bob_id = ids[1];
    members[0].request_remove(&bob_id).await.unwrap();
    assert_eq!(members[0].routing_label(), 1, "removal rotated the founder");

    // Carol (current, still a member) receives the single removal commit on the control
    // topic and rotates to the same label. (Exactly one event is queued;
    // `MemNetwork::next_event` blocks, so a second `run_once` here would deadlock.)
    members[2].run_once().await.unwrap();
    assert_eq!(
        members[2].routing_label(),
        1,
        "Carol rotated with the group"
    );

    let alice_ns = members[0].rendezvous_namespaces(rz)[0].clone();
    let carol_ns = members[2].rendezvous_namespaces(rz)[0].clone();
    let removed_ns_after = members[1].rendezvous_namespaces(rz)[0].clone();

    assert_eq!(
        alice_ns, carol_ns,
        "the remaining members converge on the new namespace"
    );
    assert_ne!(
        alice_ns, removed_ns_after,
        "the removed member cannot compute the post-removal namespace (forward secrecy)"
    );
    assert_eq!(
        removed_ns_after, removed_ns_before,
        "the removed member is frozen on its pre-removal namespace"
    );
}
