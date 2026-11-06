//! DRAFT, cfg(test)-only private V2 genesis metadata DATA. No production namespace handler,
//! source/fee/writer witness, Saved result, Server, Current or projection issuer exists here.
//! Root's actual opener/consumer must select originals and cross the real barriers; a valid
//! MAC, caller expectation, signature or descriptor cannot substitute for those capabilities.

use super::archive_bootstrap::founder_codec::{
    self, DebitId, IntentId, InvocationData, TargetMarkerId,
};
use super::archive_current_namespace_codec::CurrentLayoutData;
use crate::store::{frame, unframe};
use crate::AppError;
use catcoms_crypto::keystore::{seal, unseal};
use catcoms_replication::chat::{
    acceptance_tree::TreeContext,
    owner_header::SignedOwnerHeader,
    owner_profile::{AdmissionBody, AdmissionOrigin, SignedOwnerProfile},
    owner_state::StateFrame,
};
use catcoms_rt::CryptoRngCore;
use catcoms_sync::owner_v2::{BootstrapSourceContextData, BootstrapSourceId};
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

pub(in crate::store) const MAX_PIN_PLAIN: usize = 1024;
pub(in crate::store) const MAX_PIN_FRAME: usize = MAX_PIN_PLAIN + 40;
pub(in crate::store) const MAX_HEAD_PLAIN: usize = 4096;
pub(in crate::store) const MAX_HEAD_FRAME: usize = MAX_HEAD_PLAIN + 40;
pub(in crate::store) const MAX_ACTIVATION_PLAIN: usize = 8192;
pub(in crate::store) const MAX_ACTIVATION_FRAME: usize = MAX_ACTIVATION_PLAIN + 40;
pub(in crate::store) const ACTIVATION_METADATA_FEE: usize = 512 * 1024;
pub(in crate::store) const SCOPE_BUFFER_FEE: usize = 4 * (32 * 1024 * 1024 + 40) + 256 * 1024;

const PIN_TAG: &[u8] = b"catcoms/private-chat-v2-group-pin/v1\0";
const HEAD_TAG: &[u8] = b"catcoms/private-chat-v2-genesis-head/v1\0";
const APP_TAG: &[u8] = b"catcoms/private-chat-v2-genesis-app/v1\0";
const ACTIVATION_TAG: &[u8] = b"catcoms/private-chat-v2-genesis-activation/v1\0";
const VERSION: u16 = 1;
const MAX_GROUP: usize = 256;
const MAX_CONTEXT: usize = 692;
const MAX_APP: usize = 305;
const MAX_NAME: usize = 256;
const MAX_PROFILE: usize = 494;
const MAX_ADMISSION: usize = 490;
const MAX_HEADER: usize = 634;
const MAX_GENESIS_IMAGE: usize = 676;

const _: () = assert!(1019 <= MAX_PIN_PLAIN && 3170 <= MAX_HEAD_PLAIN);
const _: () = assert!(5404 <= MAX_ACTIVATION_PLAIN);
const _: () = assert!(
    catcoms_replication::chat::owner_profile::MAX_PROFILE_BYTES == MAX_PROFILE
        && catcoms_replication::chat::owner_profile::MAX_ADMISSION_BYTES == MAX_ADMISSION
        && catcoms_replication::chat::owner_header::MAX_HEADER_BYTES == MAX_HEADER
);

fn refused() -> AppError {
    AppError::Invalid("V2 genesis metadata is unavailable or unsupported".into())
}
fn field<'a>(d: &mut Decoder<'a>, maximum: usize) -> Result<&'a [u8], AppError> {
    let bytes = d.get_bytes().map_err(|_| refused())?;
    if bytes.len() > maximum {
        return Err(refused());
    }
    Ok(bytes)
}
fn digest(d: &mut Decoder<'_>) -> Result<[u8; 32], AppError> {
    field(d, 32)?.try_into().map_err(|_| refused())
}
fn put(e: &mut Encoder, value: &[u8]) -> Result<(), AppError> {
    e.put_bytes(value).map_err(|_| refused())?;
    Ok(())
}
fn tag(e: &mut Encoder, value: &[u8]) -> Result<(), AppError> {
    put(e, value)?;
    e.put_u16(VERSION);
    Ok(())
}
fn input<'a>(raw: &'a [u8], maximum: usize, expected: &[u8]) -> Result<Decoder<'a>, AppError> {
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
fn finish(e: Encoder, maximum: usize) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if e.len() > maximum {
        return Err(refused());
    }
    Ok(Zeroizing::new(e.finish()))
}
fn seal_data(
    key: &[u8; 32],
    domain: &str,
    plain: &[u8],
    maximum: usize,
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if plain.is_empty() || plain.len() > maximum {
        return Err(refused());
    }
    let derived = Zeroizing::new(blake3::derive_key(domain, key));
    Ok(Zeroizing::new(frame(&seal(&derived, plain, rng)?)))
}
fn open_data(
    key: &[u8; 32],
    domain: &str,
    raw: &[u8],
    maximum: usize,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if raw.len() <= 40 || raw.len() > maximum + 40 {
        return Err(refused());
    }
    let derived = Zeroizing::new(blake3::derive_key(domain, key));
    let plain = Zeroizing::new(unseal(&derived, &unframe(raw)?)?);
    if plain.is_empty() || plain.len() > maximum {
        return Err(refused());
    }
    Ok(plain)
}

macro_rules! frame_id {
    ($name:ident, $domain:literal) => {
        /// Complete ciphertext DATA label, nonce included. This does not prove a file or save.
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub(in crate::store) struct $name([u8; 32]);
        impl $name {
            pub(in crate::store) fn from_bytes(value: [u8; 32]) -> Self {
                Self(value)
            }
            pub(in crate::store) fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
            pub(in crate::store) fn of(raw: &[u8]) -> Self {
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
frame_id!(V2GroupPinId, b"catcoms/private-chat-v2-group-pin-id/v1\0");
// These labels follow the cold reader's current test-only declaration. The warm writer
// compares its actual original Arc/File owners, not reconstructed head/seed DATA labels.
#[cfg(test)]
frame_id!(
    V2GenesisHeadId,
    b"catcoms/private-chat-v2-genesis-head-id/v1\0"
);
#[cfg(test)]
frame_id!(
    V2ActivationSeedId,
    b"catcoms/private-chat-v2-genesis-activation-id/v1\0"
);

/// Root creates this DATA from its actual SourceSaved capture and immutable original debit.
/// A caller constructing the same labels still has no source, quota, row or Current authority.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::store) struct SavedSourceExpectationData {
    pub(in crate::store) invocation: InvocationData,
    pub(in crate::store) debit_id: DebitId,
    pub(in crate::store) terminal_id: IntentId,
    pub(in crate::store) source_id: BootstrapSourceId,
    pub(in crate::store) marker_id: TargetMarkerId,
    pub(in crate::store) context: BootstrapSourceContextData,
    pub(in crate::store) debit_display_name: String,
}
impl std::fmt::Debug for SavedSourceExpectationData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedSourceExpectationData")
            .finish_non_exhaustive()
    }
}
impl SavedSourceExpectationData {
    pub(in crate::store) fn layout_data(&self) -> CurrentLayoutData {
        CurrentLayoutData {
            vault: self.invocation.vault,
            root: self.invocation.root,
        }
    }
    pub(in crate::store) fn validate_data(&self) -> Result<(), AppError> {
        let c = self.context.spec_data();
        if c.vault != self.invocation.vault
            || c.scope_domain != self.invocation.vault
            || c.planned_revision != 1
            || c.group.is_empty()
            || c.group.len() > MAX_GROUP
            || self.debit_display_name.len() > MAX_NAME
        {
            return Err(refused());
        }
        Ok(())
    }
}

/// Original public representations borrowed from the opened source/Factory seed. This is DATA,
/// not an observation that the Profile was created, selected, saved or made Current.
pub(in crate::store) struct GenesisPublicData<'a> {
    pub(in crate::store) profile: &'a SignedOwnerProfile,
    pub(in crate::store) admission: &'a AdmissionBody,
    pub(in crate::store) header: &'a SignedOwnerHeader,
    pub(in crate::store) image: &'a [u8],
}
impl std::fmt::Debug for GenesisPublicData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenesisPublicData").finish_non_exhaustive()
    }
}

fn profile_matches_source_data(
    expected: &SavedSourceExpectationData,
    profile: &SignedOwnerProfile,
) -> Result<(), AppError> {
    expected.validate_data()?;
    let c = expected.context.spec_data();
    if profile.spec().group.len() > MAX_GROUP || !profile.verify() {
        return Err(refused());
    }
    // Exactly the existing actual founder producer's DATA label, not a runtime observation.
    let mut creation = blake3::Hasher::new();
    creation.update(b"mewtual/chat/founder-creation/v2\0");
    creation.update(&(c.group.len() as u32).to_be_bytes());
    creation.update(c.group);
    creation.update(&c.signer);
    creation.update(&c.fresh_attempt);
    if profile.spec().group.as_slice() != c.group
        || profile.spec().founder != c.signer
        || profile.id() != c.profile
        || profile.spec().creation != *creation.finalize().as_bytes()
    {
        return Err(refused());
    }
    Ok(())
}

/// Immutable GROUP-root content DATA. A parser never selects this as a trusted mounted root.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::store) struct GroupPinData {
    pub(in crate::store) layout: CurrentLayoutData,
    pub(in crate::store) group: Vec<u8>,
    pub(in crate::store) profile: SignedOwnerProfile,
    pub(in crate::store) invocation: [u8; 32],
    pub(in crate::store) debit_id: DebitId,
    pub(in crate::store) terminal_id: IntentId,
    pub(in crate::store) source_id: BootstrapSourceId,
}
impl std::fmt::Debug for GroupPinData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupPinData").finish_non_exhaustive()
    }
}
impl GroupPinData {
    pub(in crate::store) fn from_source_data(
        expected: &SavedSourceExpectationData,
        profile: &SignedOwnerProfile,
    ) -> Result<Self, AppError> {
        profile_matches_source_data(expected, profile)?;
        Ok(Self {
            layout: expected.layout_data(),
            group: expected.context.spec_data().group.to_vec(),
            profile: profile.clone(),
            invocation: expected.invocation.invocation,
            debit_id: expected.debit_id,
            terminal_id: expected.terminal_id,
            source_id: expected.source_id,
        })
    }
    pub(in crate::store) fn validate_data(&self) -> Result<(), AppError> {
        // Verify every supplied representation BEFORE any semantic duplicate/root comparison.
        if self.group.is_empty()
            || self.group.len() > MAX_GROUP
            || self.profile.spec().group.len() > MAX_GROUP
            || !self.profile.verify()
            || self.profile.spec().group != self.group
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn require_source_data(
        &self,
        expected: &SavedSourceExpectationData,
    ) -> Result<(), AppError> {
        self.validate_data()?;
        profile_matches_source_data(expected, &self.profile)?;
        if self.layout != expected.layout_data()
            || self.invocation != expected.invocation.invocation
            || self.debit_id != expected.debit_id
            || self.terminal_id != expected.terminal_id
            || self.source_id != expected.source_id
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn encode_data(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.validate_data()?;
        let profile = self.profile.encode();
        if profile.len() > MAX_PROFILE {
            return Err(refused());
        }
        let mut e = Encoder::with_capacity(MAX_PIN_PLAIN);
        tag(&mut e, PIN_TAG)?;
        put(&mut e, &self.layout.vault)?;
        put(&mut e, &self.layout.root)?;
        put(&mut e, &self.group)?;
        e.put_u8(2).put_u8(1);
        put(&mut e, &profile)?;
        for value in [
            &self.invocation,
            self.debit_id.as_bytes(),
            self.terminal_id.as_bytes(),
            self.source_id.as_bytes(),
        ] {
            put(&mut e, value)?;
        }
        finish(e, MAX_PIN_PLAIN)
    }
    pub(in crate::store) fn decode_data(
        raw: &[u8],
        layout: CurrentLayoutData,
        expected_group: &[u8],
    ) -> Result<Self, AppError> {
        if expected_group.is_empty() || expected_group.len() > MAX_GROUP {
            return Err(refused());
        }
        let mut d = input(raw, MAX_PIN_PLAIN, PIN_TAG)?;
        let actual_layout = CurrentLayoutData {
            vault: digest(&mut d)?,
            root: digest(&mut d)?,
        };
        let group = field(&mut d, MAX_GROUP)?;
        if d.get_u8().map_err(|_| refused())? != 2 || d.get_u8().map_err(|_| refused())? != 1 {
            return Err(refused());
        }
        let profile = field(&mut d, MAX_PROFILE)?;
        let invocation = digest(&mut d)?;
        let debit_id = DebitId::from_bytes(digest(&mut d)?);
        let terminal_id = IntentId::from_bytes(digest(&mut d)?);
        let source_id = BootstrapSourceId::from_bytes(digest(&mut d)?);
        d.finish().map_err(|_| refused())?;
        // All variable ranges and EOF precede any copied group or typed Profile allocation.
        let profile = SignedOwnerProfile::decode(profile).map_err(|_| refused())?;
        if actual_layout != layout || group != expected_group {
            return Err(refused());
        }
        let value = Self {
            layout,
            group: group.to_vec(),
            profile,
            invocation,
            debit_id,
            terminal_id,
            source_id,
        };
        value.validate_data()?;
        Ok(value)
    }
}
pub(in crate::store) fn seal_pin_data(
    key: &[u8; 32],
    data: &GroupPinData,
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    seal_data(
        key,
        "catcoms/private-chat-v2-group-pin-key/v1",
        &data.encode_data()?,
        MAX_PIN_PLAIN,
        rng,
    )
}
pub(in crate::store) fn open_pin_data(
    key: &[u8; 32],
    layout: CurrentLayoutData,
    group: &[u8],
    raw: &[u8],
) -> Result<GroupPinData, AppError> {
    let plain = open_data(
        key,
        "catcoms/private-chat-v2-group-pin-key/v1",
        raw,
        MAX_PIN_PLAIN,
    )?;
    GroupPinData::decode_data(&plain, layout, group)
}

fn encode_app_data(name: &str) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if name.len() > MAX_NAME {
        return Err(refused());
    }
    let mut e = Encoder::with_capacity(MAX_APP);
    tag(&mut e, APP_TAG)?;
    put(&mut e, name.as_bytes())?;
    finish(e, MAX_APP)
}
fn check_app_data(raw: &[u8], expected: &str) -> Result<(), AppError> {
    let mut d = input(raw, MAX_APP, APP_TAG)?;
    let name = field(&mut d, MAX_NAME)?;
    d.finish().map_err(|_| refused())?;
    if std::str::from_utf8(name).map_err(|_| refused())? != expected {
        return Err(refused());
    }
    Ok(())
}

/// Exact local genesis-head content DATA. It is not a selected/saved/current head or writer.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::store) struct GenesisHeadData {
    pub(in crate::store) source: SavedSourceExpectationData,
    pub(in crate::store) pin_id: V2GroupPinId,
    pub(in crate::store) admission: AdmissionBody,
    pub(in crate::store) header: SignedOwnerHeader,
    pub(in crate::store) image: Zeroizing<Vec<u8>>,
}
impl std::fmt::Debug for GenesisHeadData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenesisHeadData").finish_non_exhaustive()
    }
}
impl GenesisHeadData {
    pub(in crate::store) fn from_source_data(
        expected: &SavedSourceExpectationData,
        pin_id: V2GroupPinId,
        pin: &GroupPinData,
        public: &GenesisPublicData<'_>,
    ) -> Result<Self, AppError> {
        expected.validate_data()?;
        if public.image.len() > MAX_GENESIS_IMAGE
            || public.admission.spec().group.len() > MAX_GROUP
            || public.header.spec().group.len() > MAX_GROUP
        {
            return Err(refused());
        }
        let value = Self {
            source: expected.clone(),
            pin_id,
            admission: public.admission.clone(),
            header: public.header.clone(),
            image: Zeroizing::new(public.image.to_vec()),
        };
        value.validate_data(pin)?;
        value.require_original_public_data(pin, public)?;
        Ok(value)
    }
    pub(in crate::store) fn validate_data(&self, pin: &GroupPinData) -> Result<(), AppError> {
        self.source.validate_data()?;
        pin.require_source_data(&self.source)?;
        if self.admission.spec().group.len() > MAX_GROUP
            || self.header.spec().group.len() > MAX_GROUP
            || self.image.len() > MAX_GENESIS_IMAGE
            || !self.header.verify()
        {
            return Err(refused());
        }
        let c = self.source.context.spec_data();
        if self.admission.id() != c.admission || self.header.id() != c.header {
            return Err(refused());
        }
        let AdmissionOrigin::Founder { creation } = &self.admission.spec().origin else {
            return Err(refused());
        };
        if *creation != pin.profile.spec().creation {
            return Err(refused());
        }
        let image = StateFrame::decode(&self.image).map_err(|_| refused())?;
        image
            .verify_genesis_binding(&pin.profile, &self.admission, &self.header)
            .map_err(|_| refused())?;
        let slot = image.slots().next().ok_or_else(refused)?;
        let empty = TreeContext {
            profile: pin.profile.id(),
            admission: self.admission.admission_id(),
        }
        .empty_hash();
        if slot.tip_body != self.admission.id()
            || slot.accepted_count != 0
            || slot.accepted_root != empty
        {
            return Err(refused());
        }
        Ok(())
    }
    /// Exact representative equality, not semantic-CID equality. Every supplied signature is
    /// checked before comparison; invalid-first duplicates cannot bypass cryptographic work.
    pub(in crate::store) fn require_original_public_data(
        &self,
        pin: &GroupPinData,
        original: &GenesisPublicData<'_>,
    ) -> Result<(), AppError> {
        self.validate_data(pin)?;
        profile_matches_source_data(&self.source, original.profile)?;
        if original.admission.spec().group.len() > MAX_GROUP
            || original.header.spec().group.len() > MAX_GROUP
            || original.image.len() > MAX_GENESIS_IMAGE
            || !original.header.verify()
        {
            return Err(refused());
        }
        if pin.profile.encode() != original.profile.encode()
            || self.admission.encode() != original.admission.encode()
            || self.header.encode() != original.header.encode()
            || self.image.as_slice() != original.image
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn encode_data(
        &self,
        pin: &GroupPinData,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.validate_data(pin)?;
        let context = founder_codec::encode_context(&self.source.context)?;
        let app = encode_app_data(&self.source.debit_display_name)?;
        let admission = self.admission.encode();
        let header = self.header.encode();
        if admission.len() > MAX_ADMISSION || header.len() > MAX_HEADER {
            return Err(refused());
        }
        let mut e = Encoder::with_capacity(MAX_HEAD_PLAIN);
        tag(&mut e, HEAD_TAG)?;
        e.put_u8(2).put_u8(1).put_u64(1).put_u8(0);
        let layout = self.source.layout_data();
        for value in [
            &layout.vault,
            &layout.root,
            self.pin_id.as_bytes(),
            &self.source.invocation.invocation,
            self.source.debit_id.as_bytes(),
            self.source.terminal_id.as_bytes(),
            self.source.source_id.as_bytes(),
            self.source.marker_id.as_bytes(),
        ] {
            put(&mut e, value)?;
        }
        put(&mut e, &context)?;
        put(&mut e, &app)?;
        e.put_u64(2);
        put(&mut e, &admission)?;
        put(&mut e, &header)?;
        put(&mut e, &self.image)?;
        finish(e, MAX_HEAD_PLAIN)
    }
    pub(in crate::store) fn decode_data(
        raw: &[u8],
        expected: &SavedSourceExpectationData,
        expected_pin_id: V2GroupPinId,
        pin: &GroupPinData,
    ) -> Result<Self, AppError> {
        expected.validate_data()?;
        let mut d = input(raw, MAX_HEAD_PLAIN, HEAD_TAG)?;
        if d.get_u8().map_err(|_| refused())? != 2
            || d.get_u8().map_err(|_| refused())? != 1
            || d.get_u64().map_err(|_| refused())? != 1
            || d.get_u8().map_err(|_| refused())? != 0
        {
            return Err(refused());
        }
        let layout = CurrentLayoutData {
            vault: digest(&mut d)?,
            root: digest(&mut d)?,
        };
        let pin_id = V2GroupPinId::from_bytes(digest(&mut d)?);
        let invocation = digest(&mut d)?;
        let debit = DebitId::from_bytes(digest(&mut d)?);
        let terminal = IntentId::from_bytes(digest(&mut d)?);
        let source = BootstrapSourceId::from_bytes(digest(&mut d)?);
        let marker = TargetMarkerId::from_bytes(digest(&mut d)?);
        let context = field(&mut d, MAX_CONTEXT)?;
        let app = field(&mut d, MAX_APP)?;
        if d.get_u64().map_err(|_| refused())? != 2 {
            return Err(refused());
        }
        let admission = field(&mut d, MAX_ADMISSION)?;
        let header = field(&mut d, MAX_HEADER)?;
        let image = field(&mut d, MAX_GENESIS_IMAGE)?;
        d.finish().map_err(|_| refused())?;
        // Complete outer ranges/EOF BEFORE expected encoding, clones or typed public copies.
        let expected_context = founder_codec::encode_context(&expected.context)?;
        if layout != expected.layout_data()
            || pin_id != expected_pin_id
            || invocation != expected.invocation.invocation
            || debit != expected.debit_id
            || terminal != expected.terminal_id
            || source != expected.source_id
            || marker != expected.marker_id
            || context != expected_context.as_slice()
        {
            return Err(refused());
        }
        check_app_data(app, &expected.debit_display_name)?;
        let admission = AdmissionBody::decode(admission).map_err(|_| refused())?;
        let header = SignedOwnerHeader::decode(header).map_err(|_| refused())?;
        let value = Self {
            source: expected.clone(),
            pin_id,
            admission,
            header,
            image: Zeroizing::new(image.to_vec()),
        };
        value.validate_data(pin)?;
        Ok(value)
    }
}
pub(in crate::store) fn seal_head_data(
    key: &[u8; 32],
    data: &GenesisHeadData,
    pin: &GroupPinData,
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    seal_data(
        key,
        "catcoms/private-chat-v2-genesis-head-key/v1",
        &data.encode_data(pin)?,
        MAX_HEAD_PLAIN,
        rng,
    )
}
pub(in crate::store) fn open_head_data(
    key: &[u8; 32],
    expected: &SavedSourceExpectationData,
    pin_id: V2GroupPinId,
    pin: &GroupPinData,
    raw: &[u8],
) -> Result<GenesisHeadData, AppError> {
    let plain = open_data(
        key,
        "catcoms/private-chat-v2-genesis-head-key/v1",
        raw,
        MAX_HEAD_PLAIN,
    )?;
    GenesisHeadData::decode_data(&plain, expected, pin_id, pin)
}

/// Complete original cipher references plus validated content DATA. The source-free parser
/// cannot create original mount/source/writer custody, select a Current head or authorize I/O.
#[cfg(test)]
pub(in crate::store) struct ActivationSeedData<'a> {
    pub(in crate::store) pin_frame: &'a [u8],
    pub(in crate::store) head_frame: &'a [u8],
    pub(in crate::store) pin: GroupPinData,
    pub(in crate::store) head: GenesisHeadData,
}
#[cfg(test)]
impl std::fmt::Debug for ActivationSeedData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivationSeedData").finish_non_exhaustive()
    }
}
pub(in crate::store) fn encode_activation_seed_data(
    key: &[u8; 32],
    expected: &SavedSourceExpectationData,
    pin_frame: &[u8],
    head_frame: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    expected.validate_data()?;
    if pin_frame.len() > MAX_PIN_FRAME || head_frame.len() > MAX_HEAD_FRAME {
        return Err(refused());
    }
    let pin = open_pin_data(
        key,
        expected.layout_data(),
        expected.context.spec_data().group,
        pin_frame,
    )?;
    pin.require_source_data(expected)?;
    open_head_data(key, expected, V2GroupPinId::of(pin_frame), &pin, head_frame)?;
    let mut e = Encoder::with_capacity(MAX_ACTIVATION_PLAIN);
    tag(&mut e, ACTIVATION_TAG)?;
    let layout = expected.layout_data();
    for value in [
        &layout.vault,
        &layout.root,
        expected.source_id.as_bytes(),
        expected.terminal_id.as_bytes(),
    ] {
        put(&mut e, value)?;
    }
    put(&mut e, pin_frame)?;
    put(&mut e, head_frame)?;
    finish(e, MAX_ACTIVATION_PLAIN)
}
// Move this cold DATA decoder with its real cold consumer when that subtree is activated.
#[cfg(test)]
pub(in crate::store) fn parse_activation_seed_data<'a>(
    key: &[u8; 32],
    expected: &SavedSourceExpectationData,
    plain: &'a [u8],
) -> Result<ActivationSeedData<'a>, AppError> {
    expected.validate_data()?;
    let mut d = input(plain, MAX_ACTIVATION_PLAIN, ACTIVATION_TAG)?;
    let layout = CurrentLayoutData {
        vault: digest(&mut d)?,
        root: digest(&mut d)?,
    };
    let source = BootstrapSourceId::from_bytes(digest(&mut d)?);
    let terminal = IntentId::from_bytes(digest(&mut d)?);
    let pin_frame = field(&mut d, MAX_PIN_FRAME)?;
    let head_frame = field(&mut d, MAX_HEAD_FRAME)?;
    d.finish().map_err(|_| refused())?;
    if layout != expected.layout_data()
        || source != expected.source_id
        || terminal != expected.terminal_id
    {
        return Err(refused());
    }
    let pin = open_pin_data(key, layout, expected.context.spec_data().group, pin_frame)?;
    pin.require_source_data(expected)?;
    let head = open_head_data(key, expected, V2GroupPinId::of(pin_frame), &pin, head_frame)?;
    Ok(ActivationSeedData {
        pin_frame,
        head_frame,
        pin,
        head,
    })
}
pub(in crate::store) fn seal_activation_seed_data(
    key: &[u8; 32],
    plain: &[u8],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    seal_data(
        key,
        "catcoms/private-chat-v2-genesis-activation-key/v1",
        plain,
        MAX_ACTIVATION_PLAIN,
        rng,
    )
}
#[cfg(test)]
pub(in crate::store) fn open_activation_seed_data(
    key: &[u8; 32],
    raw: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    open_data(
        key,
        "catcoms/private-chat-v2-genesis-activation-key/v1",
        raw,
        MAX_ACTIVATION_PLAIN,
    )
}

#[cfg(test)]
#[path = "archive_current_codec_tests.rs"]
mod tests;
