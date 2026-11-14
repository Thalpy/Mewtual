//! Closed index3 lifecycle DATA. Physical selection, membership, completed
//! journals, native authorization and runtime admission are separate proofs.
//! A structurally valid successor must never act as any of those authorities.
use super::super::data as original;
use super::capability::Capability;
use super::{refused, AppError};
use zeroize::Zeroizing;

/// Every index3 starts from one selected, authenticated index2. Its generation
/// one may therefore contain native/terminal rows; index2's G1 rule stays closed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct MigrationOrigin {
    pub(super) generation: u64,
    pub(super) frame: [u8; 32],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store::critical_namespaces::native_registry) enum BirthKind {
    Founder,
    Joined,
    LegacyJoin,
}

/// Immutable creation receipt. Local names and refreshed invites are not part
/// of token lookup: request commits to the original input, not today's display.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::store::critical_namespaces::native_registry) struct Birth {
    pub(in crate::store::critical_namespaces::native_registry) kind: BirthKind,
    pub(in crate::store::critical_namespaces::native_registry) invocation: [u8; 32],
    pub(in crate::store::critical_namespaces::native_registry) request: [u8; 32],
    pub(in crate::store::critical_namespaces::native_registry) signer: [u8; 32],
    pub(in crate::store::critical_namespaces::native_registry) network: [u8; 32],
    pub(in crate::store::critical_namespaces::native_registry) join_context: Option<[u8; 32]>,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum LegacyOrigin {
    /// Imported rows have no invented receipt for an unobserved old command.
    Imported,
    Admitted(Birth),
}

/// Historical joined binding, NOT a durable namespace6 pin or membership proof.
/// Its real producer must own those proofs and the original applied own-Add.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct JoinedBinding {
    pub(super) vault: [u8; 32],
    pub(super) layout: [u8; 32],
    pub(super) group: Vec<u8>,
    pub(super) signer: [u8; 32],
    pub(super) source: [u8; 32],
    pub(super) profile: [u8; 32],
    pub(super) pin: [u8; 32],
    pub(super) join_context: [u8; 32],
    pub(super) own_add: [u8; 32],
    // Immutable facts installed by the original joined source, not present-time
    // authority. Live replay/current selection advances independently.
    pub(super) initial_governing_tip: [u8; 32],
    pub(super) initial_current_basis: [u8; 32],
    // Original own-Add epoch, not the running node's current epoch. Later
    // membership transitions need the independently replayed live source.
    pub(super) joined_epoch: u64,
}

#[derive(PartialEq, Eq)]
pub(super) enum Backend {
    Legacy {
        origin: LegacyOrigin,
        invite: String,
    },
    RetiredLegacy {
        origin: LegacyOrigin,
        invite: String,
        terminal: [u8; 32],
    },
    Pending {
        birth: Birth,
        network_frame: Zeroizing<Vec<u8>>,
    },
    Founder {
        birth: Birth,
        binding: original::FounderBindingData,
    },
    RetiredFounder {
        birth: Birth,
        binding: original::FounderBindingData,
        terminal: [u8; 32],
    },
    Joined {
        birth: Birth,
        binding: JoinedBinding,
    },
    RetiredJoined {
        birth: Birth,
        binding: JoinedBinding,
        terminal: [u8; 32],
    },
    // These reasons belong only to the real founder creation journal. Joined
    // and LegacyJoin need their own actual terminal-journal contracts before
    // their abandonment can be added to an enabled persistence format.
    AbandonedFounder {
        birth: Birth,
        reason: original::AbandonedCreateReasonData,
        terminal: [u8; 32],
    },
}
impl Backend {
    pub(super) fn birth(&self) -> Option<&Birth> {
        match self {
            Self::Legacy { origin, .. } | Self::RetiredLegacy { origin, .. } => match origin {
                LegacyOrigin::Imported => None,
                LegacyOrigin::Admitted(birth) => Some(birth),
            },
            Self::Pending { birth, .. }
            | Self::Founder { birth, .. }
            | Self::RetiredFounder { birth, .. }
            | Self::Joined { birth, .. }
            | Self::RetiredJoined { birth, .. }
            | Self::AbandonedFounder { birth, .. } => Some(birth),
        }
    }
    fn active(&self) -> bool {
        matches!(
            self,
            Self::Legacy { .. } | Self::Founder { .. } | Self::Joined { .. }
        )
    }
}

#[derive(PartialEq, Eq)]
pub(super) struct Row {
    pub(super) id: u64,
    pub(super) identity: [u8; 32],
    // Immutable authorization classification; never a renameable display flag.
    pub(super) is_dm: bool,
    pub(super) name: String,
    // Shared by rename and invite refresh, so delayed metadata cannot rebase.
    pub(super) metadata_revision: u64,
    pub(super) backend: Backend,
}

#[derive(PartialEq, Eq)]
pub(super) struct Index {
    pub(super) capability: Capability,
    pub(super) origin: MigrationOrigin,
    pub(super) generation: u64,
    pub(super) parent: Option<[u8; 32]>,
    pub(super) last_issued_id: u64,
    // Includes terminal rows/receipts. Compaction is a separate future format.
    pub(super) rows: Vec<Row>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Change {
    Birth,
    Activate,
    Abandon,
    Retire,
    Rename,
    RefreshLegacyInvite,
}

impl Index {
    /// Bound structure before encoding/copying, under the physical owner's
    /// complete selected/candidate/frame/metadata payment. No SDK proof occurs.
    pub(super) fn validate_data(&self) -> Result<(), AppError> {
        if self.origin.generation == 0
            || self.generation == 0
            || (self.generation == 1) != self.parent.is_none()
            || self.rows.len() > original::MAX_ROWS
        {
            return Err(refused());
        }
        let mut previous = None;
        let mut identities = Vec::with_capacity(self.rows.len());
        let mut invocations = Vec::with_capacity(self.rows.len());
        for row in &self.rows {
            // Generation one is a migration, never a shortcut around the
            // ordinary birth/activate/metadata transitions. Preserve v2's
            // reachable backend grammar as well as every immutable receipt.
            if self.generation == 1 {
                let from_v2 = match &row.backend {
                    Backend::Legacy {
                        origin: LegacyOrigin::Imported,
                        ..
                    } => true,
                    Backend::Pending { birth, .. }
                    | Backend::Founder { birth, .. }
                    | Backend::RetiredFounder { birth, .. }
                    | Backend::AbandonedFounder { birth, .. } => birth.kind == BirthKind::Founder,
                    _ => false,
                };
                if row.metadata_revision != 0 || !from_v2 {
                    return Err(refused());
                }
                // The predecessor itself permitted only imported Legacy in
                // its G1, with exact highwater. Do not widen that rule through
                // an otherwise valid version3 migration-origin label.
                if self.origin.generation == 1
                    && !matches!(
                        row.backend,
                        Backend::Legacy {
                            origin: LegacyOrigin::Imported,
                            ..
                        }
                    )
                {
                    return Err(refused());
                }
            }
            if previous.is_some_and(|id| id >= row.id)
                || row.id > self.last_issued_id
                || row.name.len() > original::MAX_NAME
            {
                return Err(refused());
            }
            previous = Some(row.id);
            identities.push(row.identity);
            if let Some(birth) = row.backend.birth() {
                if (birth.kind == BirthKind::Founder) != birth.join_context.is_none() {
                    return Err(refused());
                }
                invocations.push(birth.invocation);
            }
            match &row.backend {
                Backend::Legacy { origin, invite }
                | Backend::RetiredLegacy { origin, invite, .. } => {
                    if invite.len() > original::MAX_INVITE
                        || matches!(origin, LegacyOrigin::Admitted(birth) if birth.kind != BirthKind::LegacyJoin)
                    {
                        return Err(refused());
                    }
                }
                Backend::Pending { network_frame, .. } => {
                    if !(40..=64 * 1024).contains(&network_frame.len()) {
                        return Err(refused());
                    }
                }
                Backend::Founder { birth, binding }
                | Backend::RetiredFounder { birth, binding, .. } => {
                    if birth.kind != BirthKind::Founder
                        || birth.signer != binding.signer
                        || binding.vault != self.capability.original.vault
                        || binding.layout != self.capability.original.root
                        || binding.group.is_empty()
                        || binding.group.len() > 256
                    {
                        return Err(refused());
                    }
                }
                Backend::Joined { birth, binding }
                | Backend::RetiredJoined { birth, binding, .. } => {
                    if birth.kind != BirthKind::Joined
                        || birth.signer != binding.signer
                        || birth.join_context != Some(binding.join_context)
                        || binding.vault != self.capability.original.vault
                        || binding.layout != self.capability.original.root
                        || binding.group.is_empty()
                        || binding.group.len() > 256
                    {
                        return Err(refused());
                    }
                }
                Backend::AbandonedFounder { birth, .. } => {
                    if birth.kind != BirthKind::Founder {
                        return Err(refused());
                    }
                }
            }
        }
        if self.generation == 1
            && self.origin.generation == 1
            && self.last_issued_id != self.rows.last().map_or(0, |row| row.id)
        {
            return Err(refused());
        }
        identities.sort_unstable();
        invocations.sort_unstable();
        if identities.windows(2).any(|v| v[0] == v[1])
            || invocations.windows(2).any(|v| v[0] == v[1])
        {
            return Err(refused());
        }
        Ok(())
    }

    /// Exactly one closed operation, with original physical parent proof still
    /// owed by the writer. There is no generic snapshot/back-end conversion.
    pub(super) fn successor_data(&self, next: &Self, frame: [u8; 32]) -> Result<Change, AppError> {
        // Capacity includes the admitted row's eventual activation/terminal,
        // not just current ciphertext. Metadata growth cannot spend that debt.
        self.encoded_length()?;
        next.encoded_length()?;
        if self.capability != next.capability
            || self.origin != next.origin
            || self.generation.checked_add(1) != Some(next.generation)
            || next.parent != Some(frame)
        {
            return Err(refused());
        }
        if next.rows.len() == self.rows.len() + 1 {
            let (added, prefix) = next.rows.split_last().ok_or_else(refused)?;
            if prefix != self.rows.as_slice()
                || self.last_issued_id.checked_add(1) != Some(added.id)
                || next.last_issued_id != added.id
                || added.metadata_revision != 0
                || !matches!(added.backend, Backend::Pending { .. })
            {
                return Err(refused());
            }
            return Ok(Change::Birth);
        }
        if next.rows.len() != self.rows.len() || next.last_issued_id != self.last_issued_id {
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
                || old.is_dm != new.is_dm
                || old.backend.birth() != new.backend.birth()
            {
                return Err(refused());
            }
            changed = Some(
                if old.backend == new.backend
                    && old.backend.active()
                    && old.name != new.name
                    && old.metadata_revision.checked_add(1) == Some(new.metadata_revision)
                {
                    Change::Rename
                } else if old.name == new.name {
                    match (&old.backend, &new.backend) {
                        (
                            Backend::Legacy {
                                origin: a,
                                invite: x,
                            },
                            Backend::Legacy {
                                origin: b,
                                invite: y,
                            },
                        ) if a == b
                            && x != y
                            && old.metadata_revision.checked_add(1)
                                == Some(new.metadata_revision) =>
                        {
                            Change::RefreshLegacyInvite
                        }
                        _ if old.metadata_revision != new.metadata_revision => {
                            return Err(refused())
                        }
                        (Backend::Pending { birth, .. }, Backend::Founder { .. })
                            if birth.kind == BirthKind::Founder =>
                        {
                            Change::Activate
                        }
                        (Backend::Pending { birth, .. }, Backend::Joined { .. })
                            if birth.kind == BirthKind::Joined =>
                        {
                            Change::Activate
                        }
                        (
                            Backend::Pending { birth, .. },
                            Backend::Legacy {
                                origin: LegacyOrigin::Admitted(_),
                                ..
                            },
                        ) if birth.kind == BirthKind::LegacyJoin => Change::Activate,
                        (Backend::Pending { birth, .. }, Backend::AbandonedFounder { .. })
                            if birth.kind == BirthKind::Founder =>
                        {
                            Change::Abandon
                        }
                        (
                            Backend::Legacy {
                                origin: a,
                                invite: x,
                            },
                            Backend::RetiredLegacy {
                                origin: b,
                                invite: y,
                                ..
                            },
                        ) if a == b && x == y => Change::Retire,
                        (
                            Backend::Founder { binding: a, .. },
                            Backend::RetiredFounder { binding: b, .. },
                        ) if a == b => Change::Retire,
                        (
                            Backend::Joined { binding: a, .. },
                            Backend::RetiredJoined { binding: b, .. },
                        ) if a == b => Change::Retire,
                        _ => return Err(refused()),
                    }
                } else {
                    return Err(refused());
                },
            );
        }
        changed.ok_or_else(refused)
    }

    pub(super) fn lookup_data(
        &self,
        token: [u8; 32],
        request: [u8; 32],
    ) -> Result<Option<&Row>, AppError> {
        let Some(row) = self.rows.iter().find(|row| {
            row.backend
                .birth()
                .is_some_and(|birth| birth.invocation == token)
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
