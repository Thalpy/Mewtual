//! Bounded, closed index3 byte grammar. Callers authenticate and fund the
//! complete selected/candidate/AEAD/row overlap before using this DATA codec.
//! It selects no files and grants no native, membership, SDK or write authority.
use super::super::{capability::field32, data as original};
use super::capability::{Capability, INDEX_FORMAT};
use super::data::*;
use super::{refused, AppError};
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

const TAG: &[u8] = b"catcoms/private-native-backend-index/v3\0";
const MAX_FOUNDER_BINDING: usize = 6 * 36 + 4 + 256;
const MAX_JOINED_BINDING: usize = 10 * 36 + 4 + 256 + 8;
// A minimum-size old2 PendingFounder may still need its full binding and
// terminal. This468B growth exceeds old Legacy/active retirement's36B. Reserve
// it for EVERY predecessor row, plus v3 metadata/migration and first-parent
// growth: a valid full old2 table must never lose its ability to finish/Leave.
const MAX_V2_ROW_COMPLETION_GROWTH: usize = MAX_FOUNDER_BINDING + 36 - (4 + 40);
pub(super) const MAX_PLAIN: usize = original::MAX_PLAIN
    + 45
    + 9 * original::MAX_ROWS
    + 36
    + MAX_V2_ROW_COMPLETION_GROWTH * original::MAX_ROWS;
pub(super) const MAX_FRAME: usize = MAX_PLAIN + 40;
const HEADER_BYTES: usize = 4 + TAG.len() + 2 + 3 * 36 + 45 + 8 + 1 + 8 + 4;
const MAX_NETWORK_FRAME: usize = 64 * 1024;

fn bounded_string<'a>(input: &mut Decoder<'a>, maximum: usize) -> Result<&'a str, AppError> {
    let text = input.get_str().map_err(|_| refused())?;
    if text.len() > maximum {
        return Err(refused());
    }
    Ok(text)
}
fn group(input: &mut Decoder<'_>) -> Result<Vec<u8>, AppError> {
    let value = input.get_bytes().map_err(|_| refused())?;
    if value.is_empty() || value.len() > 256 {
        return Err(refused());
    }
    Ok(value.to_vec())
}
fn read_birth(input: &mut Decoder<'_>) -> Result<Birth, AppError> {
    let kind = match input.get_u8().map_err(|_| refused())? {
        1 => BirthKind::Founder,
        2 => BirthKind::Joined,
        3 => BirthKind::LegacyJoin,
        4 => BirthKind::NativeFounder,
        _ => return Err(refused()),
    };
    Ok(Birth {
        kind,
        invocation: field32(input)?,
        request: field32(input)?,
        signer: field32(input)?,
        network: field32(input)?,
        join_context: if kind.is_founder() {
            None
        } else {
            Some(field32(input)?)
        },
    })
}
fn put_birth(output: &mut Encoder, value: &Birth) -> Result<(), AppError> {
    output.put_u8(match value.kind {
        BirthKind::Founder => 1,
        BirthKind::Joined => 2,
        BirthKind::LegacyJoin => 3,
        BirthKind::NativeFounder => 4,
    });
    for field in [value.invocation, value.request, value.signer, value.network] {
        output.put_bytes(&field).map_err(|_| refused())?;
    }
    if let Some(context) = value.join_context {
        output.put_bytes(&context).map_err(|_| refused())?;
    }
    Ok(())
}
fn birth_length(value: &Birth) -> usize {
    1 + 4 * 36 + if value.join_context.is_some() { 36 } else { 0 }
}

#[cfg(test)]
mod native_birth_tests {
    use super::*;
    #[test]
    fn native_birth_discriminant_preserves_all_existing_encodings_and_uses_four() {
        for (kind, tag) in [
            (BirthKind::Founder, 1),
            (BirthKind::Joined, 2),
            (BirthKind::LegacyJoin, 3),
            (BirthKind::NativeFounder, 4),
        ] {
            let birth = Birth {
                kind,
                invocation: [1; 32],
                request: [2; 32],
                signer: [3; 32],
                network: [4; 32],
                join_context: if kind.is_founder() {
                    None
                } else {
                    Some([5; 32])
                },
            };
            let mut output = Encoder::with_capacity(256);
            put_birth(&mut output, &birth).unwrap();
            let raw = output.finish();
            assert_eq!(raw[0], tag);
            assert_eq!(raw.len(), if kind.is_founder() { 145 } else { 181 });
            let mut input = Decoder::new(&raw);
            assert!(read_birth(&mut input).unwrap() == birth);
            input.finish().unwrap();
        }
    }
}
fn read_legacy_origin(input: &mut Decoder<'_>) -> Result<LegacyOrigin, AppError> {
    match input.get_u8().map_err(|_| refused())? {
        0 => Ok(LegacyOrigin::Imported),
        1 => Ok(LegacyOrigin::Admitted(read_birth(input)?)),
        _ => Err(refused()),
    }
}
fn put_legacy_origin(output: &mut Encoder, value: &LegacyOrigin) -> Result<(), AppError> {
    match value {
        LegacyOrigin::Imported => {
            output.put_u8(0);
        }
        LegacyOrigin::Admitted(birth) => {
            output.put_u8(1);
            put_birth(output, birth)?;
        }
    }
    Ok(())
}
fn read_founder(input: &mut Decoder<'_>) -> Result<original::FounderBindingData, AppError> {
    Ok(original::FounderBindingData {
        vault: field32(input)?,
        layout: field32(input)?,
        group: group(input)?,
        signer: field32(input)?,
        source: field32(input)?,
        profile: field32(input)?,
        pin: field32(input)?,
    })
}
fn put_founder(output: &mut Encoder, value: &original::FounderBindingData) -> Result<(), AppError> {
    output.put_bytes(&value.vault).map_err(|_| refused())?;
    output.put_bytes(&value.layout).map_err(|_| refused())?;
    output.put_bytes(&value.group).map_err(|_| refused())?;
    for field in [value.signer, value.source, value.profile, value.pin] {
        output.put_bytes(&field).map_err(|_| refused())?;
    }
    Ok(())
}
fn read_joined(input: &mut Decoder<'_>) -> Result<JoinedBinding, AppError> {
    Ok(JoinedBinding {
        vault: field32(input)?,
        layout: field32(input)?,
        group: group(input)?,
        signer: field32(input)?,
        source: field32(input)?,
        profile: field32(input)?,
        pin: field32(input)?,
        join_context: field32(input)?,
        own_add: field32(input)?,
        initial_governing_tip: field32(input)?,
        initial_current_basis: field32(input)?,
        joined_epoch: input.get_u64().map_err(|_| refused())?,
    })
}
fn put_joined(output: &mut Encoder, value: &JoinedBinding) -> Result<(), AppError> {
    output.put_bytes(&value.vault).map_err(|_| refused())?;
    output.put_bytes(&value.layout).map_err(|_| refused())?;
    output.put_bytes(&value.group).map_err(|_| refused())?;
    for field in [
        value.signer,
        value.source,
        value.profile,
        value.pin,
        value.join_context,
        value.own_add,
        value.initial_governing_tip,
        value.initial_current_basis,
    ] {
        output.put_bytes(&field).map_err(|_| refused())?;
    }
    output.put_u64(value.joined_epoch);
    Ok(())
}

impl Index {
    pub(super) fn encoded_length(&self) -> Result<usize, AppError> {
        self.lengths_data().map(|(encoded, _)| encoded)
    }

    /// Required final footprint, not just today's bytes. Birth/metadata
    /// admission must preserve this capacity before any irreversible work.
    /// Ordinary IO/disk failure is separate; this prevents format exhaustion
    /// from stranding a previously admitted activation or retirement.
    pub(super) fn reserved_length_data(&self) -> Result<usize, AppError> {
        self.lengths_data().map(|(_, reserved)| reserved)
    }

    fn lengths_data(&self) -> Result<(usize, usize), AppError> {
        self.validate_data()?;
        let mut length = HEADER_BYTES + self.parent.map_or(0, |_| 36);
        // G1 has no parent yet; its first successor necessarily has one.
        let mut reserved = HEADER_BYTES + 36;
        for row in &self.rows {
            let payload = match &row.backend {
                Backend::Legacy { origin, invite }
                | Backend::RetiredLegacy { origin, invite, .. } => {
                    1 + match origin {
                        LegacyOrigin::Imported => 0,
                        LegacyOrigin::Admitted(birth) => birth_length(birth),
                    } + 4
                        + invite.len()
                        + if matches!(row.backend, Backend::RetiredLegacy { .. }) {
                            36
                        } else {
                            0
                        }
                }
                Backend::Pending {
                    birth,
                    network_frame,
                } => birth_length(birth) + 4 + network_frame.len(),
                Backend::Founder { birth, binding }
                | Backend::RetiredFounder { birth, binding, .. } => {
                    birth_length(birth)
                        + 6 * 36
                        + 4
                        + binding.group.len()
                        + if matches!(row.backend, Backend::RetiredFounder { .. }) {
                            36
                        } else {
                            0
                        }
                }
                Backend::Joined { birth, binding }
                | Backend::RetiredJoined { birth, binding, .. } => {
                    birth_length(birth)
                        + 10 * 36
                        + 4
                        + binding.group.len()
                        + 8
                        + if matches!(row.backend, Backend::RetiredJoined { .. }) {
                            36
                        } else {
                            0
                        }
                }
                Backend::AbandonedFounder { birth, .. } => birth_length(birth) + 1 + 36,
            };
            let final_payload = match &row.backend {
                Backend::Pending { birth, .. } => {
                    let final_state = match birth.kind {
                        BirthKind::Founder | BirthKind::NativeFounder => {
                            birth_length(birth) + MAX_FOUNDER_BINDING + 36
                        }
                        BirthKind::Joined => birth_length(birth) + MAX_JOINED_BINDING + 36,
                        BirthKind::LegacyJoin => {
                            1 + birth_length(birth) + 4 + original::MAX_INVITE + 36
                        }
                    };
                    payload.max(final_state)
                }
                Backend::Legacy { .. } | Backend::Founder { .. } | Backend::Joined { .. } => {
                    payload + 36
                }
                _ => payload,
            };
            let row_bytes = 1 + 8 + 36 + 1 + 4 + row.name.len() + 8;
            length = length
                .checked_add(row_bytes)
                .and_then(|bytes| bytes.checked_add(payload))
                .filter(|bytes| *bytes <= MAX_PLAIN)
                .ok_or_else(refused)?;
            reserved = reserved
                .checked_add(row_bytes)
                .and_then(|bytes| bytes.checked_add(final_payload))
                .filter(|bytes| *bytes <= MAX_PLAIN)
                .ok_or_else(refused)?;
        }
        Ok((length, reserved))
    }

    pub(super) fn encode_data(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let length = self.encoded_length()?;
        let mut output = Encoder::with_capacity(length);
        // Keep the encoder outside the fallible body and always take its bytes
        // into the zeroizing holder before propagating ordinary encode errors.
        let result = (|| {
            output.put_bytes(TAG).map_err(|_| refused())?;
            output.put_u16(INDEX_FORMAT);
            for field in [
                self.capability.original.vault,
                self.capability.original.root,
                self.capability.id_data()?,
            ] {
                output.put_bytes(&field).map_err(|_| refused())?;
            }
            output.put_u8(2).put_u64(self.origin.generation);
            output
                .put_bytes(&self.origin.frame)
                .map_err(|_| refused())?;
            output
                .put_u64(self.generation)
                .put_u8(u8::from(self.parent.is_some()));
            if let Some(parent) = self.parent {
                output.put_bytes(&parent).map_err(|_| refused())?;
            }
            output
                .put_u64(self.last_issued_id)
                .put_u32(self.rows.len() as u32);
            for row in &self.rows {
                let tag = match row.backend {
                    Backend::Legacy { .. } => 0,
                    Backend::RetiredLegacy { .. } => 1,
                    Backend::Pending { .. } => 2,
                    Backend::Founder { .. } => 3,
                    Backend::RetiredFounder { .. } => 4,
                    Backend::Joined { .. } => 5,
                    Backend::RetiredJoined { .. } => 6,
                    Backend::AbandonedFounder { .. } => 7,
                };
                output.put_u8(tag).put_u64(row.id);
                output.put_bytes(&row.identity).map_err(|_| refused())?;
                output.put_u8(u8::from(row.is_dm));
                output.put_str(&row.name).map_err(|_| refused())?;
                output.put_u64(row.metadata_revision);
                match &row.backend {
                    Backend::Legacy { origin, invite }
                    | Backend::RetiredLegacy { origin, invite, .. } => {
                        put_legacy_origin(&mut output, origin)?;
                        output.put_str(invite).map_err(|_| refused())?;
                        if let Backend::RetiredLegacy { terminal, .. } = &row.backend {
                            output.put_bytes(terminal).map_err(|_| refused())?;
                        }
                    }
                    Backend::Pending {
                        birth,
                        network_frame,
                    } => {
                        put_birth(&mut output, birth)?;
                        output.put_bytes(network_frame).map_err(|_| refused())?;
                    }
                    Backend::Founder { birth, binding }
                    | Backend::RetiredFounder { birth, binding, .. } => {
                        put_birth(&mut output, birth)?;
                        put_founder(&mut output, binding)?;
                        if let Backend::RetiredFounder { terminal, .. } = &row.backend {
                            output.put_bytes(terminal).map_err(|_| refused())?;
                        }
                    }
                    Backend::Joined { birth, binding }
                    | Backend::RetiredJoined { birth, binding, .. } => {
                        put_birth(&mut output, birth)?;
                        put_joined(&mut output, binding)?;
                        if let Backend::RetiredJoined { terminal, .. } = &row.backend {
                            output.put_bytes(terminal).map_err(|_| refused())?;
                        }
                    }
                    Backend::AbandonedFounder {
                        birth,
                        reason,
                        terminal,
                    } => {
                        put_birth(&mut output, birth)?;
                        output.put_u8(match reason {
                            original::AbandonedCreateReasonData::BeforeSdk => 0,
                            original::AbandonedCreateReasonData::UnexposedAfterSdk => 1,
                        });
                        output.put_bytes(terminal).map_err(|_| refused())?;
                    }
                }
            }
            Ok::<(), AppError>(())
        })();
        let bytes = Zeroizing::new(output.finish());
        result?;
        if bytes.len() != length {
            return Err(refused());
        }
        Ok(bytes)
    }

    pub(super) fn decode_data(raw: &[u8], expected: Capability) -> Result<Self, AppError> {
        if raw.len() > MAX_PLAIN {
            return Err(refused());
        }
        let mut input = Decoder::new(raw);
        if input.get_bytes().map_err(|_| refused())? != TAG
            || input.get_u16().map_err(|_| refused())? != INDEX_FORMAT
            || field32(&mut input)? != expected.original.vault
            || field32(&mut input)? != expected.original.root
            || field32(&mut input)? != expected.id_data()?
            || input.get_u8().map_err(|_| refused())? != 2
        {
            return Err(refused());
        }
        let origin = MigrationOrigin {
            generation: input.get_u64().map_err(|_| refused())?,
            frame: field32(&mut input)?,
        };
        let generation = input.get_u64().map_err(|_| refused())?;
        let parent = match input.get_u8().map_err(|_| refused())? {
            0 => None,
            1 => Some(field32(&mut input)?),
            _ => return Err(refused()),
        };
        let last_issued_id = input.get_u64().map_err(|_| refused())?;
        let count = input.get_u32().map_err(|_| refused())? as usize;
        if count > original::MAX_ROWS {
            return Err(refused());
        }
        let mut rows = Vec::with_capacity(count);
        for _ in 0..count {
            let tag = input.get_u8().map_err(|_| refused())?;
            let id = input.get_u64().map_err(|_| refused())?;
            let identity = field32(&mut input)?;
            let is_dm = match input.get_u8().map_err(|_| refused())? {
                0 => false,
                1 => true,
                _ => return Err(refused()),
            };
            let name = bounded_string(&mut input, original::MAX_NAME)?.to_owned();
            let metadata_revision = input.get_u64().map_err(|_| refused())?;
            let backend = match tag {
                0 | 1 => {
                    let origin = read_legacy_origin(&mut input)?;
                    let invite = bounded_string(&mut input, original::MAX_INVITE)?.to_owned();
                    if tag == 0 {
                        Backend::Legacy { origin, invite }
                    } else {
                        Backend::RetiredLegacy {
                            origin,
                            invite,
                            terminal: field32(&mut input)?,
                        }
                    }
                }
                2 => {
                    let birth = read_birth(&mut input)?;
                    let frame = input.get_bytes().map_err(|_| refused())?;
                    if !(40..=MAX_NETWORK_FRAME).contains(&frame.len()) {
                        return Err(refused());
                    }
                    Backend::Pending {
                        birth,
                        network_frame: Zeroizing::new(frame.to_vec()),
                    }
                }
                3 | 4 => {
                    let birth = read_birth(&mut input)?;
                    let binding = read_founder(&mut input)?;
                    if tag == 3 {
                        Backend::Founder { birth, binding }
                    } else {
                        Backend::RetiredFounder {
                            birth,
                            binding,
                            terminal: field32(&mut input)?,
                        }
                    }
                }
                5 | 6 => {
                    let birth = read_birth(&mut input)?;
                    let binding = read_joined(&mut input)?;
                    if tag == 5 {
                        Backend::Joined { birth, binding }
                    } else {
                        Backend::RetiredJoined {
                            birth,
                            binding,
                            terminal: field32(&mut input)?,
                        }
                    }
                }
                7 => {
                    let birth = read_birth(&mut input)?;
                    let reason = match input.get_u8().map_err(|_| refused())? {
                        0 => original::AbandonedCreateReasonData::BeforeSdk,
                        1 => original::AbandonedCreateReasonData::UnexposedAfterSdk,
                        _ => return Err(refused()),
                    };
                    Backend::AbandonedFounder {
                        birth,
                        reason,
                        terminal: field32(&mut input)?,
                    }
                }
                _ => return Err(refused()),
            };
            rows.push(Row {
                id,
                identity,
                is_dm,
                name,
                metadata_revision,
                backend,
            });
        }
        input.finish().map_err(|_| refused())?;
        let value = Self {
            capability: expected,
            origin,
            generation,
            parent,
            last_issued_id,
            rows,
        };
        if value.encoded_length()? != raw.len() {
            return Err(refused());
        }
        Ok(value)
    }
}
