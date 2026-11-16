//! Exact first-S operation semantics DATA, never an issuer of signing, Saved,
//! work, source or verifier authority. The actual producer separately owns/matches
//! the SAME opaque SDK plan and live verifier before using these original DATA loans.
use super::*;
pub(super) mod completed;
use crate::store::persistence::v2_text::ordinary_data::TextUnsignedIntentFrameData;
use catcoms_sync::owner_v2::V2UnsignedAppendDataView;

/// Bounded original loans only; this aggregate is no selected-I/signing permission.
pub(super) struct FirstSOriginalData<'a> {
    pub(super) intent_frame: &'a [u8],
    pub(super) carry: &'a RetirementCarryData,
    pub(super) unsigned: &'a V2UnsignedAppendDataView<'a>,
    pub(super) raw: &'a [u8; 64],
}
pub(super) fn verify_first_s_data(
    key: &[u8; 32],
    wal: &TextWorkWalFrameData<'_>,
    parent: &TextHeadData,
    parent_frame: &[u8],
    original: FirstSOriginalData<'_>,
) -> Result<(), AppError> {
    if wal.operation != TextWorkWalOperationData::RawSignature
        || wal.parent_frame != parent_frame
        || wal.install_parent != TextHeadIdData::of_data(parent_frame)
        || wal.candidate == wal.install_parent
        || wal.candidate != TextHeadIdData::of_data(wal.candidate_frame)
        || wal.parent_frame.len() > 1725
        || wal.candidate_frame.len() > 1725
        || wal.context != parent.context.id_data()?
    {
        return Err(refused());
    }
    let TextPendingData::Unsigned {
        transaction: i,
        intent,
    } = parent.pending
    else {
        return Err(refused());
    };
    // The exact selected I ciphertext, not an unrelated parsed intent body, binds
    // this raw signature to its original preimage and all captured source labels.
    let intent_id = TextControlKindData::Intent.id_data(original.intent_frame);
    if intent_id.as_bytes() != intent.as_bytes() || original.intent_frame.len() > 2849 {
        return Err(refused());
    }
    let intent_plain = open_control_data(key, intent_id, original.intent_frame)?;
    let stored = TextUnsignedIntentFrameData::decode_data(&intent_plain, &parent.context)?;
    let b = stored.bindings;
    let TextParentData::Operational(pre_i) = parent.previous else {
        return Err(refused());
    };
    // The I carry has the original pre-A and O_A roots. Adding only actual O_I
    // yields precisely three sorted roots; first S replaces no private token page.
    if original.carry.roots_data().len() != 2
        || !original.carry.entries_data().is_empty()
        || !original.carry.roots_data().contains(&pre_i)
    {
        return Err(refused());
    }
    let TextWorkWalCleanupData::Carry(raw_carry) = &wal.cleanup else {
        return Err(refused());
    };
    let carry = RetirementCarryData::decode_data(raw_carry)?;
    let mut roots = [
        original.carry.roots_data()[0],
        original.carry.roots_data()[1],
        wal.install_parent,
    ];
    roots.sort_unstable();
    if carry.roots_data() != roots || !carry.entries_data().is_empty() {
        return Err(refused());
    }
    if i.transaction != wal.transaction
        || i.public_parent != wal.public_parent
        || i.captured_head != pre_i
        || b.context != wal.context
        || b.transaction != i.transaction
        || b.retain != i.retain
        || b.token != i.token
        || b.request != i.request
        || b.pre_intent_head != i.captured_head
        || b.pre_intent_archive != i.captured_archive
        || b.public_parent != wal.public_parent
        || b.accepted_root != i.accepted_capture
        || b.unsigned_commitment != original.unsigned.intent
        || b.pre_intent_archive != parent.archive
        || b.accepted_root
            != index_capture_data(
                wal.context,
                TextIndexCaptureRoleData::Directory,
                parent.directory,
            )?
        || b.token_root
            != index_capture_data(wal.context, TextIndexCaptureRoleData::Token, parent.tokens)?
        || b.issuer != parent.context.spec_data().signer
        || !stored.matches_unsigned_data(original.unsigned)
        || !verify_with_public_bytes(
            &parent.context.spec_data().signer,
            original.unsigned.signing_bytes,
            original.raw,
        )
    {
        return Err(refused());
    }
    original
        .unsigned
        .record
        .matches_action(original.unsigned.action)
        .map_err(|_| refused())?;
    let plain = open_control_data(
        key,
        TextControlKindData::Head.id_data(wal.candidate_frame),
        wal.candidate_frame,
    )?;
    let candidate = TextHeadData::decode_data(&plain)?;
    candidate.require_successor_data(parent, wal.install_parent)?;
    let TextPendingData::Raw {
        transaction: s,
        intent: s_intent,
        signature,
    } = candidate.pending
    else {
        return Err(refused());
    };
    if candidate.archive != parent.archive
        || candidate.directory != parent.directory
        || candidate.tokens != parent.tokens
        || candidate.publications != parent.publications
        || candidate.queue_front != parent.queue_front
        || candidate.queue_next != parent.queue_next
        || s != i
        || s_intent != intent
    {
        return Err(refused());
    }
    let mut entries = wal.artifacts.entries_data();
    let artifact = entries.next().ok_or_else(refused)??;
    if entries.next().is_some()
        || artifact.role_data() != TextArtifactRoleData::Signature
        || artifact.frame_data().len() != 405
        || artifact.id_data() != *signature.as_bytes()
        || artifact.id_data()
            != *TextControlKindData::Signature
                .id_data(artifact.frame_data())
                .as_bytes()
    {
        return Err(refused());
    }
    let plain = open_control_data(
        key,
        TextControlKindData::Signature.id_data(artifact.frame_data()),
        artifact.frame_data(),
    )?;
    let record = RawSignatureRecordData::decode_data(&plain)?;
    if record.context != wal.context
        || record.transaction != i.transaction
        || record.intent != intent
        || record.pre_intent_head != pre_i
        || record.public_parent != wal.public_parent
        || record.unsigned_commitment != original.unsigned.intent
        || record.action != original.unsigned.action.id()
        || &record.raw != original.raw
        || !verify_with_public_bytes(
            &parent.context.spec_data().signer,
            original.unsigned.signing_bytes,
            &record.raw,
        )
        || candidate.live_bytes != parent.live_bytes.checked_add(405).ok_or_else(refused)?
        || candidate.live_objects != parent.live_objects.checked_add(1).ok_or_else(refused)?
    {
        return Err(refused());
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod tests;
