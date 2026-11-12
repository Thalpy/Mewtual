//! Deterministic member-outbox storage regressions over real files.
//!
//! No fixture issues authority. Signed originals are genuine device signatures wrapped by the
//! cfg(test) durability fixture; the save admission is the cfg(test) durability-only
//! constructor; the vault lease is a real OS session lock. Randomness is a seeded ChaCha20.
//! Crashes are injected at named boundaries (`Cut`) and simulated by dropping the owner and
//! reopening cold; torn writes are simulated by truncating or editing files afterwards.
use super::data::{
    MemberOutboxContextData, MemberOutboxContextSpec, MemberReceiptData, MemberRequestData,
    MemberRequestToken, MemberSignedOriginal,
};
use super::engine::{
    MemberDebtBatch, MemberDebtClass, MemberEntryStatus, MemberLookup, MemberOutbox,
    MemberOutboxError, MemberOutboxOpen, MemberOutboxParentDirectory, MemberOutboxPressure,
    MemberOutboxQuota, MemberOutboxRefusal, MemberOutboxRegistry, MEMBER_OUTBOX_APPEND_FEE,
    MEMBER_OUTBOX_CHECKPOINT_FEE, MEMBER_OUTBOX_DEBT_FEE, MEMBER_OUTBOX_DEFAULT_QUOTA,
    MEMBER_OUTBOX_LOOKUP_FEE, MEMBER_OUTBOX_OWNER_FEE, MEMBER_OUTBOX_RECOVERY_FEE,
};
use super::frames::{
    classify_stage, ControlKind, DispositionCode, HeadData, ObjectRole, StageShape,
    PAGE_FRAME_OVERHEAD,
};
use super::transaction::{
    Cut, HookAction, MemberAppendOutcome, MemberAppendRun, MemberOutboxSaveAdmission,
    PreparedMemberAppend, ReturnedMemberOriginal, HOOKS,
};
use super::trie::TrieLimits;
use crate::store::critical_namespaces::read_pool::{
    CatalogReadReservation, CriticalCatalogReadPool,
};
use catcoms_crypto::DeviceKeypair;
use catcoms_replication::chat::owner_facts::OwnerFactId;
use catcoms_replication::chat::owner_header::OwnerTenureId;
use catcoms_replication::chat::{
    ActionId, ActionSpec, BasisId, Body, GrantId, Realm, SignedAction,
};
use catcoms_storage::private_chat_page::PreparedPrivateChatPage;
use catcoms_storage::{acquire_vault_session, VaultPhysicalWorkLease, VaultSessionGuard};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

const KEY: [u8; 32] = [0x4b; 32];
const GROUP: &[u8] = b"member-outbox-test-group";
const CHANNEL: u128 = 7;

fn token(n: u8) -> MemberRequestToken {
    MemberRequestToken::from_bytes([n; 32])
}
fn admission(n: u8) -> OwnerFactId {
    OwnerFactId::from_bytes([n; 32])
}

fn context_for(device: &DeviceKeypair, tenure: u8) -> MemberOutboxContextData {
    MemberOutboxContextData::from_spec(MemberOutboxContextSpec {
        vault: [0x11; 32],
        layout: [0x12; 32],
        pin: [0x13; 32],
        profile: OwnerFactId::from_bytes([0x14; 32]),
        tenure: OwnerTenureId::from_bytes([tenure; 32]),
        group_nonce: [0x15; 32],
        device: device.verifying_key().to_bytes(),
        group: GROUP,
    })
    .unwrap()
}

fn arm(cut: Cut, action: HookAction) {
    HOOKS.with(|hooks| hooks.borrow_mut().push((cut, action)));
}
fn disarm() {
    HOOKS.with(|hooks| hooks.borrow_mut().clear());
}

fn refusal(error: &MemberOutboxError) -> &MemberOutboxRefusal {
    error
        .refusal()
        .unwrap_or_else(|| panic!("expected a refusal, got {error:?}"))
}

fn request_of(t: MemberRequestToken, action: &SignedAction) -> MemberRequestData<'_> {
    MemberRequestData {
        token: t,
        channel: action.spec().realm.channel(),
        parents: &action.spec().parents,
        body: &action.spec().body,
    }
}

fn truncate(path: &Path, length: u64) {
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_len(length)
        .unwrap();
}

fn namespace_files(owner: &MemberOutbox) -> std::collections::BTreeMap<OsString, Vec<u8>> {
    fs::read_dir(owner.namespace_path_for_test())
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), fs::read(entry.path()).unwrap())
        })
        .collect()
}
fn flip_byte(path: &Path, at: usize) {
    let mut bytes = fs::read(path).unwrap();
    bytes[at] ^= 0x5a;
    fs::write(path, bytes).unwrap();
}

struct Fixture {
    root: tempfile::TempDir,
    parent: MemberOutboxParentDirectory,
    session: Option<VaultSessionGuard>,
    registry: MemberOutboxRegistry,
    pool: CriticalCatalogReadPool,
    device: DeviceKeypair,
    rng: ChaCha20Rng,
}
impl Fixture {
    fn new(seed: u8) -> Self {
        disarm();
        let root = tempfile::tempdir().unwrap();
        let parent_path = root.path().join("member-outboxes");
        fs::create_dir(&parent_path).unwrap();
        let session = acquire_vault_session(root.path().join("vault")).unwrap();
        Self {
            parent: MemberOutboxParentDirectory::temporary_fixture_for_test(&parent_path).unwrap(),
            root,
            session: Some(session),
            registry: MemberOutboxRegistry::new(),
            pool: CriticalCatalogReadPool::new(512 * 1024 * 1024).unwrap(),
            device: DeviceKeypair::from_seed(&[seed; 32]),
            rng: ChaCha20Rng::seed_from_u64(u64::from(seed)),
        }
    }
    fn vault(&self) -> PathBuf {
        self.root.path().join("vault")
    }
    fn parent_path(&self) -> PathBuf {
        self.root.path().join("member-outboxes")
    }
    fn recover(&mut self, owner: &mut MemberOutbox) -> Result<(), MemberOutboxError> {
        let lease = self.lease();
        let work = self.pay(MEMBER_OUTBOX_RECOVERY_FEE);
        owner.recover(&KEY, &mut self.rng, lease, work)
    }
    fn context(&self) -> MemberOutboxContextData {
        context_for(&self.device, 0x21)
    }
    fn pay(&self, fee: usize) -> CatalogReadReservation {
        self.pool.try_reserve(fee).unwrap()
    }
    fn lease(&self) -> VaultPhysicalWorkLease {
        self.session
            .as_ref()
            .expect("live vault session")
            .retain_for_physical_work()
    }
    fn open_full(
        &mut self,
        registry: MemberOutboxRegistry,
        context: MemberOutboxContextData,
        quota: MemberOutboxQuota,
        limits: Option<TrieLimits>,
        create: bool,
    ) -> Result<MemberOutbox, MemberOutboxError> {
        let metadata = self.pay(MEMBER_OUTBOX_OWNER_FEE);
        let work = self.pay(MEMBER_OUTBOX_RECOVERY_FEE);
        let lease = self.lease();
        let request = MemberOutboxOpen {
            registry: &registry,
            parent: &self.parent,
            context,
            quota,
            create,
            pool: &self.pool,
            metadata,
            work,
            lease,
        };
        match limits {
            None => MemberOutbox::open(request, &KEY, &mut self.rng),
            Some(limits) => {
                MemberOutbox::open_with_limits_for_test(request, &KEY, &mut self.rng, limits)
            }
        }
    }
    fn try_open(&mut self) -> Result<MemberOutbox, MemberOutboxError> {
        let context = self.context();
        self.open_full(
            self.registry.clone(),
            context,
            MEMBER_OUTBOX_DEFAULT_QUOTA,
            None,
            true,
        )
    }
    fn open(&mut self) -> MemberOutbox {
        self.try_open().expect("open member outbox")
    }
    fn sign(&self, text: &str, nonce: u8) -> SignedAction {
        self.sign_reply(text, nonce, None)
    }
    fn sign_reply(&self, text: &str, nonce: u8, reply: Option<ActionId>) -> SignedAction {
        SignedAction::sign(
            &self.device,
            ActionSpec {
                realm: Realm::new(GROUP, CHANNEL).unwrap(),
                basis: BasisId::from_bytes([0x31; 32]),
                grant: GrantId::from_bytes([0x32; 32]),
                nonce: [nonce; 16],
                presented_ms: 1_000 + u64::from(nonce),
                parents: reply.into_iter().collect(),
                body: Body::Create {
                    text: text.into(),
                    reply_to: reply,
                },
            },
        )
        .unwrap()
    }
    fn prepare(
        &mut self,
        owner: &MemberOutbox,
        t: MemberRequestToken,
        action: SignedAction,
        under: OwnerFactId,
    ) -> Result<PreparedMemberAppend, Box<ReturnedMemberOriginal>> {
        let payment = self.pay(MEMBER_OUTBOX_APPEND_FEE);
        owner.prepare_append(
            &KEY,
            t,
            MemberSignedOriginal::durability_fixture_for_test(action, under),
            &mut self.rng,
            payment,
        )
    }
    fn run(&self, owner: MemberOutbox, prepared: PreparedMemberAppend) -> MemberAppendRun {
        let admission = MemberOutboxSaveAdmission::durability_only_for_test(
            prepared.admission_facts(),
            self.lease(),
        );
        prepared
            .admit(owner, admission)
            .map_err(|refused| refused.refusal)
            .expect("admitted")
            .run()
    }
    fn save(
        &mut self,
        owner: MemberOutbox,
        t: MemberRequestToken,
        action: SignedAction,
        under: OwnerFactId,
    ) -> (MemberOutbox, MemberReceiptData) {
        let prepared = self.prepare(&owner, t, action, under).expect("prepared");
        let run = self.run(owner, prepared);
        match run.outcome {
            MemberAppendOutcome::Saved(receipt) => (run.owner, receipt),
            other => panic!("expected Saved, got {other:?}"),
        }
    }
    fn lookup(&self, owner: &MemberOutbox, request: &MemberRequestData<'_>) -> MemberLookup {
        owner
            .lookup(&KEY, request, self.pay(MEMBER_OUTBOX_LOOKUP_FEE))
            .expect("lookup")
    }
    fn expect_hit(
        &self,
        owner: &MemberOutbox,
        t: MemberRequestToken,
        action: &SignedAction,
    ) -> MemberEntryStatus {
        match self.lookup(owner, &request_of(t, action)) {
            MemberLookup::Hit(saved) => {
                assert_eq!(saved.original_bytes(), action.encode().as_slice());
                assert_eq!(saved.action(), action);
                assert_eq!(saved.receipt().action, action.id());
                saved.status()
            }
            other => panic!("expected Hit, got {other:?}"),
        }
    }
    fn debt(&self, owner: &MemberOutbox, current: OwnerFactId) -> MemberDebtBatch {
        owner
            .debt_batch(&KEY, current, 8, self.pay(MEMBER_OUTBOX_DEBT_FEE))
            .expect("debt batch")
    }
    fn checkpoint(&mut self, owner: MemberOutbox, batch: MemberDebtBatch) -> MemberOutbox {
        let payment = self.pay(MEMBER_OUTBOX_CHECKPOINT_FEE);
        let prepared = owner
            .prepare_checkpoint(&KEY, batch, &mut self.rng, payment)
            .map_err(|returned| returned.error)
            .expect("prepared checkpoint");
        let run = prepared.run(owner, self.lease());
        run.outcome.expect("checkpoint saved");
        run.owner
    }
}

/// The namespace holds exactly head.bin plus every reachable node, entry and chunk, and the
/// head's persistent counters equal their real sizes: nothing leaked, nothing retired early.
fn assert_consistent(owner: &MemberOutbox) {
    let head = owner.head_data_for_test();
    let mut expected: BTreeSet<OsString> = BTreeSet::new();
    let mut bytes = 0u64;
    let mut add = |path: PathBuf| {
        bytes += fs::metadata(&path).expect("live object exists").len();
        assert!(expected.insert(path.file_name().unwrap().to_owned()));
    };
    for id in owner.reachable_nodes_for_test(&KEY) {
        add(owner.object_path_for_test(ObjectRole::Node, id));
    }
    let selected = owner.clean().unwrap();
    for ordinal in 0..head.next_ordinal {
        let (entry_id, entry) = owner.debt_entry(&KEY, selected, ordinal).unwrap();
        add(owner.object_path_for_test(ObjectRole::Entry, entry_id));
        for chunk in &entry.chunks {
            add(owner.object_path_for_test(ObjectRole::Chunk, *chunk));
        }
    }
    assert_eq!(head.live_objects as usize, expected.len());
    assert_eq!(head.live_bytes, bytes);
    expected.insert("head.bin".into());
    let actual: BTreeSet<OsString> = fs::read_dir(owner.namespace_path_for_test())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn page_frame_overhead_matches_the_real_private_page_codec() {
    let mut rng = ChaCha20Rng::seed_from_u64(1);
    for size in [1usize, 100, 8 * 1024] {
        let page = PreparedPrivateChatPage::prepare(&KEY, &vec![7; size], &mut rng).unwrap();
        assert_eq!(page.as_bytes().len(), size + PAGE_FRAME_OVERHEAD);
    }
}

#[test]
fn head_files_bind_their_context_and_stage_shapes_use_only_the_rebuildable_preamble() {
    let device = DeviceKeypair::from_seed(&[1; 32]);
    let ours = context_for(&device, 0x21).id();
    let theirs = context_for(&device, 0x22).id();
    let mut rng = ChaCha20Rng::seed_from_u64(2);
    let file = HeadData::initial().seal_file(&KEY, ours, &mut rng).unwrap();
    assert_eq!(
        HeadData::open_file(&KEY, ours, &file).unwrap(),
        HeadData::initial()
    );
    assert!(HeadData::open_file(&KEY, theirs, &file).is_err());
    assert!(HeadData::open_file(&[9; 32], ours, &file).is_err());
    let zero = [0; 32];
    let pre = ControlKind::Head.preamble_len();
    assert_eq!(
        classify_stage(ControlKind::Head, ours, &zero, &file),
        StageShape::Complete
    );
    assert_eq!(
        classify_stage(ControlKind::Head, ours, &zero, &[]),
        StageShape::PreambleTorn
    );
    assert_eq!(
        classify_stage(ControlKind::Head, ours, &zero, &file[..pre - 1]),
        StageShape::PreambleTorn
    );
    assert_eq!(
        classify_stage(ControlKind::Head, ours, &zero, &file[..pre + 2]),
        StageShape::BodyTorn
    );
    assert_eq!(
        classify_stage(ControlKind::Head, ours, &zero, &file[..file.len() - 1]),
        StageShape::BodyTorn
    );
    assert_eq!(
        classify_stage(ControlKind::Head, theirs, &zero, &file),
        StageShape::Unknown
    );
    assert_eq!(
        classify_stage(ControlKind::Head, ours, &[1; 32], &file),
        StageShape::Unknown
    );
    let mut longer = file.to_vec();
    longer.push(0);
    assert_eq!(
        classify_stage(ControlKind::Head, ours, &zero, &longer),
        StageShape::Unknown
    );
}

#[test]
fn save_then_lookup_recovers_the_exact_original_and_changed_requests_conflict() {
    let mut fx = Fixture::new(3);
    let owner = fx.open();
    let action = fx.sign("hello while the founder is offline", 1);
    let (owner, receipt) = fx.save(owner, token(1), action.clone(), admission(0xa1));
    assert_eq!(receipt.ordinal, 0);
    assert_eq!(receipt.admission, admission(0xa1));
    assert_eq!(
        fx.expect_hit(&owner, token(1), &action),
        MemberEntryStatus::Pending
    );

    // Different text, different parents (a reply) and a different channel all conflict.
    let other_text = fx.sign("a different message", 1);
    let reply = fx.sign_reply("hello while the founder is offline", 1, Some(action.id()));
    for changed in [&other_text, &reply] {
        assert!(matches!(
            fx.lookup(&owner, &request_of(token(1), changed)),
            MemberLookup::Conflict(found) if *found == receipt
        ));
    }
    let moved = MemberRequestData {
        channel: CHANNEL + 1,
        ..request_of(token(1), &action)
    };
    assert!(matches!(
        fx.lookup(&owner, &moved),
        MemberLookup::Conflict(_)
    ));
    assert!(matches!(
        fx.lookup(&owner, &request_of(token(2), &action)),
        MemberLookup::Absent
    ));
    assert_consistent(&owner);

    drop(owner);
    let owner = fx.open();
    assert_eq!(
        fx.expect_hit(&owner, token(1), &action),
        MemberEntryStatus::Pending
    );
    assert_eq!(owner.pending().unwrap(), 1);
}

#[test]
fn lost_reply_retry_recovers_the_original_and_never_saves_twice() {
    let mut fx = Fixture::new(4);
    let owner = fx.open();
    let action = fx.sign("sent, reply lost", 1);
    let prepared = fx
        .prepare(&owner, token(1), action.clone(), admission(0xa1))
        .unwrap();
    // The reply is lost: the outcome is discarded without being inspected.
    let owner = fx.run(owner, prepared).owner;
    let generation = owner.head_data_for_test().generation;

    // An identical retry, even one re-signed with a fresh nonce, is the SAME request.
    let resigned = fx.sign("sent, reply lost", 9);
    assert_ne!(resigned.id(), action.id());
    for retry in [action.clone(), resigned] {
        let returned = fx
            .prepare(&owner, token(1), retry.clone(), admission(0xa1))
            .unwrap_err();
        match refusal(&returned.error) {
            MemberOutboxRefusal::AlreadySaved(receipt) => assert_eq!(receipt.action, action.id()),
            other => panic!("expected AlreadySaved, got {other:?}"),
        }
        assert_eq!(
            returned.original.action(),
            &retry,
            "custody comes back unchanged"
        );
    }
    let changed = fx.sign("a changed retry", 1);
    let returned = fx
        .prepare(&owner, token(1), changed, admission(0xa1))
        .unwrap_err();
    assert!(matches!(
        refusal(&returned.error),
        MemberOutboxRefusal::TokenConflict(_)
    ));
    assert_eq!(owner.head_data_for_test().generation, generation);
    fx.expect_hit(&owner, token(1), &action);
}

#[test]
fn duplicate_action_under_another_token_is_refused_without_overwrite() {
    let mut fx = Fixture::new(5);
    let context = fx.context();
    let owner = fx
        .open_full(
            fx.registry.clone(),
            context,
            MemberOutboxQuota {
                max_pending: 1,
                ..MEMBER_OUTBOX_DEFAULT_QUOTA
            },
            None,
            true,
        )
        .unwrap();
    let action = fx.sign("once", 1);
    let (owner, _) = fx.save(owner, token(1), action.clone(), admission(0xa1));
    let returned = fx
        .prepare(&owner, token(2), action.clone(), admission(0xa1))
        .unwrap_err();
    assert!(matches!(
        refusal(&returned.error),
        MemberOutboxRefusal::DuplicateAction
    ));
    assert!(matches!(
        fx.lookup(&owner, &request_of(token(2), &action)),
        MemberLookup::Absent
    ));
    fx.expect_hit(&owner, token(1), &action);
    assert_consistent(&owner);
}

#[test]
fn interrupted_namespace_creation_requires_parent_barrier_on_every_retry() {
    let mut fx = Fixture::new(65);
    let namespace = fx.parent_path().join(fx.context().namespace_leaf());
    arm(Cut::NamespaceCreated, HookAction::Crash);
    assert!(fx.try_open().is_err());
    assert!(namespace.is_dir());
    assert!(!namespace.join("head.bin").exists());

    // Reopening a visible directory must still reach the missing parent barrier.
    // Refusal there cannot expose an owner or let initialization acknowledge a save.
    arm(Cut::NamespaceParentSyncPending, HookAction::Crash);
    assert!(fx.try_open().is_err());
    assert!(!namespace.join("head.bin").exists());
    let synced = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = synced.clone();
    arm(
        Cut::NamespaceParentSynced,
        HookAction::Call(Box::new(move || observed.set(true))),
    );
    let owner = fx.open();
    assert!(
        synced.get(),
        "the real parent sync completed before returning the owner"
    );
    let action = fx.sign("saved after completing the original directory debt", 1);
    let (owner, _) = fx.save(owner, token(1), action.clone(), admission(0xa1));
    drop(owner);
    let reopened = fx.open();
    fx.expect_hit(&reopened, token(1), &action);
}

#[test]
fn foreign_signer_or_group_is_refused_before_any_write() {
    let mut fx = Fixture::new(6);
    let owner = fx.open();
    let stranger = DeviceKeypair::from_seed(&[0x77; 32]);
    let foreign = SignedAction::sign(
        &stranger,
        ActionSpec {
            realm: Realm::new(GROUP, CHANNEL).unwrap(),
            basis: BasisId::from_bytes([0x31; 32]),
            grant: GrantId::from_bytes([0x32; 32]),
            nonce: [1; 16],
            presented_ms: 1,
            parents: Vec::new(),
            body: Body::Create {
                text: "not my device".into(),
                reply_to: None,
            },
        },
    )
    .unwrap();
    let returned = fx
        .prepare(&owner, token(1), foreign, admission(0xa1))
        .unwrap_err();
    assert!(matches!(
        refusal(&returned.error),
        MemberOutboxRefusal::Foreign
    ));
    assert_eq!(owner.head_data_for_test(), HeadData::initial());
    assert_consistent(&owner);
}

#[test]
fn admission_must_name_the_exact_prepared_facts_and_returns_everything_when_refused() {
    let mut fx = Fixture::new(7);
    let owner = fx.open();
    let one = fx.sign("one", 1);
    let two = fx.sign("two", 2);
    let first = fx
        .prepare(&owner, token(1), one.clone(), admission(0xa1))
        .unwrap();
    let second = fx.prepare(&owner, token(2), two, admission(0xa1)).unwrap();
    let wrong =
        MemberOutboxSaveAdmission::durability_only_for_test(second.admission_facts(), fx.lease());
    let refused = *first.admit(owner, wrong).unwrap_err();
    assert!(matches!(refused.refusal, MemberOutboxRefusal::Stale));
    assert_eq!(refused.owner.head_data_for_test(), HeadData::initial());
    // Nothing was written; the same prepared append still admits under its own facts.
    let run = fx.run(refused.owner, refused.prepared);
    assert!(matches!(run.outcome, MemberAppendOutcome::Saved(_)));
    // The other prepared append started from a parent that is no longer selected.
    let facts = second.admission_facts();
    let stale = second
        .admit(
            run.owner,
            MemberOutboxSaveAdmission::durability_only_for_test(facts, fx.lease()),
        )
        .unwrap_err();
    assert!(matches!(stale.refusal, MemberOutboxRefusal::Stale));
    fx.expect_hit(&stale.owner, token(1), &one);
}

/// Every boundary of one append, crashed and reopened cold. Each cut follows a complete,
/// synced WAL stage, so recovery must finish the save exactly once and leave no garbage.
#[test]
fn append_crash_at_every_boundary_recovers_exactly_once_without_garbage() {
    for case in 0..9 {
        let mut fx = Fixture::new(10 + case);
        let owner = fx.open();
        let first = fx.sign("already saved", 1);
        let (owner, _) = fx.save(owner, token(1), first.clone(), admission(0xa1));
        let action = fx.sign("crashing save", 2);
        let prepared = fx
            .prepare(&owner, token(2), action.clone(), admission(0xa1))
            .unwrap();
        let outputs = prepared.outputs_for_test().len();
        assert!(
            !prepared.retired_for_test().is_empty(),
            "the old root is replaced"
        );
        let (cut, after_selection) = match case {
            0 => (Cut::WalStageWritten, false),
            1 => (Cut::WalMainNamed, false),
            2 => (Cut::OutputWritten(0), false),
            3 => (Cut::OutputWritten(outputs - 1), false),
            4 => (Cut::OutputsSynced, false),
            5 => (Cut::HeadStageWritten, false),
            6 => (Cut::Selected, true),
            7 => (Cut::RetiredRemoved(0), true),
            _ => (Cut::WalRetired, true),
        };
        arm(cut, HookAction::Crash);
        let run = fx.run(owner, prepared);
        match (&run.outcome, after_selection) {
            (MemberAppendOutcome::Saved(receipt), true) => assert_eq!(receipt.ordinal, 1),
            (MemberAppendOutcome::Uncertain { original, .. }, false) => {
                assert_eq!(original.action(), &action)
            }
            (other, _) => panic!("cut {cut:?}: unexpected {other:?}"),
        }
        // Every injected stop leaves the warm owner uncertain; only a cold reopen decides.
        assert!(run.owner.needs_recovery(), "cut {cut:?}");
        drop(run); // the process dies here
        let owner = fx.open();
        fx.expect_hit(&owner, token(1), &first);
        assert_eq!(
            fx.expect_hit(&owner, token(2), &action),
            MemberEntryStatus::Pending
        );
        assert_eq!(owner.head_data_for_test().next_ordinal, 2);
        assert_consistent(&owner);
    }
}

#[test]
fn a_crash_during_roll_forward_is_itself_recoverable() {
    let mut fx = Fixture::new(30);
    let owner = fx.open();
    let (owner, _) = fx.save(owner, token(1), fx.sign("base", 1), admission(0xa1));
    let action = fx.sign("rolled forward twice", 2);
    let prepared = fx
        .prepare(&owner, token(2), action.clone(), admission(0xa1))
        .unwrap();
    arm(Cut::WalMainNamed, HookAction::Crash);
    drop(fx.run(owner, prepared));
    for cut in [Cut::OutputWritten(0), Cut::HeadStageWritten, Cut::Selected] {
        arm(cut, HookAction::Crash);
        let error = fx.try_open().unwrap_err();
        assert!(
            error.refusal().is_none(),
            "an injected crash is a failure: {error:?}"
        );
    }
    let owner = fx.open();
    fx.expect_hit(&owner, token(2), &action);
    assert_consistent(&owner);
}

#[test]
fn torn_pre_commit_wal_stage_is_removed_as_owned_scratch_and_the_original_resaves() {
    let pre = ControlKind::Wal.preamble_len() as u64;
    for keep in [0, 9, pre - 1, pre, pre + 2, pre + 4, pre + 200] {
        let mut fx = Fixture::new(40);
        let owner = fx.open();
        let (owner, _) = fx.save(owner, token(1), fx.sign("base", 1), admission(0xa1));
        let head = owner.head_data_for_test();
        let action = fx.sign("torn before commit", 2);
        let prepared = fx
            .prepare(&owner, token(2), action.clone(), admission(0xa1))
            .unwrap();
        arm(Cut::WalStageWritten, HookAction::Crash);
        let run = fx.run(owner, prepared);
        let MemberAppendOutcome::Uncertain { original, .. } = run.outcome else {
            panic!("a pre-commit crash is uncertain")
        };
        let stage = run
            .owner
            .namespace_path_for_test()
            .join("outbox.wal.stage.v1");
        drop(run.owner);
        truncate(&stage, keep);
        let owner = fx.open();
        assert!(
            !stage.exists(),
            "keep {keep}: owned torn scratch is removed"
        );
        assert_eq!(owner.head_data_for_test(), head, "nothing was selected");
        assert!(matches!(
            fx.lookup(&owner, &request_of(token(2), &action)),
            MemberLookup::Absent
        ));
        assert_consistent(&owner);
        // The returned custody is the exact original; it saves normally now.
        let payment = fx.pay(MEMBER_OUTBOX_APPEND_FEE);
        let prepared = owner
            .prepare_append(&KEY, token(2), original, &mut fx.rng, payment)
            .unwrap();
        let run = fx.run(owner, prepared);
        assert!(matches!(run.outcome, MemberAppendOutcome::Saved(_)));
        fx.expect_hit(&run.owner, token(2), &action);
    }
}

#[test]
fn complete_but_corrupt_foreign_or_stale_control_files_are_preserved_and_refused() {
    // A complete WAL stage whose MAC fails.
    let mut fx = Fixture::new(41);
    let owner = fx.open();
    let prepared = fx
        .prepare(
            &owner,
            token(1),
            fx.sign("corrupt stage", 1),
            admission(0xa1),
        )
        .unwrap();
    arm(Cut::WalStageWritten, HookAction::Crash);
    let namespace = owner.namespace_path_for_test();
    drop(fx.run(owner, prepared));
    let stage = namespace.join("outbox.wal.stage.v1");
    let length = fs::metadata(&stage).unwrap().len() as usize;
    flip_byte(&stage, length - 3);
    let before = fs::read(&stage).unwrap();
    let error = fx.try_open().unwrap_err();
    assert!(matches!(refusal(&error), MemberOutboxRefusal::Incomplete));
    assert_eq!(fs::read(&stage).unwrap(), before, "preserved byte for byte");

    // A corrupt WAL MAIN.
    let mut fx = Fixture::new(42);
    let owner = fx.open();
    let prepared = fx
        .prepare(
            &owner,
            token(1),
            fx.sign("corrupt main", 1),
            admission(0xa1),
        )
        .unwrap();
    arm(Cut::WalMainNamed, HookAction::Crash);
    let namespace = owner.namespace_path_for_test();
    drop(fx.run(owner, prepared));
    let main = namespace.join("outbox.wal");
    flip_byte(&main, 200);
    let before = fs::read(&main).unwrap();
    assert!(matches!(
        refusal(&fx.try_open().unwrap_err()),
        MemberOutboxRefusal::Incomplete
    ));
    assert_eq!(fs::read(&main).unwrap(), before);

    // A corrupt selected head is never treated as a fresh outbox.
    let mut fx = Fixture::new(43);
    let owner = fx.open();
    let (owner, _) = fx.save(owner, token(1), fx.sign("selected", 1), admission(0xa1));
    let head = owner.namespace_path_for_test().join("head.bin");
    drop(owner);
    flip_byte(&head, 150);
    assert!(matches!(
        refusal(&fx.try_open().unwrap_err()),
        MemberOutboxRefusal::Incomplete
    ));

    // A head stage with no WAL MAIN is not ours.
    let mut fx = Fixture::new(44);
    let owner = fx.open();
    let namespace = owner.namespace_path_for_test();
    drop(owner);
    fs::write(namespace.join("head.stage.v1"), b"planted").unwrap();
    assert!(matches!(
        refusal(&fx.try_open().unwrap_err()),
        MemberOutboxRefusal::Incomplete
    ));
    assert_eq!(
        fs::read(namespace.join("head.stage.v1")).unwrap(),
        b"planted"
    );
}

#[test]
fn a_stale_or_foreign_wal_stage_is_preserved_even_when_it_authenticates() {
    let mut fx = Fixture::new(45);
    let owner = fx.open();
    let prepared = fx
        .prepare(&owner, token(1), fx.sign("first", 1), admission(0xa1))
        .unwrap();
    arm(Cut::WalStageWritten, HookAction::Crash);
    let namespace = owner.namespace_path_for_test();
    drop(fx.run(owner, prepared));
    let stage = namespace.join("outbox.wal.stage.v1");
    let old_stage = fs::read(&stage).unwrap();
    // Recovery promotes and finishes it; the selected head moves on.
    let owner = fx.open();
    drop(owner);
    // The same authentic stage, now bound to a parent that is no longer selected.
    fs::write(&stage, &old_stage).unwrap();
    assert!(matches!(
        refusal(&fx.try_open().unwrap_err()),
        MemberOutboxRefusal::Incomplete
    ));
    assert_eq!(fs::read(&stage).unwrap(), old_stage);
    fs::remove_file(&stage).unwrap();

    // A WAL MAIN from another context's namespace is foreign here.
    let other = context_for(&fx.device, 0x22);
    let foreign_owner = fx
        .open_full(
            fx.registry.clone(),
            other,
            MEMBER_OUTBOX_DEFAULT_QUOTA,
            None,
            true,
        )
        .unwrap();
    let prepared = fx
        .prepare(
            &foreign_owner,
            token(1),
            fx.sign("foreign", 2),
            admission(0xa1),
        )
        .unwrap();
    arm(Cut::WalMainNamed, HookAction::Crash);
    let foreign_namespace = foreign_owner.namespace_path_for_test();
    drop(fx.run(foreign_owner, prepared));
    fs::copy(
        foreign_namespace.join("outbox.wal"),
        namespace.join("outbox.wal"),
    )
    .unwrap();
    assert!(matches!(
        refusal(&fx.try_open().unwrap_err()),
        MemberOutboxRefusal::Incomplete
    ));
}

#[test]
fn torn_outputs_and_head_stage_complete_only_from_their_exact_prefix() {
    for cut in [Cut::OutputsSynced, Cut::HeadStageWritten] {
        let mut fx = Fixture::new(50);
        let owner = fx.open();
        let action = fx.sign(&"long text ".repeat(6_500), 1);
        let prepared = fx
            .prepare(&owner, token(1), action.clone(), admission(0xa1))
            .unwrap();
        let first = prepared.outputs_for_test()[0].clone();
        assert_eq!(first.role, ObjectRole::Chunk);
        let chunks = prepared
            .outputs_for_test()
            .iter()
            .filter(|o| o.role == ObjectRole::Chunk)
            .count();
        assert_eq!(chunks, 2, "a 65 KB action spans two general chunks");
        let chunk = owner.object_path_for_test(first.role, first.id);
        let stage = owner.namespace_path_for_test().join("head.stage.v1");
        arm(cut, HookAction::Crash);
        drop(fx.run(owner, prepared));
        let torn = if cut == Cut::OutputsSynced {
            &chunk
        } else {
            &stage
        };
        let full = fs::metadata(torn).unwrap().len();
        truncate(torn, full / 2);
        let owner = fx.open();
        assert_eq!(
            fs::metadata(&chunk).unwrap().len() as usize,
            first.frame.len()
        );
        fx.expect_hit(&owner, token(1), &action);
        assert_consistent(&owner);
    }

    // A same-length output with a different byte is never repaired or truncated.
    let mut fx = Fixture::new(51);
    let owner = fx.open();
    let prepared = fx
        .prepare(&owner, token(1), fx.sign("mismatch", 1), admission(0xa1))
        .unwrap();
    let first = prepared.outputs_for_test()[0].clone();
    let chunk = owner.object_path_for_test(first.role, first.id);
    arm(Cut::OutputsSynced, HookAction::Crash);
    drop(fx.run(owner, prepared));
    flip_byte(&chunk, 10);
    let before = fs::read(&chunk).unwrap();
    assert!(matches!(
        refusal(&fx.try_open().unwrap_err()),
        MemberOutboxRefusal::Incomplete
    ));
    assert_eq!(fs::read(&chunk).unwrap(), before);
}

#[test]
fn an_exact_pre_existing_output_is_adopted_and_a_different_one_is_preserved() {
    let mut fx = Fixture::new(52);
    let owner = fx.open();
    let prepared = fx
        .prepare(&owner, token(1), fx.sign("exact", 1), admission(0xa1))
        .unwrap();
    let first = prepared.outputs_for_test()[0].clone();
    fs::write(
        owner.object_path_for_test(first.role, first.id),
        &*first.frame,
    )
    .unwrap();
    let run = fx.run(owner, prepared);
    assert!(matches!(run.outcome, MemberAppendOutcome::Saved(_)));
    assert_consistent(&run.owner);

    let owner = run.owner;
    let prepared = fx
        .prepare(&owner, token(2), fx.sign("planted", 2), admission(0xa1))
        .unwrap();
    let first = prepared.outputs_for_test()[0].clone();
    let planted = owner.object_path_for_test(first.role, first.id);
    fs::write(&planted, vec![0; first.frame.len()]).unwrap();
    let mut run = fx.run(owner, prepared);
    assert!(matches!(run.outcome, MemberAppendOutcome::Uncertain { .. }));
    assert!(run.owner.needs_recovery());
    let error = fx.recover(&mut run.owner).unwrap_err();
    assert!(matches!(refusal(&error), MemberOutboxRefusal::Incomplete));
    assert_eq!(fs::read(&planted).unwrap(), vec![0; first.frame.len()]);
}

#[test]
fn interrupted_creation_completes_only_a_namespace_that_holds_nothing_else() {
    // Crash after the generation-0 stage: the complete stage is promoted.
    let mut fx = Fixture::new(60);
    arm(Cut::InitialStageWritten, HookAction::Crash);
    assert!(fx.try_open().unwrap_err().refusal().is_none());
    let owner = fx.open();
    assert_eq!(owner.head_data_for_test(), HeadData::initial());
    assert_consistent(&owner);

    // A torn creation stage alone is removed and creation restarts.
    let mut fx = Fixture::new(61);
    arm(Cut::InitialStageWritten, HookAction::Crash);
    assert!(fx.try_open().is_err());
    let namespace = fx.parent_path().join(fx.context().namespace_leaf());
    truncate(&namespace.join("head.stage.v1"), 20);
    let owner = fx.open();
    assert_eq!(owner.head_data_for_test(), HeadData::initial());

    // An empty pre-existing namespace directory is initialised.
    let mut fx = Fixture::new(62);
    fs::create_dir(fx.parent_path().join(fx.context().namespace_leaf())).unwrap();
    assert_eq!(fx.open().head_data_for_test(), HeadData::initial());

    // A populated namespace that lost head.bin is preserved, never a fresh empty outbox.
    let mut fx = Fixture::new(63);
    let owner = fx.open();
    let (owner, _) = fx.save(
        owner,
        token(1),
        fx.sign("would be lost", 1),
        admission(0xa1),
    );
    let namespace = owner.namespace_path_for_test();
    drop(owner);
    fs::remove_file(namespace.join("head.bin")).unwrap();
    let files = fs::read_dir(&namespace).unwrap().count();
    assert!(matches!(
        refusal(&fx.try_open().unwrap_err()),
        MemberOutboxRefusal::Incomplete
    ));
    assert_eq!(fs::read_dir(&namespace).unwrap().count(), files);

    // Without create, an absent namespace is reported as absent, and nothing is made.
    let mut fx = Fixture::new(64);
    let context = fx.context();
    let error = fx
        .open_full(
            fx.registry.clone(),
            context,
            MEMBER_OUTBOX_DEFAULT_QUOTA,
            None,
            false,
        )
        .unwrap_err();
    assert!(matches!(refusal(&error), MemberOutboxRefusal::Absent));
    assert_eq!(fs::read_dir(fx.parent_path()).unwrap().count(), 0);
}

#[test]
fn rejoin_under_a_new_admission_reads_history_but_never_adopts_old_pending_work() {
    let mut fx = Fixture::new(70);
    let (a1, a2) = (admission(0xa1), admission(0xa2));
    let owner = fx.open();
    let old = fx.sign("sent under A1", 1);
    let (owner, receipt) = fx.save(owner, token(1), old.clone(), a1);
    // Removal and rejoin keep the SAME Profile, tenure, group nonce and device: same context.
    drop(owner);
    let owner = fx.open();
    assert_eq!(owner.context_id(), receipt.context);
    // The historical receipt stays readable, still attributed to A1.
    match fx.lookup(&owner, &request_of(token(1), &old)) {
        MemberLookup::Hit(saved) => assert_eq!(saved.receipt().admission, a1),
        other => panic!("{other:?}"),
    }
    // A2's publisher sees A1's unsent entry as Held and cannot mark it submitted.
    let mut batch = fx.debt(&owner, a2);
    assert_eq!(batch.items()[0].class(), MemberDebtClass::Held);
    assert!(matches!(
        refusal(&batch.record_submitted(0).unwrap_err()),
        MemberOutboxRefusal::NotPublishable
    ));
    drop(batch);
    // A2 cannot take over A1's token, with the same or a changed request.
    let returned = fx
        .prepare(&owner, token(1), fx.sign("sent under A1", 5), a2)
        .unwrap_err();
    match refusal(&returned.error) {
        MemberOutboxRefusal::AlreadySaved(found) => assert_eq!(found.admission, a1),
        other => panic!("{other:?}"),
    }
    let returned = fx
        .prepare(&owner, token(1), fx.sign("rewritten by A2", 6), a2)
        .unwrap_err();
    assert!(matches!(
        refusal(&returned.error),
        MemberOutboxRefusal::TokenConflict(_)
    ));
    // A2's own message queues behind; one checkpoint settles both, each with its disposition.
    let new = fx.sign("sent under A2", 2);
    let (owner, _) = fx.save(owner, token(2), new.clone(), a2);
    let mut batch = fx.debt(&owner, a2);
    let classes: Vec<_> = batch.items().iter().map(|item| item.class()).collect();
    assert_eq!(
        classes,
        [MemberDebtClass::Held, MemberDebtClass::Publishable]
    );
    batch.record_submitted(1).unwrap();
    let owner = fx.checkpoint(owner, batch);
    assert_eq!(owner.pending().unwrap(), 0);
    assert_eq!(
        fx.expect_hit(&owner, token(1), &old),
        MemberEntryStatus::Settled(DispositionCode::Held)
    );
    assert_eq!(
        fx.expect_hit(&owner, token(2), &new),
        MemberEntryStatus::Settled(DispositionCode::Submitted)
    );
    assert_consistent(&owner);

    // A different tenure is a different context and namespace: nothing crosses over.
    let other = context_for(&fx.device, 0x22);
    let isolated = fx
        .open_full(
            fx.registry.clone(),
            other,
            MEMBER_OUTBOX_DEFAULT_QUOTA,
            None,
            true,
        )
        .unwrap();
    assert_ne!(isolated.context_id(), owner.context_id());
    assert!(matches!(
        fx.lookup(&isolated, &request_of(token(1), &old)),
        MemberLookup::Absent
    ));
}

#[test]
fn cancelled_caller_cannot_stop_admitted_work_or_let_a_competitor_overlap_it() {
    use std::sync::mpsc::channel;
    use std::time::Duration;
    let mut fx = Fixture::new(80);
    let owner = fx.open();
    let context = owner.context_id();
    let action = fx.sign("finishes after cancellation", 1);
    let prepared = fx
        .prepare(&owner, token(1), action.clone(), admission(0xa1))
        .unwrap();
    let admission_ =
        MemberOutboxSaveAdmission::durability_only_for_test(prepared.admission_facts(), fx.lease());
    let admitted = prepared.admit(owner, admission_).unwrap();
    // The Store's own session guard goes away; only the admitted worker's lease remains.
    fx.session = None;
    let (paused_tx, paused) = channel();
    let (release, release_rx) = channel::<()>();
    let (done_tx, done) = channel::<bool>();
    let worker = std::thread::spawn(move || {
        arm(
            Cut::WalMainNamed,
            HookAction::Call(Box::new(move || {
                paused_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            })),
        );
        let run = admitted.run();
        // The caller has gone; this reply is lost.
        let _ = done_tx.send(matches!(run.outcome, MemberAppendOutcome::Saved(_)));
        drop(run);
    });
    paused.recv_timeout(Duration::from_secs(10)).unwrap();
    drop(done); // the caller cancels and stops listening
    assert!(
        fx.registry.is_claimed(context),
        "cancellation does not release the claim"
    );
    assert!(
        acquire_vault_session(fx.vault()).is_err(),
        "no replacement Store can mount while admitted IO runs"
    );
    release.send(()).unwrap();
    worker.join().unwrap();
    assert!(!fx.registry.is_claimed(context));
    fx.session = Some(acquire_vault_session(fx.vault()).unwrap());
    let owner = fx.open();
    fx.expect_hit(&owner, token(1), &action);
    assert_consistent(&owner);
}

#[test]
fn a_second_owner_is_busy_and_a_stale_owner_from_another_registry_cannot_commit() {
    let mut fx = Fixture::new(81);
    let owner = fx.open();
    assert!(matches!(
        refusal(&fx.try_open().unwrap_err()),
        MemberOutboxRefusal::Busy
    ));
    // A second registry stands in for a replacement Store: same files, separate claims.
    let context = fx.context();
    let stale = fx
        .open_full(
            MemberOutboxRegistry::new(),
            context,
            MEMBER_OUTBOX_DEFAULT_QUOTA,
            None,
            true,
        )
        .unwrap();
    let won = fx.sign("committed first", 1);
    let (owner, _) = fx.save(owner, token(1), won.clone(), admission(0xa1));
    let prepared = fx
        .prepare(&stale, token(2), fx.sign("stale", 2), admission(0xa1))
        .unwrap();
    let mut run = fx.run(stale, prepared);
    assert!(matches!(run.outcome, MemberAppendOutcome::Uncertain { .. }));
    assert!(
        !run.owner
            .namespace_path_for_test()
            .join("outbox.wal.stage.v1")
            .exists(),
        "the physical precondition refused before writing anything"
    );
    fx.recover(&mut run.owner).unwrap();
    fx.expect_hit(&run.owner, token(1), &won);
    drop(owner);
}

#[test]
fn pressure_refuses_before_admission_and_evicts_nothing() {
    let mut fx = Fixture::new(90);
    let context = fx.context();
    let quota = MemberOutboxQuota {
        max_pending: 2,
        ..MEMBER_OUTBOX_DEFAULT_QUOTA
    };
    let owner = fx
        .open_full(fx.registry.clone(), context, quota, None, true)
        .unwrap();
    let one = fx.sign("one", 1);
    let two = fx.sign("two", 2);
    let (owner, _) = fx.save(owner, token(1), one.clone(), admission(0xa1));
    let (owner, _) = fx.save(owner, token(2), two.clone(), admission(0xa1));
    let head = owner.head_data_for_test();
    // Pressure must not hide the durable result of an existing token, including
    // the conflict result. Both paths run before authoring/sealing another WAL.
    let snapshot = namespace_files(&owner);
    let retry = fx
        .prepare(&owner, token(1), one.clone(), admission(0xa1))
        .unwrap_err();
    assert!(matches!(
        refusal(&retry.error),
        MemberOutboxRefusal::AlreadySaved(_)
    ));
    let changed = fx.sign("changed original request", 1);
    let conflict = fx
        .prepare(&owner, token(1), changed, admission(0xa1))
        .unwrap_err();
    assert!(matches!(
        refusal(&conflict.error),
        MemberOutboxRefusal::TokenConflict(_)
    ));
    assert_eq!(owner.head_data_for_test(), head);
    assert_eq!(namespace_files(&owner), snapshot);
    let three = fx.sign("three", 3);
    let returned = fx
        .prepare(&owner, token(3), three.clone(), admission(0xa1))
        .unwrap_err();
    assert!(matches!(
        refusal(&returned.error),
        MemberOutboxRefusal::Pressure(MemberOutboxPressure::Pending)
    ));
    assert_eq!(returned.original.action(), &three);
    assert_eq!(owner.head_data_for_test(), head);
    fx.expect_hit(&owner, token(1), &one);
    fx.expect_hit(&owner, token(2), &two);
    // Settling debt frees capacity; the audit stays.
    let mut batch = fx.debt(&owner, admission(0xa1));
    batch.record_submitted(0).unwrap();
    let owner = fx.checkpoint(owner, batch);
    let (owner, _) = fx.save(owner, token(3), three, admission(0xa1));
    fx.expect_hit(&owner, token(1), &one);
    assert_consistent(&owner);
    drop(owner);

    // Byte and object quotas, including a quota lowered below existing usage on reopen.
    for (quota, pressure) in [
        (
            MemberOutboxQuota {
                max_live_bytes: 4096,
                ..MEMBER_OUTBOX_DEFAULT_QUOTA
            },
            MemberOutboxPressure::Bytes,
        ),
        (
            MemberOutboxQuota {
                max_live_objects: 4,
                ..MEMBER_OUTBOX_DEFAULT_QUOTA
            },
            MemberOutboxPressure::Objects,
        ),
    ] {
        let context = fx.context();
        let owner = fx
            .open_full(fx.registry.clone(), context, quota, None, true)
            .unwrap();
        let returned = fx
            .prepare(&owner, token(4), fx.sign("four", 4), admission(0xa1))
            .unwrap_err();
        assert!(
            matches!(refusal(&returned.error), MemberOutboxRefusal::Pressure(p) if *p == pressure)
        );
        fx.expect_hit(&owner, token(1), &one);
        assert_eq!(owner.pending().unwrap(), 2, "existing debt is untouched");
        assert_consistent(&owner);
    }
}

#[test]
fn append_must_leave_disk_capacity_for_publication_checkpoint_progress() {
    for objects in [false, true] {
        let mut fx = Fixture::new(185);
        let limits = TrieLimits {
            leaf_rows: 3,
            max_depth: 16,
        };
        let context = fx.context();
        let owner = fx
            .open_full(
                fx.registry.clone(),
                context,
                MEMBER_OUTBOX_DEFAULT_QUOTA,
                Some(limits),
                true,
            )
            .unwrap();
        let action = fx.sign("publication progress under pressure", 1);
        let candidate = fx
            .prepare(&owner, token(1), action.clone(), admission(0xa1))
            .unwrap();
        let (bytes, count) = candidate.initial_peak_for_test();
        drop(candidate);
        drop(owner);
        let quota = MemberOutboxQuota {
            max_live_bytes: if objects {
                MEMBER_OUTBOX_DEFAULT_QUOTA.max_live_bytes
            } else {
                bytes
            },
            max_live_objects: if objects {
                count
            } else {
                MEMBER_OUTBOX_DEFAULT_QUOTA.max_live_objects
            },
            ..MEMBER_OUTBOX_DEFAULT_QUOTA
        };
        let context = fx.context();
        let owner = fx
            .open_full(fx.registry.clone(), context, quota, Some(limits), false)
            .unwrap();
        let before = namespace_files(&owner);
        match fx.prepare(&owner, token(1), action.clone(), admission(0xa1)) {
            Err(returned) => {
                // Refusing before admission is safe; silently admitting an action
                // whose publication cannot be durably checkpointed is not.
                assert!(matches!(
                    refusal(&returned.error),
                    MemberOutboxRefusal::Pressure(_)
                ));
                assert_eq!(namespace_files(&owner), before);
                assert_eq!(owner.pending().unwrap(), 0);
            }
            Ok(prepared) => {
                let run = fx.run(owner, prepared);
                assert!(matches!(run.outcome, MemberAppendOutcome::Saved(_)));
                let owner = run.owner;
                let mut batch = fx.debt(&owner, admission(0xa1));
                batch.record_submitted(0).unwrap();
                let owner = fx.checkpoint(owner, batch);
                assert_eq!(owner.pending().unwrap(), 0);
                fx.expect_hit(&owner, token(1), &action);
                assert_consistent(&owner);
            }
        }
    }
}

#[test]
fn payments_must_come_from_the_owner_pool_and_cover_the_invoice() {
    let mut fx = Fixture::new(91);
    let owner = fx.open();
    let foreign = CriticalCatalogReadPool::new(64 * 1024 * 1024).unwrap();
    for payment in [
        foreign.try_reserve(MEMBER_OUTBOX_APPEND_FEE).unwrap(),
        fx.pay(MEMBER_OUTBOX_APPEND_FEE - 1),
    ] {
        let returned = owner
            .prepare_append(
                &KEY,
                token(1),
                MemberSignedOriginal::durability_fixture_for_test(fx.sign("paid", 1), admission(1)),
                &mut fx.rng,
                payment,
            )
            .unwrap_err();
        assert!(matches!(
            refusal(&returned.error),
            MemberOutboxRefusal::Payment
        ));
    }
}

#[test]
fn index_exhaustion_refuses_before_admission_without_dropping_rows() {
    let mut fx = Fixture::new(92);
    let context = fx.context();
    let tiny = TrieLimits {
        leaf_rows: 1,
        max_depth: 1,
    };
    let mut owner = fx
        .open_full(
            fx.registry.clone(),
            context,
            MEMBER_OUTBOX_DEFAULT_QUOTA,
            Some(tiny),
            true,
        )
        .unwrap();
    let mut saved = Vec::new();
    // At most 16 single-row leaves fit; three rows per entry exhaust within six entries.
    for n in 1..=6u8 {
        let action = fx.sign(&format!("entry {n}"), n);
        match fx.prepare(&owner, token(n), action.clone(), admission(0xa1)) {
            Ok(prepared) => {
                let run = fx.run(owner, prepared);
                assert!(matches!(run.outcome, MemberAppendOutcome::Saved(_)));
                owner = run.owner;
                saved.push((token(n), action));
            }
            Err(returned) => {
                assert!(matches!(
                    refusal(&returned.error),
                    MemberOutboxRefusal::IndexExhausted
                ));
                let head = owner.head_data_for_test();
                for (t, action) in &saved {
                    fx.expect_hit(&owner, *t, action);
                }
                assert_eq!(head.next_ordinal as usize, saved.len());
                assert_consistent(&owner);
                return;
            }
        }
    }
    panic!("sixteen one-row leaves cannot hold eighteen rows");
}

#[test]
fn splits_keep_every_row_reachable_and_retire_only_replaced_nodes() {
    let mut fx = Fixture::new(93);
    let context = fx.context();
    let small = TrieLimits {
        leaf_rows: 2,
        max_depth: 8,
    };
    let mut owner = fx
        .open_full(
            fx.registry.clone(),
            context,
            MEMBER_OUTBOX_DEFAULT_QUOTA,
            Some(small),
            true,
        )
        .unwrap();
    let mut saved = Vec::new();
    for n in 1..=12u8 {
        let action = fx.sign(&format!("split {n}"), n);
        let (next, _) = fx.save(owner, token(n), action.clone(), admission(0xa1));
        owner = next;
        saved.push((token(n), action));
        assert_consistent(&owner);
    }
    assert!(
        owner.reachable_nodes_for_test(&KEY).len() > 3,
        "the root split"
    );
    drop(owner);
    let context = fx.context();
    let owner = fx
        .open_full(
            fx.registry.clone(),
            context,
            MEMBER_OUTBOX_DEFAULT_QUOTA,
            Some(small),
            true,
        )
        .unwrap();
    for (t, action) in &saved {
        fx.expect_hit(&owner, *t, action);
    }
}

#[test]
fn debt_keeps_identical_original_bytes_across_transport_epochs_and_settles_only_a_prefix() {
    let mut fx = Fixture::new(94);
    let a1 = admission(0xa1);
    let owner = fx.open();
    let one = fx.sign("epoch one", 1);
    let two = fx.sign("epoch two", 2);
    let (owner, _) = fx.save(owner, token(1), one.clone(), a1);
    let (owner, _) = fx.save(owner, token(2), two.clone(), a1);
    // "Epoch 5": read the debt, fail to submit. The outbox API takes no epoch at all.
    let first: Vec<Vec<u8>> = fx
        .debt(&owner, a1)
        .items()
        .iter()
        .map(|item| item.original_bytes().to_vec())
        .collect();
    assert_eq!(first, [one.encode(), two.encode()]);
    // "Epoch 6" after a restart: the same exact signed bytes come back.
    drop(owner);
    let owner = fx.open();
    let mut batch = fx.debt(&owner, a1);
    let second: Vec<Vec<u8>> = batch
        .items()
        .iter()
        .map(|i| i.original_bytes().to_vec())
        .collect();
    assert_eq!(second, first);
    assert_eq!(batch.items()[1].action().id(), two.id());
    // Only the later item was submitted: no prefix is settled, nothing is skipped.
    batch.record_submitted(1).unwrap();
    let payment = fx.pay(MEMBER_OUTBOX_CHECKPOINT_FEE);
    let returned = owner
        .prepare_checkpoint(&KEY, batch, &mut fx.rng, payment)
        .unwrap_err();
    assert!(matches!(
        refusal(&returned.error),
        MemberOutboxRefusal::NotPublishable
    ));
    let mut batch = returned.batch;
    let stale_copy = fx.debt(&owner, a1);
    batch.record_submitted(0).unwrap();
    let owner = fx.checkpoint(owner, batch);
    assert_eq!(owner.pending().unwrap(), 0);
    // A batch read before that checkpoint is stale and cannot move the front again.
    let payment = fx.pay(MEMBER_OUTBOX_CHECKPOINT_FEE);
    let mut stale_copy = stale_copy;
    stale_copy.record_submitted(0).unwrap();
    let returned = owner
        .prepare_checkpoint(&KEY, stale_copy, &mut fx.rng, payment)
        .unwrap_err();
    assert!(matches!(
        refusal(&returned.error),
        MemberOutboxRefusal::Stale
    ));
    // Audit survives: originals, receipts and dispositions after the debt cleared.
    for (t, action) in [(token(1), &one), (token(2), &two)] {
        assert_eq!(
            fx.expect_hit(&owner, t, action),
            MemberEntryStatus::Settled(DispositionCode::Submitted)
        );
    }
    assert_consistent(&owner);

    // A batch from another context's owner is foreign here.
    let other = context_for(&fx.device, 0x22);
    let foreign_owner = fx
        .open_full(
            fx.registry.clone(),
            other,
            MEMBER_OUTBOX_DEFAULT_QUOTA,
            None,
            true,
        )
        .unwrap();
    let (foreign_owner, _) = fx.save(foreign_owner, token(1), fx.sign("elsewhere", 3), a1);
    let mut foreign = fx.debt(&foreign_owner, a1);
    foreign.record_submitted(0).unwrap();
    let payment = fx.pay(MEMBER_OUTBOX_CHECKPOINT_FEE);
    let returned = owner
        .prepare_checkpoint(&KEY, foreign, &mut fx.rng, payment)
        .unwrap_err();
    assert!(matches!(
        refusal(&returned.error),
        MemberOutboxRefusal::Foreign
    ));
}

#[test]
fn checkpoint_crash_at_every_boundary_recovers_the_settled_front() {
    for (case, cut) in [
        Cut::WalStageWritten,
        Cut::WalMainNamed,
        Cut::OutputsSynced,
        Cut::HeadStageWritten,
        Cut::Selected,
        Cut::WalRetired,
    ]
    .into_iter()
    .enumerate()
    {
        let mut fx = Fixture::new(100 + case as u8);
        let owner = fx.open();
        let action = fx.sign("checkpointed", 1);
        let (owner, _) = fx.save(owner, token(1), action.clone(), admission(0xa1));
        let mut batch = fx.debt(&owner, admission(0xa1));
        batch.record_submitted(0).unwrap();
        let payment = fx.pay(MEMBER_OUTBOX_CHECKPOINT_FEE);
        let prepared = owner
            .prepare_checkpoint(&KEY, batch, &mut fx.rng, payment)
            .map_err(|returned| returned.error)
            .unwrap();
        arm(cut, HookAction::Crash);
        drop(prepared.run(owner, fx.lease()));
        let owner = fx.open();
        assert_eq!(owner.pending().unwrap(), 0, "cut {cut:?}");
        assert_eq!(
            fx.expect_hit(&owner, token(1), &action),
            MemberEntryStatus::Settled(DispositionCode::Submitted)
        );
        assert_consistent(&owner);
    }
}

#[test]
fn a_corrupt_entry_page_is_an_error_never_absence() {
    let mut fx = Fixture::new(110);
    let owner = fx.open();
    let action = fx.sign("will be corrupted", 1);
    let (owner, receipt) = fx.save(owner, token(1), action.clone(), admission(0xa1));
    let entry = owner.object_path_for_test(ObjectRole::Entry, receipt.entry);
    flip_byte(&entry, 100);
    let result = owner.lookup(
        &KEY,
        &request_of(token(1), &action),
        fx.pay(MEMBER_OUTBOX_LOOKUP_FEE),
    );
    assert!(
        result.is_err(),
        "corruption must not read as Absent: {result:?}"
    );
}
