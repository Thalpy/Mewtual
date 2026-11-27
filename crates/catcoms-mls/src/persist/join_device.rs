//! Secret bare-device DATA for an immutable pre-join master. This distinct framing restores no
//! group and supplies no Fresh/Current/Saved witness. Only a real authenticated selected-KP
//! Welcome can complete the separate prepared join. The provider budget is not SDK heap admission.
use super::*;
use crate::proto;
use tls_codec::Size;

const TAG: &str = "catcoms/private-prejoin-device/v1";

/// Snapshot the actual provider under its SAME read guard, checking output, table backing and
/// retained payload capacities before scanning/encoding. The caller must own the original fee
/// through this secret buffer and any candidate provider. Seal before writing to disk.
pub fn snapshot_join_device_bounded(
    device: &MlsDevice,
    max_bytes: usize,
    max_backing_bytes: usize,
) -> Result<Zeroizing<Vec<u8>>, MlsError> {
    let bad = || MlsError::Internal("prejoin provider exceeds explicit budget");
    let signer = device.public_key_ref();
    if signer.len() != 32 {
        return Err(bad());
    }
    let outer = 12usize
        .checked_add(TAG.len())
        .and_then(|fee| fee.checked_add(32))
        .ok_or_else(bad)?;
    let storage_budget = max_bytes.checked_sub(outer).ok_or_else(bad)?;
    let payload_budget = max_bytes.checked_mul(2).ok_or_else(bad)?;
    let map = device
        .provider()
        .storage()
        .values
        .read()
        .map_err(|_| MlsError::Internal("MLS storage lock poisoned"))?;
    let (count, storage_bytes) =
        preflight_map(&map, storage_budget, max_backing_bytes, payload_budget)?;
    let total = outer
        .checked_add(storage_bytes)
        .filter(|total| *total <= max_bytes)
        .ok_or_else(bad)?;
    let mut output = Zeroizing::new(Vec::with_capacity(total));
    append_preflighted_field(&mut output, TAG.as_bytes());
    output.extend_from_slice(&(storage_bytes as u32).to_be_bytes());
    output.extend_from_slice(&count.to_be_bytes());
    for (key, value) in map.iter() {
        append_preflighted_field(&mut output, key);
        append_preflighted_field(&mut output, value);
    }
    append_preflighted_field(&mut output, signer);
    debug_assert_eq!(output.len(), total);
    Ok(output)
}

/// Restore bare device DATA only. Complete borrowed framing and provider count/length/backing
/// admission precede table/payload allocation. Full-signer equality is checked after the SDK
/// signer load; an authenticated byte container alone is not a fresh admission or owner proof.
/// The SDK's typed JSON heap/work remains a separate activation gate.
pub fn restore_join_device_provider_bounded(
    source: &[u8],
    max_bytes: usize,
    max_backing_bytes: usize,
) -> Result<MlsDevice, MlsError> {
    let bad = || MlsError::Internal("unsupported prejoin provider or budget");
    if source.len() > max_bytes {
        return Err(bad());
    }
    let mut decoder = Decoder::new(source);
    if decoder.get_str().map_err(|_| bad())? != TAG {
        return Err(bad());
    }
    let storage = decoder.get_bytes().map_err(|_| bad())?;
    let signer = decoder.get_bytes().map_err(|_| bad())?;
    decoder.finish().map_err(|_| bad())?;
    if signer.len() != 32 {
        return Err(bad());
    }
    let payload = max_bytes.checked_mul(2).ok_or_else(bad)?;
    let map = provider_restore::deserialize_map_bounded(storage, max_backing_bytes, payload)?;
    let provider = OpenMlsRustCrypto::default();
    *provider
        .storage()
        .values
        .write()
        .map_err(|_| MlsError::Internal("MLS storage lock poisoned"))? = map;
    let device = MlsDevice::restore(provider, signer)?;
    if device.public_key_ref() != signer {
        return Err(bad());
    }
    Ok(device)
}

/// Restore only the expected signer into a clean provider for a same-vault
/// rejoin. The authenticated original pre-join provider may contain private
/// KeyPackages. None of those records are copied to the new device.
/// The caller must hold the original sealed source and the newly authorized
/// native key operation. Pay for both providers and the existing
/// FOUNDER_INPUT_PRIVATE_INVOICE, which includes a fixed challenge signature
/// and the canonical signer-only storage check.
pub fn restore_join_signer_only_bounded(
    source: &[u8],
    expected: &[u8; 32],
    (max_bytes, max_backing_bytes): (usize, usize),
) -> Result<MlsDevice, MlsError> {
    let original = restore_join_device_provider_bounded(source, max_bytes, max_backing_bytes)?;
    // The isolated SDK store must not materialize a large malformed key
    // record just to reject it when checking the canonical signer row.
    const MAX_SIGNER_TLS: usize = 128;
    if !original.retained_ed25519_key_matches(expected)
        || original.signer().tls_serialized_len() > MAX_SIGNER_TLS
    {
        return Err(MlsError::Internal(
            "rejoin signer differs from the original",
        ));
    }

    let isolated = OpenMlsRustCrypto::default();
    original.signer().store(isolated.storage()).map_err(proto)?;
    // Retire the entire original private provider before any new package
    // could use this isolated device. No provider or key bytes escape here.
    drop(original);
    let device = MlsDevice::restore(isolated, expected)?;
    // The existing one-signer check proves the canonical SDK row and the
    // real private/public pair without admitting any group operation.
    device.preflight_fresh_founder_input()?;
    drop(snapshot_join_device_bounded(
        &device,
        max_bytes,
        max_backing_bytes,
    )?);
    Ok(device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_prejoin_master_recreates_selected_private_keys_after_each_candidate_consumes_them() {
        let owner = MlsDevice::generate().unwrap();
        let mut group = ServerGroup::create(&owner).unwrap();
        let member = MlsDevice::generate().unwrap();
        let package = member
            .key_package_for_invite(&group.group_id(), [1; 16])
            .unwrap();
        let reference = member.key_package_reference(&package).unwrap();
        let master = snapshot_join_device_bounded(&member, 512 * 1024, 512 * 1024).unwrap();
        let original = master.clone();
        let outcome = group.add_member(&owner, package).unwrap();
        for _ in 0..2 {
            let candidate =
                restore_join_device_provider_bounded(&master, 512 * 1024, 512 * 1024).unwrap();
            let joined =
                ServerGroup::join_for_key_package(&candidate, &outcome.welcome, &reference)
                    .unwrap();
            assert!(joined.has_local_signer(&member.public_key_bytes()));
            assert_eq!(master.as_slice(), original.as_slice());
        }
    }

    #[test]
    fn private_rejoin_isolates_the_original_signer_from_old_packages() {
        let limits = (512 * 1024, 512 * 1024);
        let founder = MlsDevice::generate().unwrap();
        let mut group = ServerGroup::create(&founder).unwrap();
        let a1 = MlsDevice::generate().unwrap();
        let signer: [u8; 32] = a1.public_key_ref().try_into().unwrap();
        let first_nonce = [17; 16];
        let second_nonce = [18; 16];
        let old_package = a1
            .key_package_for_invite(&group.group_id(), first_nonce)
            .unwrap();
        let old_ref = a1.key_package_reference(&old_package).unwrap();
        let master = snapshot_join_device_bounded(&a1, limits.0, limits.1).unwrap();
        let old_bytes = master.clone();
        let old = group.add_member(&founder, old_package).unwrap();
        group.remove_member(&founder, &a1.device_id()).unwrap();
        assert!(restore_join_signer_only_bounded(&master, &[0; 32], limits).is_err());
        assert!(restore_join_signer_only_bounded(&master, &signer, (1, limits.1)).is_err());

        let a2 = restore_join_signer_only_bounded(&master, &signer, limits).unwrap();
        assert_eq!(a1.device_id(), a2.device_id());
        assert_eq!(a2.public_key_ref(), &signer);
        assert!(ServerGroup::join_for_key_package(&a2, &old.welcome, &old_ref).is_err());
        assert!(ServerGroup::join_for_key_package(&a1, &old.welcome, &old_ref).is_ok());
        let fresh = a2
            .key_package_for_invite(&group.group_id(), second_nonce)
            .unwrap();
        let fresh_ref = a2.key_package_reference(&fresh).unwrap();
        assert_ne!(old_ref, fresh_ref);
        let outcome = group.add_member(&founder, fresh).unwrap();
        let successor =
            ServerGroup::join_for_key_package(&a2, &outcome.welcome, &fresh_ref).unwrap();
        assert_eq!(
            successor.epoch_authenticator_id(),
            group.epoch_authenticator_id()
        );
        assert_eq!(master.as_slice(), old_bytes.as_slice());
    }

    #[test]
    fn private_prejoin_data_refuses_wrong_format_trailing_frames_and_explicit_provider_limits() {
        let device = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&device).unwrap();
        let legacy = snapshot_server(&device, &group).unwrap();
        assert!(restore_join_device_provider_bounded(&legacy, 512 * 1024, 512 * 1024).is_err());
        let source = snapshot_join_device_bounded(&device, 512 * 1024, 512 * 1024).unwrap();
        assert!(snapshot_join_device_bounded(&device, 1, 512 * 1024).is_err());
        assert!(snapshot_join_device_bounded(&device, 512 * 1024, 0).is_err());
        assert!(
            restore_join_device_provider_bounded(&source, source.len() - 1, 512 * 1024).is_err()
        );
        assert!(restore_join_device_provider_bounded(&source, source.len(), 0).is_err());
        let mut trailing = source.to_vec();
        trailing.push(0);
        assert!(restore_join_device_provider_bounded(&trailing, 512 * 1024, 512 * 1024).is_err());
    }
}
