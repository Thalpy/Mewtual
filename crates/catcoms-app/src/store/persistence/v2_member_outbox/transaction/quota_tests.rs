//! Pure quota arithmetic, separate from the real-file pressure regressions.
use super::*;

#[test]
fn checkpoint_reserve_covers_growth_and_refuses_arithmetic_exhaustion() {
    let mut before = HeadData::initial();
    before.next_ordinal = 3;
    before.live_bytes = 100;
    before.live_objects = 5;
    let original = checkpoint_commitment_data(&before);
    let mut after = before.clone();
    after.front = 1;
    after.live_bytes += MAX_NODE_OUTPUT_BYTES as u64;
    after.live_objects += MAX_NODE_OUTPUTS as u64;
    assert_eq!(checkpoint_commitment_data(&after), original);
    after.next_ordinal = u64::MAX;
    assert_eq!(checkpoint_commitment_data(&after), (None, None));
    after.next_ordinal = 1;
    after.front = 0;
    after.live_bytes = u64::MAX;
    after.live_objects = u64::MAX;
    assert_eq!(checkpoint_commitment_data(&after), (None, None));
}
