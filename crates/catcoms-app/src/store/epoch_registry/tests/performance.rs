//! Opt-in measurements of the actual saved-source page path, not a model of its cost.
//! Setup uses real signed changes and production admission; only the initial disk save is
//! batched through a private test seam. No production authority or quota is bypassed by serving.
//! Run one ignored case per process in release mode; see docs/P1-PERFORMANCE.md.

use super::*;
use automerge::transaction::{CommitOptions, Transactable};
use automerge::{ActorId, AutoCommit, Change, ROOT};
use catcoms_replication::epoch::{MAX_EPOCH_BYTES, MAX_EPOCH_OPERATIONS};
use catcoms_replication::registry_epoch::catchup::{
    RegistryPageCursor, RegistryPageOutcome, RegistryPageProvider, RegistryPageRequest,
    MAX_REGISTRY_PAGE_BYTES, MAX_REGISTRY_PAGE_OPS,
};
use catcoms_replication::{ReplError, SignedOp};
use catcoms_rt::{Clock, Hub, ManualClock, PeerId, SystemClock};
use catcoms_sync::ChannelSync;
use std::collections::BTreeSet;
use std::sync::Arc;

/// Timing is observational only. Protocol lifetimes always use a fixed ManualClock below.
fn timed<T>(clock: &dyn Clock, f: impl FnOnce() -> T) -> (T, u64) {
    let start = clock.monotonic_ms();
    let value = f();
    (value, clock.monotonic_ms().saturating_sub(start))
}

struct Source {
    f: Fixture,
    writer: AutoCommit,
    signed_bytes: usize,
    hashes: BTreeSet<[u8; 32]>,
    seed: Option<[u8; 32]>,
}
impl Source {
    fn new() -> Self {
        let f = Fixture::new();
        let writer =
            AutoCommit::new().with_actor(ActorId::from(f.device.device_id().as_bytes().to_vec()));
        Self {
            f,
            writer,
            signed_bytes: 0,
            hashes: BTreeSet::new(),
            seed: None,
        }
    }

    /// One evolving pointer avoids the independent 2,048-live-pointer admission limit. Marker
    /// ids remain unique, and signed advisory message bytes consume the real epoch byte budget.
    /// These byte-heavy edits are valid hostile input, not representative normal editor output.
    fn candidate(&self, n: usize, message_bytes: usize) -> (AutoCommit, SignedOp) {
        let mut writer = self.writer.clone();
        let mut nonce = [0; 16];
        nonce[..8].copy_from_slice(&(n as u64).to_be_bytes());
        nonce[8..].copy_from_slice(&self.f.source.epoch().to_be_bytes());
        let domain = RegistryOp::Put {
            key: self.f.key.clone(),
            epoch: (n % 4097) as u64,
        }
        .domain_op(&self.f.group.group_id(), nonce)
        .unwrap();
        if self.f.source.epoch() == 0 && n == 0 {
            writer
                .put(ROOT, "bucket", u64::from(self.f.key.bucket()))
                .unwrap();
            writer.put(ROOT, "epoch", 0u64).unwrap();
            writer
                .put(ROOT, "key", hex::encode(&self.f.document.logical_key))
                .unwrap();
            writer.put(ROOT, "kind", "registry").unwrap();
            writer.put(ROOT, "v", 1u64).unwrap();
        }
        writer
            .put(
                ROOT,
                format!("p/0010/{}", hex::encode(self.f.key.logical_key())),
                (n % 4097) as u64,
            )
            .unwrap();
        writer
            .put(
                ROOT,
                format!(
                    "_p1/op/{}",
                    hex::encode(domain.id(&self.f.device.device_id()))
                ),
                1u64,
            )
            .unwrap();
        writer.commit_with(CommitOptions::default().with_message("x".repeat(message_bytes)));
        let signed = SignedOp::sign_domain(
            &self.f.device,
            DocType::DocRegistry,
            self.f.source.doc_id(),
            writer.get_last_local_change().unwrap().raw_bytes().to_vec(),
            &domain,
        )
        .unwrap();
        (writer, signed)
    }

    /// Stops at the actual production admission bound for max cases, not a reduced test cap.
    /// Small smoke cases deliberately stop earlier; the operation-dense result is maximum for
    /// this encoding/fixture, not proof of a globally minimal possible signed operation.
    fn fill(&mut self, count: usize, message_bytes: usize, require_full: bool) {
        for n in 0..=count {
            let (mut next, mut signed) = self.candidate(n, message_bytes);
            let remaining = MAX_EPOCH_BYTES - self.signed_bytes;
            if signed.encode().len() > remaining && message_bytes > 0 {
                // Leave a few bytes for changes in the message-length varint, then prove that
                // the next minimal edit really refuses; do not call a 95%-full log maximal.
                let excess = signed.encode().len() - remaining;
                (next, signed) = self.candidate(n, message_bytes.saturating_sub(excess + 8));
            }
            let bytes = signed.encode().len();
            let sealed =
                SealedOp::seal(&signed, &self.f.group, &self.f.device, &mut rng()).unwrap();
            match self.f.source.ingest(&sealed, &self.f.group, &self.f.device) {
                Ok(Admission::Accepted) => {
                    assert!(n < count, "expected production capacity refusal");
                    self.hashes
                        .insert(Change::from_bytes(signed.delta).unwrap().hash().0);
                    self.writer = next;
                    self.signed_bytes += bytes;
                }
                Err(ReplError::EpochBound) if require_full => {
                    assert!(
                        self.signed_bytes + bytes > MAX_EPOCH_BYTES || n == MAX_EPOCH_OPERATIONS,
                        "a different bound stopped the capacity fixture"
                    );
                    assert_eq!(self.f.source.op_count(), self.hashes.len());
                    println!("P1_PROFILE capacity_ops={n} signed_bytes={} next_signed_bytes={bytes} limiting_bound={}",
                        self.signed_bytes, if n == MAX_EPOCH_OPERATIONS { "operations" } else { "bytes" });
                    return;
                }
                other => panic!("unexpected fixture admission: {other:?}"),
            }
            if n + 1 == count && !require_full {
                return;
            }
            if n > 0 && n % 1024 == 0 {
                println!(
                    "P1_PROFILE setup_ops={} signed_bytes={}",
                    n + 1,
                    self.signed_bytes
                );
            }
        }
        panic!("max fixture never reached a production bound");
    }

    fn rotate(&mut self) {
        let decision = self
            .f
            .source
            .new_owner_decision(&self.f.group, &self.f.device, 0, None)
            .unwrap();
        self.f
            .source
            .seal(decision.receipt().clone(), &self.f.group, 0)
            .unwrap();
        let plan = self
            .f
            .source
            .prepare_settlement(decision.close(), &self.f.group, 0)
            .unwrap();
        let seed = Change::from_bytes(plan.checkpoint().bytes().to_vec()).unwrap();
        self.seed = Some(seed.hash().0);
        self.f.source = self
            .f
            .source
            .checkpoint_successor(&plan, &self.f.group, 0)
            .unwrap();
        self.writer = AutoCommit::new()
            .with_actor(ActorId::from(self.f.device.device_id().as_bytes().to_vec()));
        self.writer.apply_changes([seed]).unwrap();
        self.signed_bytes = 0;
        self.hashes.clear();
    }

    fn wide(&mut self) {
        // Independent real members create 65 roots. The serving roster contains every author;
        // this measures wide-frontier work rather than a removed-author rejection fast path.
        for n in 0..65u8 {
            let author = MlsDevice::generate().unwrap();
            let added = self
                .f
                .group
                .add_member(&self.f.device, author.key_package().unwrap())
                .unwrap();
            let group = ServerGroup::join(&author, &added.welcome).unwrap();
            let mut branch =
                RegistryEpoch::new(&group, self.f.key.bucket(), author.device_id()).unwrap();
            let domain = RegistryOp::Put {
                key: self.f.key.clone(),
                epoch: n as u64,
            }
            .domain_op(&group.group_id(), [n; 16])
            .unwrap();
            let sealed = branch.edit(&author, &group, &mut rng(), &domain).unwrap();
            let key = group
                .channel_secret(&author, DocType::DocRegistry, branch.doc_id())
                .unwrap();
            let signed = sealed.open(&key).unwrap();
            self.signed_bytes += signed.encode().len();
            self.hashes
                .insert(Change::from_bytes(signed.delta).unwrap().hash().0);
            assert_eq!(
                self.f
                    .source
                    .ingest(&sealed, &self.f.group, &self.f.device)
                    .unwrap(),
                Admission::Accepted
            );
        }
        assert_eq!(self.hashes.len(), 65);
        assert!(
            self.f.source.catchup_frontier().heads.is_empty(),
            "wide frontier must not truncate"
        );
    }
}

/// Reuse the real signed profiling source in receiver tests; only its initial save is batched.
/// The unrelated group intentionally exercises whole-vault accounting, not target authority.
pub(crate) fn save_inventory_fixture(store: &mut ServerStore) -> PathBuf {
    save_inventory_fixture_ops(store, 2)
}

pub(super) fn save_inventory_fixture_ops(store: &mut ServerStore, count: usize) -> PathBuf {
    let mut source = Source::new();
    source.fill(count, 160_000, false);
    let path = source.f.path(store);
    let mut budget = budget(store, &source.f);
    store
        .update_registry_with_io(
            SERVER,
            &source.f.group,
            source.f.key.bucket(),
            &source.f.device,
            true,
            WritePurpose::Ordinary,
            &mut rng(),
            &mut budget,
            |unit, _| {
                *unit = source.f.source;
                Ok(())
            },
            atomic_write,
            sync_registry,
        )
        .unwrap();
    assert!(fs::metadata(&path).unwrap().len() > 256 * 1024);
    path
}

/// Measure independent phases, then the real app adapter on first/continuation requests. No
/// network wait or requester authentication is measured: requester is a trusted current member.
/// Only three pages are sampled for expensive cases; the smoke case drains all 33 operations.
fn measure(case: &str, build: impl FnOnce(&mut Source), clock: &dyn Clock, max_pages: usize) {
    println!(
        "P1_PROFILE case={case} pid={} stage=setup",
        std::process::id()
    );
    let (source, setup_ms) = timed(clock, || {
        let mut s = Source::new();
        build(&mut s);
        s
    });
    let expected = source.hashes;
    let count = expected.len();
    let signed_bytes = source.signed_bytes;
    let seed = source.seed;
    // Construction keeps a second Automerge writer. It is not part of production serving;
    // release it before phase measurements (OS lifetime peaks still include setup).
    drop(source.writer);
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let mut budget = budget(&mut store, &source.f);
    let path = source.f.path(&store);
    let scope = scope_bytes(SERVER, &source.f.document).unwrap();
    let bucket = source.f.key.bucket();
    let id = source.f.source.doc_id();
    let epoch = source.f.source.epoch();
    let before_projection = source.f.source.projection().unwrap();
    // Test-only batch installation in a brand-new vault. Every op has already passed the real
    // typed gate; the same production wrapper, snapshot codec, sealing and accounting are used.
    let (_, save_ms) = timed(clock, || {
        store
            .update_registry_with_io(
                SERVER,
                &source.f.group,
                bucket,
                &source.f.device,
                true,
                WritePurpose::Ordinary,
                &mut rng(),
                &mut budget,
                |unit, _| {
                    *unit = source.f.source;
                    Ok(())
                },
                atomic_write,
                sync_registry,
            )
            .unwrap()
    });
    let before = fs::read(&path).unwrap();
    let (cold, inventory_cold_ms) = timed(clock, || {
        let mut scan = store.scan_epoch_storage_with_studio().unwrap();
        let progress = loop {
            let p = scan.step().unwrap();
            if p.complete {
                break p;
            }
        };
        assert_eq!(progress.reused_records, 0);
        scan.finish().unwrap()
    });
    for pass in 0..3 {
        let (_, elapsed) = timed(clock, || {
            let mut scan = store.scan_studio_receive_inventory().unwrap();
            let progress = loop {
                let p = scan.step().unwrap();
                if p.complete {
                    break p;
                }
            };
            assert_eq!(progress.reused_records, 1);
            assert_eq!(progress.uncached_bytes, 0);
            let warm = scan.finish().unwrap();
            assert_eq!(
                warm.records_for_server(SERVER, &source.f.group.group_id())
                    .unwrap(),
                cold.records_for_server(SERVER, &source.f.group.group_id())
                    .unwrap()
            );
        });
        println!("P1_PROFILE inventory_cold_ms={inventory_cold_ms} inventory_warm_pass={pass} inventory_warm_ms={elapsed}");
    }
    let (held, read_ms) = timed(clock, || {
        store.read_registry_record(&scope).unwrap().unwrap()
    });
    let (_, snapshot) = decode_record(&held.plain, &scope, &source.f.document).unwrap();
    let snapshot_bytes = snapshot.len();
    let (unit, restore_ms) = timed(clock, || {
        RegistryEpoch::restore(
            snapshot,
            &source.f.group,
            bucket,
            source.f.device.device_id(),
        )
        .unwrap()
    });
    assert_eq!(unit.op_count(), count);
    assert_eq!(unit.projection().unwrap(), before_projection);
    let requester = source.f.device.device_id();
    let protocol_clock = Arc::new(ManualClock::new(1000));
    let mut provider = RegistryPageProvider::new(requester, protocol_clock.clone(), &mut rng());
    let (detached, detached_ms) = timed(clock, || {
        provider
            .page(
                &unit,
                &source.f.group,
                &source.f.device,
                RegistryPageRequest {
                    requester,
                    doc_id: id,
                    heads: &[],
                    seed,
                    cursor: None,
                },
                &mut rng(),
            )
            .unwrap()
    });
    assert!(matches!(detached, RegistryPageOutcome::Page(_)));
    drop(detached);
    drop(unit);
    drop(held);
    let key = source
        .f
        .group
        .channel_secret(&source.f.device, DocType::DocRegistry, id)
        .unwrap();
    let mut server = crate::Server {
        sync: ChannelSync::new(
            Hub::new().join(PeerId::from_u64(73)),
            source.f.group,
            source.f.device,
            rng(),
            Box::new(protocol_clock),
        ),
        device_id: requester,
        display_name: "profile".into(),
        own_message_changes: Default::default(),
        messages_cache: Default::default(),
        delivery_snapshot_revision: 0,
        devices_sig: None,
    };
    let mut provider = server
        .begin_registry_page_provider(&store, SERVER, bucket)
        .unwrap();
    let (_, prepare_ms) = timed(clock, || {
        let job = server
            .begin_registry_page_preparation(&store, &mut provider)
            .unwrap()
            .unwrap();
        let result = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(job.rebuild())
            .unwrap();
        server
            .finish_registry_page_preparation(&store, &mut provider, result)
            .unwrap();
    });
    println!("P1_PROFILE case={case} source_prepare_ms={prepare_ms}");
    let mut cursor: Option<RegistryPageCursor> = None;
    let mut seen = BTreeSet::new();
    if let Some(seed) = seed {
        seen.insert(seed);
    }
    let mut delivered = 0;
    for page_number in 0..max_pages {
        let (outcome, elapsed_ms) = timed(clock, || {
            server
                .serve_registry_page(
                    &store,
                    &mut provider,
                    RegistryPageRequest {
                        requester,
                        doc_id: id,
                        heads: &[],
                        seed,
                        cursor: cursor.as_ref().map(RegistryPageCursor::as_bytes),
                    },
                )
                .unwrap()
        });
        let RegistryPageOutcome::Page(page) = outcome else {
            panic!("expected measured page")
        };
        let framed_bytes = page
            .operations
            .iter()
            .map(|op| 4 + op.encode().len())
            .sum::<usize>();
        assert!(framed_bytes <= MAX_REGISTRY_PAGE_BYTES);
        assert!(page.operations.len() <= MAX_REGISTRY_PAGE_OPS);
        assert!(page.next.is_none() || !page.operations.is_empty());
        for op in &page.operations {
            let signed = op.open(&key).unwrap();
            assert_eq!(signed.doc_id, id);
            assert_eq!(signed.doc_type, DocType::DocRegistry);
            let change = Change::from_bytes(signed.delta).unwrap();
            assert!(change.deps().iter().all(|dep| seen.contains(&dep.0)));
            assert!(expected.contains(&change.hash().0));
            assert!(
                seen.insert(change.hash().0),
                "cursor repeated an emitted operation"
            );
        }
        delivered += page.operations.len();
        println!("P1_PROFILE case={case} page={page_number} full_path_ms={elapsed_ms} ops={} framed_bytes={framed_bytes} continuation={}", page.operations.len(), page.next.is_some());
        cursor = page.next;
        if cursor.is_none() {
            assert_eq!(delivered, count);
            break;
        }
    }
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "serving changed the saved source"
    );
    if count == 33 {
        assert!(cursor.is_none(), "smoke must drain the continuation");
    }
    println!("P1_PROFILE case={case} epoch={epoch} ops={count} signed_bytes={signed_bytes} snapshot_bytes={snapshot_bytes} file_bytes={} setup_ms={setup_ms} save_ms={save_ms} read_unseal_ms={read_ms} restore_ms={restore_ms} detached_page_ms={detached_ms} sampled_ops={delivered} complete={}", before.len(), cursor.is_none());
}

#[test]
fn registry_profile_smoke() {
    measure("smoke", |s| s.fill(33, 0, false), &ManualClock::new(0), 3);
}

#[test]
fn registry_profile_seeded_smoke() {
    measure(
        "seeded-smoke",
        |s| {
            s.fill(10, 220_000, false);
            s.rotate();
            s.fill(33, 0, false);
        },
        &ManualClock::new(0),
        3,
    );
}

#[test]
#[ignore = "opt-in release profiling; no machine-speed assertion"]
fn profile_registry_bytes() {
    measure(
        "bytes",
        |s| s.fill(MAX_EPOCH_OPERATIONS, 220_000, true),
        &SystemClock,
        3,
    );
}

#[test]
#[ignore = "opt-in release profiling; no machine-speed assertion"]
fn profile_registry_operations() {
    measure(
        "operations",
        |s| s.fill(MAX_EPOCH_OPERATIONS, 0, true),
        &SystemClock,
        3,
    );
}

#[test]
#[ignore = "opt-in release profiling; no machine-speed assertion"]
fn profile_registry_wide() {
    measure("wide", Source::wide, &SystemClock, 3);
}

#[test]
#[ignore = "opt-in release profiling; no machine-speed assertion"]
fn profile_registry_seeded() {
    measure(
        "seeded",
        |s| {
            s.fill(10, 220_000, false);
            s.rotate();
            s.fill(MAX_EPOCH_OPERATIONS, 220_000, true);
        },
        &SystemClock,
        3,
    );
}
