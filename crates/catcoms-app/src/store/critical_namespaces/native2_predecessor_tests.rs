//! Explicit CI fixture producer/verifier; the unmodified predecessor owns its
//! own acceptance table. No head-provided verdict or disabled check is accepted.
use super::*;
use crate::store::persistence::native_registry::operational as io;
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};

const SECRET: &[u8] = b"native index2 real predecessor fixtures";
const CAPACITY: usize = 1 << 30;
const CASES: [(&str, Option<io::Fault>); 18] = [
    ("control35", None),
    ("control7v1", None),
    ("claim2", Some(io::Fault::UpgradeClaim)),
    ("fence", Some(io::Fault::UpgradeFence)),
    ("partial-seed", Some(io::Fault::UpgradePartialSeed)),
    ("seed", Some(io::Fault::UpgradeSeed)),
    ("partial-index", Some(io::Fault::UpgradePartialIndex)),
    ("index-temp", Some(io::Fault::UpgradeIndexTemp)),
    ("index-renamed", Some(io::Fault::UpgradeIndexRename)),
    ("partial-marker", Some(io::Fault::UpgradePartialMarker)),
    ("marker-temp", Some(io::Fault::UpgradeMarkerTemp)),
    ("marker-renamed", Some(io::Fault::UpgradeMarkerRename)),
    ("partial-catalog", Some(io::Fault::UpgradePartialCatalog)),
    ("catalog-renamed", Some(io::Fault::UpgradeCatalogRename)),
    ("covered", Some(io::Fault::UpgradeCovered)),
    ("seed-unlinked", Some(io::Fault::UpgradeSeedUnlink)),
    ("settled2", None),
    ("unfenced-seed", Some(io::Fault::UpgradeSeed)),
];
fn current() -> V2CurrentLayoutPolicy {
    V2CurrentLayoutPolicy::new(
        ArchiveBootstrapCatalogPolicy::new(
            CriticalCatalogReadPool::new(CAPACITY).unwrap(),
            CAPACITY,
        )
        .unwrap()
        .with_bootstrap_source_data_read(),
        CanonicalFilenameInventoryPolicy::new(16, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
        CAPACITY,
    )
    .unwrap()
    .with_member_outbox(
        V2MemberOutboxPolicy::new(
            CriticalCatalogReadPool::new(CAPACITY).unwrap(),
            256,
            256 * 1024 * 1024,
            1 << 20,
            1,
            CAPACITY,
        )
        .unwrap(),
    )
    .unwrap()
}
fn native1() -> NativeRegistryPolicy {
    NativeRegistryPolicy::new(
        CriticalCatalogReadPool::new(NativeRegistryPolicy::required_capacity()).unwrap(),
        NativeRegistryPolicy::required_persistent_capacity(),
    )
    .unwrap()
}
fn native2() -> NativeIndexPolicy {
    NativeIndexPolicy::new(
        CriticalCatalogReadPool::new(NativeIndexPolicy::required_capacity()).unwrap(),
        NativeIndexPolicy::required_persistent_capacity(),
    )
    .unwrap()
}
struct NeverRng;
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("cold compatibility verification reached RNG")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("cold compatibility verification reached RNG")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("cold compatibility verification reached RNG")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("cold compatibility verification reached RNG")
    }
}
impl CryptoRng for NeverRng {}
fn root() -> PathBuf {
    PathBuf::from(
        std::env::var_os("MEWTUAL_NATIVE2_FIXTURES").expect("explicit native2 fixture destination"),
    )
}

#[test]
#[ignore = "CI-only real predecessor producer; requires an explicit fresh fixture destination"]
fn generate_native2_fixtures() {
    let root = root();
    fs::create_dir(&root).unwrap();
    for (position, (name, cut)) in CASES.into_iter().enumerate() {
        let path = root.join(name);
        let mut rng = ChaCha20Rng::seed_from_u64(0x7b00 + position as u64);
        let store = ServerStore::open(&path, SECRET, &mut rng).unwrap();
        store
            .save_registry(
                &[crate::store::ServerRecord {
                    id: 9,
                    display_name: "legacy survives".into(),
                    invite: "kept original invite".into(),
                    is_dm: true,
                }],
                &mut rng,
            )
            .unwrap();
        drop(
            store
                .into_critical_namespace_store(SECRET, &mut rng)
                .unwrap(),
        );
        if name == "control35" {
            let mut prepared =
                PreparedV2CurrentLayoutStore::open(&path, SECRET, &mut rng, current()).unwrap();
            prepared.retry(&mut rng).unwrap();
            drop(prepared.into_current_layout_store().unwrap());
            continue;
        }
        let mut installed =
            NativeRegistryInstallation::prepare(&path, SECRET, &mut rng, current(), native1())
                .unwrap();
        installed.retry().unwrap();
        let previous = installed.into_native_registry().unwrap();
        if name == "control7v1" {
            drop(previous);
            continue;
        }
        let mut upgrade = NativeIndexUpgrade::prepare(previous, native2(), &mut rng).unwrap();
        if let Some(cut) = cut {
            io::arm_fault(cut);
            assert!(upgrade.retry().is_err());
            assert!(io::take_fault_seen());
        } else {
            upgrade.retry().unwrap();
        }
        drop(upgrade);
        if name == "unfenced-seed" {
            fs::remove_file(path.join(Slot::Stage.name())).unwrap();
        }
    }
    assert_eq!(fs::read_dir(&root).unwrap().count(), 18);
    println!("native2 predecessor fixtures: 18 written");
}

#[test]
#[ignore = "CI-only head verification after the actual native1-aware predecessor gate"]
fn verify_native2_predecessor_fixtures() {
    let root = root();
    let mut checked = 0;
    for (name, _) in CASES {
        let path = root.join(name);
        if name == "control35" {
            let mut prepared =
                PreparedV2CurrentLayoutStore::open(&path, SECRET, &mut NeverRng, current())
                    .unwrap();
            prepared.retry(&mut NeverRng).unwrap();
            drop(prepared.into_current_layout_store().unwrap());
        } else if name == "control7v1" {
            assert_eq!(
                NativeRegistryStore::open(&path, SECRET, &mut NeverRng, current(), native1())
                    .unwrap()
                    .row_count_data()
                    .unwrap(),
                1
            );
        } else if ["claim2", "fence", "partial-seed"].contains(&name) {
            assert!(
                NativeIndexStore::open(&path, SECRET, &mut NeverRng, current(), native2()).is_err()
            );
            assert!(!path.join(io::UPGRADE_SEED_NAME).exists());
            assert!(!path.join(Slot::Stage.name()).exists());
            assert_eq!(
                NativeRegistryStore::open(&path, SECRET, &mut NeverRng, current(), native1())
                    .unwrap()
                    .row_count_data()
                    .unwrap(),
                1
            );
        } else {
            assert_eq!(
                NativeIndexStore::open(&path, SECRET, &mut NeverRng, current(), native2())
                    .unwrap_or_else(|error| panic!("head {name}: {error:?}"))
                    .row_count_data()
                    .unwrap(),
                1
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 18);
    println!("native2 predecessor round trip: 18 verified");
}
