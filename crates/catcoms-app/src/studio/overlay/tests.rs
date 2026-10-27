//! The Server-level Flow S stages, where live tenure enters.
use super::*;
use crate::studio::StudioOwnerTenure;
use catcoms_mls::MlsDevice;
use catcoms_replication::epoch_zero_id;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const SERVER: u64 = 7;

/// A budget over a complete inventory of `store`, scoped to `server`'s group.
fn budget<T: MeshTransport, R: CryptoRngCore>(
    store: &mut ServerStore,
    server: &mut Server<T, R>,
) -> EpochStudioBudget {
    let mut scan = store.scan_epoch_storage_with_studio().unwrap();
    while !scan.step().unwrap().complete {}
    let inventory = scan.finish().unwrap();
    server
        .sync
        .with_registry_context(|g, _, _, _| store.studio_storage_budget(SERVER, g, &inventory))
        .unwrap()
}

/// V1 at the preparation stage: a member that has never observed the owner take office is
/// refused there, with the message for *that* case, before preparation does anything else.
///
/// **Built on a real joiner, not a fabricated state.** That a plain joiner holds `Unknown` is
/// the sync layer's fact to guarantee, not this test's to assume, so it is asserted first: if a
/// joiner ever starts observing tenure, this fails on its precondition instead of passing
/// vacuously on a refusal that came from somewhere else.
///
/// **The founder is the control.** Both servers get identical arguments - a document with no
/// source, a close nobody will ever inspect - so the arguments alone would refuse either one. The
/// founder must get *past* the tenure requirement and refuse later, for a reason that is not
/// tenure; the joiner must refuse at it. That difference is what shows the refusal discriminates
/// on tenure rather than on the shared arguments.
///
/// What it kills, each checked by breaking the call site: reverting to
/// `authoring_owner_tenure_start()` produces the store's generic "needs observed owner tenure" and
/// fails the message assertion; removing the requirement entirely lets the joiner through to the
/// missing-source refusal and fails it too.
#[tokio::test]
async fn preparation_refuses_a_member_with_no_observed_tenure_and_says_which_case() {
    let hub = Hub::new();
    let alice_peer = PeerId::from_u64(1);
    let mut alice = Server::found(
        hub.join(alice_peer),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(1),
        Box::new(ManualClock::new(1_000)),
        "alice",
    )
    .unwrap();
    alice.subscribe_control().await.unwrap();
    let invite = alice.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
    let (bob, _) = tokio::join!(
        Server::join(
            hub.join(PeerId::from_u64(2)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(2),
            Box::new(ManualClock::new(1_000)),
            "bob",
            alice_peer,
            &invite,
        ),
        alice.sync_once(),
    );
    let mut bob = bob.unwrap();

    assert!(
        matches!(alice.observed_owner_tenure(), StudioOwnerTenure::Known(_)),
        "the founder must hold a known tenure for it to be the control, got {:?}",
        alice.observed_owner_tenure()
    );
    assert_eq!(
        bob.observed_owner_tenure(),
        StudioOwnerTenure::Unknown,
        "a plain joiner is expected to hold no observed tenure; if that changed, this test no \
         longer exercises the Unknown refusal"
    );

    let target = StudioTarget::Index {
        channel: crate::channel_id("general").to_be_bytes(),
    };
    let logical = target.document(&alice.group_id()).unwrap();
    let signer = MlsDevice::generate().unwrap();
    let close = CloseRecord::sign(
        &logical,
        epoch_zero_id(logical.doc_type, &logical.logical_key),
        0,
        vec![[1; 32]],
        &signer,
    )
    .unwrap();

    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(
        root.path(),
        b"tenure-prepare",
        &mut ChaCha20Rng::seed_from_u64(3),
    )
    .unwrap();

    let mut b = budget(&mut store, &mut bob);
    let refused = bob
        .prepare_studio_closing_overlay(&mut store, SERVER, target, &close, &mut b)
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("has not observed the current owner's tenure"),
        "the joiner must be refused for its tenure, specifically as Unknown; got: {refused}"
    );

    let mut b = budget(&mut store, &mut alice);
    let control = alice
        .prepare_studio_closing_overlay(&mut store, SERVER, target, &close, &mut b)
        .unwrap_err()
        .to_string();
    assert!(
        !control.contains("tenure"),
        "the founder holds a known tenure and must get past the requirement, refusing later for \
         another reason; got: {control}"
    );
}
