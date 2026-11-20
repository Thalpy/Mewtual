//! Real predecessor-binary gate for namespace5 (the member-outbox layout). CI only.
//!
//! CI copies this file, unchanged, into a `git worktree` of `ce402364` (the last commit before
//! namespace5) as `crates/catcoms-app/tests/namespace5_gate.rs`, proves it is the only change
//! there, and runs `cargo test --test namespace5_gate`. No predecessor source line is modified:
//! this drives only that commit's PUBLIC openers, against vaults written by the head under test
//! (`predecessor_fixtures::generate_namespace5_predecessor_fixtures`, an ignored in-crate
//! case). This file is never compiled at the head and must keep compiling against
//! `ce402364`'s public API alone.
//!
//! `MEWTUAL_NAMESPACE5_FIXTURES` names the fixture root; its `manifest.txt` lists one fixture
//! directory name per line and must equal [`FIXTURES`] exactly. The expected verdicts live
//! HERE, not in the head's manifest, so a head change cannot relabel a fenced state as
//! openable and still pass. Classes:
//! - `Control`: a head-written vault without namespace5. Each opener's verdict is pinned, and
//!   the Current3 (or governance) opener must fully open it (open, retry, ready store). No
//!   refusal below is vacuous: a copy, path, passphrase or format problem fails here first.
//! - `Refuse`: every opener refuses at open and the complete tree (every directory name, file
//!   name and byte) is unchanged.
//! - `MayOpen`: no content-bearing member output (a zero-byte claim, or an abandoned
//!   unauthenticated seed with no fence). Every opener's verdict must equal its verdict on the
//!   Current3 control, so the claim provably changes nothing, and the member objects survive.
//!   The full Current3 and governance runs are kept under `after/` for the head's round trip.
//! - `Unfenced`: verify-by-breaking. A refused cut with its fence REMOVED must behave like
//!   the control, proving that cut is refused because of the fence.
use catcoms_app::store::{
    ArchiveBootstrapCatalogPolicy, CanonicalFilenameInventoryPolicy, CheckpointJoinExecutionPolicy,
    CriticalCatalogReadPool, PendingJoinCatalogPolicy, PreparedArchiveBootstrapStore,
    PreparedV2CurrentLayoutStore, PreparedV2GovernanceLayoutStore, ServerStore,
    V2CurrentLayoutPolicy, V2GovernanceLayoutPolicy,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The head test module's passphrase (`current_layout::tests::SECRET`). Fixtures hold no real
/// data.
const PASSPHRASE: &[u8] = b"actual namespace3 private opener";
/// Generous fixed-file capacity: the predecessor's constructors enforce minimums only.
const CAPACITY: usize = 1 << 30;
/// Every member object a namespace5 declaration can create: the seed in the vault directory,
/// the marker and parent under `chat-owner-v2`. Spelled out: the predecessor has no constant.
const MEMBER_OBJECTS: [&str; 3] = [
    "chat-critical-namespaces.member-outbox.seed",
    "chat-owner-v2/member-outbox-layout.bin",
    "chat-owner-v2/member-outbox-v1",
];

/// Every public opener of the predecessor, with and without its JOIN-execution consumer.
const OPENERS: [&str; 10] = [
    "default",
    "catalog-read-pool",
    "pending-join",
    "join-execution",
    "archive",
    "archive+execution",
    "current3",
    "current3+execution",
    "governance",
    "governance+execution",
];
/// Verdicts on a settled Current3 vault without namespace5 (index-aligned with [`OPENERS`]).
/// Every opener below Current3 refuses the Current3 catalog; Current3 and governance open it.
const ON_CURRENT3: [bool; 10] = [
    false, false, false, false, false, false, true, true, true, true,
];
/// Verdicts on a settled governance4 vault without namespace5: only governance reads 4.
const ON_GOVERNANCE4: [bool; 10] = [
    false, false, false, false, false, false, false, false, true, true,
];

#[derive(Clone, Copy, PartialEq)]
enum Class {
    Control(&'static [bool; 10], &'static str),
    Refuse,
    MayOpen,
    Unfenced,
}

/// The exact fixture set, in manifest order, with the verdict class this gate owns.
const FIXTURES: [(&str, Class); 23] = [
    ("c3", Class::Control(&ON_CURRENT3, "current3")),
    ("c34", Class::Control(&ON_GOVERNANCE4, "governance")),
    ("s35", Class::Refuse),
    ("s345", Class::Refuse),
    ("s354", Class::Refuse),
    ("s35-used", Class::Refuse),
    ("cut240", Class::Refuse),
    ("cut241", Class::MayOpen),
    ("cut242", Class::Refuse),
    ("cut243", Class::Refuse),
    ("cut244", Class::Refuse),
    ("cut245", Class::Refuse),
    ("cut246", Class::Refuse),
    ("cut247", Class::Refuse),
    ("cut248", Class::Refuse),
    ("cut249", Class::Refuse),
    ("cut250", Class::Refuse),
    ("cut255", Class::Refuse),
    ("d227", Class::MayOpen),
    ("cut243-unfenced", Class::Unfenced),
    ("join-armed", Class::Refuse),
    ("join-stage", Class::Refuse),
    ("join-prefix", Class::Refuse),
];

type Tree = BTreeMap<PathBuf, Option<Vec<u8>>>;

/// Every directory (as `None`) and regular file (with its bytes) under `root`. Any other kind
/// fails: fixtures contain only real files and directories.
fn tree(root: &Path) -> Tree {
    fn visit(base: &Path, directory: &Path, out: &mut Tree) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path.strip_prefix(base).unwrap().to_path_buf();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                out.insert(relative, None);
                visit(base, &path, out);
            } else {
                assert!(kind.is_file(), "{}: not a regular file", path.display());
                out.insert(relative, Some(fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Tree::new();
    visit(root, root, &mut out);
    out
}

/// The member objects only: a file's bytes, `Some(empty)` for a directory, absent otherwise.
fn member_objects(root: &Path) -> Vec<(&'static str, Option<Vec<u8>>)> {
    MEMBER_OBJECTS
        .iter()
        .map(|name| {
            let path = root.join(name);
            let value = if path.is_dir() {
                Some(Vec::new())
            } else {
                fs::read(&path).ok()
            };
            (*name, value)
        })
        .collect()
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            assert!(kind.is_file());
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn pool() -> CriticalCatalogReadPool {
    CriticalCatalogReadPool::new(CAPACITY).unwrap()
}
fn archive(execution: bool) -> ArchiveBootstrapCatalogPolicy {
    let policy = ArchiveBootstrapCatalogPolicy::new(pool(), CAPACITY)
        .unwrap()
        .with_bootstrap_source_data_read();
    if execution {
        policy.with_checkpoint_join_execution(2, 10_000).unwrap()
    } else {
        policy
    }
}
fn current(execution: bool) -> V2CurrentLayoutPolicy {
    V2CurrentLayoutPolicy::new(
        archive(execution),
        CanonicalFilenameInventoryPolicy::new(16, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
        CAPACITY,
    )
    .unwrap()
}
fn governance(execution: bool) -> V2GovernanceLayoutPolicy {
    V2GovernanceLayoutPolicy::new(current(execution), CAPACITY).unwrap()
}
fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(0x5e5e_5e5e)
}

/// Whether the named predecessor opener opened `dir`. The value drops at once, releasing the
/// vault session before the caller inspects the tree. Opening only: nothing is retried.
fn opens(opener: &str, dir: &Path) -> bool {
    let rng = &mut rng();
    match opener {
        "default" => ServerStore::open(dir, PASSPHRASE, rng).is_ok(),
        "catalog-read-pool" => {
            ServerStore::open_with_catalog_read_pool(dir, PASSPHRASE, rng, pool()).is_ok()
        }
        "pending-join" => ServerStore::open_with_pending_join_catalog(
            dir,
            PASSPHRASE,
            rng,
            PendingJoinCatalogPolicy::new(pool(), CAPACITY).unwrap(),
        )
        .is_ok(),
        "join-execution" => ServerStore::open_with_checkpoint_join_execution(
            dir,
            PASSPHRASE,
            rng,
            CheckpointJoinExecutionPolicy::new(pool(), CAPACITY, 2, 10_000).unwrap(),
        )
        .is_ok(),
        "archive" => {
            PreparedArchiveBootstrapStore::open(dir, PASSPHRASE, rng, archive(false)).is_ok()
        }
        "archive+execution" => {
            PreparedArchiveBootstrapStore::open(dir, PASSPHRASE, rng, archive(true)).is_ok()
        }
        "current3" => {
            PreparedV2CurrentLayoutStore::open(dir, PASSPHRASE, rng, current(false)).is_ok()
        }
        "current3+execution" => {
            PreparedV2CurrentLayoutStore::open(dir, PASSPHRASE, rng, current(true)).is_ok()
        }
        "governance" => {
            PreparedV2GovernanceLayoutStore::open(dir, PASSPHRASE, rng, governance(false)).is_ok()
        }
        "governance+execution" => {
            PreparedV2GovernanceLayoutStore::open(dir, PASSPHRASE, rng, governance(true)).is_ok()
        }
        other => panic!("unknown opener {other}"),
    }
}

/// The full predecessor run of one wrapper: open, settle (`retry`) and a ready store. On the
/// governance wrapper this DECLARES governance4 when it is absent: the predecessor does not
/// know the member seed name, which is the downgrade interleaving the head must survive.
fn full_run(wrapper: &str, dir: &Path) {
    let rng = &mut rng();
    match wrapper {
        "current3" => {
            let mut owner =
                PreparedV2CurrentLayoutStore::open(dir, PASSPHRASE, rng, current(false))
                    .unwrap_or_else(|error| panic!("{}: {error:?}", dir.display()));
            owner.retry(rng).unwrap();
            assert!(owner.into_current_layout_store().is_ok());
        }
        "governance" => {
            let mut owner =
                PreparedV2GovernanceLayoutStore::open(dir, PASSPHRASE, rng, governance(false))
                    .unwrap_or_else(|error| panic!("{}: {error:?}", dir.display()));
            owner.retry(rng).unwrap();
            assert!(owner.into_governance_layout_store().is_ok());
        }
        other => panic!("unknown wrapper {other}"),
    }
}

/// Each opener on its own fresh copy (an opener that opens may settle and write), compared
/// with the pinned verdicts. Every copy must keep the fixture's member objects byte for byte.
fn verdicts_on_copies(name: &str, fixture: &Path, scratch: &Path, expected: &[bool; 10]) {
    let members = member_objects(fixture);
    for (slot, opener) in OPENERS.iter().enumerate() {
        let copy = scratch.join(format!("{name}-{slot}"));
        copy_tree(fixture, &copy);
        assert_eq!(opens(opener, &copy), expected[slot], "{name}: {opener}");
        assert_eq!(
            member_objects(&copy),
            members,
            "{name}: {opener} changed member objects"
        );
    }
}

#[test]
fn the_predecessor_refuses_every_content_bearing_namespace5_state_before_any_cleaner() {
    let root = PathBuf::from(
        std::env::var_os("MEWTUAL_NAMESPACE5_FIXTURES")
            .expect("MEWTUAL_NAMESPACE5_FIXTURES must name the head-generated fixture root"),
    );
    let manifest = fs::read_to_string(root.join("manifest.txt")).unwrap();
    let names: Vec<&str> = manifest.lines().filter(|line| !line.is_empty()).collect();
    let expected: Vec<&str> = FIXTURES.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, expected, "the head's fixture set changed");
    let scratch = tempfile::tempdir().unwrap();
    let after = root.join("after");
    let mut checked = BTreeMap::<&str, usize>::new();
    let mut round_trip = 0;
    for (name, class) in FIXTURES {
        let fixture = root.join(name);
        assert!(fixture.join("vault.bin").is_file(), "{name}: not a vault");
        let label = match class {
            Class::Control(verdicts, wrapper) => {
                verdicts_on_copies(name, &fixture, scratch.path(), verdicts);
                let copy = scratch.path().join(format!("{name}-full"));
                copy_tree(&fixture, &copy);
                full_run(wrapper, &copy);
                "control"
            }
            Class::Refuse => {
                let before = tree(&fixture);
                assert!(
                    member_objects(&fixture)
                        .iter()
                        .any(|(_, value)| value.is_some()),
                    "{name}: a refuse fixture must hold a member object"
                );
                for opener in OPENERS {
                    assert!(!opens(opener, &fixture), "{name}: {opener} opened it");
                    assert_eq!(tree(&fixture), before, "{name}: {opener} changed the tree");
                }
                "refuse"
            }
            Class::MayOpen => {
                assert!(
                    member_objects(&fixture)[0].1.is_some(),
                    "{name}: a may-open fixture must hold the seed name"
                );
                verdicts_on_copies(name, &fixture, scratch.path(), &ON_CURRENT3);
                for wrapper in ["current3", "governance"] {
                    let kept = after.join(format!("{name}-{wrapper}"));
                    copy_tree(&fixture, &kept);
                    full_run(wrapper, &kept);
                    assert_eq!(member_objects(&kept), member_objects(&fixture), "{name}");
                    round_trip += 1;
                }
                "may-open"
            }
            Class::Unfenced => {
                verdicts_on_copies(name, &fixture, scratch.path(), &ON_CURRENT3);
                "unfenced"
            }
        };
        *checked.entry(label).or_default() += 1;
    }
    for label in ["control", "refuse", "may-open", "unfenced"] {
        assert!(
            checked.get(label).copied().unwrap_or(0) > 0,
            "no {label} fixture"
        );
    }
    println!(
        "namespace5 predecessor gate: {} fixtures x {} openers checked: {checked:?}",
        FIXTURES.len(),
        OPENERS.len()
    );
    println!("namespace5 predecessor round trip: {round_trip} written");
}
