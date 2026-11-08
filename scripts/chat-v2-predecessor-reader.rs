//! Probe compiled against the ACTUAL f5ef22a3 production sources by
//! check-chat-v2-predecessor.py. Never add modern governance fences here.
use catcoms_app::store::{
    ArchiveBootstrapCatalogPolicy, CanonicalFilenameInventoryPolicy, CriticalCatalogReadPool,
    PreparedV2CurrentLayoutStore, V2CurrentLayoutPolicy,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let directory = args.next().expect("fixture directory");
    let expected = args.next().expect("open or refuse");
    assert!(args.next().is_none());
    let pool = CriticalCatalogReadPool::new(1024 * 1024 * 1024).unwrap();
    let archive = ArchiveBootstrapCatalogPolicy::new(pool, 512 * 1024 * 1024)
        .unwrap()
        .with_bootstrap_source_data_read();
    let policy = V2CurrentLayoutPolicy::new(
        archive,
        CanonicalFilenameInventoryPolicy::new(32, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
        512 * 1024 * 1024,
    )
    .unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x44f7);
    let opening = PreparedV2CurrentLayoutStore::open(
        directory,
        b"actual private governance metadata mount",
        &mut rng,
        policy,
    );
    if expected == "refuse" {
        match opening {
            // This is the predecessor's actual fixed-slot decoder refusal for
            // the intentionally empty legacy stage. Busy, wrong credentials or
            // an unrelated IO failure must not masquerade as compatibility.
            Err(catcoms_app::AppError::Io(message)) if message == "sealed file truncated" => {}
            other => panic!("expected predecessor stage-framing refusal, got {other:?}"),
        }
        println!("CURRENT3_REFUSED");
    } else {
        assert_eq!(expected, "open");
        let mut opening = opening.expect("clean Current3 must be supported by predecessor");
        opening.retry(&mut rng).unwrap();
        drop(opening.into_current_layout_store().unwrap());
        println!("CURRENT3_OPENED");
    }
}
