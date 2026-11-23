//! Selectors for covered governing archives. All versions use the known
//! Head envelope so older readers authenticate the box before refusing it.
use super::archive::GovernedArchiveLabels;
use super::*;
use catcoms_replication::chat::{
    cold_manifest::ArchiveHeadId,
    owner_header::{OwnerHeaderId, OwnerStateCommitment},
};

const KIND: TextControlKindData = TextControlKindData::Head;
const TAG: &[u8] = KIND.domains().0;
const FIRST_ADD_VERSION: u16 = 2;
const LATER_ADD_VERSION: u16 = 3;
const REMOVE_VERSION: u16 = 4;
pub(super) const NAME: &str = "text.add.head-stage.v2";
pub(super) const FRAME_LEN: usize = 4 + TAG.len() + 2 + 7 * 36 + 2 * 8 + 40;

/// Immutable selector data. Physical coverage and installation have their
/// own original Current, namespace and SDK ownership requirements.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct GovernedHeadData {
    pub(super) transition: GovernedArchiveLabels,
    pub(super) archive: ArchiveHeadId,
    pub(super) generation: u64,
    pub(super) kind: GovernedHeadKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum GovernedHeadKind {
    FirstAdd,
    Add,
    Remove,
}

impl GovernedHeadKind {
    fn version(self) -> u16 {
        match self {
            Self::FirstAdd => FIRST_ADD_VERSION,
            Self::Add => LATER_ADD_VERSION,
            Self::Remove => REMOVE_VERSION,
        }
    }

    fn decode(version: u16) -> Result<Self, AppError> {
        match version {
            FIRST_ADD_VERSION => Ok(Self::FirstAdd),
            LATER_ADD_VERSION => Ok(Self::Add),
            REMOVE_VERSION => Ok(Self::Remove),
            _ => Err(refused()),
        }
    }
}

impl GovernedHeadData {
    pub(super) fn after_first_archive(
        transition: GovernedArchiveLabels,
        archive: ArchiveHeadId,
        parent_generation: u64,
    ) -> Result<Self, AppError> {
        if parent_generation != 1 || transition.epoch != 1 || archive == transition.before_archive {
            return Err(refused());
        }
        Ok(Self {
            transition,
            archive,
            generation: 2,
            kind: GovernedHeadKind::FirstAdd,
        })
    }

    pub(super) fn seal_data(
        self,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.seal_kind(key, rng, GovernedHeadKind::FirstAdd)
    }

    /// Prepare a later selector against the exact encrypted predecessor. The
    /// caller still holds the original files and its independently selected
    /// archive/Core replay before covering this selector.
    pub(super) fn after_governed_archive(
        key: &[u8; 32],
        predecessor: &[u8],
        transition: GovernedArchiveLabels,
        archive: ArchiveHeadId,
    ) -> Result<Self, AppError> {
        Self::after_successor(key, predecessor, transition, archive, GovernedHeadKind::Add)
    }

    pub(super) fn after_successor(
        key: &[u8; 32],
        predecessor: &[u8],
        transition: GovernedArchiveLabels,
        archive: ArchiveHeadId,
        kind: GovernedHeadKind,
    ) -> Result<Self, AppError> {
        let original = Self::open_governed_data(key, predecessor)?;
        let generation = original.generation.checked_add(1).ok_or_else(refused)?;
        let next = Self {
            transition,
            archive,
            generation,
            kind,
        };
        next.require_kind(kind)?;
        if transition.context != original.transition.context
            || transition.before_head != TextHeadIdData::of_data(predecessor)
            || transition.before_archive != original.archive
            || original.transition.epoch.checked_add(1) != Some(transition.epoch)
            || transition.header == original.transition.header
            || transition.state == original.transition.state
            || transition.authenticator == original.transition.authenticator
        {
            return Err(refused());
        }
        Ok(next)
    }

    pub(super) fn seal_later_data(
        self,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.seal_kind(key, rng, GovernedHeadKind::Add)
    }

    pub(super) fn seal_kind(
        self,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
        kind: GovernedHeadKind,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.require_kind(kind)?;
        let mut e = Encoder::with_capacity(FRAME_LEN - 40);
        put(&mut e, TAG)?;
        e.put_u16(kind.version());
        let a = &self.transition;
        for value in [
            a.context.as_bytes(),
            a.before_head.as_bytes(),
            a.before_archive.as_bytes(),
            self.archive.as_bytes(),
            a.header.as_bytes(),
            a.state.as_bytes(),
            &a.authenticator,
        ] {
            put(&mut e, value)?;
        }
        e.put_u64(self.generation).put_u64(a.epoch);
        let plain = Zeroizing::new(e.finish());
        let key = Zeroizing::new(blake3::derive_key(KIND.domains().1, key));
        let raw = Zeroizing::new(frame(&seal(&key, &plain, rng)?));
        if raw.len() != FRAME_LEN {
            return Err(refused());
        }
        Ok(raw)
    }

    pub(super) fn open_data(key: &[u8; 32], raw: &[u8]) -> Result<Self, AppError> {
        let selected = Self::open_governed_data(key, raw)?;
        selected.require_kind(GovernedHeadKind::FirstAdd)?;
        Ok(selected)
    }

    pub(super) fn open_governed_data(key: &[u8; 32], raw: &[u8]) -> Result<Self, AppError> {
        if raw.len() != FRAME_LEN {
            return Err(refused());
        }
        let key = Zeroizing::new(blake3::derive_key(KIND.domains().1, key));
        let plain = Zeroizing::new(unseal(&key, &unframe(raw)?)?);
        let mut d = Decoder::new(&plain);
        if field(&mut d, TAG.len())? != TAG {
            return Err(refused());
        }
        let version = d.get_u16().map_err(|_| refused())?;
        let kind = GovernedHeadKind::decode(version)?;
        let context = TextContextIdData::from_bytes(b32(&mut d)?);
        let before_head = TextHeadIdData::from_bytes(b32(&mut d)?);
        let before_archive = ArchiveHeadId::from_bytes(b32(&mut d)?);
        let archive = ArchiveHeadId::from_bytes(b32(&mut d)?);
        let header = OwnerHeaderId::from_bytes(b32(&mut d)?);
        let state = OwnerStateCommitment::from_bytes(b32(&mut d)?);
        let authenticator = b32(&mut d)?;
        let generation = d.get_u64().map_err(|_| refused())?;
        let epoch = d.get_u64().map_err(|_| refused())?;
        d.finish().map_err(|_| refused())?;
        let transition = GovernedArchiveLabels {
            context,
            before_head,
            before_archive,
            header,
            state,
            epoch,
            authenticator,
        };
        let selected = Self {
            transition,
            archive,
            generation,
            kind,
        };
        selected.require_kind(kind)?;
        Ok(selected)
    }

    fn require_kind(&self, kind: GovernedHeadKind) -> Result<(), AppError> {
        let valid = match kind {
            GovernedHeadKind::FirstAdd => self.transition.epoch == 1 && self.generation == 2,
            GovernedHeadKind::Add | GovernedHeadKind::Remove => {
                self.transition.epoch >= 2
                    && self.transition.epoch.checked_add(1) == Some(self.generation)
            }
        };
        if !valid || self.kind != kind || self.archive == self.transition.before_archive {
            return Err(refused());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
