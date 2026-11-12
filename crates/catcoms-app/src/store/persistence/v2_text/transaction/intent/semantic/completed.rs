//! Completed-mode I preserves only the owning nextA's exact observed old token
//! retirement entries. All original intent/SDK/preimage/root/counter checks remain.
use super::*;
pub(in crate::store::persistence::v2_text::transaction::intent) fn verify_completed_i_data(
    key: &[u8; 32],
    wal: &TextWorkWalFrameData<'_>,
    parent: &TextHeadData,
    parent_frame: &[u8],
    archive: &TextArchiveData,
    previous_carry: &RetirementCarryData,
    original: &V2UnsignedAppendDataView<'_>,
    expected_entries: &[crate::store::persistence::v2_text::retirement::RetirementEntryData],
) -> Result<(), AppError> {
    if wal.operation != TextWorkWalOperationData::Unsigned
        || wal.parent_frame != parent_frame
        || wal.install_parent != TextHeadIdData::of_data(parent_frame)
        || wal.candidate == wal.install_parent
        || wal.candidate != TextHeadIdData::of_data(wal.candidate_frame)
        || wal.parent_frame.len() > 1725
        || wal.candidate_frame.len() > 1725
        || wal.context != parent.context.id_data()?
        || archive.spec_data().context != wal.context
        || wal.public_parent != archive.spec_data().header.id()
    {
        return Err(refused());
    }
    let TextPendingData::Retained(a) = parent.pending else {
        return Err(refused());
    };
    if a.transaction != wal.transaction || a.public_parent != wal.public_parent {
        return Err(refused());
    }
    // The caller first checks this list against the genuine owning nextA observer.
    // Preserve its exact observed replaced-token entries; parsed lists alone do not
    // supply the expectation or authorize later retirement.
    let TextParentData::Operational(pre_a) = parent.previous else {
        return Err(refused());
    };
    if previous_carry.roots_data() != [pre_a]
        || previous_carry.entries_data() != expected_entries
        || expected_entries.is_empty()
    {
        return Err(refused());
    }
    let TextWorkWalCleanupData::Carry(raw) = &wal.cleanup else {
        return Err(refused());
    };
    let carry = RetirementCarryData::decode_data(raw)?;
    let mut expected_roots = [pre_a, wal.install_parent];
    expected_roots.sort_unstable();
    if carry.roots_data() != expected_roots || carry.entries_data() != previous_carry.entries_data()
    {
        return Err(refused());
    }

    let plain = open_control_data(
        key,
        TextControlKindData::Head.id_data(wal.candidate_frame),
        wal.candidate_frame,
    )?;
    let candidate = TextHeadData::decode_data(&plain)?;
    candidate.require_successor_data(parent, wal.install_parent)?;
    let TextPendingData::Unsigned {
        transaction: i,
        intent,
    } = candidate.pending
    else {
        return Err(refused());
    };
    let accepted = index_capture_data(
        wal.context,
        TextIndexCaptureRoleData::Directory,
        parent.directory,
    )?;
    let tokens = index_capture_data(wal.context, TextIndexCaptureRoleData::Token, parent.tokens)?;
    if candidate.archive != parent.archive
        || candidate.directory != parent.directory
        || candidate.tokens != parent.tokens
        || candidate.publications != parent.publications
        || candidate.queue_front != parent.queue_front
        || candidate.queue_next != parent.queue_next
        || i.transaction != a.transaction
        || i.token != a.token
        || i.request != a.request
        || i.public_parent != a.public_parent
        || i.retain != a.retain
        || i.captured_head != wal.install_parent
        || i.captured_archive != parent.archive
        || i.accepted_capture != accepted
    {
        return Err(refused());
    }
    let mut entries = wal.artifacts.entries_data();
    let artifact = entries.next().ok_or_else(refused)??;
    if entries.next().is_some()
        || artifact.role_data() != TextArtifactRoleData::Intent
        || artifact.id_data() != *intent.as_bytes()
        || artifact.id_data()
            != *TextControlKindData::Intent
                .id_data(artifact.frame_data())
                .as_bytes()
        || artifact.frame_data().len() > 2849
    {
        return Err(refused());
    }
    let plaintext = open_control_data(
        key,
        TextControlKindData::Intent.id_data(artifact.frame_data()),
        artifact.frame_data(),
    )?;
    let stored = TextUnsignedIntentFrameData::decode_data(&plaintext, &parent.context)?;
    let b = stored.bindings;
    if b.context != wal.context
        || b.transaction != a.transaction
        || b.retain != a.retain
        || b.token != a.token
        || b.request != a.request
        || b.pre_intent_head != wal.install_parent
        || b.pre_intent_archive != parent.archive
        || b.accepted_root != accepted
        || b.token_root != tokens
        || b.public_parent != wal.public_parent
        || b.issuer != parent.context.spec_data().signer
        || b.unsigned_commitment != original.intent
        || stored.public_parent_sequence != archive.spec_data().header.spec().sequence
        || !stored.matches_unsigned_data(original)
    {
        return Err(refused());
    }
    original
        .record
        .matches_action(original.action)
        .map_err(|_| refused())?;
    if candidate.live_bytes
        != parent
            .live_bytes
            .checked_add(artifact.frame_data().len() as u128)
            .ok_or_else(refused)?
        || candidate.live_objects != parent.live_objects.checked_add(1).ok_or_else(refused)?
    {
        return Err(refused());
    }
    Ok(())
}
