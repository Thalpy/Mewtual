//! JOIN descriptor-version2 DATA reader for the explicit private checkpoint network policy.
//! Strict default Pending parsing MUST continue rejecting version2. Its concrete configured
//! execution reader requires the actual original debit producer and retained network owner.
use super::{codec, execution_state as execution, refused};
use crate::AppError;
use catcoms_wire::{Decoder, Encoder};

pub(in crate::store::critical_namespaces) const VERSION: u32 = 2;
pub(in crate::store::critical_namespaces) const TAG: &str = "catcoms/destination-join-journal/v2";

pub(in crate::store::critical_namespaces) fn source_identity(
    metadata: &codec::Metadata,
) -> execution::SourceIdentity {
    execution::SourceIdentity {
        binding: metadata.binding,
        request: metadata.request,
        package: metadata.package,
    }
}

/// Full borrowed grammar including all five original source and execution frames. This supplies
/// neither an execution grant nor source admission; actual consumers still authenticate original
/// mounted files, pins, source/private keys and independent negative custody before effects.
pub(in crate::store::critical_namespaces) fn parse<'a>(
    bytes: &'a [u8],
    domain: &[u8; 32],
) -> Result<([Option<codec::Row<'a>>; codec::MAX_ROWS], usize), AppError> {
    if bytes.len() > codec::MAX_JOIN_PAYLOAD {
        return Err(refused());
    }
    let mut input = Decoder::new(bytes);
    if input.get_str().map_err(|_| refused())? != TAG {
        return Err(refused());
    }
    let count = input.get_u32().map_err(|_| refused())? as usize;
    if count == 0 || count > codec::MAX_ROWS || count > input.remaining() / 12 {
        return Err(refused());
    }
    let mut rows: [Option<codec::Row<'a>>; codec::MAX_ROWS] = std::array::from_fn(|_| None);
    let mut active = 0;
    for index in 0..count {
        let mut frame = Decoder::new(input.get_bytes().map_err(|_| refused())?);
        let original = frame.get_bytes().map_err(|_| refused())?;
        let execution = frame.get_bytes().map_err(|_| refused())?;
        frame.finish().map_err(|_| refused())?;
        let mut row = codec::parse_original_row(original, domain)?;
        let parsed = execution::parse(
            execution,
            &source_identity(&row.metadata),
            row.metadata.revision,
        )?;
        // Original immutable/cipher frames stay unchanged; mutable execution DATA is separate
        // from the seal-once original master binding, and cannot mint its own task or ticket.
        row.metadata.execution = Some(parsed.state);
        row.execution_input = parsed.input;
        if row.metadata.scheduling == codec::Scheduling::Active {
            active += 1;
        }
        if active > 1
            || rows[..index].iter().flatten().any(|prior| {
                prior.metadata.source >= row.metadata.source
                    || (prior.metadata.group == row.metadata.group
                        && (prior.metadata.signer == row.metadata.signer
                            || prior.metadata.invite_nonce == row.metadata.invite_nonce))
            })
        {
            return Err(refused());
        }
        rows[index] = Some(row);
    }
    input.finish().map_err(|_| refused())?;
    Ok((rows, count))
}

/// Nested source+execution length, excluding its outer u32. Actual input ciphertext remains
/// borrowed until copied directly into the prepaid whole-catalog encoder.
pub(in crate::store::critical_namespaces) fn encoded_row_len(
    metadata: &codec::Metadata,
    immutable: &[u8],
    encrypted: &[u8],
    input: Option<&execution::Input<'_>>,
) -> Result<usize, AppError> {
    let state = metadata.execution.as_ref().ok_or_else(refused)?;
    let original = codec::original_row_len(immutable, encrypted)?;
    let operational =
        execution::encoded_len(state, input, &source_identity(metadata), metadata.revision)?;
    8usize
        .checked_add(original)
        .and_then(|length| length.checked_add(operational))
        .ok_or_else(refused)
}

pub(in crate::store::critical_namespaces) fn encode_row(
    output: &mut Encoder,
    metadata: &codec::Metadata,
    immutable: &[u8],
    encrypted: &[u8],
    input: Option<&execution::Input<'_>>,
) -> Result<(), AppError> {
    let length = encoded_row_len(metadata, immutable, encrypted, input)?;
    if length > codec::MAX_JOIN_PAYLOAD {
        return Err(refused());
    }
    let state = metadata.execution.as_ref().ok_or_else(refused)?;
    let operational =
        execution::encoded_len(state, input, &source_identity(metadata), metadata.revision)?;
    output.put_u32(length as u32);
    codec::encode_original_row(output, metadata, immutable, encrypted)?;
    output.put_u32(operational as u32);
    execution::encode_into(
        state,
        input,
        &source_identity(metadata),
        metadata.revision,
        output,
    )
}

/// Sole multi-row transition: never-network-exposed sources gain only empty execution DATA.
/// The actual mounted/whole-parent consumer must authorize this; parsing supplies no ticket.
pub(in crate::store::critical_namespaces) fn migration(
    old: &codec::Summary,
    next: &codec::Summary,
) -> Result<(), AppError> {
    if old.join_version != Some(codec::JOIN_VERSION)
        || next.join_version != Some(VERSION)
        || old.count == 0
        || next.count != old.count
        || old.rows.iter().flatten().count() != old.count
        || next.rows.iter().flatten().count() != next.count
    {
        return Err(refused());
    }
    for original in old.rows.iter().flatten() {
        if original.execution.is_some() {
            return Err(refused());
        }
        let actual = next
            .rows
            .iter()
            .flatten()
            .find(|row| row.source == original.source)
            .ok_or_else(refused)?;
        let mut expected = original.clone();
        expected.execution = Some(execution::State::not_granted());
        if actual != &expected {
            return Err(refused());
        }
    }
    Ok(())
}

/// Payload DATA transition only: shared outer code must check complete original parent and
/// archive preservation before calling. Counters never mint source/admission/resource custody.
pub(in crate::store::critical_namespaces) fn successor(
    old: &codec::Summary,
    next: &codec::Summary,
) -> Result<(), AppError> {
    if old.join_version != Some(VERSION)
        || next.join_version != Some(VERSION)
        || old.count == 0
        || next.count < old.count
        || next.count > old.count + 1
        || old.rows.iter().flatten().count() != old.count
        || next.rows.iter().flatten().count() != next.count
    {
        return Err(refused());
    }
    let mut changes = 0;
    for original in old.rows.iter().flatten() {
        let actual = next
            .rows
            .iter()
            .flatten()
            .find(|row| row.source == original.source)
            .ok_or_else(refused)?;
        let mut immutable = original.clone();
        immutable.revision = actual.revision;
        immutable.scheduling = actual.scheduling;
        immutable.execution = actual.execution;
        if &immutable != actual {
            return Err(refused());
        }
        let original_state = original.execution.as_ref().ok_or_else(refused)?;
        let actual_state = actual.execution.as_ref().ok_or_else(refused)?;
        if original_state == actual_state {
            if original.revision == actual.revision && original.scheduling == actual.scheduling {
                continue;
            }
            if original.revision.checked_add(1) != Some(actual.revision)
                || original.scheduling == actual.scheduling
            {
                return Err(refused());
            }
        } else {
            if original.scheduling != actual.scheduling {
                return Err(refused());
            }
            execution::successor(
                original_state,
                actual_state,
                &source_identity(original),
                original.revision,
                actual.revision,
            )?;
        }
        changes += 1;
    }
    if next.count == old.count {
        if changes != 1 {
            return Err(refused());
        }
    } else {
        if changes != 0 {
            return Err(refused());
        }
        let added = next
            .rows
            .iter()
            .flatten()
            .find(|row| {
                !old.rows
                    .iter()
                    .flatten()
                    .any(|prior| prior.source == row.source)
            })
            .ok_or_else(refused)?;
        if added.revision != 1
            || added.scheduling != codec::Scheduling::Active
            || added.execution != Some(execution::State::not_granted())
        {
            return Err(refused());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
