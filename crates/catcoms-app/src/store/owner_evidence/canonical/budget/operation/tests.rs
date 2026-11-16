//! Accounting-only regression seams, not fabricated membership/current/save authorities.
use super::*;
const RESPONSE: usize = catcoms_sync::owner_evidence::OWNER_FIRST_ADD_RESPONSE_PRIVATE_BYTES;
fn fixture(old: usize, other: usize) -> (Arc<Mutex<Runtime>>, Arc<OperationFee>) {
    let budget = OwnerEvidenceBudget::checkpoint(
        1,
        256 * 1024 * 1024,
        256 * 1024 * 1024,
        24 * 1024 * 1024,
        512 * 1024,
        512 * 1024,
    )
    .unwrap();
    let pool = Arc::new(Mutex::new(Runtime {
        budget: Some(budget),
        hot_bytes: old + other,
        ..Runtime::default()
    }));
    let fee = Arc::new(OperationFee {
        pool: Arc::downgrade(&pool),
        bytes: old,
    });
    (pool, fee)
}
#[test]
fn original_private_fee_shrinks_exactly_and_lower_clones_retain_the_response_charge() {
    let (pool, fee) = fixture(RESPONSE + 1234, 99);
    let response = OperationFee::into_response(fee).unwrap();
    assert_eq!(pool.lock().unwrap().hot_bytes, RESPONSE + 99);
    let lower = response.clone();
    drop(response);
    assert_eq!(pool.lock().unwrap().hot_bytes, RESPONSE + 99);
    drop(lower);
    assert_eq!(pool.lock().unwrap().hot_bytes, 99);
}
#[test]
fn shared_private_custody_refuses_resize_without_partial_refund_or_response() {
    let (pool, fee) = fixture(RESPONSE + 1234, 99);
    let lower = fee.clone();
    assert!(OperationFee::into_response(fee).is_err());
    assert_eq!(pool.lock().unwrap().hot_bytes, RESPONSE + 1234 + 99);
    assert_eq!(lower.bytes, RESPONSE + 1234);
    drop(lower);
    assert_eq!(pool.lock().unwrap().hot_bytes, 99);
}
#[test]
fn invalid_accounting_and_missing_budget_refuse_before_changing_the_phase() {
    let (pool, fee) = fixture(RESPONSE + 1234, 0);
    pool.lock().unwrap().hot_bytes = 1; // proven inconsistent old invoice: no underflow/partial update
    assert!(OperationFee::into_response(fee).is_err());
    assert_eq!(pool.lock().unwrap().hot_bytes, 1);
    let (pool, fee) = fixture(RESPONSE + 1234, 99);
    pool.lock().unwrap().budget = None;
    assert!(OperationFee::into_response(fee).is_err());
    assert_eq!(
        pool.lock().unwrap().hot_bytes,
        99,
        "only original fee Drop is refunded after guard release"
    );
    let (pool, fee) = fixture(RESPONSE - 1, 99);
    assert!(OperationFee::into_response(fee).is_err());
    assert_eq!(pool.lock().unwrap().hot_bytes, 99);
}
#[test]
fn poisoned_or_gone_original_pool_cannot_create_replacement_response_custody() {
    let (pool, fee) = fixture(RESPONSE + 1234, 0);
    let observed = pool.clone();
    let _ = std::panic::catch_unwind(|| {
        let _guard = observed.lock().unwrap();
        panic!("accounting poison fixture");
    });
    assert!(OperationFee::into_response(fee).is_err());
    match pool.lock() {
        Err(error) => assert_eq!(error.into_inner().hot_bytes, RESPONSE + 1234),
        Ok(_) => panic!("fixture must poison actual original mutex"),
    }
    let (pool, fee) = fixture(RESPONSE + 1234, 0);
    drop(pool);
    assert!(OperationFee::into_response(fee).is_err());
}
#[test]
fn maximum_hot_total_uses_checked_subtraction_before_addition_and_cannot_wrap() {
    let (pool, fee) = fixture(RESPONSE + 1234, 0);
    {
        let mut runtime = pool.lock().unwrap();
        runtime.hot_bytes = usize::MAX;
        runtime.budget.as_mut().unwrap().hot_bytes = usize::MAX;
    }
    let response = OperationFee::into_response(fee).unwrap();
    assert_eq!(pool.lock().unwrap().hot_bytes, usize::MAX - 1234);
    drop(response);
    assert_eq!(pool.lock().unwrap().hot_bytes, usize::MAX - 1234 - RESPONSE);
}
