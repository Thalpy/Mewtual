//! A device's MLS leaf identity.

use core::fmt;

use catcoms_crypto::DeviceId;
use openmls::prelude::*;
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;
use tls_codec::{Deserialize as _, Serialize as _};

use crate::config::{capabilities, CIPHERSUITE};
use crate::invite::{InviteError, MembershipCredential};
use crate::{proto, MlsError};

/// Serialize a KeyPackage for transport (so a joiner can send it to an inviter).
pub fn serialize_key_package(key_package: &KeyPackage) -> Result<Vec<u8>, MlsError> {
    MlsMessageOut::from(key_package.clone())
        .tls_serialize_detached()
        .map_err(proto)
}

/// The shortest time a joiner's KeyPackage must remain valid after it is admitted: 60 days.
///
/// openmls checks an added KeyPackage's lifetime against the wall clock on **every** member that
/// processes the Add, not only on the committer, and that includes a member replaying the Add from
/// the commit log long afterwards. There is no production switch to skip it. So a joiner that signs
/// a KeyPackage valid now but expiring a minute later is admitted by the committer, and then every
/// member that was offline, or catches up later, can never apply that commit and stays at the old
/// epoch until it is re-invited. The joiner chooses the lifetime, so this is checked at admission.
///
/// Honest KeyPackages carry openmls's default 84 days and are admitted within an hour of being
/// minted, so they clear this with weeks to spare; the slack also absorbs a joiner clock that is
/// behind the committer's. What remains: a member offline for longer than the remaining lifetime
/// (at least 60 days, about 84 for honest joiners) still cannot replay that Add
/// (`docs/design-mls-leaf-lifetime.md` section 6).
///
/// Only `not_after` is checked. A `not_before` set to "just now" makes members whose clocks lag
/// refuse the Add only until their clocks pass it, which catch-up retries outlive; a short
/// `not_after` is permanent.
pub const MIN_ADMITTED_KEY_PACKAGE_LIFETIME_MS: u64 = 60 * 24 * 60 * 60 * 1000;

/// Refuse a KeyPackage lifetime that ends sooner than [`MIN_ADMITTED_KEY_PACKAGE_LIFETIME_MS`]
/// after `now_ms`.
fn check_admitted_lifetime(lifetime: &Lifetime, now_ms: u64) -> Result<(), InviteError> {
    let not_after_ms = lifetime.not_after().saturating_mul(1000);
    if not_after_ms < now_ms.saturating_add(MIN_ADMITTED_KEY_PACKAGE_LIFETIME_MS) {
        return Err(InviteError::KeyPackageExpiresTooSoon);
    }
    Ok(())
}

/// The raw Ed25519 **leaf signature key** of a parsed KeyPackage.
///
/// Exposed so the admitting layer can verify a signature the *presenting device* made over its
/// own admission request without depending on openmls: a `DeviceId` is a one-way hash of this
/// key, so it cannot be recovered from the id.
pub fn key_package_signature_key(key_package: &KeyPackage) -> Vec<u8> {
    key_package.leaf_node().signature_key().as_slice().to_vec()
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
    /// snapshot (Phase 9c); the signature keypair + key/group state already live in that
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

    /// A second handle on the **same device identity**, backed by its own fresh provider.
    ///
    /// One `MlsDevice` owns exactly one openmls provider, and one provider backs exactly one
    /// group; so a device that belongs to several servers needs one `MlsDevice` per server.
    /// For a *founder/joiner* those are deliberately unrelated identities (per-server
    /// unlinkability, see `docs/design-multi-device.md`), which is why nothing else in the
    /// codebase duplicates a device. A **companion** admitted by a grant bundle is the one
    /// case that cannot work that way: every certificate in the bundle names one
    /// `new_device_id`, so the same signature key must back every server the companion joins.
    ///
    /// This copies key material *within the process* into a new in-memory provider; it never
    /// serializes a key to disk or to the wire (the design's hard "no device key is ever
    /// exported" rule is about crossing the process boundary, which this does not).
    pub fn duplicate(&self) -> Result<Self, MlsError> {
        let provider = OpenMlsRustCrypto::default();
        {
            let src = self
                .provider
                .storage()
                .values
                .read()
                .map_err(|_| MlsError::Internal("MLS storage lock poisoned"))?;
            let mut dst = provider
                .storage()
                .values
                .write()
                .map_err(|_| MlsError::Internal("MLS storage lock poisoned"))?;
            *dst = src.clone();
        }
        Self::restore(provider, &self.public_key_bytes())
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
    /// `(group_id, invite_nonce)`; so this KeyPackage can only be admitted into
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
    ///
    /// `now_ms` comes from the caller's `catcoms-rt` `Clock`. Besides openmls's own checks
    /// (signature, capabilities, and a lifetime covering the current time), the KeyPackage must
    /// stay valid for at least [`MIN_ADMITTED_KEY_PACKAGE_LIFETIME_MS`] after `now_ms`; see that
    /// constant for why.
    pub fn parse_key_package(&self, bytes: &[u8], now_ms: u64) -> Result<KeyPackage, MlsError> {
        let message = MlsMessageIn::tls_deserialize(&mut &bytes[..]).map_err(proto)?;
        let key_package = match message.extract() {
            MlsMessageBodyIn::KeyPackage(kp_in) => kp_in
                .validate(self.provider.crypto(), ProtocolVersion::Mls10)
                .map_err(proto)?,
            _ => return Err(MlsError::WrongMessageType),
        };
        check_admitted_lifetime(key_package.life_time(), now_ms)?;
        Ok(key_package)
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
