//! Version3 descriptor DATA. Its expected original is supplied by the owning
//! authenticated Current reader, never taken from this untrusted descriptor.
use super::super::capability::{field32, NativeCapabilityData};
use super::*;
use crate::store::critical_namespaces::archive_current_namespace_codec::CurrentLayoutData;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

pub(super) const NAMESPACE_VERSION: u32 = 3;
pub(super) const INDEX_FORMAT: u16 = 3;
const TAG: &[u8] = b"catcoms/private-native-backend-layout/v3\0";
const DOMAIN: &[u8] = b"catcoms/native-backend-layout-id/v3\0";
const DESCRIPTOR_BYTES: usize = 4 + TAG.len() + 2 + 3 * 36 + 2;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Capability {
    pub(super) original: NativeCapabilityData,
}
impl Capability {
    pub(super) fn for_current_data(
        layout: CurrentLayoutData,
        descriptor: &[u8],
    ) -> Result<Self, AppError> {
        Ok(Self {
            original: NativeCapabilityData::for_current_data(layout, descriptor)?,
        })
    }
    pub(super) fn encode_data(self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let mut output = Encoder::with_capacity(DESCRIPTOR_BYTES);
        output.put_bytes(TAG).map_err(|_| refused())?;
        output.put_u16(3);
        for field in [
            self.original.vault,
            self.original.root,
            self.original.id_data()?,
        ] {
            output.put_bytes(&field).map_err(|_| refused())?;
        }
        output.put_u16(INDEX_FORMAT);
        if output.len() != DESCRIPTOR_BYTES {
            return Err(refused());
        }
        Ok(Zeroizing::new(output.finish()))
    }
    pub(super) fn decode_exact(raw: &[u8], expected: Self) -> Result<Self, AppError> {
        if raw.len() != DESCRIPTOR_BYTES {
            return Err(refused());
        }
        let mut input = Decoder::new(raw);
        if input.get_bytes().map_err(|_| refused())? != TAG
            || input.get_u16().map_err(|_| refused())? != 3
            || field32(&mut input)? != expected.original.vault
            || field32(&mut input)? != expected.original.root
            || field32(&mut input)? != expected.original.id_data()?
            || input.get_u16().map_err(|_| refused())? != INDEX_FORMAT
        {
            return Err(refused());
        }
        input.finish().map_err(|_| refused())?;
        Ok(expected)
    }
    pub(super) fn id_data(self) -> Result<[u8; 32], AppError> {
        let bytes = self.encode_data()?;
        let mut hash = blake3::Hasher::new();
        hash.update(DOMAIN);
        hash.update(&(bytes.len() as u64).to_be_bytes());
        hash.update(&bytes);
        Ok(*hash.finalize().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn expected() -> Capability {
        let layout = CurrentLayoutData {
            vault: [31; 32],
            root: [32; 32],
        };
        Capability::for_current_data(layout, &layout.encode_data().unwrap()).unwrap()
    }
    #[test]
    fn lifecycle3_descriptor_is_exact_and_never_accepts_predecessor_formats() {
        let cap = expected();
        let bytes = cap.encode_data().unwrap();
        assert!(Capability::decode_exact(&bytes, cap).is_ok());
        for cut in 0..bytes.len() {
            assert!(Capability::decode_exact(&bytes[..cut], cap).is_err());
        }
        let mut trailing = bytes.to_vec();
        trailing.push(0);
        assert!(Capability::decode_exact(&trailing, cap).is_err());
        // Exact predecessor grammar, not a fabricated runtime owner. The real
        // predecessor-binary gate separately proves old-reader refusal.
        let mut v2 = Encoder::new();
        v2.put_bytes(b"catcoms/private-native-backend-layout/v2\0")
            .unwrap();
        v2.put_u16(2);
        for field in [
            cap.original.vault,
            cap.original.root,
            cap.original.id_data().unwrap(),
        ] {
            v2.put_bytes(&field).unwrap();
        }
        v2.put_u16(2);
        assert!(Capability::decode_exact(&v2.finish(), cap).is_err());
    }
    #[test]
    fn lifecycle3_descriptor_binds_the_full_original_current_descriptor() {
        let cap = expected();
        let bytes = cap.encode_data().unwrap();
        let mut wrong = cap;
        wrong.original.vault[0] ^= 1;
        assert!(Capability::decode_exact(&bytes, wrong).is_err());
        wrong = cap;
        wrong.original.root[0] ^= 1;
        assert!(Capability::decode_exact(&bytes, wrong).is_err());
        // Keep both public labels untouched but alter the full Current id.
        // A parser that only compares those labels would incorrectly accept.
        let mut substituted = bytes.to_vec();
        substituted[4 + TAG.len() + 2 + 2 * 36 + 4] ^= 1;
        assert!(Capability::decode_exact(&substituted, cap).is_err());
        assert_ne!(cap.id_data().unwrap(), wrong.id_data().unwrap());
    }
}
