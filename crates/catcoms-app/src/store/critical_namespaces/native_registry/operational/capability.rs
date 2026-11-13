//! Explicit version2 descriptor, bound to the exact original Current3 descriptor.
use super::super::super::{archive_current_namespace_codec as current, refused};
use super::super::capability::{field32, NativeCapabilityData};
use crate::AppError;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

pub(in crate::store::critical_namespaces::native_registry) const NAMESPACE_VERSION: u32 = 2;
pub(in crate::store::critical_namespaces::native_registry) const INDEX_FORMAT: u16 = 2;
const TAG: &[u8] = b"catcoms/private-native-backend-layout/v2\0";
const DOMAIN: &[u8] = b"catcoms/native-backend-layout-id/v2\0";
const DESCRIPTOR_BYTES: usize = 4 + TAG.len() + 2 + 3 * 36 + 2;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store::critical_namespaces::native_registry) struct Capability {
    // Its id commits to the FULL validated Current3 descriptor, not only public
    // vault/root labels. Keeping the original v1 labels also makes upgrading a
    // genuine G1 mapping deterministic without changing legacy stable row IDs.
    pub(in crate::store::critical_namespaces::native_registry) original: NativeCapabilityData,
}
impl Capability {
    pub(in crate::store::critical_namespaces::native_registry) fn for_current_data(
        layout: current::CurrentLayoutData,
        descriptor: &[u8],
    ) -> Result<Self, AppError> {
        Ok(Self {
            original: NativeCapabilityData::for_current_data(layout, descriptor)?,
        })
    }
    pub(in crate::store::critical_namespaces::native_registry) fn encode_data(
        self,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let mut output = Encoder::with_capacity(DESCRIPTOR_BYTES);
        output.put_bytes(TAG).map_err(|_| refused())?;
        output.put_u16(2);
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
    pub(in crate::store::critical_namespaces::native_registry) fn decode_exact(
        raw: &[u8],
        expected: Self,
    ) -> Result<Self, AppError> {
        if raw.len() != DESCRIPTOR_BYTES {
            return Err(refused());
        }
        let mut input = Decoder::new(raw);
        if input.get_bytes().map_err(|_| refused())? != TAG
            || input.get_u16().map_err(|_| refused())? != 2
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
    pub(in crate::store::critical_namespaces::native_registry) fn id_data(
        self,
    ) -> Result<[u8; 32], AppError> {
        let bytes = self.encode_data()?;
        let mut hash = blake3::Hasher::new();
        hash.update(DOMAIN);
        hash.update(&(bytes.len() as u64).to_be_bytes());
        hash.update(&bytes);
        Ok(*hash.finalize().as_bytes())
    }
}
