//! Bounded package metadata. A historical extraction is deliberately NOT an admitted/verified
//! KeyPackage: only an independent owner-signed accepted reference can authenticate that data.
use super::*;
use crate::group::MAX_MEMBER_CATALOG_CREDENTIAL_BYTES;

/// Data labels only, never proof of signature validity, lifetime, an applied Add or membership.
/// They cannot construct a KeyPackage, LocalMembershipChange or fresh join capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPackageIdentity {
    subject: [u8; 32],
    reference: [u8; 32],
    credential: [u8; 32],
}
impl KeyPackageIdentity {
    pub fn subject(&self) -> &[u8; 32] {
        &self.subject
    }
    pub fn key_package_reference(&self) -> &[u8; 32] {
        &self.reference
    }
    pub fn credential_digest(&self) -> &[u8; 32] {
        &self.credential
    }
}

/// Existing owner-observation digest, including credential TYPE and exact TLS framing. The
/// content cap precedes the sole credential serialization. A callback marks that real boundary
/// in regression tests; production passes an empty callback and uses no ambient clock/RNG.
pub(crate) fn credential_digest_with(
    credential: &Credential,
    before_encode: impl FnOnce(),
) -> Result<[u8; 32], MlsError> {
    if credential.serialized_content().len() > MAX_MEMBER_CATALOG_CREDENTIAL_BYTES {
        return Err(MlsError::Internal(
            "owner observation credential exceeds bound",
        ));
    }
    before_encode();
    let encoded = credential.tls_serialize_detached().map_err(proto)?;
    if encoded.len() > MAX_MEMBER_CATALOG_CREDENTIAL_BYTES + 8 {
        return Err(MlsError::Internal(
            "owner credential TLS frame exceeds bound",
        ));
    }
    Ok(credential_digest(&encoded))
}

pub(crate) fn credential_digest(bytes: &[u8]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(b"mewtual/chat/mls-credential/v1\0");
    hash.update(bytes);
    *hash.finalize().as_bytes()
}

#[cfg(test)]
std::thread_local! {
    static RETAINED_SDK_DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl MlsDevice {
    /// Extract metadata from an ALREADY validated KeyPackage using this same crypto provider.
    /// The caller must still apply current invite/lifetime policy before any new Add. No Add
    /// observation or fresh completion can be reconstructed from these plain data labels.
    pub fn key_package_identity(
        &self,
        package: &KeyPackage,
    ) -> Result<KeyPackageIdentity, MlsError> {
        let subject = package
            .leaf_node()
            .signature_key()
            .as_slice()
            .try_into()
            .map_err(|_| MlsError::Internal("non-Ed25519 member key"))?;
        let credential = credential_digest_with(package.leaf_node().credential(), || {})?;
        let reference = self.key_package_reference(package)?;
        Ok(KeyPackageIdentity {
            subject,
            reference,
            credential,
        })
    }

    /// Extract bounded historical labels WITHOUT verifying KP/leaf signatures or lifetime.
    ///
    /// **Never use this to admit a new package.** Its only intended consumer verifies a retained
    /// exact request against an independently pinned owner's original signed accepted reference,
    /// full subject/typed credential, Commit and Welcome chain. That attestation authenticates
    /// these historical bytes; this helper alone authenticates nothing and returns no KeyPackage.
    ///
    /// OpenMLS 0.9.2's normal `KeyPackageIn::validate` checks SystemTime even through our ordinary
    /// identification parser. Reusing it for durable accepted outcomes would strand a lost reply
    /// after the accepted package expires. This fixed-format extraction never calls `validate`.
    ///
    /// The <=32KiB source cap precedes SDK parsing. Canonical body serialization and SDK RefHash
    /// make bounded extra copies; `unverified_credential` clones the input-derived credential/key
    /// before our content check. Callers must pay those copies, and this is NOT a complete SDK
    /// typed-heap/preallocation bound. No source map, current group, clock or RNG is mutated.
    pub fn retained_key_package_identity(
        &self,
        bytes: &[u8],
    ) -> Result<KeyPackageIdentity, MlsError> {
        const MAX_BYTES: usize = 32 * 1024;
        // Pinned MLSMessage framing: u16 MLS1.0, u16 WireFormat::KeyPackage. The contained
        // KeyPackage body independently starts with u16 MLS1.0 and the exact pinned suite.
        // Compare the canonical BODY (the actual RefHash input), after checking that exact
        // fixed message header. There is no need to allocate a second complete MLSMessage copy.
        if bytes.len() < 8
            || bytes.len() > MAX_BYTES
            || bytes[..4] != [0, 1, 0, 5]
            || bytes[4..6] != [0, 1]
            || bytes[6..8] != (CIPHERSUITE as u16).to_be_bytes()
        {
            return Err(MlsError::Internal(
                "unsupported retained KeyPackage framing",
            ));
        }
        #[cfg(test)]
        RETAINED_SDK_DECODES.with(|entries| entries.set(entries.get() + 1));
        let mut remaining = bytes;
        let message = MlsMessageIn::tls_deserialize(&mut remaining).map_err(proto)?;
        if !remaining.is_empty() {
            return Err(MlsError::Internal("retained KeyPackage trailing bytes"));
        }
        let MlsMessageBodyIn::KeyPackage(package) = message.extract() else {
            return Err(MlsError::WrongMessageType);
        };
        let canonical = package.tls_serialize_detached().map_err(proto)?;
        if canonical.as_slice() != &bytes[4..] {
            return Err(MlsError::Internal("noncanonical retained KeyPackage"));
        }
        // Hash ONLY the complete canonical KP body (including its signature), exactly as
        // KeyPackage::hash_ref does. Hashing the MLSMessage header too selects another KP.
        let reference = openmls::ciphersuite::hash_ref::make_key_package_ref(
            &canonical,
            CIPHERSUITE,
            self.provider.crypto(),
        )
        .map_err(proto)?
        .as_slice()
        .try_into()
        .map_err(|_| MlsError::Internal("unexpected KeyPackage reference length"))?;
        let raw = package.unverified_credential();
        let subject = raw
            .signature_key
            .as_slice()
            .try_into()
            .map_err(|_| MlsError::Internal("non-Ed25519 member key"))?;
        let credential = credential_digest_with(&raw.credential, || {})?;
        Ok(KeyPackageIdentity {
            subject,
            reference,
            credential,
        })
    }
}

#[cfg(test)]
mod tests;
