use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::fs;

fn sealed() -> (GovernedAddFenceData, Zeroizing<Vec<u8>>) {
    let data = GovernedAddFenceData {
        context: TextContextIdData::from_bytes([1; 32]),
        parent: TextHeadIdData::from_bytes([2; 32]),
        invite: [3; 32],
        request: [4; 32],
    };
    let raw = data
        .seal_data(&[5; 32], &mut ChaCha20Rng::seed_from_u64(8))
        .unwrap();
    (data, raw)
}

#[test]
fn governed_add_fence_continues_the_same_partial_file_and_ciphertext() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join(STAGE);
    let (data, raw) = sealed();
    let mut candidate = Some(raw);
    let mut entered = false;
    let mut slot = None;
    ensure_original(&path, &mut candidate, &mut entered, &mut slot, FRAME_BYTES).unwrap();
    let original = slot.as_mut().unwrap();
    let id = original.id;
    let prefix = FRAME_BYTES / 2;
    original.file.write_all(&original.frame[..prefix]).unwrap();
    assert_eq!(
        fs::read(&path).unwrap().as_slice(),
        &original.frame[..prefix]
    );

    ensure_original(&path, &mut candidate, &mut entered, &mut slot, FRAME_BYTES).unwrap();
    let same = slot.as_mut().unwrap();
    assert_eq!(same.id, id);
    fill_original(&path, same).unwrap();
    assert!(candidate.is_none());
    assert_eq!(fs::read(&path).unwrap().as_slice(), same.frame.as_slice());
    assert!(GovernedAddFenceData::open_data(&[5; 32], &same.frame).unwrap() == data);

    let foreign = vec![0; prefix];
    fs::write(&path, &foreign).unwrap();
    assert!(fill_original(&path, same).is_err());
    assert_eq!(fs::read(&path).unwrap(), foreign);
}

#[test]
fn governed_add_fence_never_adopts_equal_preexisting_or_recreates_an_uncertain_slot() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join(STAGE);
    let (_, raw) = sealed();
    fs::write(&path, raw.as_slice()).unwrap();
    let mut candidate = Some(raw);
    let mut entered = false;
    let mut slot = None;
    assert!(ensure_original(&path, &mut candidate, &mut entered, &mut slot, FRAME_BYTES).is_err());
    assert!(entered && slot.is_none());
    assert_eq!(
        fs::read(&path).unwrap().as_slice(),
        candidate.as_ref().unwrap().as_slice()
    );

    fs::remove_file(&path).unwrap();
    assert!(ensure_original(&path, &mut candidate, &mut entered, &mut slot, FRAME_BYTES).is_err());
    assert!(!path.exists());
}

#[test]
fn governed_add_outcome_large_file_retries_only_its_exact_held_prefix() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join(OUTCOME_NAME);
    let mut candidate = Some(Zeroizing::new(vec![0x36; 2 * 1024 * 1024 + 41]));
    let mut entered = false;
    let mut slot = None;
    assert!(ensure_original(&path, &mut candidate, &mut entered, &mut slot, FRAME_BYTES).is_err());
    assert!(!entered && !path.exists());
    ensure_original(&path, &mut candidate, &mut entered, &mut slot, MAX_FRAME).unwrap();
    let original = slot.as_mut().unwrap();
    let prefix = original.frame.len() - 5;
    original.file.write_all(&original.frame[..prefix]).unwrap();
    let mut writer = OpenOptions::new().write(true).open(&path).unwrap();
    writer.seek(SeekFrom::Start((prefix - 1) as u64)).unwrap();
    writer.write_all(&[0x11]).unwrap();
    assert!(fill_original(&path, original).is_err());
    assert_eq!(writer.metadata().unwrap().len(), prefix as u64);
    writer.seek(SeekFrom::Start((prefix - 1) as u64)).unwrap();
    writer.write_all(&[0x36]).unwrap();
    fill_original(&path, original).unwrap();
    assert_eq!(
        fs::read(&path).unwrap().as_slice(),
        original.frame.as_slice()
    );
    writer.seek(SeekFrom::End(0)).unwrap();
    writer.write_all(&[0]).unwrap();
    assert!(require_exact_held(&path, original).is_err());
}
