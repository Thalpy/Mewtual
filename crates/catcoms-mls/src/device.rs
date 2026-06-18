//! A device's MLS leaf identity.

use core::fmt;

use catcoms_crypto::DeviceId;
use openmls::prelude::*;
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;

use crate::config::{capabilities, CIPHERSUITE};
use crate::invite::MembershipCredential;
use crate::{proto, MlsError};

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
