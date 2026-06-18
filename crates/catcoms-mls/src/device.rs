//! A device's MLS leaf identity.

use core::fmt;

use catcoms_crypto::DeviceId;
use openmls::prelude::*;
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;

use crate::config::{capabilities, CIPHERSUITE};
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
