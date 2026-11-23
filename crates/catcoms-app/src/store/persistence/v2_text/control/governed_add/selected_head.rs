//! Selectors for covered governed Add archives. Both versions use the known
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
pub(super) const NAME: &str = "text.add.head-stage.v2";
pub(super) const FRAME_LEN: usize = 4 + TAG.len() + 2 + 7 * 36 + 2 * 8 + 40;

/// Immutable selector data. Physical coverage and installation have their
/// own original Current, namespace and SDK ownership requirements.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct GovernedHeadData {
    pub(super) transition: GovernedArchiveLabels,
    pub(super) archive: ArchiveHeadId,
    pub(super) generation: u64,
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
        })
    }

    pub(super) fn seal_data(
        self,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.require_version(FIRST_ADD_VERSION)?;
        self.seal_version(key, rng, FIRST_ADD_VERSION)
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
        let original = Self::open_governed_data(key, predecessor)?;
        let generation = original.generation.checked_add(1).ok_or_else(refused)?;
        let next = Self {
            transition,
            archive,
            generation,
        };
        next.require_version(LATER_ADD_VERSION)?;
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
        self.require_version(LATER_ADD_VERSION)?;
        self.seal_version(key, rng, LATER_ADD_VERSION)
    }

    fn seal_version(
        self,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
        version: u16,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let mut e = Encoder::with_capacity(FRAME_LEN - 40);
        put(&mut e, TAG)?;
        e.put_u16(version);
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
        let (selected, version) = Self::decode_governed_data(key, raw)?;
        if version != FIRST_ADD_VERSION {
            return Err(refused());
        }
        Ok(selected)
    }

    pub(super) fn open_governed_data(key: &[u8; 32], raw: &[u8]) -> Result<Self, AppError> {
        Self::decode_governed_data(key, raw).map(|(head, _)| head)
    }

    fn decode_governed_data(key: &[u8; 32], raw: &[u8]) -> Result<(Self, u16), AppError> {
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
        };
        selected.require_version(version)?;
        Ok((selected, version))
    }

    fn require_version(&self, version: u16) -> Result<(), AppError> {
        let valid = match version {
            FIRST_ADD_VERSION => self.transition.epoch == 1 && self.generation == 2,
            LATER_ADD_VERSION => {
                self.transition.epoch >= 2
                    && self.transition.epoch.checked_add(1) == Some(self.generation)
            }
            _ => false,
        };
        if !valid || self.archive == self.transition.before_archive {
            return Err(refused());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
