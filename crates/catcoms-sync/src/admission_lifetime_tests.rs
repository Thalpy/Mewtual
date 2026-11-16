//! The admission lifetime policy runs on the real admission path, at this node's `Clock` time
//! (`docs/design-mls-leaf-lifetime.md` section 3).
//!
//! Other sync tests run on a `ManualClock` near 1970, where every real KeyPackage is decades from
//! expiry and the 30-day floor can never bite. Here the clock is placed relative to the
//! KeyPackage's own mint time, so the floor is exercised without reading a real clock.
use super::*;
use catcoms_rt::{Hub, ManualClock};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const DAY_MS: u64 = 24 * 3_600 * 1_000;

/// A committer whose clock is 55 days ahead of the joiner's sees an honest KeyPackage with only
/// 29 days left, refuses it through `serve_join`, and spends nothing: no commit, the invite stays
/// unconsumed. With the clocks back in step, the same invite and KeyPackage admit.
#[tokio::test]
async fn a_joiner_key_package_near_expiry_is_refused_without_spending_the_invite() {
    let hub = Hub::new();
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let clock = ManualClock::new(0);
    let mut node = ChannelSync::new(
        hub.join(PeerId::from_u64(1)),
        group,
        owner,
        ChaCha20Rng::seed_from_u64(1),
        Box::new(clock.clone()),
    );
    let invite = node.mint_invite([7; 16], u64::MAX, vec![]).unwrap();
    let joiner = MlsDevice::generate().unwrap();
    let package = joiner
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    // openmls back-dates `not_before` by one hour; taken from the KeyPackage, not a clock.
    let minted_ms = (package.life_time().not_before() + 3_600) * 1_000;
    let request = encode_join_req(&invite, &serialize_key_package(&package).unwrap());
    let epoch = node.epoch();

    clock.set_ms(minted_ms + 55 * DAY_MS);
    assert!(
        node.serve_join(PeerId::from_u64(2), &request).is_none(),
        "29 days left is under the 30-day floor"
    );
    assert_eq!(
        node.join_attempts()[0].outcome,
        JoinOutcome::AdmissionFailed
    );
    assert_eq!(node.epoch(), epoch, "no commit was made");
    assert!(
        !node.ledger.is_consumed(&invite.invite_nonce),
        "a refused KeyPackage must not spend the invite"
    );

    clock.set_ms(minted_ms);
    let answer = node
        .serve_join(PeerId::from_u64(3), &request)
        .expect("the same invite and KeyPackage admit once the clocks agree");
    assert_eq!(answer[0], 1, "admitted synchronously");
    assert_eq!(node.epoch(), epoch + 1);
    assert!(node.ledger.is_consumed(&invite.invite_nonce));
}
