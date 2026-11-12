//! Shared bounded wire helpers. Successful parsing is DATA, never a physical receipt.
use crate::AppError;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

pub(super) const VERSION: u16 = 1;

pub(super) fn refused() -> AppError {
    AppError::Invalid("Operational chat metadata is unavailable or unsupported".into())
}

pub(super) fn field<'a>(d: &mut Decoder<'a>, maximum: usize) -> Result<&'a [u8], AppError> {
    let value = d.get_bytes().map_err(|_| refused())?;
    if value.len() > maximum {
        return Err(refused());
    }
    Ok(value)
}

pub(super) fn b32(d: &mut Decoder<'_>) -> Result<[u8; 32], AppError> {
    field(d, 32)?.try_into().map_err(|_| refused())
}

pub(super) fn put(e: &mut Encoder, value: &[u8]) -> Result<(), AppError> {
    e.put_bytes(value).map_err(|_| refused())?;
    Ok(())
}

pub(super) fn tag(e: &mut Encoder, expected: &[u8]) -> Result<(), AppError> {
    put(e, expected)?;
    e.put_u16(VERSION);
    Ok(())
}

pub(super) fn input<'a>(
    raw: &'a [u8],
    maximum: usize,
    expected: &[u8],
) -> Result<Decoder<'a>, AppError> {
    if raw.len() > maximum {
        return Err(refused());
    }
    let mut d = Decoder::new(raw);
    if field(&mut d, expected.len())? != expected || d.get_u16().map_err(|_| refused())? != VERSION
    {
        return Err(refused());
    }
    Ok(d)
}

pub(super) fn finish(e: Encoder, maximum: usize) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if e.len() > maximum {
        return Err(refused());
    }
    Ok(Zeroizing::new(e.finish()))
}

macro_rules! data_id {
    ($name:ident, $domain:literal) => {
        /// Immutable commitment DATA. This label proves neither source custody nor a save.
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub(in crate::store) struct $name([u8; 32]);
        impl $name {
            pub(in crate::store) const fn from_bytes(value: [u8; 32]) -> Self {
                Self(value)
            }
            pub(in crate::store) const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
            pub(in crate::store) fn of_data(raw: &[u8]) -> Self {
                let mut hash = blake3::Hasher::new();
                hash.update($domain);
                hash.update(raw);
                Self(*hash.finalize().as_bytes())
            }
        }
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($name)).finish_non_exhaustive()
            }
        }
    };
}
pub(super) use data_id;

data_id!(
    TextContextIdData,
    b"catcoms/private-chat-text-context-id/v1\0"
);
data_id!(
    TextGenesisParentIdData,
    b"catcoms/private-chat-text-genesis-parent-id/v1\0"
);
data_id!(TextHeadIdData, b"catcoms/private-chat-text-head-id/v1\0");
data_id!(
    TextTransactionIdData,
    b"catcoms/private-chat-text-transaction/v1\0"
);
data_id!(
    TextRetirementPlanIdData,
    b"catcoms/private-chat-text-retirement-plan/v1\0"
);
