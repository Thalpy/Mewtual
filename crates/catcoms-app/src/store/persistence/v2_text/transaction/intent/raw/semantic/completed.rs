//! Completed-mode S preserves only actual nextA observed replaced-token entries
//! through original I. A parsed list never becomes deletion permission.
use super::*;
pub(in crate::store::persistence::v2_text::transaction::intent::raw) fn verify_completed_s_data(
    key: &[u8; 32],
    wal: &TextWorkWalFrameData<'_>,
    parent: &TextHeadData,
    parent_frame: &[u8],
    original: FirstSOriginalData<'_>,
    expected_entries: &[crate::store::persistence::v2_text::retirement::RetirementEntryData],
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
    // yields precisely three sorted roots while preserving the owning nextA token
    // retirement entries through the genuine original I.
    if original.carry.roots_data().len() != 2
        || original.carry.entries_data() != expected_entries
        || expected_entries.is_empty()
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
    if carry.roots_data() != roots || carry.entries_data() != expected_entries {
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
