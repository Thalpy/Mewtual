//! A device's MLS leaf identity.

use core::fmt;

use catcoms_crypto::DeviceId;
use openmls::prelude::*;
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;
use tls_codec::{Deserialize as _, Serialize as _};

use crate::config::{capabilities, CIPHERSUITE};
use crate::invite::MembershipCredential;
use crate::{proto, MlsError};

/// Serialize a KeyPackage for transport (so a joiner can send it to an inviter).
pub fn serialize_key_package(key_package: &KeyPackage) -> Result<Vec<u8>, MlsError> {
    MlsMessageOut::from(key_package.clone())
        .tls_serialize_detached()
        .map_err(proto)
}

/// One device's MLS identity: its signature keypair, Basic credential, and the
/// openmls provider holding its key/group state. The device id is the
/// content-address of the signature public key, so the leaf key *is* the device
/// identity.
pub struct MlsDevice {
    provider: OpenMlsRustCrypto,
    signer: SignatureKeyPair,
    credential: CredentialWithKey,
    device_id: DeviceId,
}

impl MlsDevice {
    /// Generate a fresh device identity.
    pub fn generate() -> Result<Self, MlsError> {
        let provider = OpenMlsRustCrypto::default();
        let signer = SignatureKeyPair::new(CIPHERSUITE.signature_algorithm()).map_err(proto)?;
        signer.store(provider.storage()).map_err(proto)?;

        let device_id = DeviceId::from_public_key_bytes(signer.public());
        let credential = CredentialWithKey {
            credential: BasicCredential::new(device_id.as_bytes().to_vec()).into(),
            signature_key: signer.public().into(),
        };
        Ok(Self {
            provider,
            signer,
            credential,
            device_id,
        })
    }

    /// Reconstruct a device from a `provider` whose storage has been **restored** from a
    /// snapshot (Phase 9c) — the signature keypair + key/group state already live in that
    /// storage. The signer is read back from storage; the credential + device id re-derive
    /// from its public key. See [`crate::persist`].
    pub(crate) fn restore(
        provider: OpenMlsRustCrypto,
        public_key: &[u8],
    ) -> Result<Self, MlsError> {
        let signer = SignatureKeyPair::read(
            provider.storage(),
            public_key,
            CIPHERSUITE.signature_algorithm(),
        )
        .ok_or(MlsError::Internal("signer missing from restored storage"))?;
        let device_id = DeviceId::from_public_key_bytes(signer.public());
        let credential = CredentialWithKey {
            credential: BasicCredential::new(device_id.as_bytes().to_vec()).into(),
            signature_key: signer.public().into(),
        };
        Ok(Self {
            provider,
            signer,
            credential,
            device_id,
        })
    }

    /// This device's content-addressed id.
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Build a fresh single-use KeyPackage to be published for joining a group.
    /// The private bundle is stored in this device's provider so it can later
    /// process the resulting Welcome.
    pub fn key_package(&self) -> Result<KeyPackage, MlsError> {
        let bundle = KeyPackage::builder()
            .leaf_node_capabilities(capabilities())
            .build(
                CIPHERSUITE,
                &self.provider,
                &self.signer,
                self.credential.clone(),
            )
            .map_err(proto)?;
        Ok(bundle.key_package().clone())
    }

    /// Build a KeyPackage whose leaf credential is bound to a specific
    /// `(group_id, invite_nonce)` — so this KeyPackage can only be admitted into
    /// that group via that invite, and cannot be replayed elsewhere.
    pub fn key_package_for_invite(
        &self,
        group_id: &[u8],
        invite_nonce: [u8; 16],
    ) -> Result<KeyPackage, MlsError> {
        let membership = MembershipCredential {
            device_id: self.device_id,
            group_id: group_id.to_vec(),
            invite_nonce,
        };
        let credential = CredentialWithKey {
            credential: BasicCredential::new(membership.encode()).into(),
            signature_key: self.signer.public().into(),
        };
        let bundle = KeyPackage::builder()
            .leaf_node_capabilities(capabilities())
            .build(CIPHERSUITE, &self.provider, &self.signer, credential)
            .map_err(proto)?;
        Ok(bundle.key_package().clone())
    }

    /// This device's MLS leaf signature public key (raw bytes). Other members
    /// verify this device's signatures against it.
    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.signer.public().to_vec()
    }

    /// Sign arbitrary bytes with this device's MLS leaf key (e.g. inner-signing a
    /// replicated CRDT op so authorship survives transport re-encryption).
    pub fn sign(&self, payload: &[u8]) -> Result<[u8; 64], MlsError> {
        self.sign_raw(payload)
    }

    /// Parse and validate a KeyPackage received over the wire (from a joiner),
    /// using this device's crypto backend.
    pub fn parse_key_package(&self, bytes: &[u8]) -> Result<KeyPackage, MlsError> {
        let message = MlsMessageIn::tls_deserialize(&mut &bytes[..]).map_err(proto)?;
        match message.extract() {
            MlsMessageBodyIn::KeyPackage(kp_in) => kp_in
                .validate(self.provider.crypto(), ProtocolVersion::Mls10)
                .map_err(proto),
            _ => Err(MlsError::WrongMessageType),
        }
    }

    /// Sign raw bytes with this device's MLS leaf key (used for invite tokens).
    pub(crate) fn sign_raw(&self, payload: &[u8]) -> Result<[u8; 64], MlsError> {
        use openmls_traits::signatures::Signer;
        let sig = self
            .signer
            .sign(payload)
            .map_err(|e| MlsError::Protocol(format!("{e:?}")))?;
        sig.as_slice()
            .try_into()
            .map_err(|_| MlsError::Internal("unexpected signature length"))
    }

    pub(crate) fn provider(&self) -> &OpenMlsRustCrypto {
        &self.provider
    }

    pub(crate) fn signer(&self) -> &SignatureKeyPair {
        &self.signer
    }

    pub(crate) fn credential(&self) -> CredentialWithKey {
        self.credential.clone()
    }
}

impl fmt::Debug for MlsDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MlsDevice")
            .field("device_id", &self.device_id)
            .finish_non_exhaustive()
    }
}
