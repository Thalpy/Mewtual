//! Actual owning first-promotion facade, separate from ordinary posting or native rollout.
//! All three inputs are real settled whole owners; no source/node/SDK extractor is used.
mod create;
// Founder-as-ordinary-member lane: general verifier, plan and retained-device signing.
mod member;
// Member save: the two-phase path into the real member outbox on the fixture's own vault.
mod member_save;
mod recovery;
use super::*;
use crate::store::owner_evidence::canonical::text_lane::{
    active_text_lane_count_for_test, V2_TEXT_LANE_BUFFER_BYTES,
};
use crate::store::persistence::{
    PreparedV2TextHost, PreparedV2TextRead, PreparedV2TextSourceObserver, V2TextHostPolicy,
    V2_TEXT_SOURCE_OBSERVER_BYTES,
};

fn sdk_entries() -> usize {
    super::super::super::super::SDK_ENTRIES.with(|entries| entries.get())
}

fn host_policy() -> V2TextHostPolicy {
    V2TextHostPolicy::new(1024 * 1024 * 1024, 1024 * 1024).unwrap()
}

#[test]
fn actual_three_family_host_empty_promotion_preserves_original_scope_source_and_sdk() {
    for family in 0..3 {
        let original = text_fixture(family, 0x7ea00 + family as u64);
        let store = original.layout.checked_store().unwrap();
        let owner = original.owner.borrow_current().unwrap();
        let source_id = owner.source().data().source;
        let expected_nonce = owner.source().data().context.spec_data().scope_nonce;
        let target = owner.source().source().capture.target.to_path_buf();
        let original_source = std::fs::read(target.join("source.bin")).unwrap();
        let original_head = std::fs::read(target.join("head.bin")).unwrap();
        let original_scope = std::fs::read(store.dir.join("chat-admissions.bin")).unwrap();
        let preparation = PreparedV2TextSourceObserver::prepare(store, &owner).unwrap();
        let origin = store
            .with_v2_text_current(&owner, |admission| {
                let namespace = store.lock_v2_text_owner_namespace(admission)?;
                namespace.require_installed_text_writer(admission.writer_observation()?)?;
                assert_eq!(admission.scope_nonce_data()?, expected_nonce);
                assert_eq!(admission.source().data().source, source_id);
                preparation.capture(admission)
            })
            .unwrap();
        drop(preparation);
        drop(owner);
        let entries = sdk_entries();
        let mut host = PreparedV2TextHost::from_read(
            PreparedV2TextRead::from_original_owner(original.owner),
            host_policy(),
        );
        assert!(origin.is_live());
        host.retry_empty_promotion(
            &original.layout,
            &mut ChaCha20Rng::seed_from_u64(0x7eb00 + family as u64),
        )
        .unwrap();
        let selected_head = std::fs::read(target.join("head.bin")).unwrap();
        assert_ne!(selected_head, original_head);
        assert_eq!(active_text_lane_count_for_test(store), 0);
        assert_eq!(sdk_entries(), entries);
        assert_eq!(
            std::fs::read(target.join("source.bin")).unwrap(),
            original_source
        );
        assert_eq!(
            std::fs::read(store.dir.join("chat-admissions.bin")).unwrap(),
            original_scope
        );
        // This retained observer contains weak identities ONLY. It cannot keep the old
        // graph/read/Scope alive if the facade had replaced or dropped that whole owner.
        assert!(
            origin.is_live(),
            "family {family}: original source/Scope/writer survived"
        );
        for _ in 0..2 {
            host.retry_empty_promotion(&original.layout, &mut NeverRng)
                .unwrap();
            assert_eq!(
                std::fs::read(target.join("head.bin")).unwrap(),
                selected_head
            );
            assert_eq!(active_text_lane_count_for_test(store), 0);
            assert_eq!(sdk_entries(), entries);
        }
        // Successful retirement clears temporary capture/prepared fields. That must NOT
        // reopen the obsolete original-genesis cleanup entry point after O_G selection.
        assert!(host
            .retry_original_seed_settlement(&original.layout, &mut NeverRng)
            .is_err());
        assert_eq!(
            std::fs::read(target.join("head.bin")).unwrap(),
            selected_head
        );
        // The actual facade owns its query/path/reader through retries. This is only a
        // readonly selected-token lookup: no workspace-clear/new-Create permission is
        // inferred from its real empty initial root. Another token cannot replace it.
        let token = [family as u8 + 1; 32];
        host.retry_token_lookup(&original.layout, token).unwrap();
        host.retry_token_lookup(&original.layout, token).unwrap();
        assert!(host.retry_token_lookup(&original.layout, [99; 32]).is_err());
        assert_eq!(
            std::fs::read(target.join("head.bin")).unwrap(),
            selected_head
        );
        assert_eq!(active_text_lane_count_for_test(store), 0);
        assert_eq!(sdk_entries(), entries);
        host.cancel_and_drain_admitted_promotion().unwrap();
        assert!(!origin.is_live());
        assert!(host.retry_token_lookup(&original.layout, token).is_err());
        assert!(host
            .retry_empty_promotion(&original.layout, &mut NeverRng)
            .is_err());
        assert_eq!(
            std::fs::read(target.join("head.bin")).unwrap(),
            selected_head
        );
        assert_eq!(
            std::fs::read(target.join("source.bin")).unwrap(),
            original_source
        );
        assert_eq!(
            std::fs::read(store.dir.join("chat-admissions.bin")).unwrap(),
            original_scope
        );
        assert_eq!(active_text_lane_count_for_test(store), 0);
        assert_eq!(sdk_entries(), entries);
    }
}

struct InvoiceRng {
    inner: ChaCha20Rng,
    pool: critical_namespaces::read_pool::CriticalCatalogReadPool,
    peak: usize,
}
impl InvoiceRng {
    fn record_paid_charge(&mut self) {
        self.peak = self.peak.max(self.pool.charged_for_test());
    }
}
impl RngCore for InvoiceRng {
    fn next_u32(&mut self) -> u32 {
        self.record_paid_charge();
        self.inner.next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.record_paid_charge();
        self.inner.next_u64()
    }
    fn fill_bytes(&mut self, bytes: &mut [u8]) {
        self.record_paid_charge();
        self.inner.fill_bytes(bytes);
    }
    fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), rand_core::Error> {
        self.record_paid_charge();
        self.inner.try_fill_bytes(bytes)
    }
}
impl CryptoRng for InvoiceRng {}

#[test]
fn actual_host_retires_the_full_work_payment_after_successful_empty_promotion() {
    const READ_FEE: usize = 1024 * 1024;
    const WORK_FEE: usize = 16 * 1024 * 1024;
    let original = text_fixture(0, 0x7ea10);
    let store = original.layout.checked_store().unwrap();
    let pool = store
        .archive_bootstrap_policy
        .as_ref()
        .unwrap()
        .pending
        .pool
        .clone();
    let before = pool.charged_for_test();
    let mut rng = InvoiceRng {
        inner: ChaCha20Rng::seed_from_u64(0x7eb10),
        pool: pool.clone(),
        peak: before,
    };
    let mut host = PreparedV2TextHost::from_read(
        PreparedV2TextRead::from_original_owner(original.owner),
        host_policy(),
    );
    host.retry_empty_promotion(&original.layout, &mut rng)
        .unwrap();
    // Real RNG entry follows BOTH actual prepaid capture/work allocations, before the
    // later independently-paid source observer and persistent fixed lane backing.
    assert_eq!(rng.peak, before + READ_FEE + WORK_FEE);
    assert_eq!(
        pool.charged_for_test(),
        before + READ_FEE + V2_TEXT_SOURCE_OBSERVER_BYTES + V2_TEXT_LANE_BUFFER_BYTES
    );
    assert_eq!(active_text_lane_count_for_test(store), 0);
    let after = pool.charged_for_test();
    host.retry_empty_promotion(&original.layout, &mut NeverRng)
        .unwrap();
    assert_eq!(
        pool.charged_for_test(),
        after,
        "rechecking selected O issues no new work debit"
    );
    host.cancel_and_drain_admitted_promotion().unwrap();
    // Revocation is not a refund. Historical Saved and the original whole owner's
    // independently-paid runtime stay retained until those real values actually drain.
    assert_eq!(pool.charged_for_test(), after);
}

#[test]
fn actual_host_cancel_before_first_promotion_cannot_create_or_rewrite_any_control() {
    let original = text_fixture(0, 0x7ea20);
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let target = owner.source().source().capture.target.to_path_buf();
    let head = std::fs::read(target.join("head.bin")).unwrap();
    let scope = std::fs::read(store.dir.join("chat-admissions.bin")).unwrap();
    drop(owner);
    let entries = sdk_entries();
    let mut host = PreparedV2TextHost::from_read(
        PreparedV2TextRead::from_original_owner(original.owner),
        host_policy(),
    );
    host.cancel_and_drain_admitted_promotion().unwrap();
    host.cancel_and_drain_admitted_promotion().unwrap();
    assert!(host
        .retry_empty_promotion(&original.layout, &mut NeverRng)
        .is_err());
    assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
    assert_eq!(
        std::fs::read(store.dir.join("chat-admissions.bin")).unwrap(),
        scope
    );
    assert!(!target.join("text.seed").exists());
    assert!(!target.join("text.pages").exists());
    assert!(!target.join("head.stage").exists());
    assert_eq!(active_text_lane_count_for_test(store), 0);
    assert_eq!(sdk_entries(), entries);
}
