//! Logical Head2 selector for a covered governed Add archive. Use the known
//! Head envelope so older readers authenticate the box and refuse version 2.
use super::archive::GovernedArchiveLabels;
use super::*;
use catcoms_replication::chat::{
    cold_manifest::ArchiveHeadId,
    owner_header::{OwnerHeaderId, OwnerStateCommitment},
};

const KIND: TextControlKindData = TextControlKindData::Head;
const TAG: &[u8] = KIND.domains().0;
const VERSION: u16 = 2;
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
    pub(super) fn after_archive(
        transition: GovernedArchiveLabels,
        archive: ArchiveHeadId,
        parent_generation: u64,
    ) -> Result<Self, AppError> {
        if parent_generation == 0 || transition.epoch == 0 || archive == transition.before_archive {
            return Err(refused());
        }
        Ok(Self {
            transition,
            archive,
            generation: parent_generation.checked_add(1).ok_or_else(refused)?,
        })
    }

    pub(super) fn seal_data(
        self,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.require_valid()?;
        let mut e = Encoder::with_capacity(FRAME_LEN - 40);
        put(&mut e, TAG)?;
        e.put_u16(VERSION);
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
        if raw.len() != FRAME_LEN {
            return Err(refused());
        }
        let key = Zeroizing::new(blake3::derive_key(KIND.domains().1, key));
        let plain = Zeroizing::new(unseal(&key, &unframe(raw)?)?);
        let mut d = Decoder::new(&plain);
        if field(&mut d, TAG.len())? != TAG || d.get_u16().map_err(|_| refused())? != VERSION {
            return Err(refused());
        }
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
        selected.require_valid()?;
        Ok(selected)
    }

    fn require_valid(&self) -> Result<(), AppError> {
        if self.generation < 2
            || self.transition.epoch == 0
            || self.archive == self.transition.before_archive
        {
            return Err(refused());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
