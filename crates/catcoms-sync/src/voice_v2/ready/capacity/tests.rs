use super::*;
use crate::voice_v2::resources::GROUP_BYTES;
use crate::voice_v2::*;

#[test]
fn ready_peak_uses_actual_maximum_wire_and_fits_beside_both_actual_lowers() {
    let mut signers = [
        DeviceKeypair::from_seed(&[1; 32]),
        DeviceKeypair::from_seed(&[2; 32]),
    ];
    signers.sort_by_key(DeviceKeypair::device_id);
    let mut helpers = vec![
        DeviceKeypair::from_seed(&[3; 32]).device_id(),
        DeviceKeypair::from_seed(&[4; 32]).device_id(),
    ];
    helpers.sort();
    let object = SignedObject::issue(
        Scope::for_group(b"Ready prepaid peak", Some([8; 32]), u128::MAX),
        &signers[1],
        [9; 32],
        1000,
        16000,
        Body::PairReady {
            header: ReadyHeader {
                epoch: u64::MAX,
                target: signers[0].device_id(),
                source_join: [10; 32],
                target_join: [11; 32],
                source_key_ref: [12; 32],
                target_key_ref: [13; 32],
                handshake: [14; 32],
                finish_message: [15; 32],
                finish_object_hash: [16; 32],
                helpers,
            },
        },
    )
    .unwrap();
    assert_eq!(object.encode().len(), MAX_WIRE);
    assert_eq!(
        METADATA + CACHE + RECEIPT + SOURCE + 2 * LOWER + ACK_CRYPTO,
        30551
    );

    let pool = Arc::new(Resources::default());
    let group = [1; 32];
    let capacity = ReadyCapacity::new(&pool, group).unwrap();
    let lane = pool.reserve_crypto_lane(group).unwrap();
    let forwards = pool.group_forward_lanes(group).unwrap();
    let pressure = pool
        .charge(group, GROUP_BYTES - pool.counts().0, ResourceKind::Retained)
        .unwrap();
    let budget = ByteBudget::Reserved(capacity.reserve.clone());
    let cache = budget
        .charge(&pool, group, CACHE, ResourceKind::Retained)
        .unwrap();
    let receipt = budget
        .charge(&pool, group, RECEIPT, ResourceKind::Retained)
        .unwrap();
    let crypto = ByteBudget::ReservedLane {
        reservation: capacity.reserve.clone(),
        lane: lane.clone(),
    };
    let scratch = crypto
        .charge(&pool, group, 6144, ResourceKind::Crypto)
        .unwrap();
    let source = budget
        .charge(&pool, group, SOURCE, ResourceKind::Retained)
        .unwrap();
    drop(scratch);
    let first = forwards
        .charge(&pool, group, &capacity.reserve, LOWER)
        .unwrap();
    let second = forwards
        .charge(&pool, group, &capacity.reserve, LOWER)
        .unwrap();
    assert!(forwards.charge(&pool, group, &capacity.reserve, 1).is_err());
    let ack = crypto
        .charge(&pool, group, ACK_CRYPTO, ResourceKind::Crypto)
        .unwrap();
    assert_eq!(pool.counts().0, GROUP_BYTES);
    capacity.close();
    lane.close();
    assert!(crypto
        .charge(&pool, group, 1, ResourceKind::Crypto)
        .is_err());
    drop((
        cache, receipt, source, first, second, ack, crypto, budget, forwards, lane, capacity,
        pressure,
    ));
    assert_eq!(pool.counts(), (0, 0));
}
