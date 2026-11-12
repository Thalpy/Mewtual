//! Original-aware owning cuts and mid-seal cancellation use real S/Core/SDK fixtures.
use super::*;
use rand_core::{CryptoRng, RngCore};
fn selected_s(number: u64) -> RawFixture {
    let mut f = raw_fixture(0, number);
    let stage = stage_s(&f);
    f.signature.run_stage(stage).unwrap();
    let commit = commit_s(&f);
    f.signature.run_commit(commit).unwrap();
    f
}
#[test]
fn actual_b_missing_candidate_preserves_whole_s_then_real_candidate_allows_transfer() {
    let f = selected_s(55000);
    let RawFixture {
        signature,
        original,
        lane: _lane,
        predecessor: _predecessor,
        ..
    } = f;
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let target = owner.source().source().capture.target.to_path_buf();
    let before_head = std::fs::read(target.join("head.bin")).unwrap();
    let before_wal = std::fs::read(target.join("text.wal")).unwrap();
    let original_s = signature.observe_original();
    let mut input = Some(signature);
    let mut output = None;
    assert!(PreparedV2TextAcceptedCreate::take_into(
        store,
        owner.source(),
        &mut input,
        &mut output
    )
    .is_err());
    assert!(input.is_some() && output.is_none());
    original_s
        .require_same_original(&input.as_ref().unwrap().observe_original())
        .unwrap();
    assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), before_head);
    assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), before_wal);
    drop(owner);
    input
        .as_mut()
        .unwrap()
        .materialize_candidate_after_saved_s(store, &original.owner)
        .unwrap();
    let owner = original.owner.borrow_current().unwrap();
    PreparedV2TextAcceptedCreate::take_into(store, owner.source(), &mut input, &mut output)
        .unwrap();
    assert!(input.is_none() && output.is_some());
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
fn actual_b_first_nonce_stop_keeps_original_sealed_page_and_never_reseals() {
    let mut f = selected_s(55100);
    let store = f.original.layout.checked_store().unwrap();
    f.signature
        .materialize_candidate_after_saved_s(store, &f.original.owner)
        .unwrap();
    let RawFixture {
        signature,
        original,
        lane: _lane,
        predecessor: _predecessor,
        ..
    } = f;
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let target = owner.source().source().capture.target.to_path_buf();
    let before_head = std::fs::read(target.join("head.bin")).unwrap();
    let before_wal = std::fs::read(target.join("text.wal")).unwrap();
    let mut input = Some(signature);
    let mut accepted = None;
    PreparedV2TextAcceptedCreate::take_into(store, owner.source(), &mut input, &mut accepted)
        .unwrap();
    let mut accepted = accepted.unwrap();
    let revoke = owner.warm_scope().unwrap().revocation();
    let mut rng = AfterNonce {
        inner: ChaCha20Rng::seed_from_u64(55101),
        after: Some(move || revoke.revoke()),
    };
    assert!(accepted
        .prepare_candidates(store, owner.source(), &mut rng)
        .is_err());
    let first = accepted
        .cached_first_artifact_frame_for_test()
        .unwrap()
        .to_vec();
    assert!(!first.is_empty());
    assert!(!accepted.has_ready_data() && !accepted.has_saved_data());
    assert!(accepted
        .prepare_candidates(store, owner.source(), &mut NeverRng)
        .is_err());
    assert_eq!(
        accepted.cached_first_artifact_frame_for_test().unwrap(),
        first
    );
    assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), before_head);
    assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), before_wal);
    assert!(store.with_v2_text_current(&owner, |_| Ok(())).is_err());
}
