//! Pinned MLS configuration: one ciphersuite, ciphertext-only wire format, and
//! capabilities locked to that single ciphersuite (a hard downgrade floor).

use openmls::prelude::*;

/// The single pinned ciphersuite for all Mewtual groups
/// (`0x0003`: X25519 + ChaCha20-Poly1305 + SHA-256 + Ed25519).
pub const CIPHERSUITE: Ciphersuite =
    Ciphersuite::MLS_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519;

/// Capabilities advertising exactly our one ciphersuite and the Basic credential.
/// Locking this prevents any future second ciphersuite from being negotiated in
/// band (a downgrade vector).
pub(crate) fn capabilities() -> Capabilities {
    Capabilities::new(
        None,                           // default protocol version (MLS 1.0)
        Some(&[CIPHERSUITE]),           // only our ciphersuite
        None,                           // default extensions
        None,                           // default proposals
        Some(&[CredentialType::Basic]), // Basic credentials only
    )
}

/// The group-creation config: pinned ciphersuite, ciphertext-only wire format,
/// ratchet-tree extension (so Welcomes carry the tree), locked capabilities.
pub(crate) fn create_config() -> MlsGroupCreateConfig {
    MlsGroupCreateConfig::builder()
        .ciphersuite(CIPHERSUITE)
        .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .use_ratchet_tree_extension(true)
        .capabilities(capabilities())
        .build()
}

/// The join config, matching the create config's wire format and ratchet-tree use.
pub(crate) fn join_config() -> MlsGroupJoinConfig {
    MlsGroupJoinConfig::builder()
        .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .use_ratchet_tree_extension(true)
        .build()
}
