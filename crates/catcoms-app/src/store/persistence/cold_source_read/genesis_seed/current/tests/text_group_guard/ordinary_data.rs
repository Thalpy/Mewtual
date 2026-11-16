//! Genuine node/candidate DATA tests, not A/I/S/B saves or admission to publish.
//! The signer here creates an ordinary action only; no test calls the Append key cut.
use super::*;
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use crate::store::persistence::v2_text::{
    PreparedV2RetainData, PreparedV2RetainOriginalCustody, TextArchiveData, TextArtifactRoleData,
    TEXT_TRANSACTION_METADATA_FEE,
};
use catcoms_replication::chat::cold_manifest::ManifestPage;
use catcoms_storage::private_chat_page::open_private_chat_page;
use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
use catcoms_sync::owner_v2::{V2CreateRequest, CREATE_DATA_BUFFER_BYTES};

mod intent;

#[test]
fn actual_original_custody_refuses_wrong_credit_without_taking_and_keeps_sdk_fee_after_cancel() {
    use crate::store::critical_namespaces::read_pool::CriticalCatalogReadPool;
    use std::sync::atomic::{AtomicBool, Ordering};

    let mut f = super::promotion::promoted_text_fixture(0, 0x7f240);
    let selected = selected_parent(&f);
    let pool = f
        .layout
        .checked_store()
        .unwrap()
        .archive_bootstrap_policy
        .as_ref()
        .unwrap()
        .pending
        .pool
        .clone();
    let before = pool.charged_for_test();
    let mut data = Some(retain(&f, &selected, "own the original SDK credit"));
    let mut fee = Some(attach_original(
        &mut f,
        data.as_mut().unwrap(),
        "own the original SDK credit",
    ));
    let store = f.layout.checked_store().unwrap();
    let mut custody = None;
    assert!(
        PreparedV2RetainOriginalCustody::take_complete_into(&mut data, &mut fee, &mut custody)
            .is_err()
    );
    assert!(data.is_some() && fee.is_some() && custody.is_none());
    let owner = f.owner.borrow_current().unwrap();
    data.as_mut()
        .unwrap()
        .prepare_pages(
            store,
            owner.source(),
            &mut ChaCha20Rng::seed_from_u64(0x7f241),
        )
        .unwrap();
    let original_fee = fee.take().unwrap();
    let wrong_pool = CriticalCatalogReadPool::new(CREATE_DATA_BUFFER_BYTES).unwrap();
    fee = Some(wrong_pool.try_reserve(CREATE_DATA_BUFFER_BYTES).unwrap());
    assert!(
        PreparedV2RetainOriginalCustody::take_complete_into(&mut data, &mut fee, &mut custody)
            .is_err()
    );
    assert!(data.as_ref().unwrap().has_complete_data());
    assert!(fee.is_some() && custody.is_none());
    drop(fee.take());
    fee = Some(original_fee);
    let sdk_refunded = Arc::new(AtomicBool::new(false));
    fee.as_mut()
        .unwrap()
        .observe_drop_for_test(sdk_refunded.clone());
    let observation = data.as_ref().unwrap().observe_original();
    let weak_work = Arc::downgrade(data.as_ref().unwrap().work_resources());
    let charged = pool.charged_for_test();
    owner.warm_scope().unwrap().revocation().revoke();
    // Cancellation blocks new positive work, but must not strand a completed owning
    // handoff or refund the original SDK payment while its packet is still retained.
    PreparedV2RetainOriginalCustody::take_complete_into(&mut data, &mut fee, &mut custody).unwrap();
    assert!(data.is_none() && fee.is_none());
    assert!(!sdk_refunded.load(Ordering::SeqCst));
    assert_eq!(pool.charged_for_test(), charged);
    let retained = custody.as_ref().unwrap();
    retained.validate_accounts(store).unwrap();
    retained
        .observe_original()
        .require_same_original(&observation)
        .unwrap();
    assert!(retained
        .producer()
        .validate_resources_original(store, owner.source())
        .is_err());
    PreparedV2RetainOriginalCustody::take_complete_into(&mut data, &mut fee, &mut custody).unwrap();
    assert_eq!(pool.charged_for_test(), charged);
    drop(custody);
    assert!(sdk_refunded.load(Ordering::SeqCst));
    assert!(weak_work.upgrade().is_none());
    assert_eq!(
        pool.charged_for_test(),
        before + TEXT_TRANSACTION_METADATA_FEE
    );
    drop(observation);
    assert_eq!(pool.charged_for_test(), before);
}

fn selected_parent(f: &TextFixture) -> PreparedV2TextParentCapture {
    let store = f.layout.checked_store().unwrap();
    let owner = f.owner.borrow_current().unwrap();
    let mut selected = PreparedV2TextParentCapture::prepare(store, owner.source()).unwrap();
    let resources = selected.resources().clone();
    store
        .with_v2_text_current(&owner, |a| {
            let guard = store.lock_v2_text_current_group(a, &resources)?;
            selected.capture(&guard).map(|_| ())
        })
        .unwrap();
    selected
}

fn retain(
    f: &TextFixture,
    selected: &PreparedV2TextParentCapture,
    text: &str,
) -> PreparedV2RetainData {
    let owner = f.owner.borrow_current().unwrap();
    PreparedV2RetainData::prepare(
        f.layout.checked_store().unwrap(),
        owner.source(),
        selected.selected_data().unwrap(),
        V2CreateRequest {
            token: [11; 32],
            channel: 7,
            text,
        },
    )
    .unwrap()
}

fn attach_original(
    f: &mut TextFixture,
    data: &mut PreparedV2RetainData,
    text: &str,
) -> CatalogReadReservation {
    let store = f.layout.checked_store().unwrap();
    // The SDK's own1MiB is ADDITIONAL to source/read/metadata/work. Keep it through the
    // detached original; the physical invoice is not borrowed as this producer credit.
    let fee = store
        .archive_bootstrap_policy
        .as_ref()
        .unwrap()
        .pending
        .pool
        .try_reserve(CREATE_DATA_BUFFER_BYTES)
        .unwrap();
    let mut prepared = None;
    f.owner
        .prepare_create_data_for_test(
            store,
            V2CreateRequest {
                token: [11; 32],
                channel: 7,
                text,
            },
            &mut prepared,
        )
        .unwrap();
    let mut packet = Some(prepared.take().unwrap().into_handoff_data().unwrap());
    let owner = f.owner.borrow_current().unwrap();
    data.attach_packet(store, owner.source(), &mut packet)
        .unwrap();
    assert!(packet.is_none());
    data.attach_packet(store, owner.source(), &mut packet)
        .unwrap();
    assert!(
        packet.is_none(),
        "idempotent retry preserves the consumed original slot"
    );
    fee
}

#[test]
fn actual_retain_metadata_and_workspace_aliases_keep_their_independent_exact_payments() {
    let f = super::promotion::promoted_text_fixture(0, 0x7f200);
    let selected = selected_parent(&f);
    let store = f.layout.checked_store().unwrap();
    let pool = store
        .archive_bootstrap_policy
        .as_ref()
        .unwrap()
        .pending
        .pool
        .clone();
    let before = pool.charged_for_test();
    let data = retain(&f, &selected, "original");
    assert_eq!(
        pool.charged_for_test(),
        before + TEXT_TRANSACTION_METADATA_FEE + 16 * 1024 * 1024
    );
    let observation = data.observe_original();
    let weak_work = Arc::downgrade(data.work_resources());
    let claim = Arc::new(data.claim_workspace_preparation().unwrap());
    let ready_alias = claim.clone();
    assert!(data.claim_workspace_preparation().is_err());
    drop(claim);
    assert!(
        data.claim_workspace_preparation().is_err(),
        "actual Ready/source alias still owns claim"
    );
    drop(ready_alias);
    let claim = data.claim_workspace_preparation().unwrap();
    drop(data);
    assert!(
        weak_work.upgrade().is_some(),
        "actual capture claim still owns bulk payment"
    );
    assert_eq!(
        pool.charged_for_test(),
        before + TEXT_TRANSACTION_METADATA_FEE + 16 * 1024 * 1024
    );
    drop(claim);
    assert!(weak_work.upgrade().is_none());
    assert_eq!(
        pool.charged_for_test(),
        before + TEXT_TRANSACTION_METADATA_FEE,
        "negative observation keeps only its NEW metadata fee"
    );
    drop(weak_work);
    let other = retain(&f, &selected, "original");
    assert!(
        observation
            .require_same_original(&other.observe_original())
            .is_err(),
        "equal selected/context/request labels cannot adopt another original plan"
    );
    drop(other);
    drop(observation);
    assert_eq!(pool.charged_for_test(), before);
}

#[test]
fn actual_three_family_maximum_retain_reuses_all_original_frames_and_manifest_parents() {
    for family in 0..3 {
        let mut f = super::promotion::promoted_text_fixture(family, 0x7f210 + family as u64);
        let selected = selected_parent(&f);
        let text = "m".repeat(catcoms_replication::chat::MAX_TEXT_BYTES);
        let mut data = retain(&f, &selected, &text);
        let _create_fee = attach_original(&mut f, &mut data, &text);
        let store = f.layout.checked_store().unwrap();
        let owner = f.owner.borrow_current().unwrap();
        let target = owner.source().source().capture.target.to_path_buf();
        let head = std::fs::read(target.join("head.bin")).unwrap();
        data.prepare_pages(
            store,
            owner.source(),
            &mut ChaCha20Rng::seed_from_u64(0x7f220 + family as u64),
        )
        .unwrap();
        assert!(data.has_complete_data());
        assert!(data.require_unmaterialized_data().is_err());
        data.require_selected_data(selected.selected_data().unwrap())
            .unwrap();
        let frames: Vec<_> = data
            .artifact_frames_data()
            .map(|(role, id, raw)| (role, id, raw.to_vec()))
            .collect();
        assert_eq!(
            frames
                .iter()
                .filter(|(role, _, _)| *role == TextArtifactRoleData::ActionChunk)
                .count(),
            2
        );
        assert_eq!(
            frames
                .iter()
                .filter(|(role, _, _)| *role == TextArtifactRoleData::CipherChunk)
                .count(),
            3
        );
        assert_eq!(frames.len(), 8);
        let key = Zeroizing::new(store.keys.db_key().unwrap());
        let (_, manifest_id, manifest_frame) = frames
            .iter()
            .find(|(role, _, _)| *role == TextArtifactRoleData::Manifest)
            .unwrap();
        let manifest_plain = open_private_chat_page(
            &key,
            catcoms_storage::private_chat_page::PrivateChatPageId::from_bytes(*manifest_id),
            manifest_frame,
        )
        .unwrap();
        let manifest = ManifestPage::decode(&manifest_plain).unwrap();
        let parent = selected.selected_data().unwrap().archive_data().spec_data();
        assert_eq!(manifest.previous_manifest(), Some(parent.manifest));
        assert_eq!(
            manifest.original_source_head(),
            Some(data.parent_archive_data())
        );
        assert_eq!(manifest.sequence(), parent.manifest_sequence + 1);
        assert_eq!(manifest.before_header(), Some(parent.header.id()));
        assert_eq!(manifest.after_header(), parent.header.id());
        let (_, _, archive_frame) = frames
            .iter()
            .find(|(role, _, _)| *role == TextArtifactRoleData::Archive)
            .unwrap();
        // Independent frozen control KDF, as in the SDK envelope conformance tests.
        let archive_key = Zeroizing::new(blake3::derive_key(
            "catcoms/private-chat-text-archive-key/v1",
            &key[..],
        ));
        let archive_plain = Zeroizing::new(
            catcoms_crypto::unseal(&archive_key, &crate::store::unframe(archive_frame).unwrap())
                .unwrap(),
        );
        let archive = TextArchiveData::decode_data(&archive_plain).unwrap();
        let a = archive.spec_data();
        assert_eq!(a.previous, Some(data.parent_archive_data()));
        assert_eq!(a.manifest_sequence, parent.manifest_sequence + 1);
        assert_eq!(a.header, parent.header);
        assert_eq!(a.image, parent.image);
        data.prepare_pages(store, owner.source(), &mut NeverRng)
            .unwrap();
        assert_eq!(
            data.artifact_frames_data()
                .map(|(role, id, raw)| (role, id, raw.to_vec()))
                .collect::<Vec<_>>(),
            frames
        );
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
        assert!(
            !target.join("text.wal").exists(),
            "DATA construction made no journal or selector write"
        );
    }
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
fn actual_cancellation_during_page_seal_keeps_the_original_output_before_refusal() {
    let mut f = super::promotion::promoted_text_fixture(0, 0x7f230);
    let selected = selected_parent(&f);
    let mut data = retain(&f, &selected, "cached first page");
    let _create_fee = attach_original(&mut f, &mut data, "cached first page");
    let store = f.layout.checked_store().unwrap();
    let owner = f.owner.borrow_current().unwrap();
    let revocation = owner.warm_scope().unwrap().revocation();
    let mut rng = AfterNonce {
        inner: ChaCha20Rng::seed_from_u64(0x7f231),
        after: Some(move || revocation.revoke()),
    };
    assert!(data.prepare_pages(store, owner.source(), &mut rng).is_err());
    let cached: Vec<_> = data
        .artifact_frames_data()
        .map(|(role, id, raw)| (role, id, raw.to_vec()))
        .collect();
    assert_eq!(
        cached.len(),
        1,
        "first exact encrypted output was cached before the negative postcheck"
    );
    assert!(!data.has_complete_data());
    assert!(data
        .prepare_pages(store, owner.source(), &mut NeverRng)
        .is_err());
    assert_eq!(
        data.artifact_frames_data()
            .map(|(role, id, raw)| (role, id, raw.to_vec()))
            .collect::<Vec<_>>(),
        cached
    );
}
