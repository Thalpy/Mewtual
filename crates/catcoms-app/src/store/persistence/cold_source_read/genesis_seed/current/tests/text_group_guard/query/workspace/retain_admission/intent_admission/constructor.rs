//! Genuine pre-transfer inputs: no SDK plan/accepted absence or receipt is invented.
use super::*;
use rand_core::{CryptoRng, RngCore};

struct Inputs {
    retained: Option<PreparedV2TextRetainTransaction>,
    unsigned: Option<PreparedV2UnsignedOriginalCustody>,
    query: Option<PreparedV2TextAcceptedQuery>,
    original: TextFixture,
}
fn inputs(number: u64) -> Inputs {
    let (mut f, selected) = retain_fixture_with_selected(0, number);
    let stage = stage_admission(&f);
    f.transaction.run_stage(stage).unwrap();
    let commit = commit_admission(&f);
    f.transaction.run_commit(commit).unwrap();
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let mut query =
        PreparedV2TextAcceptedQuery::prepare(store, owner.source(), &f.transaction).unwrap();
    let mut reader =
        PreparedV2TextAcceptedQueryLease::prepare(store, owner.source(), &query).unwrap();
    let mut capture =
        Some(PreparedV2TextAcceptedQueryCapture::prepare(store, owner.source(), &query).unwrap());
    let resources = query.resources().clone();
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_accepted_query_into(
                &mut query,
                &mut reader,
                &mut capture,
                &mut admitted,
            )
        })
        .unwrap();
    reader.drain_unused_outside_writers();
    query.run(admitted.take().unwrap()).unwrap();
    drop(owner);
    let unsigned = PreparedV2UnsignedOriginalCustody::prepare_for_test(
        store,
        &f.original.owner,
        selected.selected_data().unwrap(),
        &f.transaction,
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let RetainFixture {
        transaction,
        original,
        ..
    } = f;
    Inputs {
        retained: Some(transaction),
        unsigned: Some(unsigned),
        query: Some(query),
        original,
    }
}

#[test]
fn actual_i_wrong_opaque_sdk_plan_preserves_all_three_owning_input_slots() {
    let mut first = inputs(0x7fbc0);
    let mut other = inputs(0x7fbd0);
    std::mem::swap(&mut first.unsigned, &mut other.unsigned);
    let sdk = first.unsigned.as_ref().unwrap().observe_original();
    let action = first.retained.as_ref().unwrap().observe_original();
    let store = first.original.layout.checked_store().unwrap();
    let owner = first.original.owner.borrow_current().unwrap();
    let target = owner.source().source().capture.target.to_path_buf();
    let before_head = std::fs::read(target.join("head.bin")).unwrap();
    let before_wal = std::fs::read(target.join("text.wal")).unwrap();
    let mut out = None;
    assert!(PreparedV2TextUnsignedIntent::take_into(
        store,
        owner.source(),
        &mut first.retained,
        &mut first.unsigned,
        &mut first.query,
        &mut out
    )
    .is_err());
    assert!(
        first.retained.is_some()
            && first.unsigned.is_some()
            && first.query.is_some()
            && out.is_none()
    );
    sdk.require_same_original(&first.unsigned.as_ref().unwrap().observe_original())
        .unwrap();
    action
        .require_same_original(&first.retained.as_ref().unwrap().observe_original())
        .unwrap();
    first
        .query
        .as_ref()
        .unwrap()
        .require_same_retained_data(first.retained.as_ref().unwrap())
        .unwrap();
    assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), before_head);
    assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), before_wal);
    // Correcting only the actual whole original input (with its SAME fees) now permits
    // transfer. The failed attempt neither consumed nor replaced the actual originals.
    std::mem::swap(&mut first.unsigned, &mut other.unsigned);
    PreparedV2TextUnsignedIntent::take_into(
        store,
        owner.source(),
        &mut first.retained,
        &mut first.unsigned,
        &mut first.query,
        &mut out,
    )
    .unwrap();
    assert!(
        first.retained.is_none()
            && first.unsigned.is_none()
            && first.query.is_none()
            && out.is_some()
    );
    assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), before_wal);
}

struct AfterNonce<R, F> {
    inner: R,
    after: Option<F>,
}
impl<R: RngCore, F: FnOnce()> AfterNonce<R, F> {
    fn hit(&mut self) {
        if let Some(after) = self.after.take() {
            after();
        }
    }
}
impl<R: RngCore, F: FnOnce()> RngCore for AfterNonce<R, F> {
    fn next_u32(&mut self) -> u32 {
        let n = self.inner.next_u32();
        self.hit();
        n
    }
    fn next_u64(&mut self) -> u64 {
        let n = self.inner.next_u64();
        self.hit();
        n
    }
    fn fill_bytes(&mut self, dst: &mut [u8]) {
        self.inner.fill_bytes(dst);
        self.hit();
    }
    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), rand_core::Error> {
        self.inner.try_fill_bytes(dst)?;
        self.hit();
        Ok(())
    }
}
impl<R: RngCore + CryptoRng, F: FnOnce()> CryptoRng for AfterNonce<R, F> {}

#[test]
fn actual_i_scope_stop_during_first_nonce_keeps_exact_ciphertext_before_refusal() {
    let mut input = inputs(0x7fbf0);
    let store = input.original.layout.checked_store().unwrap();
    let owner = input.original.owner.borrow_current().unwrap();
    let target = owner.source().source().capture.target.to_path_buf();
    let before_head = std::fs::read(target.join("head.bin")).unwrap();
    let before_wal = std::fs::read(target.join("text.wal")).unwrap();
    let mut intent = None;
    PreparedV2TextUnsignedIntent::take_into(
        store,
        owner.source(),
        &mut input.retained,
        &mut input.unsigned,
        &mut input.query,
        &mut intent,
    )
    .unwrap();
    let mut intent = intent.unwrap();
    let revoke = owner.warm_scope().unwrap().revocation();
    let mut rng = AfterNonce {
        inner: ChaCha20Rng::seed_from_u64(0x7fbf1),
        after: Some(move || revoke.revoke()),
    };
    assert!(intent
        .prepare_candidates(store, owner.source(), &mut rng)
        .is_err());
    let (_, original) = intent
        .original_output_for_test(V2TextIntentTestCut::IntentCreated)
        .unwrap();
    assert!(!original.is_empty());
    assert!(intent
        .original_output_for_test(V2TextIntentTestCut::HeadCreated)
        .is_err());
    assert!(intent
        .original_output_for_test(V2TextIntentTestCut::WalCreated)
        .is_err());
    assert!(intent
        .prepare_candidates(store, owner.source(), &mut NeverRng)
        .is_err());
    assert_eq!(
        intent
            .original_output_for_test(V2TextIntentTestCut::IntentCreated)
            .unwrap()
            .1,
        original
    );
    assert!(!intent.has_ready_data() && !intent.has_saved_data());
    assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), before_head);
    assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), before_wal);
    assert!(store.with_v2_text_current(&owner, |_| Ok(())).is_err());
}
