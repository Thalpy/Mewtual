//! Read-only exact CURRENT provider comparison for a retained private initial snapshot. This is
//! content equality, not a Fresh/Current/save witness or a complete typed SDK heap admission.
use super::*;

/// Compare complete original snapshot bytes to the SAME live provider under explicit backing,
/// payload and encoded-size bounds. No provider copy, sort, output buffer or replacement snapshot
/// is created. Row order is the original encoder's map iteration order; rehashing can conservatively
/// refuse even with equal unordered contents. Never normalize or mutate the retained original.
pub fn compare_server_snapshot_bounded(
    device: &MlsDevice,
    group: &ServerGroup,
    snapshot: &[u8],
    max_bytes: usize,
    max_backing_bytes: usize,
) -> Result<(), MlsError> {
    let bad = || MlsError::Internal("MLS source differs from retained bounded snapshot");
    if snapshot.len() > max_bytes {
        return Err(bad());
    }
    let mut d = Decoder::new(snapshot);
    let storage = d.get_bytes().map_err(|_| bad())?;
    let public = d.get_bytes().map_err(|_| bad())?;
    let gid = d.get_bytes().map_err(|_| bad())?;
    d.finish().map_err(|_| bad())?;
    if public != device.public_key_ref() || gid != group.group_id_ref() {
        return Err(bad());
    }
    let outer = 12usize
        .checked_add(public.len())
        .and_then(|n| n.checked_add(gid.len()))
        .ok_or_else(bad)?;
    let storage_budget = max_bytes.checked_sub(outer).ok_or_else(bad)?;
    let payload_budget = max_bytes.checked_mul(2).ok_or_else(bad)?;
    let map = device
        .provider()
        .storage()
        .values
        .read()
        .map_err(|_| bad())?;
    // This O(1) physical backing admission precedes any iterator, including tombstone-heavy
    // tables. Keep this guard through payload/count/output preflight AND every comparison.
    let (count, size) = preflight_map(&map, storage_budget, max_backing_bytes, payload_budget)?;
    if storage.len() != size || outer.checked_add(size) != Some(snapshot.len()) {
        return Err(bad());
    }
    let mut rows = Decoder::new(storage);
    if rows.get_u32().map_err(|_| bad())? != count {
        return Err(bad());
    }
    for (key, value) in map.iter() {
        if rows.get_bytes().map_err(|_| bad())? != key.as_slice()
            || rows.get_bytes().map_err(|_| bad())? != value.as_slice()
        {
            return Err(bad());
        }
    }
    rows.finish().map_err(|_| bad())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> (MlsDevice, ServerGroup) {
        let device = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&device).unwrap();
        (device, group)
    }
    fn backing(device: &MlsDevice) -> usize {
        device
            .provider()
            .storage()
            .values
            .read()
            .unwrap()
            .allocation_size()
    }
    #[test]
    fn actual_current_provider_matches_without_encoding_or_replacing_original() {
        let (device, group) = source();
        let snapshot = snapshot_server_bounded(&device, &group, 1024 * 1024, 1024 * 1024).unwrap();
        let before = SNAPSHOT_ENCODING_ENTRIES.with(std::cell::Cell::get);
        for _ in 0..3 {
            compare_server_snapshot_bounded(
                &device,
                &group,
                &snapshot,
                1024 * 1024,
                backing(&device),
            )
            .unwrap();
        }
        assert_eq!(SNAPSHOT_ENCODING_ENTRIES.with(std::cell::Cell::get), before);
        let auth = group.epoch_authenticator_id();
        device
            .key_package_for_invite(group.group_id_ref(), [1; 16])
            .unwrap();
        assert_eq!(group.epoch_authenticator_id(), auth);
        assert!(compare_server_snapshot_bounded(
            &device,
            &group,
            &snapshot,
            1024 * 1024,
            backing(&device)
        )
        .is_err());
        assert_eq!(SNAPSHOT_ENCODING_ENTRIES.with(std::cell::Cell::get), before);
    }
    #[test]
    fn actual_backing_refuses_before_rows_and_full_framing_or_foreign_source_refuses() {
        let (device, group) = source();
        let snapshot = snapshot_server(&device, &group).unwrap();
        let scanned = SNAPSHOT_SCANNED_ROWS.with(std::cell::Cell::get);
        assert!(compare_server_snapshot_bounded(
            &device,
            &group,
            &snapshot,
            1024 * 1024,
            backing(&device) - 1
        )
        .is_err());
        assert_eq!(SNAPSHOT_SCANNED_ROWS.with(std::cell::Cell::get), scanned);
        let (other, other_group) = source();
        assert!(compare_server_snapshot_bounded(
            &other,
            &other_group,
            &snapshot,
            1024 * 1024,
            backing(&other)
        )
        .is_err());
        for end in 0..snapshot.len() {
            assert!(compare_server_snapshot_bounded(
                &device,
                &group,
                &snapshot[..end],
                1024 * 1024,
                backing(&device)
            )
            .is_err());
        }
        let mut trailing = snapshot.to_vec();
        trailing.push(0);
        assert!(compare_server_snapshot_bounded(
            &device,
            &group,
            &trailing,
            1024 * 1024,
            backing(&device)
        )
        .is_err());
        assert!(compare_server_snapshot_bounded(
            &device,
            &group,
            &snapshot,
            usize::MAX,
            backing(&device)
        )
        .is_err());
    }
}
