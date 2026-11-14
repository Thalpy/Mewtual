//! Real producer fixtures for the independently pinned Native4 predecessor.
//! The old binary owns its own verdict table; no acceptance flag is exported.
use super::*;
use crate::store::critical_namespaces::native_registry::{
    data::FounderBindingData,
    lifecycle::founder::{Birth, BirthKind},
    preparation::tests::{current_policy, NeverRng, SECRET},
};
use crate::store::persistence::native_registry::operational::{arm_fault, take_fault_seen, Fault};
use crate::store::{NativeCreationStateData, NativeIndexStore};
use rand_core::SeedableRng;
use std::{
    fs,
    path::{Path, PathBuf},
};

const CASES: [&str; 6] = [
    "selected3",
    "data-pending",
    "data-active",
    "native-pending",
    "native-stage",
    "native-active",
];
fn root() -> PathBuf {
    PathBuf::from(
        std::env::var_os("MEWTUAL_NATIVE4_FIXTURES")
            .expect("explicit fresh Native4 fixture destination"),
    )
}
fn require_fixture_inventory(root: &Path) {
    let names: std::collections::BTreeSet<_> = fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            assert!(
                entry.file_type().unwrap().is_dir(),
                "fixture entries must be real directories"
            );
            entry.file_name().into_string().unwrap()
        })
        .collect();
    assert_eq!(
        names,
        CASES.into_iter().map(str::to_owned).collect(),
        "every fixture must have a pinned predecessor verdict"
    );
}
fn copy_fixture(from: &Path, to: &Path) {
    fs::create_dir(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            copy_fixture(&entry.path(), &target);
        } else {
            assert!(kind.is_file(), "only real fixture files may be copied");
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
#[ignore = "CI-only actual predecessor fixtures; requires an explicit fresh destination"]
async fn generate_native4_fixtures() {
    let root = root();
    fs::create_dir(&root).unwrap();
    let control = root.join("selected3");
    fs::create_dir(&control).unwrap();
    drop(selected(&control));

    // DATA kind1 remains supported. These controls deliberately assert only
    // the predecessor's DATA opener, never a native caller or SDK grant.
    let control = root.join("data-active");
    fs::create_dir(&control).unwrap();
    let (vault, budget) = selected(&control);
    let context = sources(&vault)
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .loan
        .body
        .as_ref()
        .unwrap()
        .work
        .native_network_context()
        .unwrap();
    drop(vault);
    let mut owner = NativeIndexStore::open_lifecycle3_for_test(
        &control,
        SECRET,
        &mut NeverRng,
        current_policy(),
    )
    .unwrap();
    let pending = founder::Pending {
        identity: [21; 32],
        birth: Birth {
            kind: BirthKind::Founder,
            invocation: [22; 32],
            request: [23; 32],
            signer: [24; 32],
            network: [25; 32],
            join_context: None,
        },
        name: "DATA founder control".into(),
        is_dm: false,
        network_frame: Zeroizing::new(vec![26; 40]),
    };
    let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(0xe101);
    let (status, debt) = owner.birth_founder_data(&pending, &mut rng).unwrap();
    drop(debt);
    copy_fixture(&control, &root.join("data-pending"));
    let (_, debt) = owner
        .activate_founder_data(
            &founder::Activate {
                id: status.id,
                identity: pending.identity,
                birth: pending.birth.clone(),
                binding: FounderBindingData {
                    vault: context.0,
                    layout: context.1,
                    group: vec![27; 32],
                    signer: pending.birth.signer,
                    source: [28; 32],
                    profile: [29; 32],
                    pin: [30; 32],
                },
            },
            &mut rng,
        )
        .unwrap();
    drop(debt);
    drop(owner);

    // One original real producer supplies Pending, a complete activation
    // Stage, and Active. Copies are cold DATA snapshots, not new SDK owners.
    let path = root.join("native-active");
    fs::create_dir(&path).unwrap();
    let (vault, budget) = selected(&path);
    let owner = vault
        .capture_native_sources(
            super::super::super::tests::policy()
                .prepare::<catcoms_net::MeshService, rand_chacha::ChaCha20Rng>()
                .unwrap(),
        )
        .finish()
        .unwrap();
    owner
        .prepare_creation(
            command(&vault, &budget),
            request(1, "Original"),
            mesh_inputs(0xe111),
        )
        .unwrap()
        .prepare_device()
        .unwrap();
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_network()
        .unwrap();
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .cover_pending_network()
        .unwrap();
    copy_fixture(&path, &root.join("native-pending"));
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_transport()
        .unwrap();
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_founder()
        .unwrap();
    arm_fault(Fault::IndexStage);
    assert_eq!(
        owner
            .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
            .unwrap()
            .cover_founder_activation(),
        Err(NativeTextCreationRefusal::Storage)
    );
    assert!(take_fault_seen());
    copy_fixture(&path, &root.join("native-stage"));
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .cover_founder_activation()
        .unwrap();
    drop(owner);
    drop(vault);
    require_fixture_inventory(&root);
    println!("native4 predecessor fixtures: 6 written");
}

#[test]
#[ignore = "CI-only round trip after actual predecessor probes finish"]
fn verify_native4_predecessor_fixtures() {
    let root = root();
    require_fixture_inventory(&root);
    for case in CASES {
        let path = root.join(case);
        let mut owner = NativeIndexStore::open_lifecycle3_for_test(
            &path,
            SECRET,
            &mut NeverRng,
            current_policy(),
        )
        .unwrap();
        assert_eq!(
            owner.row_count_data().unwrap(),
            if case == "selected3" { 1 } else { 2 }
        );
        if case != "selected3" {
            let (token, digest) = if case.starts_with("data-") {
                ([22; 32], [23; 32])
            } else {
                let input = request(1, "Original");
                (input.invocation, input.digest)
            };
            let status = owner.lookup_creation_data(token, digest).unwrap().unwrap();
            assert_eq!(
                status.state,
                if case.ends_with("pending") {
                    NativeCreationStateData::Pending
                } else {
                    NativeCreationStateData::Founder
                }
            );
        }
        assert!(!index_stage(&path).exists());
    }
    println!("native4 predecessor round trip: 6 verified");
}
