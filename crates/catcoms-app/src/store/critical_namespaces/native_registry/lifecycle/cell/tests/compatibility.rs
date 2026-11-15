//! Explicit predecessor fixtures. The independently pinned reader harness
//! owns its acceptance table; these tests never supply it verdict flags.
use super::*;
use crate::store::critical_namespaces::native_registry::preparation::tests::{initialize, records};
use crate::store::{
    CriticalCatalogReadPool, NativeIndexPolicy, NativeIndexUpgrade, NativeRegistryInstallation,
    NativeRegistryPolicy, NativeRegistryStore,
};

const CASES: [&str; 11] = [
    "control1",
    "control2",
    "selected3",
    "renamed3",
    "refreshed3",
    "refresh-stage3",
    "partial3",
    "rename-stage3",
    "duplicate3",
    "bad-mac3",
    "authenticated-invalid3",
];
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
fn root() -> PathBuf {
    PathBuf::from(
        std::env::var_os("MEWTUAL_NATIVE3_FIXTURES").expect("explicit fresh fixture destination"),
    )
}

#[test]
#[ignore = "CI-only real predecessor producer; requires an explicit fresh fixture destination"]
fn generate_native3_fixtures() {
    let root = root();
    fs::create_dir(&root).unwrap();
    for (n, case) in CASES.into_iter().enumerate() {
        let path = root.join(case);
        fs::create_dir(&path).unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc701 + n as u64);
        if case == "control1" || case == "control2" {
            initialize(&path, Some(&records()), &mut rng);
            let mut first = NativeRegistryInstallation::prepare(
                &path,
                SECRET,
                &mut rng,
                current_policy(),
                native1(),
            )
            .unwrap();
            first.retry().unwrap();
            let first = first.into_native_registry().unwrap();
            if case == "control2" {
                let mut second = NativeIndexUpgrade::prepare(first, native2(), &mut rng).unwrap();
                second.retry().unwrap();
                drop(second.into_index_store().unwrap());
            } else {
                drop(first);
            }
        } else {
            let mut owner = mounted(&path, &mut rng);
            match case {
                "renamed3" => {
                    let input = request(&owner);
                    let (_, debt) = owner.rename_metadata_data(&input, &mut rng).unwrap();
                    drop(debt);
                }
                "refreshed3" | "refresh-stage3" => {
                    let base = request(&owner);
                    let input = super::super::super::refresh::Request {
                        id: base.id,
                        identity: base.identity,
                        expected_revision: base.expected_revision,
                        invite: "refreshed compatibility invite".into(),
                    };
                    if case == "refresh-stage3" {
                        cuts::arm_fault(cuts::Fault::IndexStage);
                    }
                    let result = owner.refresh_legacy_invite_data(&input, &mut rng);
                    if case == "refresh-stage3" {
                        assert!(result.is_err());
                        assert!(cuts::take_fault_seen());
                    } else {
                        let (_, debt) = result.unwrap();
                        drop(debt);
                    }
                }
                "partial3" | "rename-stage3" => {
                    let input = request(&owner);
                    cuts::arm_fault(if case == "partial3" {
                        cuts::Fault::Index3PartialStage
                    } else {
                        cuts::Fault::IndexStage
                    });
                    assert!(owner.rename_metadata_data(&input, &mut rng).is_err());
                    assert!(cuts::take_fault_seen());
                }
                "duplicate3" | "bad-mac3" => {
                    let mut bytes = fs::read(file(
                        &path,
                        super::super::super::super::preparation::INDEX_NAME,
                    ))
                    .unwrap();
                    if case == "bad-mac3" {
                        *bytes.last_mut().unwrap() ^= 1;
                    }
                    fs::write(
                        file(
                            &path,
                            super::super::super::super::preparation::INDEX_STAGE_NAME,
                        ),
                        bytes,
                    )
                    .unwrap();
                }
                "authenticated-invalid3" => {
                    let store = owner.actual_store_for_test().unwrap();
                    let key = Zeroizing::new(store.keys.db_key().unwrap());
                    let derived =
                        Zeroizing::new(blake3::derive_key(preparation::INDEX_KEY, key.as_slice()));
                    let frame = crate::store::frame(
                        &catcoms_crypto::seal(&derived, b"", &mut rng).unwrap(),
                    );
                    fs::write(
                        file(
                            &path,
                            super::super::super::super::preparation::INDEX_STAGE_NAME,
                        ),
                        frame,
                    )
                    .unwrap();
                }
                "selected3" => {}
                _ => unreachable!(),
            }
            drop(owner);
        }
    }
    println!("native3 predecessor fixtures: 11 written");
}

#[test]
#[ignore = "CI-only round trip; requires all actual predecessor probes to have returned"]
fn verify_native3_predecessor_fixtures() {
    let root = root();
    let mut checked = 0;
    for case in CASES {
        let path = root.join(case);
        match case {
            "control1" => assert_eq!(
                NativeRegistryStore::open(
                    &path,
                    SECRET,
                    &mut NeverRng,
                    current_policy(),
                    native1()
                )
                .unwrap()
                .row_count_data()
                .unwrap(),
                1
            ),
            "control2" => assert_eq!(
                NativeIndexStore::open(&path, SECRET, &mut NeverRng, current_policy(), native2())
                    .unwrap()
                    .row_count_data()
                    .unwrap(),
                1
            ),
            "authenticated-invalid3" => {
                let stage = file(
                    &path,
                    super::super::super::super::preparation::INDEX_STAGE_NAME,
                );
                let before = fs::read(&stage).unwrap();
                assert!(NativeIndexStore::open_lifecycle3_for_test(
                    &path,
                    SECRET,
                    &mut NeverRng,
                    current_policy()
                )
                .is_err());
                assert_eq!(fs::read(stage).unwrap(), before);
            }
            _ => {
                let mut owner = reopen(&path);
                assert_eq!(owner.row_count_data().unwrap(), 1);
                let index = selected(&owner);
                let renamed = matches!(case, "renamed3" | "rename-stage3");
                let refreshed = matches!(case, "refreshed3" | "refresh-stage3");
                assert_eq!(
                    index.rows[0].metadata_revision,
                    u64::from(renamed || refreshed)
                );
                if refreshed {
                    let data::Backend::Legacy { invite, .. } = &index.rows[0].backend else {
                        panic!("Legacy")
                    };
                    assert_eq!(invite, "refreshed compatibility invite");
                }
                assert_eq!(
                    index.rows[0].name,
                    if renamed {
                        "my local group name"
                    } else {
                        "original legacy"
                    }
                );
                assert!(!file(
                    &path,
                    super::super::super::super::preparation::INDEX_STAGE_NAME
                )
                .exists());
            }
        }
        checked += 1;
    }
    assert_eq!(checked, 11);
    println!("native3 predecessor round trip: 11 verified");
}
