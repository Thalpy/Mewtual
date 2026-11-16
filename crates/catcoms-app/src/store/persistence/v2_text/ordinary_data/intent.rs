//! Exact unsigned-child intent DATA, separate from a saved intent or signer capability.
//!
//! A borrowed decoder checks framing, stable context and internal record/image bindings.
//! It does not establish replay derivation, accepted absence, physical selection or the
//! original opaque SDK plan. The actual host must compare the SAME retained plan outside
//! writers, then obtain its concrete selected-I signing guard before the raw key cut.
use super::super::common::*;
use super::super::context::TextContextData;
use super::super::request::{TextRequestDigestData, TextRetainIdData};
use crate::AppError;
use catcoms_replication::chat::cold_manifest::ArchiveHeadId;
use catcoms_replication::chat::owner_bodies::{AcceptanceRecord, AppendBody};
use catcoms_replication::chat::owner_header::{HeaderKind, OwnerHeaderId};
use catcoms_replication::chat::owner_state::StateFrame;
use catcoms_sync::owner_v2::V2UnsignedAppendDataView;
use catcoms_wire::Encoder;
use zeroize::Zeroizing;

const TAG: &[u8] = b"catcoms/private-chat-text-intent/v1\0";
pub(in crate::store) const MAX_UNSIGNED_INTENT_PLAIN: usize = 2809;
const _: () =
    assert!(4 + TAG.len() + 2 + 12 * 36 + 8 + 524 + 680 + 571 + 552 == MAX_UNSIGNED_INTENT_PLAIN);

/// Persistent labels only. No selected-source, signing, SDK, File or close owner can be
/// recovered from this struct. The pre-I head/archive are O_A/H_A, never O_I or a self ID.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct TextUnsignedIntentBindingsData {
    pub(in crate::store) context: TextContextIdData,
    pub(in crate::store) transaction: TextTransactionIdData,
    pub(in crate::store) retain: TextRetainIdData,
    pub(in crate::store) token: [u8; 32],
    pub(in crate::store) request: TextRequestDigestData,
    pub(in crate::store) pre_intent_head: TextHeadIdData,
    pub(in crate::store) pre_intent_archive: ArchiveHeadId,
    pub(in crate::store) accepted_root: [u8; 32],
    pub(in crate::store) token_root: [u8; 32],
    pub(in crate::store) public_parent: OwnerHeaderId,
    pub(in crate::store) issuer: [u8; 32],
    pub(in crate::store) unsigned_commitment: [u8; 32],
}

/// All variable fields borrow the caller's already-paid complete plaintext. The only
/// temporary allocations are bounded canonical Core record/body checks; no full history,
/// copied image or semantic-header parser is introduced here.
pub(in crate::store) struct TextUnsignedIntentFrameData<'a> {
    pub(in crate::store) bindings: TextUnsignedIntentBindingsData,
    pub(in crate::store) public_parent_sequence: u64,
    signing: &'a [u8],
    image: &'a [u8],
    record: &'a [u8],
    append: &'a [u8],
}

fn unsigned_commitment(signing: &[u8], image: &[u8]) -> [u8; 32] {
    // Exact original Core UnsignedAppendStepData commitment grammar. This checks DATA
    // consistency only; it cannot create that opaque plan or its selected-I authority.
    let mut h = blake3::Hasher::new();
    h.update(b"catcoms/v2-unsigned-append-intent/v1\0");
    h.update(&(signing.len() as u64).to_be_bytes());
    h.update(signing);
    h.update(&(image.len() as u64).to_be_bytes());
    h.update(image);
    *h.finalize().as_bytes()
}

impl TextUnsignedIntentFrameData<'_> {
    fn validate_data(&self, context: &TextContextData) -> Result<(), AppError> {
        let b = self.bindings;
        let c = context.spec_data();
        if b.context != context.id_data()?
            || b.issuer != c.signer
            || b.transaction
                != TextTransactionIdData::from_request_data(b.context, b.token, b.request)
            || self.signing.is_empty()
            || self.signing.len() > 520
            || self.image.is_empty()
            || self.image.len() > 676
            || self.record.is_empty()
            || self.record.len() > 567
            || self.append.is_empty()
            || self.append.len() > 548
            || unsigned_commitment(self.signing, self.image) != b.unsigned_commitment
        {
            return Err(refused());
        }
        let record = AcceptanceRecord::decode(self.record).map_err(|_| refused())?;
        let append = AppendBody::decode(self.append).map_err(|_| refused())?;
        let frame = StateFrame::decode(self.image).map_err(|_| refused())?;
        let meta = frame.meta();
        let r = record.spec();
        let a = append.spec();
        if record.encode() != self.record
            || append.encode() != self.append
            || r.group.as_slice() != c.group
            || r.subject != c.signer
            || r.context.profile != c.profile
            || r.context.admission.as_bytes() != c.admission.as_bytes()
            || a.group.as_slice() != c.group
            || a.previous_tip != b.public_parent
            || a.old_tip.context != r.context
            || a.new_tip.context != r.context
            || a.leaves.as_slice() != [record.leaf()]
            || meta.group != c.group
            || meta.profile != c.profile
            || meta.issuer != c.signer
            || meta.tenure != c.tenure
            || meta.parent != b.public_parent
            || meta.sequence
                != self
                    .public_parent_sequence
                    .checked_add(1)
                    .ok_or_else(refused)?
            || meta.kind != HeaderKind::AcceptanceAppend
            || meta.body != append.id()
            || meta.mls_epoch != 0
        {
            return Err(refused());
        }
        // Structural consistency above is not a fold from the real parent/frontier. Cold
        // recovery derives that actual mathematical plan and compares all four originals.
        Ok(())
    }

    /// Bounded DATA equality, intended OUTSIDE host writers. A true result supplies no
    /// signing authority and does not replace the actual original-plan observation.
    pub(in crate::store) fn matches_unsigned_data(
        &self,
        view: &V2UnsignedAppendDataView<'_>,
    ) -> bool {
        self.bindings.unsigned_commitment == view.intent
            && self.signing == view.signing_bytes
            && self.image == view.image
            && self.record == view.record.encode()
            && self.append == view.body.encode()
    }
}

impl<'a> TextUnsignedIntentFrameData<'a> {
    pub(in crate::store) fn decode_data(
        raw: &'a [u8],
        context: &TextContextData,
    ) -> Result<Self, AppError> {
        let mut d = input(raw, MAX_UNSIGNED_INTENT_PLAIN, TAG)?;
        let bindings = TextUnsignedIntentBindingsData {
            context: TextContextIdData::from_bytes(b32(&mut d)?),
            transaction: TextTransactionIdData::from_bytes(b32(&mut d)?),
            retain: TextRetainIdData::from_bytes(b32(&mut d)?),
            token: b32(&mut d)?,
            request: TextRequestDigestData::from_bytes(b32(&mut d)?),
            pre_intent_head: TextHeadIdData::from_bytes(b32(&mut d)?),
            pre_intent_archive: ArchiveHeadId::from_bytes(b32(&mut d)?),
            accepted_root: b32(&mut d)?,
            token_root: b32(&mut d)?,
            public_parent: OwnerHeaderId::from_bytes(b32(&mut d)?),
            issuer: b32(&mut d)?,
            unsigned_commitment: b32(&mut d)?,
        };
        let public_parent_sequence = d.get_u64().map_err(|_| refused())?;
        let signing = field(&mut d, 520)?;
        let image = field(&mut d, 676)?;
        let record = field(&mut d, 567)?;
        let append = field(&mut d, 548)?;
        d.finish().map_err(|_| refused())?;
        let parsed = Self {
            bindings,
            public_parent_sequence,
            signing,
            image,
            record,
            append,
        };
        parsed.validate_data(context)?;
        Ok(parsed)
    }
}

/// Encode the actual unsigned SDK view under the caller's real prepaid work invoice.
/// This neither saves I nor chooses a new child: it cannot call a key, RNG or exporter.
pub(in crate::store) fn encode_unsigned_intent_data(
    context: &TextContextData,
    bindings: TextUnsignedIntentBindingsData,
    public_parent_sequence: u64,
    view: &V2UnsignedAppendDataView<'_>,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    view.record
        .matches_action(view.action)
        .map_err(|_| refused())?;
    let record = Zeroizing::new(view.record.encode());
    let append = Zeroizing::new(view.body.encode());
    let data = TextUnsignedIntentFrameData {
        bindings,
        public_parent_sequence,
        signing: view.signing_bytes,
        image: view.image,
        record: &record,
        append: &append,
    };
    data.validate_data(context)?;
    if bindings.unsigned_commitment != view.intent {
        return Err(refused());
    }
    let mut e = Encoder::with_capacity(MAX_UNSIGNED_INTENT_PLAIN);
    tag(&mut e, TAG)?;
    for digest in [
        bindings.context.as_bytes(),
        bindings.transaction.as_bytes(),
        bindings.retain.as_bytes(),
        &bindings.token,
        bindings.request.as_bytes(),
        bindings.pre_intent_head.as_bytes(),
        bindings.pre_intent_archive.as_bytes(),
        &bindings.accepted_root,
        &bindings.token_root,
        bindings.public_parent.as_bytes(),
        &bindings.issuer,
        &bindings.unsigned_commitment,
    ] {
        put(&mut e, digest)?;
    }
    e.put_u64(public_parent_sequence);
    for bytes in [data.signing, data.image, data.record, data.append] {
        put(&mut e, bytes)?;
    }
    finish(e, MAX_UNSIGNED_INTENT_PLAIN)
}

macro_rules! redacted {
    ($($t:ty),+) => {$(
        impl std::fmt::Debug for $t {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("UnsignedIntentData { .. }")
            }
        }
    )+};
}
redacted!(
    TextUnsignedIntentBindingsData,
    TextUnsignedIntentFrameData<'_>
);
