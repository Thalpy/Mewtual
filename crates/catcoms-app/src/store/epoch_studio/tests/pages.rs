use super::*;
use catcoms_replication::studio::catchup::{
    StudioOpPage, StudioPageOutcome, StudioPageProvider, StudioPageRequest, MAX_STUDIO_PAGE_OPS,
};
use catcoms_rt::ManualClock;

struct Sender {
    device: MlsDevice,
    group: ServerGroup,
    unit: StudioEpoch,
    pager: StudioPageProvider,
}
impl Sender {
    fn new(f: &mut Fixture) -> Self {
        let device = MlsDevice::generate().unwrap();
        let add = f
            .group
            .add_member(&f.device, device.key_package().unwrap())
            .unwrap();
        let group = ServerGroup::join(&device, &add.welcome).unwrap();
        let unit = StudioEpoch::new(&group, f.target, device.device_id()).unwrap();
        let pager = StudioPageProvider::new(
            device.device_id(),
            Arc::new(ManualClock::new(1000)),
            &mut rng(),
        );
        Self {
            device,
            group,
            unit,
            pager,
        }
    }
    fn edit(&mut self, f: &Fixture, n: u8) -> SealedOp {
        let mut op = if n == 0 { f.insert() } else { f.title() };
        if n == 0 && matches!(f.target, StudioTarget::Index { .. }) {
            op.body = IndexOp::PutObject {
                object: [1; 16],
                kind: StudioKind::Flipnote,
                title: "private moon".into(),
                created_by: self.device.device_id(),
                ts: 100,
                expiry: StudioExpiry::Never,
            }
            .encode()
            .unwrap();
        }
        op.nonce = [n; 16];
        self.unit
            .edit_or_reseal(&self.device, &self.group, &mut rng(), &op, 100)
            .unwrap()
    }
    fn page(&mut self, f: &Fixture, heads: &[[u8; 32]], cursor: Option<&[u8]>) -> StudioOpPage {
        let StudioPageOutcome::Page(page) = self
            .pager
            .page(
                &self.unit,
                &self.group,
                &self.device,
                StudioPageRequest {
                    requester: f.device.device_id(),
                    doc_id: self.unit.doc_id(),
                    heads,
                    seed: None,
                    cursor,
                },
                &mut rng(),
            )
            .unwrap()
        else {
            panic!("expected Studio page")
        };
        page
    }
}
fn receive(
    store: &mut ServerStore,
    f: &Fixture,
    page: &[SealedOp],
    b: &mut EpochStudioBudget,
) -> Result<StudioPageAdmission, AppError> {
    store.ingest_studio_page(
        SERVER,
        &f.group,
        f.target,
        f.id,
        &f.device,
        page,
        &mut rng(),
        b,
    )
}

#[test]
fn studio_pages_distinct_members_save_multiple_pages_then_reopen_without_own_intents() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let mut f = Fixture::new(art);
        let mut sender = Sender::new(&mut f);
        for n in 0..70 {
            sender.edit(&f, n);
        }
        let mut b = budget(&mut store, &f);
        let mut cursor = None;
        let mut accepted = 0;
        let before = crate::store::studio_full_restores_for_test();
        loop {
            let page = sender.page(
                &f,
                &[],
                cursor
                    .as_ref()
                    .map(|c: &catcoms_replication::studio::catchup::StudioPageCursor| c.as_bytes()),
            );
            let admitted = receive(&mut store, &f, &page.operations, &mut b).unwrap();
            accepted += admitted.accepted;
            assert_eq!(admitted.duplicates, 0);
            assert_eq!(
                crate::store::studio_full_restores_for_test(),
                before,
                "each saved page reuses the existing owned source"
            );
            let again = receive(&mut store, &f, &page.operations, &mut b).unwrap();
            assert_eq!(again.accepted, 0);
            assert_eq!(again.duplicates, page.operations.len());
            assert_eq!(again.frontier.heads, admitted.frontier.heads);
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(accepted, 70);
        assert_eq!(
            f.intents(&store),
            0,
            "receiving foreign history does not invent own intents"
        );
        drop(store);
        let store = open(root.path());
        assert_eq!(
            f.load(&store).unwrap().projection().unwrap(),
            sender.unit.projection().unwrap()
        );
    }
}

#[test]
fn studio_pages_bad_middle_and_missing_dependency_save_no_prefix_and_drop_warm_unit() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let mut f = Fixture::new(true);
    let mut sender = Sender::new(&mut f);
    let first = sender.edit(&f, 0);
    let second = sender.edit(&f, 1);
    let third = sender.edit(&f, 2);
    let mut b = budget(&mut store, &f);
    receive(&mut store, &f, &[first], &mut b).unwrap();
    let original = fs::read(f.path(&store)).unwrap();
    let mut damaged = third.clone();
    *damaged.blob.ciphertext.last_mut().unwrap() ^= 1;
    assert!(receive(&mut store, &f, &[second.clone(), damaged], &mut b).is_err());
    assert_eq!(fs::read(f.path(&store)).unwrap(), original);
    let mut false_domain = f.title();
    false_domain.nonce = [98; 16];
    let key = sender
        .group
        .channel_secret(&sender.device, third.doc_type, third.doc_id)
        .unwrap();
    let false_signed = SignedOp::sign_domain(
        &sender.device,
        third.doc_type,
        third.doc_id,
        third.open(&key).unwrap().delta,
        &false_domain,
    )
    .unwrap();
    let false_sealed =
        SealedOp::seal(&false_signed, &sender.group, &sender.device, &mut rng()).unwrap();
    assert!(
        receive(&mut store, &f, &[second.clone(), false_sealed], &mut b).is_err(),
        "a valid author's signature does not replace causal/domain validation"
    );
    assert_eq!(fs::read(f.path(&store)).unwrap(), original);
    assert!(!store.studio_source_is_warm(SERVER, &f.group, f.target, &f.device));
    assert!(receive(&mut store, &f, std::slice::from_ref(&third), &mut b).is_err());
    assert_eq!(fs::read(f.path(&store)).unwrap(), original);
    // Explicit preparation/retry uses the original cursor/page; no mutated in-memory prefix
    // survived the rejection. Normal small cold fallback is safe too, but not a large fallback.
    let state = f.load(&store).unwrap();
    store.retain_studio_source(&f.group, &f.device, state);
    let result = receive(&mut store, &f, &[second, third], &mut b).unwrap();
    assert_eq!(result.accepted, 2);
    assert_eq!(f.load(&store).unwrap().op_count(), 3);
}

#[test]
fn studio_pages_large_saved_provider_and_receiver_reuse_and_reject_changed_bytes() {
    let provider_root = tempfile::tempdir().unwrap();
    let receiver_root = tempfile::tempdir().unwrap();
    let mut provider_store = open(provider_root.path());
    let mut receiver_store = open(receiver_root.path());
    let mut f = Fixture::new(true);
    let mut sender = Sender::new(&mut f);
    // The existing real signed-history fixture exceeds the cold-source rail. There is no
    // unverified dummy padding or authority inferred from copying another member's vault.
    super::performance::save_studio_source_fixture(
        &mut provider_store,
        SERVER,
        &sender.group,
        &sender.device,
        f.target,
    );
    let mut request = StudioPageRequest {
        requester: f.device.device_id(),
        doc_id: f.id,
        heads: &[],
        seed: None,
        cursor: None,
    };
    assert!(
        provider_store
            .serve_studio_page(
                SERVER,
                &sender.group,
                f.target,
                &sender.device,
                &mut sender.pager,
                request,
                &mut rng()
            )
            .is_err(),
        "serving never rebuilds a cold source"
    );
    provider_store
        .with_studio_source(SERVER, &sender.group, f.target, &sender.device, |state| {
            Ok(state.op_count())
        })
        .unwrap();
    let logical = f.target.document(&sender.group.group_id()).unwrap();
    let source_path = provider_store.studio_epoch_path(&scope_bytes(SERVER, &logical).unwrap());
    let original = fs::read(&source_path).unwrap();
    let mut b = budget(&mut receiver_store, &f);
    let mut cursor = None;
    let mut count = 0;
    let restores = crate::store::studio_full_restores_for_test();
    loop {
        request = StudioPageRequest {
            requester: f.device.device_id(),
            doc_id: f.id,
            heads: &[],
            seed: None,
            cursor: cursor
                .as_ref()
                .map(|c: &catcoms_replication::studio::catchup::StudioPageCursor| c.as_bytes()),
        };
        let StudioPageOutcome::Page(page) = provider_store
            .serve_studio_page(
                SERVER,
                &sender.group,
                f.target,
                &sender.device,
                &mut sender.pager,
                request,
                &mut rng(),
            )
            .unwrap()
        else {
            panic!("expected page")
        };
        count += receive(&mut receiver_store, &f, &page.operations, &mut b)
            .unwrap()
            .accepted;
        assert_eq!(crate::store::studio_full_restores_for_test(), restores);
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(count, 3);
    assert!(fs::metadata(f.path(&receiver_store)).unwrap().len() > 256 * 1024);
    assert!(receiver_store.studio_source_is_warm(SERVER, &f.group, f.target, &f.device));
    assert_eq!(
        fs::read(&source_path).unwrap(),
        original,
        "serving is read-only"
    );
    let mut damaged = original.clone();
    *damaged.last_mut().unwrap() ^= 1;
    fs::write(&source_path, damaged).unwrap();
    // Invalid requester must reject before I/O and cannot discard a good retained slot. A
    // valid requester then encounters the actual damage; stale plaintext never gets served.
    request = StudioPageRequest {
        requester: crate::DeviceId::from_bytes([0; 32]),
        doc_id: f.id,
        heads: &[],
        seed: None,
        cursor: None,
    };
    assert!(provider_store
        .serve_studio_page(
            SERVER,
            &sender.group,
            f.target,
            &sender.device,
            &mut sender.pager,
            request,
            &mut rng()
        )
        .is_err());
    assert!(provider_store.studio_source_is_warm(SERVER, &sender.group, f.target, &sender.device));
    request = StudioPageRequest {
        requester: f.device.device_id(),
        doc_id: f.id,
        heads: &[],
        seed: None,
        cursor: None,
    };
    assert!(provider_store
        .serve_studio_page(
            SERVER,
            &sender.group,
            f.target,
            &sender.device,
            &mut sender.pager,
            request,
            &mut rng()
        )
        .is_err());
    assert!(!provider_store.studio_source_is_warm(SERVER, &sender.group, f.target, &sender.device));
}

#[test]
fn studio_pages_empty_duplicate_closed_wrong_epoch_and_caps_are_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let mut f = Fixture::new(true);
    let mut sender = Sender::new(&mut f);
    let op = sender.edit(&f, 0);
    let mut b = budget(&mut store, &f);
    let empty = receive(&mut store, &f, &[], &mut b).unwrap();
    assert!(empty.frontier.heads.is_empty());
    assert!(
        !f.path(&store).exists(),
        "empty epoch zero must remain genuinely absent"
    );
    assert!(store
        .ingest_studio_page(
            SERVER,
            &f.group,
            f.target,
            f.id + 1,
            &f.device,
            &[],
            &mut rng(),
            &mut b
        )
        .is_err());
    assert!(receive(
        &mut store,
        &f,
        &vec![op.clone(); MAX_STUDIO_PAGE_OPS + 1],
        &mut b
    )
    .is_err());
    let mut wrong = op.clone();
    wrong.epoch += 1;
    assert!(receive(&mut store, &f, &[wrong], &mut b).is_err());
    let mut huge = op.clone();
    huge.blob.ciphertext = vec![0; MAX_INBOUND_CIPHERTEXT];
    assert!(receive(&mut store, &f, &[huge.clone(), huge], &mut b)
        .unwrap_err()
        .to_string()
        .contains("byte cap"));
    assert!(!f.path(&store).exists());
    receive(&mut store, &f, std::slice::from_ref(&op), &mut b).unwrap();
    let original = fs::read(f.path(&store)).unwrap();
    let result = receive(&mut store, &f, &[], &mut b).unwrap();
    assert_eq!(result.frontier.heads.len(), 1);
    assert_eq!(fs::read(f.path(&store)).unwrap(), original);
    let state = f.load(&store).unwrap();
    store
        .seal_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            f.receipt(&state, 7),
            0,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    // Drop the stale warm unit explicitly as ordinary successful view access would do.
    let state = f.load(&store).unwrap();
    store.retain_studio_source(&f.group, &f.device, state);
    let closing = fs::read(f.path(&store)).unwrap();
    assert!(receive(&mut store, &f, &[], &mut b).is_err());
    assert!(receive(&mut store, &f, &[op], &mut b).is_err());
    assert_eq!(fs::read(f.path(&store)).unwrap(), closing);
}

#[test]
fn studio_pages_write_and_flush_failure_require_exact_retry_after_reconciliation() {
    // Before-write, after-complete-atomic-write and duplicate-flush errors are different
    // uncertainty windows. None returns a page acknowledgement or retires an intent.
    for failure in 0..4 {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let mut f = Fixture::new(true);
        let mut sender = Sender::new(&mut f);
        let first = sender.edit(&f, 0);
        let second = sender.edit(&f, 1);
        let third = sender.edit(&f, 2);
        let mut b = budget(&mut store, &f);
        receive(&mut store, &f, std::slice::from_ref(&first), &mut b).unwrap();
        let original = fs::read(f.path(&store)).unwrap();
        let page = if failure == 3 {
            vec![]
        } else if failure == 2 {
            vec![first]
        } else {
            vec![second, third]
        };
        let result = store.ingest_studio_page_with_io(
            SERVER,
            &f.group,
            f.target,
            f.id,
            &f.device,
            &page,
            &mut rng(),
            &mut b,
            |path, bytes| {
                if failure == 1 {
                    atomic_write(path, bytes)?;
                }
                Err(invalid("injected write failure"))
            },
            |_, _| Err(invalid("injected flush failure")),
        );
        assert!(result.is_err());
        assert!(b.requires_reconciliation());
        assert!(!store.studio_source_is_warm(SERVER, &f.group, f.target, &f.device));
        if failure == 1 {
            assert_eq!(f.load(&store).unwrap().op_count(), 3);
        } else {
            assert_eq!(fs::read(f.path(&store)).unwrap(), original);
        }
        assert!(
            receive(&mut store, &f, &page, &mut b).is_err(),
            "uncertain budget cannot authorize retry"
        );
        b = budget(&mut store, &f);
        let result = receive(&mut store, &f, &page, &mut b).unwrap();
        assert_eq!(result.accepted, if failure == 0 { 2 } else { 0 });
        assert_eq!(result.duplicates, if failure == 0 { 0 } else { page.len() });
        assert_eq!(f.intents(&store), 0);
    }
}
