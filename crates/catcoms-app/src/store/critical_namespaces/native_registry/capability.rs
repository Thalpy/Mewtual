//! Immutable independent native-registry descriptor. Never reinterpret ID5v1.
use super::super::{archive_current_namespace_codec as current, refused};
use crate::AppError;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

pub(super) const NAMESPACE_ID: u32 = 7;
pub(super) const NAMESPACE_VERSION: u32 = 1;
pub(super) const INDEX_FORMAT: u16 = 1;
const TAG: &[u8] = b"catcoms/private-native-backend-layout/v1\0";
const CURRENT_DOMAIN: &[u8] = b"catcoms/native-backend-current-descriptor/v1\0";
const CAPABILITY_DOMAIN: &[u8] = b"catcoms/native-backend-layout-id/v1\0";
pub(super) const DESCRIPTOR_BYTES: usize = 4 + TAG.len() + 2 + 3 * 36 + 2;

/// Read-only labels. Equality verifies context DATA, not mounted capability.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store::critical_namespaces) struct NativeCapabilityData {
    pub(super) vault: [u8; 32],
    pub(super) root: [u8; 32],
    current: [u8; 32],
}

impl std::fmt::Debug for NativeCapabilityData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeCapabilityData { .. }")
    }
}

pub(super) fn field32(d: &mut Decoder<'_>) -> Result<[u8; 32], AppError> {
    d.get_bytes()
        .map_err(|_| refused())?
        .try_into()
        .map_err(|_| refused())
}
impl NativeCapabilityData {
    pub(in crate::store::critical_namespaces) fn for_current_data(
        layout: current::CurrentLayoutData,
        descriptor: &[u8],
    ) -> Result<Self, AppError> {
        current::CurrentLayoutData::decode_data(descriptor, layout)?;
        let mut hash = blake3::Hasher::new();
        hash.update(CURRENT_DOMAIN);
        hash.update(&(descriptor.len() as u64).to_be_bytes());
        hash.update(descriptor);
        Ok(Self {
            vault: layout.vault,
            root: layout.root,
            current: *hash.finalize().as_bytes(),
        })
    }
    pub(in crate::store::critical_namespaces) fn encode_data(
        self,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let mut output = Encoder::with_capacity(DESCRIPTOR_BYTES);
        output.put_bytes(TAG).map_err(|_| refused())?;
        output.put_u16(1);
        for bytes in [self.vault, self.root, self.current] {
            output.put_bytes(&bytes).map_err(|_| refused())?;
        }
        output.put_u16(INDEX_FORMAT);
        if output.len() != DESCRIPTOR_BYTES {
            return Err(refused());
        }
        Ok(Zeroizing::new(output.finish()))
    }
    pub(in crate::store::critical_namespaces) fn decode_exact(
        raw: &[u8],
        expected: Self,
    ) -> Result<Self, AppError> {
        if raw.len() != DESCRIPTOR_BYTES {
            return Err(refused());
        }
        let mut input = Decoder::new(raw);
        if input.get_bytes().map_err(|_| refused())? != TAG
            || input.get_u16().map_err(|_| refused())? != 1
        {
            return Err(refused());
        }
        let actual = Self {
            vault: field32(&mut input)?,
            root: field32(&mut input)?,
            current: field32(&mut input)?,
        };
        if input.get_u16().map_err(|_| refused())? != INDEX_FORMAT {
            return Err(refused());
        }
        input.finish().map_err(|_| refused())?;
        if actual != expected {
            return Err(refused());
        }
        Ok(actual)
    }
    /// Domain-separated DATA context for the native index and declaration seed.
    /// Hashing runs outside writers, under the caller's prepaid buffer/work fee.
    pub(super) fn id_data(self) -> Result<[u8; 32], AppError> {
        let descriptor = self.encode_data()?;
        let mut hash = blake3::Hasher::new();
        hash.update(CAPABILITY_DOMAIN);
        hash.update(&(descriptor.len() as u64).to_be_bytes());
        hash.update(&descriptor);
        Ok(*hash.finalize().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected() -> NativeCapabilityData {
        let current = current::CurrentLayoutData {
            vault: [1; 32],
            root: [2; 32],
        };
        NativeCapabilityData::for_current_data(current, &current.encode_data().unwrap()).unwrap()
    }

    #[test]
    fn native_capability_has_exact_framing_and_context() {
        let value = expected();
        let raw = value.encode_data().unwrap();
        assert_eq!(raw.len(), DESCRIPTOR_BYTES);
        assert_eq!(
            NativeCapabilityData::decode_exact(&raw, value).unwrap(),
            value
        );
        for end in 0..raw.len() {
            assert!(NativeCapabilityData::decode_exact(&raw[..end], value).is_err());
        }
        let mut extra = raw.to_vec();
        extra.push(0);
        assert!(NativeCapabilityData::decode_exact(&extra, value).is_err());
        for offset in [4, 4 + TAG.len() + 1, 4 + TAG.len() + 2 + 4, raw.len() - 1] {
            let mut changed = raw.to_vec();
            changed[offset] ^= 1;
            assert!(NativeCapabilityData::decode_exact(&changed, value).is_err());
        }
        let foreign = NativeCapabilityData {
            root: [99; 32],
            ..value
        };
        assert!(NativeCapabilityData::decode_exact(&raw, foreign).is_err());
        assert_ne!(foreign.id_data().unwrap(), value.id_data().unwrap());
    }

    #[test]
    fn native_capability_does_not_accept_an_arbitrary_current_descriptor() {
        let layout = current::CurrentLayoutData {
            vault: [1; 32],
            root: [2; 32],
        };
        let mut raw = layout.encode_data().unwrap();
        raw.push(0);
        assert!(NativeCapabilityData::for_current_data(layout, &raw).is_err());
        let foreign = current::CurrentLayoutData {
            root: [3; 32],
            ..layout
        };
        assert!(
            NativeCapabilityData::for_current_data(layout, &foreign.encode_data().unwrap())
                .is_err()
        );
    }
}
