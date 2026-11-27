//! Match the existing persisted invitation context across warm and cold
//! publishers. Both callers hold the original genesis and pin files.
use crate::store::critical_namespaces::archive_bootstrap::founder_codec::encode_context;
use crate::AppError;
use catcoms_sync::owner_v2::BootstrapSourceContextData;

pub(in crate::store) fn invite_base_context(
    original: &BootstrapSourceContextData,
    genesis_frame: &[u8],
    scope_nonce: [u8; 32],
    pin_frame: &[u8],
) -> Result<[u8; 32], AppError> {
    let encoded = encode_context(original)?;
    let mut hash = blake3::Hasher::new();
    hash.update(b"mewtual/private-chat/invite-context/v1\0");
    hash.update(&(encoded.len() as u64).to_be_bytes());
    hash.update(&encoded);
    hash.update(blake3::hash(genesis_frame).as_bytes());
    hash.update(&scope_nonce);
    hash.update(blake3::hash(pin_frame).as_bytes());
    Ok(*hash.finalize().as_bytes())
}
