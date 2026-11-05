//! Borrowed, two-pass provider-map restore. This closes provider table/payload preflight only;
//! OpenMLS's typed JSON/group loads still need their own separately paid bounded consumer.
use super::*;

#[cfg(test)]
std::thread_local! {
    static RESTORE_MAP_ALLOCATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod malformed_tests;

/// Restore the same legacy MLS frame while bounding the provider map BEFORE allocating it.
///
/// This is not a covering-save, Current or fresh-admission witness. The caller must own the
/// original captured source and reserve encoded input, table backing, copied Vec payload and
/// the SDK's separate typed decode/group heap before calling. This method does not by itself
/// bound SDK heap expansion or make generic Server::restore a bounded production reader.
/// Unlike the legacy decoder, this path borrows outer frames and refuses duplicate provider keys.
pub fn restore_server_provider_bounded(
    snapshot: &[u8],
    max_bytes: usize,
    max_backing_bytes: usize,
) -> Result<(MlsDevice, ServerGroup), MlsError> {
    let bad = || MlsError::Internal("MLS restore exceeds explicit provider budget");
    if snapshot.len() > max_bytes {
        return Err(bad());
    }
    let payload_budget = max_bytes.checked_mul(2).ok_or_else(bad)?;
    let mut outer = Decoder::new(snapshot);
    let storage = outer.get_bytes().map_err(|_| bad())?;
    let signer = outer.get_bytes().map_err(|_| bad())?;
    let group = outer.get_bytes().map_err(|_| bad())?;
    outer.finish().map_err(|_| bad())?;
    if signer.len() != 32 || group.is_empty() || group.len() > 256 {
        return Err(bad());
    }
    let map = deserialize_map_bounded(storage, max_backing_bytes, payload_budget)?;
    let provider = OpenMlsRustCrypto::default();
    *provider
        .storage()
        .values
        .write()
        .map_err(|_| MlsError::Internal("MLS storage lock poisoned"))? = map;
    restore_device_and_group(provider, signer, group)
}

/// A deliberately conservative bound for the EXACT pinned hashbrown0.16.1 allocation algorithm.
/// Its load factor needs <=next_power_of_two(2*count) buckets, minimum4 for this Vec-pair type.
/// Each bucket owns one inline Vec pair. Add one control byte/bucket, up to64-byte control-group
/// tail and alignment slack. The audited Group implementations use at most16-byte groups;64
/// intentionally overpays all supported targets. Re-review this proof on any hashbrown upgrade.
/// No probe map/allocation is created to discover a fee after the fact.
fn restore_backing_upper_bound(count: usize) -> Option<usize> {
    if count == 0 {
        return Some(0);
    }
    let buckets = count.checked_mul(2)?.max(4).checked_next_power_of_two()?;
    let alignment = std::mem::align_of::<(Vec<u8>, Vec<u8>)>().max(64);
    buckets
        .checked_mul(std::mem::size_of::<(Vec<u8>, Vec<u8>)>())?
        .checked_add(alignment - 1)?
        .checked_add(buckets)?
        .checked_add(64)
        .filter(|bytes| *bytes <= isize::MAX as usize)
}

pub(super) fn deserialize_map_bounded(
    bytes: &[u8],
    max_backing_bytes: usize,
    max_payload_bytes: usize,
) -> Result<StorageMap, MlsError> {
    let bad = || MlsError::Internal("MLS restore exceeds explicit provider budget");
    let mut decoder = Decoder::new(bytes);
    let count = usize::try_from(decoder.get_u32().map_err(|_| bad())?).map_err(|_| bad())?;
    // Exact encoded minimum and independently computed backing fee precede entry traversal AND
    // the first map/Vec allocation. Malformed huge declared counts therefore do not reserve first.
    if bytes.len() < 4 || count > (bytes.len() - 4) / 8 {
        return Err(bad());
    }
    let backing = restore_backing_upper_bound(count)
        .filter(|fee| *fee <= max_backing_bytes)
        .ok_or_else(bad)?;
    let mut payload = 0usize;
    for _ in 0..count {
        let key = decoder.get_bytes().map_err(|_| bad())?;
        let value = decoder.get_bytes().map_err(|_| bad())?;
        payload = payload
            .checked_add(key.len())
            .and_then(|bytes| bytes.checked_add(value.len()))
            .filter(|bytes| *bytes <= max_payload_bytes)
            .ok_or_else(bad)?;
    }
    decoder.finish().map_err(|_| bad())?;

    // Only this point allocates. Preallocating the admitted row count avoids growth having two
    // table buffers alive at once; each to_vec below requests exactly the already-paid length.
    #[cfg(test)]
    if count > 0 {
        RESTORE_MAP_ALLOCATIONS.with(|allocations| allocations.set(allocations.get() + 1));
    }
    let mut map: StorageMap = StorageMap::with_capacity_and_hasher(count, Default::default());
    debug_assert!(
        map.allocation_size() <= backing,
        "pinned table-layout bound drifted"
    );
    let mut decoder = Decoder::new(bytes);
    decoder.get_u32().map_err(|_| bad())?;
    for _ in 0..count {
        let key = decoder.get_bytes().map_err(|_| bad())?;
        let value = decoder.get_bytes().map_err(|_| bad())?;
        if map.contains_key(key) {
            return Err(bad());
        }
        map.insert(key.to_vec(), value.to_vec());
    }
    decoder.finish().map_err(|_| bad())?;
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(rows: &[(&[u8], &[u8])]) -> Vec<u8> {
        let mut encoder = Encoder::new();
        encoder.put_u32(rows.len() as u32);
        for (key, value) in rows {
            encoder.put_bytes(key).unwrap();
            encoder.put_bytes(value).unwrap();
        }
        encoder.finish()
    }

    #[test]
    fn provider_restore_refuses_counts_backing_payload_and_truncation_before_first_allocation() {
        let bytes = encoded(&[(b"a", b"b")]);
        let backing = restore_backing_upper_bound(1).unwrap();
        let before = RESTORE_MAP_ALLOCATIONS.with(std::cell::Cell::get);
        assert!(deserialize_map_bounded(&bytes, backing - 1, 2).is_err());
        assert!(deserialize_map_bounded(&bytes, backing, 1).is_err());
        assert!(deserialize_map_bounded(&bytes[..bytes.len() - 1], backing, 2).is_err());
        assert!(deserialize_map_bounded(&u32::MAX.to_be_bytes(), usize::MAX, usize::MAX).is_err());
        assert_eq!(RESTORE_MAP_ALLOCATIONS.with(std::cell::Cell::get), before);
        let map = deserialize_map_bounded(&bytes, backing, 2).unwrap();
        assert_eq!(map.get(b"a".as_slice()).unwrap(), b"b");
        assert!(map.allocation_size() <= backing);
    }

    #[test]
    fn provider_restore_empty_and_duplicate_rows_never_invent_a_unique_source() {
        let empty = deserialize_map_bounded(&encoded(&[]), 0, 0).unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.allocation_size(), 0);
        let duplicates = encoded(&[(b"a", b"b"), (b"a", b"c")]);
        assert!(
            deserialize_map_bounded(&duplicates, restore_backing_upper_bound(2).unwrap(), 4)
                .is_err()
        );
        assert!(restore_backing_upper_bound(usize::MAX).is_none());
    }

    #[test]
    fn provider_restore_conservative_layout_covers_actual_pinned_map_allocations() {
        for count in 0..=4096 {
            let map: StorageMap = StorageMap::with_capacity_and_hasher(count, Default::default());
            assert!(
                map.allocation_size() <= restore_backing_upper_bound(count).unwrap(),
                "count {count}"
            );
        }
    }

    #[test]
    fn provider_restore_reads_actual_legacy_mls_and_preserves_signer_epoch_and_group() {
        let alice = MlsDevice::generate().unwrap();
        let mut group = ServerGroup::create(&alice).unwrap();
        let bob = MlsDevice::generate().unwrap();
        group
            .add_member(&alice, bob.key_package().unwrap())
            .unwrap();
        let bytes = snapshot_server(&alice, &group).unwrap();
        let (restored, actual) =
            restore_server_provider_bounded(&bytes, bytes.len(), 1024 * 1024).unwrap();
        assert_eq!(restored.device_id(), alice.device_id());
        assert_eq!(actual.group_id(), group.group_id());
        assert_eq!(
            actual.epoch_authenticator_id(),
            group.epoch_authenticator_id()
        );
        assert_eq!(actual.member_count(), group.member_count());
        assert!(restored.sign(b"bounded provider restore").is_ok());
    }

    #[test]
    fn provider_restore_preserves_frozen_openmls_0_8_1_identity_and_pending_shapes() {
        // These immutable fixtures were written by the old shipped SDK, not regenerated by
        // this new provider. Both parsers must retain the SAME signer and MLS authenticator.
        for bytes in [
            include_bytes!("fixtures/openmls-0.8.1/founder.snapshot").as_slice(),
            include_bytes!("fixtures/openmls-0.8.1/member.snapshot").as_slice(),
            include_bytes!("fixtures/openmls-0.8.1/staged_add.snapshot").as_slice(),
            include_bytes!("fixtures/openmls-0.8.1/staged_remove.snapshot").as_slice(),
        ] {
            let (old_device, old_group) = restore_server(bytes).unwrap();
            let (new_device, new_group) =
                restore_server_provider_bounded(bytes, bytes.len(), 1024 * 1024).unwrap();
            assert_eq!(new_device.device_id(), old_device.device_id());
            assert_eq!(new_group.group_id(), old_group.group_id());
            assert_eq!(
                new_group.epoch_authenticator_id(),
                old_group.epoch_authenticator_id()
            );
            assert_eq!(new_group.member_count(), old_group.member_count());
        }
    }
}
