use super::*;
use crate::store::{
    epoch_budget::{EpochStorageBudget, StorageScope},
    EpochIntentBudget,
};
use crate::DeviceId;
use catcoms_mls::MlsDevice;
use catcoms_replication::{
    epoch_zero_id,
    registry::{registry_document, PointerKey, RegistryOp},
    registry_epoch::catchup::{RegistryOpPage, RegistryPageCursor},
};
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use catcoms_wire::DocType;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const SERVER: u64 = 95;
fn rng() -> ChaCha20Rng {
    ChaCha20Rng::from_seed([95; 32])
}
struct Fixture {
    root: tempfile::TempDir,
    store: ServerStore,
    server: Server<MemNetwork, ChaCha20Rng>,
    budget: EpochStorageBudget,
    intents: EpochIntentBudget,
    key: PointerKey,
    id: u128,
    requester: DeviceId,
    clock: ManualClock,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut store = ServerStore::open(root.path(), b"page-store", &mut rng()).unwrap();
        let clock = ManualClock::new(1000);
        let mut server = Server::found(
            Hub::new().join(PeerId::from_u64(1)),
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(clock.clone()),
            "pages",
        )
        .unwrap();
        let requester = server
            .sync
            .with_registry_context(|_, d, _, _| d.device_id());
        let key = PointerKey::new(DocType::StudioObject, b"private-durable-page".to_vec()).unwrap();
        let logical = registry_document(&server.group_id(), key.bucket()).unwrap();
        let id = epoch_zero_id(DocType::DocRegistry, &logical.logical_key);
        let mut scan = store.scan_epoch_storage_with_registry().unwrap();
        while !scan.step().unwrap().complete {}
        let inventory = scan.finish().unwrap();
        let budget = EpochStorageBudget::from_inventory(
            StorageScope::new(SERVER, &server.group_id()).unwrap(),
            inventory
                .records_for_server(SERVER, &server.group_id())
                .unwrap(),
        )
        .unwrap();
        let intents = EpochIntentBudget::from_inventory(&inventory).unwrap();
        Self {
            root,
            store,
            server,
            budget,
            intents,
            key,
            id,
            requester,
            clock,
        }
    }
    fn edit(&mut self, n: u8) {
        let op = RegistryOp::Put {
            key: self.key.clone(),
            epoch: n as u64,
        }
        .domain_op(&self.server.group_id(), [n; 16])
        .unwrap();
        self.server
            .sync
            .with_registry_context(|g, d, _, r| {
                self.store.edit_registry_epoch(
                    SERVER,
                    g,
                    self.key.bucket(),
                    self.id,
                    d,
                    op,
                    r,
                    &mut self.budget,
                    &mut self.intents,
                )
            })
            .unwrap();
    }
    fn begin(&mut self) -> ServerRegistryPageProvider {
        self.server
            .begin_registry_page_provider(&self.store, SERVER, self.key.bucket())
            .unwrap()
    }
    fn serve(
        &mut self,
        provider: &mut ServerRegistryPageProvider,
        cursor: Option<&[u8]>,
    ) -> Result<RegistryPageOutcome, AppError> {
        self.server.serve_registry_page(
            &self.store,
            provider,
            RegistryPageRequest {
                requester: self.requester,
                doc_id: self.id,
                heads: &[],
                seed: None,
                cursor,
            },
        )
    }
    fn page(
        &mut self,
        provider: &mut ServerRegistryPageProvider,
        cursor: Option<&[u8]>,
    ) -> RegistryOpPage {
        let RegistryPageOutcome::Page(page) = self.serve(provider, cursor).unwrap() else {
            panic!("expected page");
        };
        page
    }
    fn file(&self) -> std::path::PathBuf {
        std::fs::read_dir(self.root.path().join("servers"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "registry-epoch"))
            .unwrap()
    }
}

#[test]
fn registry_page_store_serves_only_saved_history_without_writes_or_intent_retirement() {
    let mut f = Fixture::new();
    let mut provider = f.begin();
    assert!(matches!(
        f.serve(&mut provider, None).unwrap(),
        RegistryPageOutcome::Restart
    ));
    for n in 0..33 {
        f.edit(n);
    }
    let path = f.file();
    let before = std::fs::read(&path).unwrap();
    let first = f.page(&mut provider, None);
    assert_eq!(first.operations.len(), 32);
    let second = f.page(
        &mut provider,
        first.next.as_ref().map(RegistryPageCursor::as_bytes),
    );
    assert_eq!(second.operations.len(), 1);
    assert!(second.next.is_none());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let pending = f
        .server
        .sync
        .with_registry_context(|g, _, _, _| {
            f.store.load_epoch_intents(
                SERVER,
                &registry_document(&g.group_id(), f.key.bucket()).unwrap(),
            )
        })
        .unwrap();
    assert_eq!(pending.pending().len(), 33);
    assert_eq!(format!("{provider:?}"), "ServerRegistryPageProvider { .. }");
    assert!(!format!("{second:?}").contains("private-durable-page"));
}

#[test]
fn registry_page_store_rejects_mount_server_and_bucket_retargeting() {
    let mut f = Fixture::new();
    for n in 0..33 {
        f.edit(n);
    }
    let mut provider = f.begin();
    let cursor = f.page(&mut provider, None).next.unwrap();
    let mut other = f
        .server
        .begin_registry_page_provider(&f.store, SERVER + 1, f.key.bucket())
        .unwrap();
    assert!(f.serve(&mut other, Some(cursor.as_bytes())).is_err());
    let mut other = f
        .server
        .begin_registry_page_provider(&f.store, SERVER, f.key.bucket().wrapping_add(1))
        .unwrap();
    assert!(f.serve(&mut other, Some(cursor.as_bytes())).is_err());
    let snap = f.server.snapshot().unwrap();
    f.server = Server::restore(
        &snap,
        Hub::new().join(PeerId::from_u64(1)),
        rng(),
        Box::new(f.clock.clone()),
        "restored",
    )
    .unwrap();
    assert!(f.serve(&mut provider, Some(cursor.as_bytes())).is_err());
    let mut provider = f.begin();
    drop(f.store);
    f.store = ServerStore::open(f.root.path(), b"page-store", &mut rng()).unwrap();
    assert!(f.serve(&mut provider, None).is_err());
    let mut fresh = f.begin();
    assert_eq!(f.page(&mut fresh, None).operations.len(), 32);
}

#[test]
fn registry_page_store_preflights_untrusted_cursor_before_reading_corrupt_source() {
    let mut f = Fixture::new();
    for n in 0..33 {
        f.edit(n);
    }
    let mut provider = f.begin();
    let cursor = f.page(&mut provider, None).next.unwrap();
    // Corrupt only this fixture's managed file: invalid/expired cursors must never try reading it.
    let path = f.file();
    std::fs::write(&path, b"not a sealed registry record").unwrap();
    let mut bad = cursor.as_bytes().to_vec();
    bad[10] ^= 1;
    let error = f.serve(&mut provider, Some(&bad)).unwrap_err().to_string();
    assert!(
        error.contains("epoch-close signature or authority is invalid"),
        "{error}"
    );
    assert!(
        f.serve(&mut provider, Some(cursor.as_bytes())).is_err(),
        "valid request surfaces corrupt storage, never empty success"
    );
    f.clock.advance_ms(600_000);
    assert!(matches!(
        f.serve(&mut provider, Some(cursor.as_bytes())).unwrap(),
        RegistryPageOutcome::Restart
    ));
    assert_eq!(
        std::fs::read(path).unwrap(),
        b"not a sealed registry record"
    );
}
