//! Snapshot + restore a server's MLS state for disk persistence (Phase 9c).
//!
//! openmls (0.8 and 0.9 alike) has **no group-snapshot API**: the group + the signature keypair
//! live in the provider's `MemoryStorage`. So [`snapshot_server`] serializes that whole storage
//! (plus the signer public key + the group id) into one blob, and [`restore_server`] populates a
//! fresh provider's storage with it and reloads the device + group via `MlsGroup::load`.
//!
//! That map's JSON is therefore the persisted format: an openmls upgrade must keep reading maps
//! written by the previous version (`compat_tests` pins openmls 0.8.1).
//!
//! The snapshot blob is **secret**; it contains the signer private key and the MLS group
//! secrets. This module does the *serialization* only; the persistence layer **seals the
//! blob under `mls_seal_key`** (the vault, Phase 9a/9b) before it ever touches disk, exactly
//! as the [`crate::SealingBlobStore`]-style sealing-at-the-storage-boundary does for blobs.

use openmls_memory_storage::StorageMap;
use openmls_rust_crypto::OpenMlsRustCrypto;
use openmls_traits::OpenMlsProvider;
use zeroize::Zeroizing;

use catcoms_wire::{Decoder, Encoder};

use crate::{MlsDevice, MlsError, ServerGroup};

mod provider_restore;
pub use provider_restore::restore_server_provider_bounded;
mod join_device;
pub use join_device::{restore_join_device_provider_bounded, snapshot_join_device_bounded};
mod snapshot_compare;
pub use snapshot_compare::compare_server_snapshot_bounded;

// Test-only work boundary, local to each synchronous test thread. Mark legacy entry too so a
// future "serialize everything, then check length" implementation cannot pass refusal tests.
#[cfg(test)]
std::thread_local! {
    static SNAPSHOT_ENCODING_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static SNAPSHOT_SCANNED_ROWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
fn note_snapshot_encoding() {
    SNAPSHOT_ENCODING_ENTRIES.with(|entries| entries.set(entries.get() + 1));
}

/// Serialize a server's full MLS state; provider storage + signer public key + group id;
/// into a single blob. **Secret**: it contains private keys, so seal it before persisting.
pub fn snapshot_server(
    device: &MlsDevice,
    group: &ServerGroup,
) -> Result<Zeroizing<Vec<u8>>, MlsError> {
    #[cfg(test)]
    note_snapshot_encoding();
    // openmls' built-in storage `serialize` is test-only, but `values` is public; so we
    // serialize the key/value map ourselves with the canonical wire codec.
    let storage = {
        let map = device
            .provider()
            .storage()
            .values
            .read()
            .map_err(|_| MlsError::Internal("MLS storage lock poisoned"))?;
        serialize_map(&map)?
    };
    let pk = device.public_key_bytes();
    let gid = group.group_id();

    let mut e = Encoder::new();
    let too_large = || MlsError::Internal("MLS snapshot too large to encode");
    e.put_bytes(&storage).map_err(|_| too_large())?;
    e.put_bytes(&pk).map_err(|_| too_large())?;
    e.put_bytes(&gid).map_err(|_| too_large())?;
    Ok(Zeroizing::new(e.finish()))
}

/// Serialize the SAME legacy snapshot bytes under separate output and table-backing budgets.
///
/// This is the owner-candidate persistence path, not a new format or a membership capability.
/// Check count, every field length and the complete encoded size BEFORE allocating output, while
/// holding the SAME provider read guard through encoding. No preflight/unlock/second-clone race
/// exists. The locally patched, exactly pinned provider exposes its ACTUAL table allocation
/// (buckets, Vec headers and control storage). Check that O(1) value before the first iteration:
/// reported capacity can fall after tombstone erasure without releasing any backing allocation.
/// The output cap independently bounds every encoded key/value and the actual output allocation.
/// Retained Vec payload CAPACITIES must additionally fit twice that output cap, a checked local
/// resource policy (not an assumption about SDK allocation). Oversized spare capacity refuses;
/// this read never shrinks it. The caller pays that full payload allowance separately from backing.
/// Neither cap grants membership or covering-save authority, and these are not whole-process RSS
/// limits. The caller must reserve backing, retained provider payload and simultaneous enclosing
/// candidate/sealing allocations before taking ownership. Refusal never compacts/mutates source.
pub fn snapshot_server_bounded(
    device: &MlsDevice,
    group: &ServerGroup,
    max_bytes: usize,
    max_backing_bytes: usize,
) -> Result<Zeroizing<Vec<u8>>, MlsError> {
    let too_large = || MlsError::Internal("MLS snapshot exceeds explicit byte budget");
    let public_key = device.public_key_ref();
    let group_id = group.group_id_ref();
    if public_key.len() > u32::MAX as usize || group_id.len() > u32::MAX as usize {
        return Err(too_large());
    }
    let outer = 12usize
        .checked_add(public_key.len())
        .and_then(|size| size.checked_add(group_id.len()))
        .ok_or_else(too_large)?;
    let storage_budget = max_bytes.checked_sub(outer).ok_or_else(too_large)?;
    let payload_budget = max_bytes.checked_mul(2).ok_or_else(too_large)?;
    let map = device
        .provider()
        .storage()
        .values
        .read()
        .map_err(|_| MlsError::Internal("MLS storage lock poisoned"))?;
    let (count, storage_size) =
        preflight_map(&map, storage_budget, max_backing_bytes, payload_budget)?;
    let total = outer.checked_add(storage_size).ok_or_else(too_large)?;
    if total > max_bytes {
        return Err(too_large());
    }
    // All checks precede the ONLY output allocation. Keep this read guard until the buffer is
    // complete: the provider's public interior storage can otherwise change through another handle.
    #[cfg(test)]
    note_snapshot_encoding();
    let mut output = Zeroizing::new(Vec::with_capacity(total));
    output.extend_from_slice(&(storage_size as u32).to_be_bytes());
    output.extend_from_slice(&count.to_be_bytes());
    for (key, value) in map.iter() {
        append_preflighted_field(&mut output, key);
        append_preflighted_field(&mut output, value);
    }
    append_preflighted_field(&mut output, public_key);
    append_preflighted_field(&mut output, group_id);
    debug_assert_eq!(output.len(), total);
    Ok(output)
}

/// No allocation, compaction or cloning occurs here. S is generic only so a deterministic
/// tombstone fixture can exercise the EXACT production preflight with a deliberately colliding
/// hasher; MemoryStorage itself always uses RandomState. The caller must retain the original
/// provider guard through encoding. Physical table backing and Vec payloads are separate fees.
fn preflight_map<S: std::hash::BuildHasher>(
    map: &StorageMap<S>,
    storage_budget: usize,
    max_backing_bytes: usize,
    max_payload_bytes: usize,
) -> Result<(u32, usize), MlsError> {
    let too_large = || MlsError::Internal("MLS snapshot exceeds explicit byte budget");
    let count = u32::try_from(map.len()).map_err(|_| too_large())?;
    // Refuse BEFORE the first iterator or entry visit, even if a tombstone-heavy table reports
    // almost no spare capacity. allocation_size belongs to the actual table under this guard.
    if storage_budget < 4
        || map.len() > (storage_budget - 4) / 8
        || map.allocation_size() > max_backing_bytes
    {
        return Err(too_large());
    }
    let mut storage_size = 4usize;
    let mut retained_payload = 0usize;
    for (key, value) in map.iter() {
        #[cfg(test)]
        SNAPSHOT_SCANNED_ROWS.with(|rows| rows.set(rows.get() + 1));
        retained_payload = retained_payload
            .checked_add(key.capacity())
            .and_then(|size| size.checked_add(value.capacity()))
            .filter(|size| *size <= max_payload_bytes)
            .ok_or_else(too_large)?;
        if key.len() > u32::MAX as usize || value.len() > u32::MAX as usize {
            return Err(too_large());
        }
        storage_size = storage_size
            .checked_add(8)
            .and_then(|size| size.checked_add(key.len()))
            .and_then(|size| size.checked_add(value.len()))
            .filter(|size| *size <= storage_budget && *size <= u32::MAX as usize)
            .ok_or_else(too_large)?;
    }
    Ok((count, storage_size))
}

// The caller owns both the exact length preflight and any interior-data read guard. This fixed
// framing is byte-identical to Encoder::put_bytes; the zeroizing buffer also covers unwind/error.
fn append_preflighted_field(output: &mut Vec<u8>, field: &[u8]) {
    let length =
        u32::try_from(field.len()).expect("snapshot field was preflighted under its read guard");
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(field);
}

/// Reconstruct a `(device, group)` from a [`snapshot_server`] blob.
pub fn restore_server(snapshot: &[u8]) -> Result<(MlsDevice, ServerGroup), MlsError> {
    let bad = || MlsError::Internal("bad MLS snapshot");
    let mut d = Decoder::new(snapshot);
    let storage_bytes = d.get_bytes().map_err(|_| bad())?.to_vec();
    let public_key = d.get_bytes().map_err(|_| bad())?.to_vec();
    let group_id = d.get_bytes().map_err(|_| bad())?.to_vec();
    d.finish()
        .map_err(|_| MlsError::Internal("trailing MLS snapshot bytes"))?;

    // Populate a fresh provider's storage with the restored key/value map.
    let provider = OpenMlsRustCrypto::default();
    let map = deserialize_map(&storage_bytes)?;
    {
        let mut values = provider
            .storage()
            .values
            .write()
            .map_err(|_| MlsError::Internal("MLS storage lock poisoned"))?;
        *values = map;
    }

    restore_device_and_group(provider, &public_key, &group_id)
}

// Both parsers reach the SAME strict SDK/signing-leaf check. The bounded provider parser does
// not make an inactive/removed group runnable or weaken the legacy stored-state contract.
fn restore_device_and_group(
    provider: OpenMlsRustCrypto,
    public_key: &[u8],
    group_id: &[u8],
) -> Result<(MlsDevice, ServerGroup), MlsError> {
    let device = MlsDevice::restore(provider, public_key)?;
    let group = ServerGroup::load(&device, group_id)?;
    // Cross-check that the restored device actually belongs to the restored group, so a
    // mismatched snapshot fails loudly here rather than deep inside a later MLS operation.
    if !group.contains_device(&device.device_id()) {
        return Err(MlsError::Internal(
            "restored device is not a member of the restored group",
        ));
    }
    Ok((device, group))
}

/// Encode the storage `key -> value` map: `count(u32) ‖ (len-prefixed key ‖ value)*`.
fn serialize_map(map: &StorageMap) -> Result<Vec<u8>, MlsError> {
    let mut e = Encoder::new();
    let count = u32::try_from(map.len())
        .map_err(|_| MlsError::Internal("MLS storage has too many entries"))?;
    e.put_u32(count);
    let too_large = || MlsError::Internal("MLS storage entry too large");
    for (k, v) in map.iter() {
        e.put_bytes(k).map_err(|_| too_large())?;
        e.put_bytes(v).map_err(|_| too_large())?;
    }
    Ok(e.finish())
}

/// Decode a storage map produced by [`serialize_map`].
fn deserialize_map(bytes: &[u8]) -> Result<StorageMap, MlsError> {
    let bad = || MlsError::Internal("bad MLS storage snapshot");
    let mut d = Decoder::new(bytes);
    let count = d.get_u32().map_err(|_| bad())?;
    let mut map = StorageMap::default();
    for _ in 0..count {
        let k = d.get_bytes().map_err(|_| bad())?.to_vec();
        let v = d.get_bytes().map_err(|_| bad())?.to_vec();
        map.insert(k, v);
    }
    d.finish().map_err(|_| bad())?;
    Ok(map)
}

// Frozen openmls 0.8.1 fixtures: stored state from the alpha builds must keep restoring after any
// openmls upgrade. See the module docs before touching the fixture files.
#[cfg(test)]
mod compat_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_wire::DocType;

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
    fn snapshot_round_trips_a_group_and_preserves_key_material() {
        let alice = MlsDevice::generate().unwrap();
        let mut group = ServerGroup::create(&alice).unwrap();
        // Add a member so the group has non-trivial ratchet/epoch state.
        let bob = MlsDevice::generate().unwrap();
        let _ = group
            .add_member(&alice, bob.key_package().unwrap())
            .unwrap();

        let epoch = group.epoch();
        let gid = group.group_id();
        let members = group.member_count();
        let key_before = group.channel_secret(&alice, DocType::Channel, 1).unwrap();

        // Snapshot Alice's view, then restore from the serialized bytes.
        let snap = snapshot_server(&alice, &group).unwrap();
        let (alice2, group2) = restore_server(&snap).unwrap();

        // Identity + group state survive…
        assert_eq!(alice2.device_id(), alice.device_id());
        assert_eq!(group2.epoch(), epoch);
        assert_eq!(group2.group_id(), gid);
        assert_eq!(group2.member_count(), members);
        // …and the restored group derives the SAME channel key (the secret state persisted)…
        assert_eq!(
            group2.channel_secret(&alice2, DocType::Channel, 1).unwrap(),
            key_before,
            "the restored group must derive identical channel keys"
        );
        // …and the restored signer still signs.
        assert!(alice2.sign(b"after restore").is_ok());
    }

    #[test]
    fn snapshot_round_trips_a_pending_staged_commit() {
        // The whole approach rests on openmls persisting `group_state = PendingCommit` to
        // storage. Snapshot a group with a staged (un-merged) commit, restore it, and merge
        //; locking in that the pending state survives a snapshot.
        let alice = MlsDevice::generate().unwrap();
        let mut group = ServerGroup::create(&alice).unwrap();
        let bob = MlsDevice::generate().unwrap();
        let _ = group.stage_add(&alice, bob.key_package().unwrap()).unwrap();
        let staged_epoch = group.epoch();

        let snap = snapshot_server(&alice, &group).unwrap();
        let (alice2, mut group2) = restore_server(&snap).unwrap();
        assert_eq!(group2.epoch(), staged_epoch);

        // The restored group adopts its staged commit (the pending state persisted).
        group2.merge_staged_self(&alice2).unwrap();
        assert_eq!(group2.epoch(), staged_epoch + 1);
        assert_eq!(group2.member_count(), 2);
    }

    #[test]
    fn bounded_snapshot_matches_legacy_bytes_and_exact_budget_before_allocation() {
        let alice = MlsDevice::generate().unwrap();
        let mut group = ServerGroup::create(&alice).unwrap();
        let bob = MlsDevice::generate().unwrap();
        group
            .add_member(&alice, bob.key_package().unwrap())
            .unwrap();
        let legacy = snapshot_server(&alice, &group).unwrap();
        for budget in [legacy.len(), legacy.len() + 1] {
            let before = SNAPSHOT_ENCODING_ENTRIES.with(std::cell::Cell::get);
            let bounded = snapshot_server_bounded(&alice, &group, budget, backing(&alice)).unwrap();
            assert_eq!(
                SNAPSHOT_ENCODING_ENTRIES.with(std::cell::Cell::get),
                before + 1
            );
            assert_eq!(bounded.as_slice(), legacy.as_slice());
            let (restored, actual) = restore_server(&bounded).unwrap();
            assert_eq!(restored.device_id(), alice.device_id());
            assert_eq!(actual.epoch(), group.epoch());
            assert_eq!(actual.group_id(), group.group_id());
        }
        for budget in [0, 1, legacy.len() - 1] {
            let before = SNAPSHOT_ENCODING_ENTRIES.with(std::cell::Cell::get);
            assert!(snapshot_server_bounded(&alice, &group, budget, backing(&alice)).is_err());
            assert_eq!(
                SNAPSHOT_ENCODING_ENTRIES.with(std::cell::Cell::get),
                before,
                "refuse before output allocation/legacy encoding work"
            );
        }
        assert_eq!(
            snapshot_server(&alice, &group).unwrap().as_slice(),
            legacy.as_slice()
        );
    }

    #[test]
    fn bounded_snapshot_accounts_for_all_provider_rows_without_mutating_source() {
        let alice = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&alice).unwrap();
        let original = snapshot_server(&alice, &group).unwrap();
        let label = b"unknown provider row is still retained private state".to_vec();
        let value = vec![0x5a; 1024 * 1024];
        alice
            .provider()
            .storage()
            .values
            .write()
            .unwrap()
            .insert(label.clone(), value.clone());
        let before = SNAPSHOT_ENCODING_ENTRIES.with(std::cell::Cell::get);
        assert!(snapshot_server_bounded(&alice, &group, original.len(), backing(&alice)).is_err());
        assert_eq!(
            SNAPSHOT_ENCODING_ENTRIES.with(std::cell::Cell::get),
            before,
            "huge field is refused before any complete snapshot encoding"
        );
        assert_eq!(
            alice
                .provider()
                .storage()
                .values
                .read()
                .unwrap()
                .get(&label),
            Some(&value)
        );
        let complete = snapshot_server(&alice, &group).unwrap();
        assert_eq!(
            snapshot_server_bounded(&alice, &group, complete.len(), backing(&alice))
                .unwrap()
                .as_slice(),
            complete.as_slice()
        );
        let (restored, actual) = restore_server(&complete).unwrap();
        assert_eq!(restored.device_id(), alice.device_id());
        assert_eq!(actual.group_id(), group.group_id());
    }

    #[test]
    fn bounded_snapshot_refuses_sparse_actual_backing_before_scanning_or_encoding() {
        let alice = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&alice).unwrap();
        let allocation = {
            let mut map = alice.provider().storage().values.write().unwrap();
            for index in 0..65_536u64 {
                let mut key = b"bounded-snapshot-capacity-fixture-".to_vec();
                key.extend_from_slice(&index.to_be_bytes());
                map.insert(key, Vec::new());
            }
            map.retain(|key, _| !key.starts_with(b"bounded-snapshot-capacity-fixture-"));
            map.allocation_size()
        };
        let before_bytes = snapshot_server(&alice, &group).unwrap();
        assert!(
            allocation > before_bytes.len(),
            "real sparse provider must exceed its independent backing fee"
        );
        let before = SNAPSHOT_ENCODING_ENTRIES.with(std::cell::Cell::get);
        let scanned = SNAPSHOT_SCANNED_ROWS.with(std::cell::Cell::get);
        assert!(
            snapshot_server_bounded(&alice, &group, before_bytes.len(), allocation - 1).is_err()
        );
        assert_eq!(SNAPSHOT_ENCODING_ENTRIES.with(std::cell::Cell::get), before);
        assert_eq!(SNAPSHOT_SCANNED_ROWS.with(std::cell::Cell::get), scanned);
        assert_eq!(backing(&alice), allocation);
        assert_eq!(
            snapshot_server(&alice, &group).unwrap().as_slice(),
            before_bytes.as_slice(),
            "refusal must not compact/mutate the actual provider"
        );
    }

    #[test]
    fn bounded_snapshot_backing_and_encoded_limits_are_independent_actual_provider_fees() {
        let alice = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&alice).unwrap();
        let legacy = snapshot_server(&alice, &group).unwrap();
        let allocation = backing(&alice);
        assert!(allocation > 0);
        let scanned = SNAPSHOT_SCANNED_ROWS.with(std::cell::Cell::get);
        let encoded = SNAPSHOT_ENCODING_ENTRIES.with(std::cell::Cell::get);
        assert!(snapshot_server_bounded(&alice, &group, legacy.len() * 2, allocation - 1).is_err());
        assert_eq!(SNAPSHOT_SCANNED_ROWS.with(std::cell::Cell::get), scanned);
        assert_eq!(
            SNAPSHOT_ENCODING_ENTRIES.with(std::cell::Cell::get),
            encoded
        );
        assert!(snapshot_server_bounded(&alice, &group, 1, allocation * 2).is_err());
        assert_eq!(SNAPSHOT_SCANNED_ROWS.with(std::cell::Cell::get), scanned);
        let exact = snapshot_server_bounded(&alice, &group, legacy.len(), allocation).unwrap();
        assert_eq!(exact.as_slice(), legacy.as_slice());
        assert_eq!(
            backing(&alice),
            allocation,
            "read must not compact the source"
        );
    }

    #[test]
    fn bounded_snapshot_counts_tombstoned_allocation_even_when_reported_capacity_decreases() {
        // This deterministic collision fixture calls the SAME private preflight as the actual
        // provider. It does not substitute a weak hasher into any real credential/provider.
        #[derive(Default)]
        struct Collision;
        impl std::hash::Hasher for Collision {
            fn finish(&self) -> u64 {
                0
            }
            fn write(&mut self, _bytes: &[u8]) {}
        }
        let mut map: StorageMap<std::hash::BuildHasherDefault<Collision>> = StorageMap::default();
        for n in 0..4096u64 {
            map.insert(n.to_be_bytes().to_vec(), Vec::new());
        }
        let allocated = map.allocation_size();
        let reported = map.capacity();
        for n in 0..4095u64 {
            assert!(map.remove(n.to_be_bytes().as_slice()).is_some());
        }
        assert_eq!(map.len(), 1);
        assert_eq!(map.allocation_size(), allocated);
        assert!(
            map.capacity() < reported,
            "fixture must exercise DELETED tombstones"
        );
        let scanned = SNAPSHOT_SCANNED_ROWS.with(std::cell::Cell::get);
        assert!(preflight_map(&map, usize::MAX, allocated - 1, 40).is_err());
        assert_eq!(SNAPSHOT_SCANNED_ROWS.with(std::cell::Cell::get), scanned);
        assert_eq!(preflight_map(&map, 20, allocated, 40).unwrap(), (1, 20));
        assert_eq!(map.allocation_size(), allocated);
    }

    #[test]
    fn bounded_snapshot_empty_map_has_zero_backing_fee_without_a_probe_allocation() {
        let map: StorageMap = StorageMap::default();
        assert_eq!(map.allocation_size(), 0);
        let scanned = SNAPSHOT_SCANNED_ROWS.with(std::cell::Cell::get);
        assert_eq!(preflight_map(&map, 4, 0, 0).unwrap(), (0, 4));
        assert_eq!(SNAPSHOT_SCANNED_ROWS.with(std::cell::Cell::get), scanned);
        assert_eq!(map.allocation_size(), 0);
    }

    #[test]
    fn bounded_snapshot_charges_actual_vec_spare_capacity_and_refuses_payload_invoice_overflow() {
        let alice = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&alice).unwrap();
        let key = b"retained-spare-capacity".to_vec();
        let mut value = Vec::with_capacity(1024 * 1024);
        value.push(1);
        let capacity = value.capacity();
        alice
            .provider()
            .storage()
            .values
            .write()
            .unwrap()
            .insert(key.clone(), value);
        let original = snapshot_server(&alice, &group).unwrap();
        let allocation = backing(&alice);
        let encoded = SNAPSHOT_ENCODING_ENTRIES.with(std::cell::Cell::get);
        assert!(
            original.len() * 2 < capacity,
            "fixture must have large spare capacity"
        );
        assert!(snapshot_server_bounded(&alice, &group, original.len(), allocation).is_err());
        assert_eq!(
            SNAPSHOT_ENCODING_ENTRIES.with(std::cell::Cell::get),
            encoded
        );
        assert_eq!(
            alice.provider().storage().values.read().unwrap()[&key].capacity(),
            capacity
        );
        assert_eq!(backing(&alice), allocation);
        let scanned = SNAPSHOT_SCANNED_ROWS.with(std::cell::Cell::get);
        assert!(snapshot_server_bounded(&alice, &group, usize::MAX, allocation).is_err());
        assert_eq!(SNAPSHOT_SCANNED_ROWS.with(std::cell::Cell::get), scanned);
        assert_eq!(
            SNAPSHOT_ENCODING_ENTRIES.with(std::cell::Cell::get),
            encoded
        );
        assert_eq!(
            snapshot_server(&alice, &group).unwrap().as_slice(),
            original.as_slice()
        );
    }

    #[test]
    fn a_corrupt_snapshot_is_rejected() {
        assert!(restore_server(b"not a valid snapshot").is_err());
    }
}
