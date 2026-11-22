use super::*;

#[test]
fn governed_add_sdk_budget_keeps_the_exact_debit_in_the_admission() {
    let bytes = GovernedAddSdkReservation::required_bytes().unwrap();
    let short = CriticalCatalogReadPool::new(bytes - 1).unwrap();
    assert!(GovernedAddSdkReservation::prepare(&short).is_err());
    assert_eq!(short.charged_for_test(), 0);

    let exact = CriticalCatalogReadPool::new(bytes).unwrap();
    let admission = GovernedAddSdkReservation::prepare(&exact).unwrap();
    assert!(admission.supports_routing_data());
    assert_eq!(exact.charged_for_test(), bytes);
    assert!(GovernedAddSdkReservation::prepare(&exact.clone()).is_err());
    drop(admission);
    assert_eq!(exact.charged_for_test(), 0);
    assert!(GovernedAddSdkReservation::prepare(&exact).is_ok());
}
