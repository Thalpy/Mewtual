//! Bounded version2 index and semantic successor checks. The physical serializer
//! must authenticate/hold the selected frame and prove the parent hash itself.
//! These types deliberately expose no generic snapshot publication API.
use super::super::super::refused;
use super::super::{capability::field32, data as old};
use super::capability::{Capability, INDEX_FORMAT};
use crate::AppError;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

const TAG: &[u8] = b"catcoms/private-native-backend-index/v2\0";
pub(super) const MAX_PLAIN: usize = old::MAX_PLAIN;
pub(super) const MAX_FRAME: usize = MAX_PLAIN + 40;
pub(super) const MAX_NETWORK_FRAME: usize = 64 * 1024;
const HEADER_BYTES: usize = 4 + TAG.len() + 2 + 3 * 36 + 8 + 1 + 8 + 4;
const BIRTH_BYTES: usize = 1 + 4 * 36;

/// Immutable original request receipt. The original request digest is computed
/// before generated nonces/identity exist; network commits to the funded actual
/// full public key and algorithm, not ciphertext, routes or mutable port fields.
/// All states, including retired/abandoned, keep this receipt for token lookup.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Birth {
    pub(super) invocation: [u8; 32],
    pub(super) request: [u8; 32],
    pub(super) signer: [u8; 32],
    pub(super) network: [u8; 32],
}

#[derive(PartialEq, Eq)]
pub(super) enum Backend {
    // Existing legacy rows are copied exactly from genuine G1. This version's
    // new-birth operation is deliberately FounderV2-only; it does not invent
    // idempotency receipts for an old legacy invocation it never observed.
    Legacy {
        invite: String,
    },
    Pending {
        birth: Birth,
        network_frame: Zeroizing<Vec<u8>>,
    },
    Founder {
        birth: Birth,
        binding: old::FounderBindingData,
    },
    Retired {
        birth: Birth,
        binding: old::FounderBindingData,
        scope_terminal: [u8; 32],
    },
    Abandoned {
        birth: Birth,
        reason: old::AbandonedCreateReasonData,
        journal_terminal: [u8; 32],
    },
}
impl Backend {
    pub(super) fn birth(&self) -> Option<&Birth> {
        match self {
            Self::Legacy { .. } => None,
            Self::Pending { birth, .. }
            | Self::Founder { birth, .. }
            | Self::Retired { birth, .. }
            | Self::Abandoned { birth, .. } => Some(birth),
        }
    }
}
#[derive(PartialEq, Eq)]
pub(super) struct Row {
    pub(super) id: u64,
    pub(super) identity: [u8; 32],
    pub(super) name: String,
    pub(super) is_dm: bool,
    pub(super) backend: Backend,
}
#[derive(PartialEq, Eq)]
pub(super) struct Index {
    pub(super) capability: Capability,
    pub(super) generation: u64,
    pub(super) parent: Option<[u8; 32]>,
    // Never decrease, even if a later concrete legacy projection removes rows.
    // Native terminal receipts themselves are never recycled or removed.
    pub(super) last_issued_id: u64,
    pub(super) rows: Vec<Row>,
}

fn string<'a>(input: &mut Decoder<'a>, limit: usize) -> Result<&'a str, AppError> {
    let value = input.get_str().map_err(|_| refused())?;
    if value.len() > limit {
        return Err(refused());
    }
    Ok(value)
}
fn birth(input: &mut Decoder<'_>) -> Result<Birth, AppError> {
    // Closed FounderV2 request kind. Future kinds need their own actual owning
    // consumer and explicit grammar; they cannot fall back to a legacy runtime.
    if input.get_u8().map_err(|_| refused())? != 1 {
        return Err(refused());
    }
    Ok(Birth {
        invocation: field32(input)?,
        request: field32(input)?,
        signer: field32(input)?,
        network: field32(input)?,
    })
}
fn put_birth(output: &mut Encoder, birth: &Birth) -> Result<(), AppError> {
    output.put_u8(1);
    for field in [birth.invocation, birth.request, birth.signer, birth.network] {
        output.put_bytes(&field).map_err(|_| refused())?;
    }
    Ok(())
}
fn binding(input: &mut Decoder<'_>, cap: Capability) -> Result<old::FounderBindingData, AppError> {
    let vault = field32(input)?;
    let layout = field32(input)?;
    let group = input.get_bytes().map_err(|_| refused())?;
    if group.is_empty() || group.len() > 256 {
        return Err(refused());
    }
    let value = old::FounderBindingData {
        vault,
        layout,
        group: group.to_vec(),
        signer: field32(input)?,
        source: field32(input)?,
        profile: field32(input)?,
        pin: field32(input)?,
    };
    binding_length(&value, cap)?;
    Ok(value)
}
fn binding_length(value: &old::FounderBindingData, cap: Capability) -> Result<usize, AppError> {
    if value.vault != cap.original.vault
        || value.layout != cap.original.root
        || value.group.is_empty()
        || value.group.len() > 256
    {
        return Err(refused());
    }
    Ok(6 * 36 + 4 + value.group.len())
}
fn put_binding(output: &mut Encoder, value: &old::FounderBindingData) -> Result<(), AppError> {
    output.put_bytes(&value.vault).map_err(|_| refused())?;
    output.put_bytes(&value.layout).map_err(|_| refused())?;
    output.put_bytes(&value.group).map_err(|_| refused())?;
    for field in [value.signer, value.source, value.profile, value.pin] {
        output.put_bytes(&field).map_err(|_| refused())?;
    }
    Ok(())
}

impl Index {
    /// Only genuine old Legacy-only G1 is eligible. Decode/authentication and
    /// original held-file checks remain the physical migration owner's work.
    pub(super) fn migrate_v1_data(old: old::IndexData) -> Result<Self, AppError> {
        if old.generation != 1 || old.parent.is_some() || old.rows.len() > old::MAX_ROWS {
            return Err(refused());
        }
        let mut rows = Vec::with_capacity(old.rows.len());
        for row in old.rows {
            let old::BackendData::Legacy { invite } = row.backend else {
                return Err(refused());
            };
            rows.push(Row {
                id: row.id,
                identity: row.identity,
                name: row.name,
                is_dm: row.is_dm,
                backend: Backend::Legacy { invite },
            });
        }
        let value = Self {
            capability: Capability {
                original: old.capability,
            },
            generation: 1,
            parent: None,
            last_issued_id: rows.last().map_or(0, |row| row.id),
            rows,
        };
        value.encoded_length()?;
        Ok(value)
    }

    /// Before allocation/copying, callers reserve complete selected/candidate,
    /// plaintext/ciphertext, fixed uniqueness scratch and row/string overlap.
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
        {
            return Err(refused());
        }
        let generation = input.get_u64().map_err(|_| refused())?;
        let parent = match input.get_u8().map_err(|_| refused())? {
            0 => None,
            1 => Some(field32(&mut input)?),
            _ => return Err(refused()),
        };
        let last_issued_id = input.get_u64().map_err(|_| refused())?;
        let count = input.get_u32().map_err(|_| refused())? as usize;
        if count > old::MAX_ROWS {
            return Err(refused());
        }
        let mut rows = Vec::with_capacity(count);
        for _ in 0..count {
            let tag = input.get_u8().map_err(|_| refused())?;
            let id = input.get_u64().map_err(|_| refused())?;
            let identity = field32(&mut input)?;
            let name = string(&mut input, old::MAX_NAME)?.to_owned();
            let is_dm = match input.get_u8().map_err(|_| refused())? {
                0 => false,
                1 => true,
                _ => return Err(refused()),
            };
            let backend = match tag {
                0 => Backend::Legacy {
                    invite: string(&mut input, old::MAX_INVITE)?.to_owned(),
                },
                1 => {
                    let birth = birth(&mut input)?;
                    let frame = input.get_bytes().map_err(|_| refused())?;
                    if !(40..=MAX_NETWORK_FRAME).contains(&frame.len()) {
                        return Err(refused());
                    }
                    Backend::Pending {
                        birth,
                        network_frame: Zeroizing::new(frame.to_vec()),
                    }
                }
                2 => Backend::Founder {
                    birth: birth(&mut input)?,
                    binding: binding(&mut input, expected)?,
                },
                3 => Backend::Retired {
                    birth: birth(&mut input)?,
                    binding: binding(&mut input, expected)?,
                    scope_terminal: field32(&mut input)?,
                },
                4 => {
                    let birth = birth(&mut input)?;
                    let reason = match input.get_u8().map_err(|_| refused())? {
                        0 => old::AbandonedCreateReasonData::BeforeSdk,
                        1 => old::AbandonedCreateReasonData::UnexposedAfterSdk,
                        _ => return Err(refused()),
                    };
                    Backend::Abandoned {
                        birth,
                        reason,
                        journal_terminal: field32(&mut input)?,
                    }
                }
                _ => return Err(refused()),
            };
            rows.push(Row {
                id,
                identity,
                name,
                is_dm,
                backend,
            });
        }
        input.finish().map_err(|_| refused())?;
        let value = Self {
            capability: expected,
            generation,
            parent,
            last_issued_id,
            rows,
        };
        value.encoded_length()?;
        Ok(value)
    }

    fn encoded_length(&self) -> Result<usize, AppError> {
        if self.generation == 0
            || (self.generation == 1) != self.parent.is_none()
            || self.rows.len() > old::MAX_ROWS
        {
            return Err(refused());
        }
        // The only initial producer is empty or an exact legacy G1 migration.
        // Cold authentication must not smuggle an active native row past the
        // required durable Birth/activation successors or invent spent row IDs.
        if self.generation == 1
            && (self.last_issued_id != self.rows.last().map_or(0, |row| row.id)
                || self
                    .rows
                    .iter()
                    .any(|row| !matches!(&row.backend, Backend::Legacy { .. })))
        {
            return Err(refused());
        }
        let mut length = HEADER_BYTES + self.parent.map_or(0, |_| 36);
        let mut previous = None;
        let mut identities = Vec::with_capacity(self.rows.len());
        let mut invocations = Vec::with_capacity(self.rows.len());
        for row in &self.rows {
            if previous.is_some_and(|id| id >= row.id)
                || row.id > self.last_issued_id
                || row.name.len() > old::MAX_NAME
            {
                return Err(refused());
            }
            previous = Some(row.id);
            identities.push(row.identity);
            if let Some(birth) = row.backend.birth() {
                invocations.push(birth.invocation);
            }
            let payload = match &row.backend {
                Backend::Legacy { invite } => {
                    if invite.len() > old::MAX_INVITE {
                        return Err(refused());
                    }
                    4 + invite.len()
                }
                Backend::Pending { network_frame, .. } => {
                    if !(40..=MAX_NETWORK_FRAME).contains(&network_frame.len()) {
                        return Err(refused());
                    }
                    BIRTH_BYTES + 4 + network_frame.len()
                }
                Backend::Founder { birth, binding } | Backend::Retired { birth, binding, .. } => {
                    if birth.signer != binding.signer {
                        return Err(refused());
                    }
                    BIRTH_BYTES
                        + binding_length(binding, self.capability)?
                        + if matches!(row.backend, Backend::Retired { .. }) {
                            36
                        } else {
                            0
                        }
                }
                Backend::Abandoned { .. } => BIRTH_BYTES + 1 + 36,
            };
            length = length
                .checked_add(1 + 8 + 36 + 4 + row.name.len() + 1)
                .and_then(|n| n.checked_add(payload))
                .filter(|n| *n <= MAX_PLAIN)
                .ok_or_else(refused)?;
        }
        identities.sort_unstable();
        invocations.sort_unstable();
        if identities.windows(2).any(|pair| pair[0] == pair[1])
            || invocations.windows(2).any(|pair| pair[0] == pair[1])
        {
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
            self.capability.original.vault,
            self.capability.original.root,
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
        output
            .put_u64(self.last_issued_id)
            .put_u32(self.rows.len() as u32);
        for row in &self.rows {
            let tag = match &row.backend {
                Backend::Legacy { .. } => 0,
                Backend::Pending { .. } => 1,
                Backend::Founder { .. } => 2,
                Backend::Retired { .. } => 3,
                Backend::Abandoned { .. } => 4,
            };
            output.put_u8(tag).put_u64(row.id);
            output.put_bytes(&row.identity).map_err(|_| refused())?;
            output.put_str(&row.name).map_err(|_| refused())?;
            output.put_u8(u8::from(row.is_dm));
            match &row.backend {
                Backend::Legacy { invite } => {
                    output.put_str(invite).map_err(|_| refused())?;
                }
                Backend::Pending {
                    birth,
                    network_frame,
                } => {
                    put_birth(&mut output, birth)?;
                    output.put_bytes(network_frame).map_err(|_| refused())?;
                }
                Backend::Founder { birth, binding } => {
                    put_birth(&mut output, birth)?;
                    put_binding(&mut output, binding)?;
                }
                Backend::Retired {
                    birth,
                    binding,
                    scope_terminal,
                } => {
                    put_birth(&mut output, birth)?;
                    put_binding(&mut output, binding)?;
                    output.put_bytes(scope_terminal).map_err(|_| refused())?;
                }
                Backend::Abandoned {
                    birth,
                    reason,
                    journal_terminal,
                } => {
                    put_birth(&mut output, birth)?;
                    output.put_u8(match reason {
                        old::AbandonedCreateReasonData::BeforeSdk => 0,
                        old::AbandonedCreateReasonData::UnexposedAfterSdk => 1,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Change {
    Birth,
    Activate,
    Retire,
    Abandon,
}

impl Index {
    /// Structural admission only. The writer must additionally own the actual
    /// source/journal/native evidence for the returned operation. An authenticated
    /// arbitrary next-generation snapshot is never a semantic successor.
    pub(super) fn successor_data(
        &self,
        next: &Self,
        parent_frame: [u8; 32],
    ) -> Result<Change, AppError> {
        self.encoded_length()?;
        next.encoded_length()?;
        if self.capability != next.capability
            || self.generation.checked_add(1) != Some(next.generation)
            || next.parent != Some(parent_frame)
        {
            return Err(refused());
        }
        if next.rows.len() == self.rows.len() + 1 {
            let (new, prefix) = next.rows.split_last().ok_or_else(refused)?;
            if prefix != self.rows.as_slice()
                || self.last_issued_id.checked_add(1) != Some(new.id)
                || next.last_issued_id != new.id
                || !matches!(new.backend, Backend::Pending { .. })
            {
                return Err(refused());
            }
            return Ok(Change::Birth);
        }
        if self.rows.len() != next.rows.len() || self.last_issued_id != next.last_issued_id {
            return Err(refused());
        }
        let mut changed = None;
        for (old, new) in self.rows.iter().zip(&next.rows) {
            if old == new {
                continue;
            }
            if changed.is_some()
                || old.id != new.id
                || old.identity != new.identity
                || old.name != new.name
                || old.is_dm != new.is_dm
                || old.backend.birth() != new.backend.birth()
            {
                return Err(refused());
            }
            changed = Some(match (&old.backend, &new.backend) {
                (Backend::Pending { .. }, Backend::Founder { .. }) => Change::Activate,
                (Backend::Pending { .. }, Backend::Abandoned { .. }) => Change::Abandon,
                (Backend::Founder { binding: old, .. }, Backend::Retired { binding: new, .. })
                    if old == new =>
                {
                    Change::Retire
                }
                _ => return Err(refused()),
            });
        }
        changed.ok_or_else(refused)
    }

    /// Token lookup survives unrelated updates and every terminal state. Caller
    /// invokes it BEFORE RNG, seal, network construction or device/SDK work.
    pub(super) fn lookup_data(
        &self,
        invocation: [u8; 32],
        request: [u8; 32],
    ) -> Result<Option<&Row>, AppError> {
        let Some(row) = self.rows.iter().find(|row| {
            row.backend
                .birth()
                .is_some_and(|birth| birth.invocation == invocation)
        }) else {
            return Ok(None);
        };
        if row
            .backend
            .birth()
            .is_none_or(|birth| birth.request != request)
        {
            return Err(refused());
        }
        Ok(Some(row))
    }
}

#[cfg(test)]
mod tests;
