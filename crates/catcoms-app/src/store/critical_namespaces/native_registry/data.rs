//! Bounded native-index DATA. No decoded row grants a runtime or legacy fallback.
//! The physical handler owns authentication, selection, payment and all IO.
use super::super::refused;
use super::capability::{field32, NativeCapabilityData, INDEX_FORMAT};
use crate::store::ServerRecord;
use crate::AppError;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

const TAG: &[u8] = b"catcoms/private-native-backend-index/v1\0";
pub(super) const GENERATION1_HEADER_BYTES: usize = 4 + TAG.len() + 2 + 3 * 36 + 8 + 1 + 4;
pub(super) const MAX_ROWS: usize = 1024;
pub(super) const MAX_PLAIN: usize = 4 * 1024 * 1024;
pub(super) const MAX_FRAME: usize = MAX_PLAIN + 40;
pub(super) const MAX_NAME: usize = 256;
pub(super) const MAX_INVITE: usize = 64 * 1024;
const MAX_GROUP: usize = 256;

/// A complete founder binding is historical DATA only. The actual whole Host
/// and native row/session still establish authority before installing a runtime.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct FounderBindingData {
    pub(super) vault: [u8; 32],
    pub(super) layout: [u8; 32],
    pub(super) group: Vec<u8>,
    pub(super) signer: [u8; 32],
    pub(super) source: [u8; 32],
    pub(super) profile: [u8; 32],
    pub(super) pin: [u8; 32],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CreationBackendData {
    Legacy,
    FounderV2,
}

/// Pending input labels deliberately contain no guessed group, source or pin.
/// Its network association names the exact reserved native network record.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct PendingCreateData {
    pub(super) backend: CreationBackendData,
    pub(super) invocation: [u8; 32],
    pub(super) signer: [u8; 32],
    pub(super) network: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AbandonedCreateReasonData {
    /// Actual creation-journal evidence proves that the SDK ticket was unspent.
    BeforeSdk,
    /// Actual journal/source evidence proves spent but unexposed, without a
    /// possibly accepted source. The native recovery transaction must establish it.
    UnexposedAfterSdk,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum BackendData {
    Legacy {
        invite: String,
    },
    Pending(PendingCreateData),
    Founder(FounderBindingData),
    RetiredFounder {
        binding: FounderBindingData,
        scope_terminal: [u8; 32],
    },
    AbandonedCreate {
        pending: PendingCreateData,
        reason: AbandonedCreateReasonData,
        journal_terminal: [u8; 32],
    },
}
#[derive(Clone, PartialEq, Eq)]
pub(super) struct RowData {
    pub(super) id: u64,
    pub(super) identity: [u8; 32],
    pub(super) name: String,
    pub(super) is_dm: bool,
    pub(super) backend: BackendData,
}

/// One whole selected snapshot. The parent is the preceding FRAMED ciphertext
/// identity. Neither a decoded generation nor this DATA type performs a CAS.
#[derive(PartialEq, Eq)]
pub(super) struct IndexData {
    pub(super) capability: NativeCapabilityData,
    pub(super) generation: u64,
    pub(super) parent: Option<[u8; 32]>,
    pub(super) rows: Vec<RowData>,
}
impl std::fmt::Debug for IndexData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeIndexData")
            .field("generation", &self.generation)
            .field("rows", &self.rows.len())
            .finish_non_exhaustive()
    }
}

fn bounded_str<'a>(input: &mut Decoder<'a>, maximum: usize) -> Result<&'a str, AppError> {
    let value = input.get_str().map_err(|_| refused())?;
    if value.len() > maximum {
        return Err(refused());
    }
    Ok(value)
}
fn boolean(input: &mut Decoder<'_>) -> Result<bool, AppError> {
    match input.get_u8().map_err(|_| refused())? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(refused()),
    }
}
fn decode_pending(input: &mut Decoder<'_>) -> Result<PendingCreateData, AppError> {
    let backend = match input.get_u8().map_err(|_| refused())? {
        0 => CreationBackendData::Legacy,
        1 => CreationBackendData::FounderV2,
        _ => return Err(refused()),
    };
    Ok(PendingCreateData {
        backend,
        invocation: field32(input)?,
        signer: field32(input)?,
        network: field32(input)?,
    })
}
fn decode_binding(
    input: &mut Decoder<'_>,
    capability: NativeCapabilityData,
) -> Result<FounderBindingData, AppError> {
    let vault = field32(input)?;
    let layout = field32(input)?;
    let group = input.get_bytes().map_err(|_| refused())?;
    if vault != capability.vault
        || layout != capability.root
        || group.is_empty()
        || group.len() > MAX_GROUP
    {
        return Err(refused());
    }
    // Validate the variable group length BEFORE copying; fixed identities are
    // parsed before constructing the returned owned value.
    let signer = field32(input)?;
    let source = field32(input)?;
    let profile = field32(input)?;
    let pin = field32(input)?;
    Ok(FounderBindingData {
        vault,
        layout,
        group: group.to_vec(),
        signer,
        source,
        profile,
        pin,
    })
}

impl IndexData {
    /// Caller prepays raw/unframed/plaintext, cloned strings/groups, bounded
    /// metadata and uniqueness scratch before invoking this DATA decoder.
    pub(super) fn decode_data(
        raw: &[u8],
        expected: NativeCapabilityData,
    ) -> Result<Self, AppError> {
        if raw.len() > MAX_PLAIN {
            return Err(refused());
        }
        let mut input = Decoder::new(raw);
        if input.get_bytes().map_err(|_| refused())? != TAG
            || input.get_u16().map_err(|_| refused())? != INDEX_FORMAT
            || field32(&mut input)? != expected.vault
            || field32(&mut input)? != expected.root
            || field32(&mut input)? != expected.id_data()?
        {
            return Err(refused());
        }
        let generation = input.get_u64().map_err(|_| refused())?;
        let parent = match input.get_u8().map_err(|_| refused())? {
            0 => None,
            1 => Some(field32(&mut input)?),
            _ => return Err(refused()),
        };
        if generation == 0 || (generation == 1) != parent.is_none() {
            return Err(refused());
        }
        let count = input.get_u32().map_err(|_| refused())? as usize;
        if count > MAX_ROWS {
            return Err(refused());
        }
        let mut rows = Vec::with_capacity(count);
        let mut identities = Vec::with_capacity(count);
        let mut previous_id = None;
        for _ in 0..count {
            let tag = input.get_u8().map_err(|_| refused())?;
            let id = input.get_u64().map_err(|_| refused())?;
            if previous_id.is_some_and(|previous| previous >= id) {
                return Err(refused());
            }
            previous_id = Some(id);
            let identity = field32(&mut input)?;
            let name = bounded_str(&mut input, MAX_NAME)?.to_owned();
            let is_dm = boolean(&mut input)?;
            let backend = match tag {
                0 => BackendData::Legacy {
                    invite: bounded_str(&mut input, MAX_INVITE)?.to_owned(),
                },
                1 => BackendData::Pending(decode_pending(&mut input)?),
                2 => BackendData::Founder(decode_binding(&mut input, expected)?),
                3 => BackendData::RetiredFounder {
                    binding: decode_binding(&mut input, expected)?,
                    scope_terminal: field32(&mut input)?,
                },
                4 => {
                    let pending = decode_pending(&mut input)?;
                    let reason = match input.get_u8().map_err(|_| refused())? {
                        0 => AbandonedCreateReasonData::BeforeSdk,
                        1 => AbandonedCreateReasonData::UnexposedAfterSdk,
                        _ => return Err(refused()),
                    };
                    BackendData::AbandonedCreate {
                        pending,
                        reason,
                        journal_terminal: field32(&mut input)?,
                    }
                }
                _ => return Err(refused()),
            };
            rows.push(RowData {
                id,
                identity,
                name,
                is_dm,
                backend,
            });
            identities.push(identity);
        }
        input.finish().map_err(|_| refused())?;
        // Canonical numeric order catches duplicate IDs without a growing map.
        // Separate stable identities must also be unique across distinct IDs.
        identities.sort_unstable();
        if identities.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(refused());
        }
        Ok(Self {
            capability: expected,
            generation,
            parent,
            rows,
        })
    }

    /// Compatibility DATA for the first declaration only. No writer/SDK/runtime
    /// follows. Original bytes/identity and writer exclusion belong to the opener.
    pub(super) fn migrate_legacy_data(
        capability: NativeCapabilityData,
        original_frame: Option<[u8; 32]>,
        records: &[ServerRecord],
    ) -> Result<Self, AppError> {
        if records.len() > MAX_ROWS {
            return Err(refused());
        }
        // Pay/allocate only bounded numeric scratch until the COMPLETE output
        // geometry and duplicate IDs have passed. Four MiB of invite payload
        // alone is already too much once the row/header framing is counted.
        let mut ids = Vec::with_capacity(records.len());
        let mut length = GENERATION1_HEADER_BYTES;
        for record in records {
            if record.display_name.len() > MAX_NAME || record.invite.len() > MAX_INVITE {
                return Err(refused());
            }
            length = length
                .checked_add(1 + 8 + 36 + 4 + record.display_name.len() + 1 + 4)
                .and_then(|n| n.checked_add(record.invite.len()))
                .filter(|n| *n <= MAX_PLAIN)
                .ok_or_else(refused)?;
            ids.push(record.id);
        }
        ids.sort_unstable();
        if ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(refused());
        }
        drop(ids);
        let mut rows = Vec::with_capacity(records.len());
        let context = capability.id_data()?;
        for record in records {
            let mut hash = blake3::Hasher::new();
            hash.update(b"catcoms/native-legacy-row-id/v1\0");
            hash.update(&context);
            match original_frame {
                None => {
                    hash.update(&[0]);
                }
                Some(frame) => {
                    hash.update(&[1]);
                    hash.update(&frame);
                }
            }
            hash.update(&record.id.to_be_bytes());
            rows.push(RowData {
                id: record.id,
                identity: *hash.finalize().as_bytes(),
                name: record.display_name.clone(),
                is_dm: record.is_dm,
                backend: BackendData::Legacy {
                    invite: record.invite.clone(),
                },
            });
        }
        rows.sort_unstable_by_key(|row| row.id);
        let result = Self {
            capability,
            generation: 1,
            parent: None,
            rows,
        };
        // The encoder's complete preflight rejects duplicate IDs and aggregate
        // framing overflow as well as malformed individual fields.
        result.encoded_length()?;
        Ok(result)
    }

    fn encoded_length(&self) -> Result<usize, AppError> {
        if self.rows.len() > MAX_ROWS
            || self.generation == 0
            || (self.generation == 1) != self.parent.is_none()
        {
            return Err(refused());
        }
        let mut length = GENERATION1_HEADER_BYTES + self.parent.map_or(0, |_| 36);
        let mut previous_id = None;
        let mut identities = Vec::with_capacity(self.rows.len());
        for row in &self.rows {
            if previous_id.is_some_and(|previous| previous >= row.id) || row.name.len() > MAX_NAME {
                return Err(refused());
            }
            previous_id = Some(row.id);
            identities.push(row.identity);
            let payload = match &row.backend {
                BackendData::Legacy { invite } if invite.len() <= MAX_INVITE => 4 + invite.len(),
                BackendData::Legacy { .. } => return Err(refused()),
                BackendData::Pending(_) => 1 + 3 * 36,
                BackendData::AbandonedCreate { .. } => 1 + 3 * 36 + 1 + 36,
                BackendData::Founder(binding) | BackendData::RetiredFounder { binding, .. } => {
                    if binding.vault != self.capability.vault
                        || binding.layout != self.capability.root
                        || binding.group.is_empty()
                        || binding.group.len() > MAX_GROUP
                    {
                        return Err(refused());
                    }
                    6 * 36
                        + 4
                        + binding.group.len()
                        + if matches!(row.backend, BackendData::RetiredFounder { .. }) {
                            36
                        } else {
                            0
                        }
                }
            };
            length = length
                .checked_add(1 + 8 + 36 + 4 + row.name.len() + 1)
                .and_then(|n| n.checked_add(payload))
                .filter(|n| *n <= MAX_PLAIN)
                .ok_or_else(refused)?;
        }
        identities.sort_unstable();
        if identities.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(refused());
        }
        Ok(length)
    }

    pub(super) fn encode_data(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let length = self.encoded_length()?;
        let mut output = Encoder::with_capacity(length);
        output.put_bytes(TAG).map_err(|_| refused())?;
        output.put_u16(INDEX_FORMAT);
        for field in [
            self.capability.vault,
            self.capability.root,
            self.capability.id_data()?,
        ] {
            output.put_bytes(&field).map_err(|_| refused())?;
        }
        output
            .put_u64(self.generation)
            .put_u8(u8::from(self.parent.is_some()));
        if let Some(parent) = self.parent {
            output.put_bytes(&parent).map_err(|_| refused())?;
        }
        output.put_u32(self.rows.len() as u32);
        for row in &self.rows {
            let tag = match row.backend {
                BackendData::Legacy { .. } => 0,
                BackendData::Pending(_) => 1,
                BackendData::Founder(_) => 2,
                BackendData::RetiredFounder { .. } => 3,
                BackendData::AbandonedCreate { .. } => 4,
            };
            output.put_u8(tag).put_u64(row.id);
            output.put_bytes(&row.identity).map_err(|_| refused())?;
            output.put_str(&row.name).map_err(|_| refused())?;
            output.put_u8(u8::from(row.is_dm));
            match &row.backend {
                BackendData::Legacy { invite } => {
                    output.put_str(invite).map_err(|_| refused())?;
                }
                BackendData::Pending(pending) => encode_pending(&mut output, pending)?,
                BackendData::Founder(binding) => encode_binding(&mut output, binding)?,
                BackendData::RetiredFounder {
                    binding,
                    scope_terminal,
                } => {
                    encode_binding(&mut output, binding)?;
                    output.put_bytes(scope_terminal).map_err(|_| refused())?;
                }
                BackendData::AbandonedCreate {
                    pending,
                    reason,
                    journal_terminal,
                } => {
                    encode_pending(&mut output, pending)?;
                    output.put_u8(match reason {
                        AbandonedCreateReasonData::BeforeSdk => 0,
                        AbandonedCreateReasonData::UnexposedAfterSdk => 1,
                    });
                    output.put_bytes(journal_terminal).map_err(|_| refused())?;
                }
            }
        }
        if output.len() != length {
            return Err(refused());
        }
        Ok(Zeroizing::new(output.finish()))
    }
}
fn encode_pending(output: &mut Encoder, pending: &PendingCreateData) -> Result<(), AppError> {
    output.put_u8(match pending.backend {
        CreationBackendData::Legacy => 0,
        CreationBackendData::FounderV2 => 1,
    });
    for field in [pending.invocation, pending.signer, pending.network] {
        output.put_bytes(&field).map_err(|_| refused())?;
    }
    Ok(())
}
fn encode_binding(output: &mut Encoder, binding: &FounderBindingData) -> Result<(), AppError> {
    for field in [binding.vault, binding.layout] {
        output.put_bytes(&field).map_err(|_| refused())?;
    }
    output.put_bytes(&binding.group).map_err(|_| refused())?;
    for field in [binding.signer, binding.source, binding.profile, binding.pin] {
        output.put_bytes(&field).map_err(|_| refused())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
