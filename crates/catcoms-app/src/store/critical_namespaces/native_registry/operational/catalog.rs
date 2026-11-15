//! Closed whole-catalog 7/v1 -> 7/v2 successor. No general replace-entry writer.
use super::super::super::{catalog_envelope as outer, pending_join_codec as grammar, refused};
use super::super::capability::NativeCapabilityData;
use super::capability::Capability;
use crate::AppError;
use catcoms_wire::Encoder;
use zeroize::Zeroizing;

/// The actual upgrade owner funds both complete catalog frames and seal overlap
/// before calling this pure builder. Every unrelated namespace stays byte exact.
pub(super) fn upgrade_data(
    original: &[u8],
    domain: &[u8; 32],
    expected: NativeCapabilityData,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let known = grammar::parse_native_registry_layout(original, domain, true, false)?;
    if known.count != 0 {
        return Err(refused());
    }
    let actual = NativeCapabilityData::for_current_data(
        known.current_layout.ok_or_else(refused)?,
        known.current_payload.ok_or_else(refused)?,
    )?;
    if actual != expected {
        return Err(refused());
    }
    NativeCapabilityData::decode_exact(
        known.native_registry_payload.ok_or_else(refused)?,
        expected,
    )?;
    let old = outer::parse(original, domain, outer::MAX_ENVELOPE_PLAIN)?;
    if old.format != outer::Format::Namespaces {
        return Err(refused());
    }
    let old_descriptor = expected.encode_data()?;
    let descriptor = Capability { original: expected }.encode_data()?;
    let generation = old.generation.checked_add(1).ok_or_else(refused)?;
    let length = original
        .len()
        .checked_sub(old_descriptor.len())
        .and_then(|n| n.checked_add(descriptor.len()))
        .filter(|n| *n <= outer::MAX_ENVELOPE_PLAIN)
        .ok_or_else(refused)?;
    let mut output = Encoder::with_capacity(length);
    output.put_str(outer::TAG_V2).map_err(|_| refused())?;
    output.put_bytes(domain).map_err(|_| refused())?;
    output.put_u64(generation);
    output.put_bytes(&old.digest).map_err(|_| refused())?;
    output.put_u32(u32::try_from(old.count).map_err(|_| refused())?);
    let mut replaced = false;
    for index in 0..old.count {
        let entry = old.entry(index)?;
        if entry.id == 7 {
            if replaced || entry.version != 1 || entry.payload != old_descriptor.as_slice() {
                return Err(refused());
            }
            output
                .put_u32(7)
                .put_u32(super::capability::NAMESPACE_VERSION);
            output.put_bytes(&descriptor).map_err(|_| refused())?;
            replaced = true;
        } else {
            output.put_u32(entry.id).put_u32(entry.version);
            output.put_bytes(entry.payload).map_err(|_| refused())?;
        }
    }
    if !replaced || output.len() != length {
        return Err(refused());
    }
    Ok(Zeroizing::new(output.finish()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::critical_namespaces::{
        archive_bootstrap, archive_current_namespace_codec as current, catalog_declaration,
        member_outbox_namespace_codec as member,
    };

    fn original() -> (Zeroizing<Vec<u8>>, NativeCapabilityData) {
        let layout = current::CurrentLayoutData {
            vault: [1; 32],
            root: [2; 32],
        };
        let mut output = Encoder::new();
        output.put_str(outer::TAG_V2).unwrap();
        output.put_bytes(&layout.vault).unwrap();
        output.put_u64(2);
        output.put_bytes(&[3; 32]).unwrap();
        // An empty JOIN has no namespace entry. A declared JOIN v1 must retain
        // at least one row, even if paused; a zero-count payload is invalid.
        output.put_u32(2);
        for (id, payload) in [
            (
                2,
                archive_bootstrap::Descriptor {
                    vault: layout.vault,
                    root: layout.root,
                }
                .encode()
                .unwrap()
                .to_vec(),
            ),
            (3, layout.encode_data().unwrap().to_vec()),
        ] {
            output.put_u32(id).put_u32(1);
            output.put_bytes(&payload).unwrap();
        }
        let base = member::prepare_declaration_data(&output.finish(), &layout.vault).unwrap();
        let cap =
            NativeCapabilityData::for_current_data(layout, &layout.encode_data().unwrap()).unwrap();
        (
            catalog_declaration::insert_entry_data(
                &base,
                &layout.vault,
                7,
                1,
                &cap.encode_data().unwrap(),
            )
            .unwrap(),
            cap,
        )
    }

    #[test]
    fn native_index2_catalog_upgrade_changes_only_the_exact_seven_entry() {
        let (old, cap) = original();
        let next = upgrade_data(&old, &cap.vault, cap).unwrap();
        let before = outer::parse(&old, &cap.vault, outer::MAX_ENVELOPE_PLAIN).unwrap();
        let after = outer::parse(&next, &cap.vault, outer::MAX_ENVELOPE_PLAIN).unwrap();
        assert_eq!(before.count, after.count);
        assert_eq!(after.generation, before.generation + 1);
        assert_eq!(after.parent, Some(before.digest));
        for index in 0..before.count {
            let a = before.entry(index).unwrap();
            let b = after.entry(index).unwrap();
            assert_eq!(a.id, b.id);
            if a.id == 7 {
                assert_eq!((a.version, b.version), (1, 2));
                assert!(Capability::decode_exact(b.payload, Capability { original: cap }).is_ok());
            } else {
                assert_eq!(a.version, b.version);
                assert_eq!(a.payload, b.payload);
            }
        }
        assert!(grammar::parse_native_registry_layout(&next, &cap.vault, true, false).is_err());
        assert!(grammar::parse_operational_native_registry_layout(&next, &cap.vault, true).is_ok());
        assert!(grammar::parse_operational_native_registry_layout(&old, &cap.vault, true).is_err());
        assert!(upgrade_data(&next, &cap.vault, cap).is_err());
        assert!(upgrade_data(&old, &[99; 32], cap).is_err());
    }

    #[test]
    fn native_index2_catalog_upgrade_refuses_foreign_current_binding_and_generation_exhaustion() {
        let (old, cap) = original();
        let foreign = current::CurrentLayoutData {
            vault: cap.vault,
            root: [99; 32],
        };
        let foreign =
            NativeCapabilityData::for_current_data(foreign, &foreign.encode_data().unwrap())
                .unwrap();
        assert!(upgrade_data(&old, &cap.vault, foreign).is_err());
        let mut exhausted = old.to_vec();
        let offset = 4 + outer::TAG_V2.len() + 36;
        exhausted[offset..offset + 8].copy_from_slice(&u64::MAX.to_be_bytes());
        assert!(upgrade_data(&exhausted, &cap.vault, cap).is_err());
    }
}
