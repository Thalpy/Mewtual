//! Genuine source input does not bypass the early unsupported-platform refusal. This body
//! is compiled/run on its platform; Windows results must not be reported as its evidence.
use super::*;
use crate::store::critical_namespaces::current_layout::cold_read::tests::{fixture, ColdFixture};
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn unsupported_repair_refuses_before_new_wrapper_scope_credit_or_maintenance() {
    let ColdFixture {
        source,
        layout,
        pool,
        mut rng,
        directory,
        ..
    } = fixture(0xc817, true);
    let expected_source = source.data().source;
    let baseline = pool.charged_for_test();
    let mut prepared = PreparedColdV2GenesisInitializing::new(
        source,
        Arc::new(Hub::new().join(PeerId::from_u64(0xc817))),
        ChaCha20Rng::seed_from_u64(0xc817),
        Arc::new(ManualClock::new(30)),
    );
    assert!(prepared.retry(&layout, &mut rng).is_err());
    assert!(prepared.fee.is_none() && prepared.capture.is_none());
    assert!(prepared.task.is_none() && prepared.scope.is_none() && prepared.writer.is_none());
    assert_eq!(pool.charged_for_test(), baseline);
    assert_eq!(
        prepared.graph.source().unwrap().data().source,
        expected_source
    );
    drop(prepared);
    drop(layout);
    drop(directory);
}
