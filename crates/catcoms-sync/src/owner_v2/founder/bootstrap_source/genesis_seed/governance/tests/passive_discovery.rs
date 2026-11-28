//! Real MLS Add/Welcome plus the authenticated Core prefix for its two
//! members. This fixture checks the exchange codec; the App still owns
//! passive policy, real transport leases, and response admission.
use super::*;
use crate::{
    owner_v2::{DiscoveryEndpoint, OpenedV2MemberDiscovery, V2MemberDependencyRoute},
    V2MemberContactData, V2_MEMBER_CONTACT_BYTES,
};
use catcoms_replication::chat::{owner_facts::OwnerFactId, V2GeneralPrefixVerifier};
use catcoms_rt::PeerId;
use rand_core::SeedableRng;

fn signed_contact(
    verifier: &V2GeneralPrefixVerifier,
    admission: OwnerFactId,
    device: &MlsDevice,
    transport_key: u8,
) -> (PeerId, [u8; V2_MEMBER_CONTACT_BYTES]) {
    let mut peer = [transport_key; 38];
    peer[..6].copy_from_slice(&[0, 36, 8, 1, 18, 32]);
    let replay = verifier.replay_data();
    let mut raw = [0; V2_MEMBER_CONTACT_BYTES];
    let (preimage, signature) = raw.split_at_mut(V2_MEMBER_CONTACT_BYTES - 64);
    let mut fields = &mut preimage[..];
    for part in [
        b"mewtual/chat/v2/member-contact/v1\0".as_slice(),
        replay.profile().id().as_bytes(),
        replay.header().id().as_bytes(),
        admission.as_bytes(),
        &replay.mls_epoch_data().to_be_bytes(),
        &peer,
        &7012u16.to_be_bytes(),
    ] {
        let (target, rest) = fields.split_at_mut(part.len());
        target.copy_from_slice(part);
        fields = rest;
    }
    assert!(fields.is_empty());
    signature.copy_from_slice(&device.sign(preimage).unwrap());
    (PeerId::new(*blake3::hash(&peer).as_bytes()), raw)
}

#[test]
fn passive_discovery_requires_both_current_signed_peers_and_the_exact_reply() {
    let (original, _reservation, _opened, _fee) = prepared_node(0xf101);
    let mut node = original.prepare_governance_data().unwrap();
    let genesis = node.genesis_data();
    let mut selected = governance(
        genesis.profile,
        genesis.admission,
        genesis.header,
        genesis.state,
    );
    let founder = genesis.admission.admission_id();
    let stale = signed_contact(
        &selected.verifier,
        founder,
        &node.node.seed.body.device,
        0x61,
    );

    let native = Native::new();
    let paid = Arc::new(AtomicUsize::new(0));
    let join = Request::new(&node, 74);
    begin(&mut node, &native, &paid, join.data());
    node.execute_add_data_native(join.data()).unwrap();
    node.prepare_admission_data(selected.verifier.replay_data())
        .unwrap();
    node.sign_admission_native().unwrap();
    node.materialize_admission_data().unwrap();
    let welcome = node.completed_add_data().unwrap().welcome_data().to_vec();
    let member = node
        .pending
        .as_ref()
        .unwrap()
        .body
        .as_ref()
        .unwrap()
        .admission_id();
    let _retired = install_for_test(&mut node, &mut selected.verifier);
    let joined = ServerGroup::join(&join._peer, &welcome).unwrap();
    let a = &node.node.seed.body;
    let (requester, sender) = signed_contact(&selected.verifier, founder, &a.device, 0x61);
    let (provider, receiver) = signed_contact(&selected.verifier, member, &join._peer, 0x62);
    let sender = V2MemberContactData::decode(&sender).unwrap();
    let receiver = V2MemberContactData::decode(&receiver).unwrap();
    let route = V2MemberDependencyRoute {
        requester,
        provider,
    };
    let send = DiscoveryEndpoint {
        group: &a.group,
        device: &a.device,
        verifier: &selected.verifier,
        route,
        local: founder,
    };
    let receive = DiscoveryEndpoint {
        group: &joined,
        device: &join._peer,
        verifier: &selected.verifier,
        route,
        local: member,
    };
    let mut rng = ChaCha20Rng::seed_from_u64(0xf102);

    let request = send.prepare(&sender, &mut rng).unwrap();
    assert!(OpenedV2MemberDiscovery::recognizes(request.wire_data()));
    let opened = receive.open_request(request.wire_data()).unwrap();
    assert_eq!(opened.requester_data().peer_data(), requester);
    assert_eq!(opened.requester_data().admission_data(), founder);
    assert!(opened.requester_data().is_live_cached());
    let reply = receive.seal_reply(&opened, &receiver, &mut rng).unwrap();
    let checked = send.open_reply(&request, &reply).unwrap();
    assert_eq!(checked.peer_data(), provider);
    assert_eq!(checked.admission_data(), member);
    assert!(checked.is_live_cached());

    let next = send.prepare(&sender, &mut rng).unwrap();
    assert!(send.open_reply(&next, &reply).is_err());
    assert!(send.open_reply(&request, request.wire_data()).is_err());
    assert!(receive.open_request(&reply).is_err());
    let mut changed = reply;
    *changed.last_mut().unwrap() ^= 1;
    assert!(send.open_reply(&request, &changed).is_err());
    assert!(receive.seal_reply(&opened, &sender, &mut rng).is_err());
    assert!(send.prepare(&receiver, &mut rng).is_err());
    assert!(send
        .prepare(&V2MemberContactData::decode(&stale.1).unwrap(), &mut rng)
        .is_err());

    let impersonator = DiscoveryEndpoint {
        route: V2MemberDependencyRoute {
            requester: PeerId::from_u64(0xf103),
            provider,
        },
        ..receive
    };
    assert!(impersonator.open_request(request.wire_data()).is_err());
    let other_group = ServerGroup::create(&join._peer).unwrap();
    let wrong_group = DiscoveryEndpoint {
        group: &other_group,
        route,
        ..impersonator
    };
    assert!(wrong_group.open_request(request.wire_data()).is_err());
    selected.live.0.store(false, SeqCst);
    assert!(!opened.requester_data().is_live_cached());
    assert!(!checked.is_live_cached());
}
