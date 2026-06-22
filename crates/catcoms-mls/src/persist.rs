//! Snapshot + restore a server's MLS state for disk persistence (Phase 9c).
//!
//! openmls 0.8 has **no group-snapshot API**: the group + the signature keypair live in the
//! provider's `MemoryStorage`. So [`snapshot_server`] serializes that whole storage (plus
//! the signer public key + the group id) into one blob, and [`restore_server`] populates a
//! fresh provider's storage with it and reloads the device + group via `MlsGroup::load`.
//!
//! The snapshot blob is **secret** — it contains the signer private key and the MLS group
//! secrets. This module does the *serialization* only; the persistence layer **seals the
//! blob under `mls_seal_key`** (the vault, Phase 9a/9b) before it ever touches disk, exactly
//! as the [`crate::SealingBlobStore`]-style sealing-at-the-storage-boundary does for blobs.

use std::collections::HashMap;

use openmls_rust_crypto::OpenMlsRustCrypto;
use openmls_traits::OpenMlsProvider;
use zeroize::Zeroizing;

use catcoms_wire::{Decoder, Encoder};

use crate::{MlsDevice, MlsError, ServerGroup};

/// Serialize a server's full MLS state — provider storage + signer public key + group id —
/// into a single blob. **Secret**: it contains private keys, so seal it before persisting.
pub fn snapshot_server(
    device: &MlsDevice,
    group: &ServerGroup,
) -> Result<Zeroizing<Vec<u8>>, MlsError> {
    // openmls' built-in storage `serialize` is test-only, but `values` is public — so we
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

    let device = MlsDevice::restore(provider, &public_key)?;
    let group = ServerGroup::load(&device, &group_id)?;
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
fn serialize_map(map: &HashMap<Vec<u8>, Vec<u8>>) -> Result<Vec<u8>, MlsError> {
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
fn deserialize_map(bytes: &[u8]) -> Result<HashMap<Vec<u8>, Vec<u8>>, MlsError> {
    let bad = || MlsError::Internal("bad MLS storage snapshot");
    let mut d = Decoder::new(bytes);
    let count = d.get_u32().map_err(|_| bad())?;
    let mut map = HashMap::new();
    for _ in 0..count {
        let k = d.get_bytes().map_err(|_| bad())?.to_vec();
        let v = d.get_bytes().map_err(|_| bad())?.to_vec();
        map.insert(k, v);
    }
    d.finish().map_err(|_| bad())?;
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_wire::DocType;

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
        // — locking in that the pending state survives a snapshot.
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
    fn a_corrupt_snapshot_is_rejected() {
        assert!(restore_server(b"not a valid snapshot").is_err());
    }
}
