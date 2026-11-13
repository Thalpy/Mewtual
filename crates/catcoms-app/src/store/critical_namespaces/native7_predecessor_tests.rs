//! CI-only fixture producer. The unmodified predecessor owns its own verdict table;
//! this producer cannot relabel a protected state as an acceptable old-reader result.
use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};

const SECRET: &[u8] = b"namespace7 real predecessor fixtures";
const CAPACITY: usize = 1 << 30;
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
fn native() -> NativeRegistryPolicy {
    NativeRegistryPolicy::new(
        CriticalCatalogReadPool::new(NativeRegistryPolicy::required_capacity()).unwrap(),
        NativeRegistryPolicy::required_persistent_capacity(),
    )
    .unwrap()
}

#[test]
#[ignore = "CI-only real predecessor fixture producer; requires an empty explicit destination"]
fn generate_native7_fixtures() {
    let root = PathBuf::from(
        std::env::var_os("MEWTUAL_NAMESPACE7_FIXTURES").expect("explicit fixture destination"),
    );
    // Never overwrite or clean an existing tree, including after a failed earlier run.
    fs::create_dir(&root).unwrap();
    let cases = [
        ("control35", None),
        ("claim1", Some(1)),
        ("cut2", Some(2)),
        ("cut3", Some(3)),
        ("cut4", Some(4)),
        ("cut5", Some(5)),
        ("cut6", Some(6)),
        ("cut7", Some(7)),
        ("cut8", Some(8)),
        ("cut9", Some(9)),
        ("cut10", Some(10)),
        ("cut11", Some(11)),
        ("cut12", Some(12)),
        ("cut13", Some(13)),
        ("settled", Some(0)),
        ("unfenced4", Some(4)),
    ];
    for (position, (name, point)) in cases.iter().enumerate() {
        let path = root.join(name);
        let mut rng = ChaCha20Rng::seed_from_u64(0x77f0 + position as u64);
        let store = ServerStore::open(&path, SECRET, &mut rng).unwrap();
        store
            .save_registry(
                &[ServerRecord {
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
        let mut owner =
            NativeRegistryInstallation::prepare(&path, SECRET, &mut rng, current(), native())
                .unwrap();
        match point {
            None => drop(owner),
            Some(0) => {
                owner.retry().unwrap();
                drop(owner.into_native_registry().unwrap());
            }
            Some(point) => {
                super::super::persistence::native_registry::arm_native_install_fault(*point);
                assert!(owner.retry().is_err());
                assert!(super::super::persistence::native_registry::take_native_install_fault());
                drop(owner);
                if *name == "unfenced4" {
                    fs::remove_file(path.join(Slot::Stage.name())).unwrap();
                }
            }
        }
    }
    let manifest = cases
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(root.join("manifest.txt"), manifest).unwrap();
    println!("namespace7 predecessor fixtures: {} written", cases.len());
}

struct NeverRng;
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("cold predecessor round trip drew randomness")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("cold predecessor round trip drew randomness")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("cold predecessor round trip drew randomness")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("cold predecessor round trip drew randomness")
    }
}
impl CryptoRng for NeverRng {}

#[test]
#[ignore = "CI-only head reopening the real predecessor's claim-window governance output"]
fn verify_native7_predecessor_claim_recovery() {
    let root = PathBuf::from(
        std::env::var_os("MEWTUAL_NAMESPACE7_FIXTURES").expect("explicit fixture destination"),
    );
    let path = root.join("after-governance-claim");
    let fixed = [
        path.join(Slot::Current.name()),
        path.join(Slot::Stage.name()),
        path.join(governance_layout::GOVERNANCE_SEED),
        path.join(ARCHIVE_BOOTSTRAP_ROOT)
            .join(governance_layout::GOVERNANCE_MARKER),
    ];
    let before = fixed
        .iter()
        .map(|path| fs::read(path).ok())
        .collect::<Vec<_>>();
    assert!(NativeRegistryStore::open(&path, SECRET, &mut NeverRng, current(), native()).is_err());
    assert!(!path
        .join("chat-critical-namespaces.native-registry.seed")
        .exists());
    assert_eq!(
        fixed
            .iter()
            .map(|path| fs::read(path).ok())
            .collect::<Vec<_>>(),
        before
    );
    let mut actual = PreparedV2GovernanceLayoutStore::open(
        &path,
        SECRET,
        &mut NeverRng,
        V2GovernanceLayoutPolicy::new(current(), CAPACITY).unwrap(),
    )
    .unwrap();
    actual.retry(&mut NeverRng).unwrap();
    assert!(actual.into_governance_layout_store().is_ok());
    println!("namespace7 predecessor round trip: 1 verified");
}
