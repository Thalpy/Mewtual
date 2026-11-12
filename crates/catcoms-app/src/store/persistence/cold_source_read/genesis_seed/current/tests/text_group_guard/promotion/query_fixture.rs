//! A genuine selected empty operational parent for paired reader tests. Every input is
//! the same complete original owner; no decoded selector/SDK or fabricated root is adopted.
use super::*;

pub(in crate::store::persistence::cold_source_read::genesis_seed::current::tests::text_group_guard) fn promoted_text_fixture(
    family: usize,
    number: u64,
) -> TextFixture {
    let mut fixture = promotion_fixture(family, number);
    stage_and_commit(&mut fixture);
    let mut lane = Some(fixture.lane);
    finish_promotion_lane(&fixture.original, &fixture.prepared, &mut lane);
    drop(fixture.prepared);
    fixture.original
}
