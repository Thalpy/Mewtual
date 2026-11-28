//! Structural disk tests only. Decoded DATA supplies no pin or dial authority.
use super::*;
use crate::store::critical_namespaces::read_pool::CriticalCatalogReadPool;
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};

struct NeverRng;
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("exact retry reached RNG")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("exact retry reached RNG")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("exact retry reached RNG")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("exact retry reached RNG")
    }
}
impl rand_core::CryptoRng for NeverRng {}

fn root(temp: &tempfile::TempDir) -> HeldDirectory {
    let path = temp.path().join("vault");
    std::fs::create_dir(&path).unwrap();
    HeldDirectory::open(path).unwrap()
}

fn plan(
    root: &HeldDirectory,
    key: &[u8; 32],
    scope: [u8; 32],
    reservation: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<FounderRouteWrite, AppError> {
    let pool = CriticalCatalogReadPool::new(FOUNDER_ROUTE_FEE).unwrap();
    let fee = pool.try_reserve(FOUNDER_ROUTE_FEE).unwrap();
    let PreparedRouteFrame {
        frame,
        selected,
        stage,
    } = prepare_frame(root, key, scope, reservation, rng)?;
    Ok(FounderRouteWrite {
        scope,
        reservation: Zeroizing::new(reservation.to_vec()),
        frame,
        selected,
        stage,
        _fee: fee,
    })
}

#[test]
fn founder_route_exact_saved_retry_has_no_rng_and_never_crosses_scope() {
    let temp = tempfile::tempdir().unwrap();
    let root = root(&temp);
    let key = scoped_key(&[1; 32], [2; 32]);
    let mut rng = ChaCha20Rng::seed_from_u64(0x7f01);
    let mut first = plan(&root, &key, [2; 32], b"bounded reservation DATA", &mut rng).unwrap();
    cover_frame(&root, &key, &mut first, || Ok(()), || Ok(())).unwrap();
    let original = std::fs::read(root.child(NAME)).unwrap();
    let mut retry = plan(
        &root,
        &key,
        [2; 32],
        b"bounded reservation DATA",
        &mut NeverRng,
    )
    .unwrap();
    assert_eq!(retry.frame.as_slice(), original);
    cover_frame(&root, &key, &mut retry, || Ok(()), || Ok(())).unwrap();
    assert_eq!(std::fs::read(root.child(NAME)).unwrap(), original);
    assert!(!root.child(STAGE).exists());
    assert!(plan(
        &root,
        &key,
        [3; 32],
        b"bounded reservation DATA",
        &mut NeverRng
    )
    .is_err());
    assert!(plan(
        &root,
        &key,
        [2; 32],
        b"changed reservation DATA",
        &mut NeverRng
    )
    .is_err());
    assert!(plan(
        &root,
        &scoped_key(&[4; 32], [2; 32]),
        [2; 32],
        b"bounded reservation DATA",
        &mut NeverRng
    )
    .is_err());
    assert_eq!(std::fs::read(root.child(NAME)).unwrap(), original);
}

#[test]
fn founder_route_complete_stage_retries_without_redrawing_and_partial_stage_stays_held() {
    let temp = tempfile::tempdir().unwrap();
    let root = root(&temp);
    let key = scoped_key(&[5; 32], [6; 32]);
    let mut rng = ChaCha20Rng::seed_from_u64(0x7f02);
    let mut first = plan(&root, &key, [6; 32], b"same signed preimage DATA", &mut rng).unwrap();
    first.stage = Some(open_exact_write(&root.child(STAGE), WriteMode::Create).unwrap());
    finish_exact_write(
        &root.child(STAGE),
        first.stage.as_mut().unwrap(),
        &first.frame[..13],
    )
    .unwrap();
    root.sync().unwrap();
    let partial = std::fs::read(root.child(STAGE)).unwrap();
    assert!(plan(
        &root,
        &key,
        [6; 32],
        b"same signed preimage DATA",
        &mut NeverRng
    )
    .is_err());
    assert_eq!(std::fs::read(root.child(STAGE)).unwrap(), partial);
    assert!(!root.child(NAME).exists());
    // The real warm owner retains the exact frame and first File before
    // the write. Only this prefix may finish; cold has no such witness.
    finish_exact_write(
        &root.child(STAGE),
        first.stage.as_mut().unwrap(),
        &first.frame,
    )
    .unwrap();
    root.sync().unwrap();
    let complete = first.frame.clone();
    drop(first);
    let mut cold = plan(
        &root,
        &key,
        [6; 32],
        b"same signed preimage DATA",
        &mut NeverRng,
    )
    .unwrap();
    assert_eq!(cold.frame, complete);
    cover_frame(&root, &key, &mut cold, || Ok(()), || Ok(())).unwrap();
    assert_eq!(
        std::fs::read(root.child(NAME)).unwrap(),
        complete.as_slice()
    );
    assert!(!root.child(STAGE).exists());
}

#[test]
fn founder_route_warm_stage_replacement_and_late_source_refusal_preserve_original_work() {
    let temp = tempfile::tempdir().unwrap();
    let root = root(&temp);
    let key = scoped_key(&[7; 32], [8; 32]);
    let mut rng = ChaCha20Rng::seed_from_u64(0x7f03);
    let mut first = plan(&root, &key, [8; 32], b"original reservation DATA", &mut rng).unwrap();
    let mut checks = 0;
    assert!(cover_frame(
        &root,
        &key,
        &mut first,
        || {
            checks += 1;
            if checks == 2 {
                Err(refused())
            } else {
                Ok(())
            }
        },
        || Ok(())
    )
    .is_err());
    assert!(root.child(STAGE).is_file());
    assert!(!root.child(NAME).exists());
    let parked = temp.path().join("original-stage");
    std::fs::rename(root.child(STAGE), &parked).unwrap();
    std::fs::write(root.child(STAGE), &first.frame).unwrap();
    assert!(cover_frame(&root, &key, &mut first, || Ok(()), || Ok(())).is_err());
    assert!(!root.child(NAME).exists());
    assert_eq!(
        std::fs::read(root.child(STAGE)).unwrap(),
        first.frame.as_slice()
    );
    std::fs::remove_file(root.child(STAGE)).unwrap();
    std::fs::rename(parked, root.child(STAGE)).unwrap();
    cover_frame(&root, &key, &mut first, || Ok(()), || Ok(())).unwrap();
    assert_eq!(
        std::fs::read(root.child(NAME)).unwrap(),
        first.frame.as_slice()
    );
}

#[test]
fn founder_route_cancelled_first_write_creates_neither_stage_nor_selected_file() {
    let temp = tempfile::tempdir().unwrap();
    let root = root(&temp);
    let key = scoped_key(&[9; 32], [10; 32]);
    let mut rng = ChaCha20Rng::seed_from_u64(0x7f04);
    let mut original = plan(&root, &key, [10; 32], b"exact pre-admission DATA", &mut rng).unwrap();
    let frame = original.frame.clone();
    let mut admissions = 0;
    assert!(cover_frame(
        &root,
        &key,
        &mut original,
        || Ok(()),
        || {
            admissions += 1;
            Err(refused())
        }
    )
    .is_err());
    assert_eq!(admissions, 1);
    assert!(!root.child(STAGE).exists());
    assert!(!root.child(NAME).exists());
    assert!(original.stage.is_none());
    // Fresh original-source admission may finish the same retained plan;
    // the refused attempt did not replace its ciphertext or draw again.
    cover_frame(&root, &key, &mut original, || Ok(()), || Ok(())).unwrap();
    assert_eq!(std::fs::read(root.child(NAME)).unwrap(), frame.as_slice());
}

#[test]
fn founder_route_create_new_final_preserves_a_racing_foreign_destination() {
    let temp = tempfile::tempdir().unwrap();
    let root = root(&temp);
    let key = scoped_key(&[11; 32], [12; 32]);
    let mut rng = ChaCha20Rng::seed_from_u64(0x7f05);
    let mut original = plan(&root, &key, [12; 32], b"one original DATA", &mut rng).unwrap();
    let mut checks = 0;
    assert!(cover_frame(
        &root,
        &key,
        &mut original,
        || {
            checks += 1;
            if checks == 2 {
                std::fs::write(root.child(NAME), b"foreign destination").unwrap();
            }
            Ok(())
        },
        || Ok(())
    )
    .is_err());
    assert_eq!(
        std::fs::read(root.child(NAME)).unwrap(),
        b"foreign destination"
    );
    assert!(root.child(STAGE).is_file());
    assert!(original.selected.is_none());
}

#[test]
fn founder_route_warm_selected_retry_rejects_an_authenticated_in_place_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let root = root(&temp);
    let key = scoped_key(&[13; 32], [14; 32]);
    let mut rng = ChaCha20Rng::seed_from_u64(0x7f06);
    let mut original = plan(&root, &key, [14; 32], b"same reservation DATA", &mut rng).unwrap();
    cover_frame(&root, &key, &mut original, || Ok(()), || Ok(())).unwrap();
    let path = root.child(NAME);
    // Independently seal identical semantics with a different nonce. Copy
    // those authentic bytes into the SAME selected inode and preserve it.
    let mut e = Encoder::with_capacity(MAX_PLAIN);
    e.put_bytes(TAG).unwrap();
    e.put_u16(1);
    e.put_bytes(&[14; 32]).unwrap();
    e.put_bytes(b"same reservation DATA").unwrap();
    let alternate = frame(&seal(&key, &e.finish(), &mut rng).unwrap());
    assert_ne!(alternate.as_slice(), original.frame.as_slice());
    assert_eq!(
        decode_frame(&key, [14; 32], &alternate).unwrap().as_slice(),
        b"same reservation DATA"
    );
    std::fs::write(&path, &alternate).unwrap();
    assert!(cover_frame(&root, &key, &mut original, || Ok(()), || Ok(())).is_err());
    assert_eq!(std::fs::read(path).unwrap(), alternate);
}

#[test]
fn founder_route_cold_dual_file_cut_requires_identical_bytes_and_repairs_neither_file() {
    let temp = tempfile::tempdir().unwrap();
    let root = root(&temp);
    let key = scoped_key(&[15; 32], [16; 32]);
    let mut rng = ChaCha20Rng::seed_from_u64(0x7f07);
    let mut original = plan(&root, &key, [16; 32], b"same reservation DATA", &mut rng).unwrap();
    let mut checks = 0;
    // Real write ordering: Stage and final are both synced, but the owner
    // is interrupted before Stage retirement. This is not a fabricated
    // selected route or a fake registration witness.
    assert!(cover_frame(
        &root,
        &key,
        &mut original,
        || {
            checks += 1;
            if checks == 3 {
                Err(refused())
            } else {
                Ok(())
            }
        },
        || Ok(())
    )
    .is_err());
    assert_eq!(checks, 3);
    let bytes = original.frame.clone();
    drop(original);
    let mut read = read_selected_frame(&root).unwrap().unwrap();
    assert_eq!(
        decode_frame(&key, [16; 32], &read.frame)
            .unwrap()
            .as_slice(),
        b"same reservation DATA"
    );
    require_selected_frame(&root, &mut read).unwrap();
    assert!(read.selected.is_some() && read.stage.is_some());
    drop(read);
    assert_eq!(std::fs::read(root.child(NAME)).unwrap(), bytes.as_slice());
    assert_eq!(std::fs::read(root.child(STAGE)).unwrap(), bytes.as_slice());
    // An exact saved preparation also reuses these bytes without drawing.
    // Read itself leaves both names intact and performs no cleanup.
    drop(
        plan(
            &root,
            &key,
            [16; 32],
            b"same reservation DATA",
            &mut NeverRng,
        )
        .unwrap(),
    );
    let mut e = Encoder::with_capacity(MAX_PLAIN);
    e.put_bytes(TAG).unwrap();
    e.put_u16(1);
    e.put_bytes(&[16; 32]).unwrap();
    e.put_bytes(b"same reservation DATA").unwrap();
    let alternate = frame(&seal(&key, &e.finish(), &mut rng).unwrap());
    assert_ne!(alternate.as_slice(), bytes.as_slice());
    // Even a second valid seal of the same plaintext is contradictory:
    // Stage must be an exact redundant copy of the selected final.
    assert_eq!(
        decode_frame(&key, [16; 32], &alternate).unwrap().as_slice(),
        b"same reservation DATA"
    );
    std::fs::write(root.child(STAGE), &alternate).unwrap();
    assert!(read_selected_frame(&root).is_err());
    assert!(plan(
        &root,
        &key,
        [16; 32],
        b"same reservation DATA",
        &mut NeverRng
    )
    .is_err());
    assert_eq!(std::fs::read(root.child(NAME)).unwrap(), bytes.as_slice());
    assert_eq!(std::fs::read(root.child(STAGE)).unwrap(), alternate);
}
