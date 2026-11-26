//! Unsigned governing Remove data from the actual selected public replay.
use super::*;
use crate::chat::owner_bodies::RemovalSpec;
use crate::chat::owner_header::{semantic_spec, HeaderSpec};

pub struct UnsignedRemovalStepData {
    body: RemovalBody,
    header: HeaderSpec,
    image: Vec<u8>,
    signing: Vec<u8>,
}

impl std::fmt::Debug for UnsignedRemovalStepData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnsignedRemovalStepData")
            .finish_non_exhaustive()
    }
}

impl UnsignedRemovalStepData {
    pub fn body_data(&self) -> &RemovalBody {
        &self.body
    }

    pub fn header_spec_data(&self) -> &HeaderSpec {
        &self.header
    }

    pub fn image_data(&self) -> &[u8] {
        &self.image
    }

    pub fn signing_bytes_data(&self) -> &[u8] {
        &self.signing
    }
}

impl ReplayMath {
    /// Build the frozen extent from the actual current slot. The caller must
    /// separately prove the exact MLS operation and its private signing cut.
    pub fn prepare_removal_data(
        &self,
        admission: OwnerBodyId,
        commit: [u8; 32],
    ) -> Result<UnsignedRemovalStepData, ChatError> {
        let index = self.find(admission)?;
        let old = &self.slots[index];
        if old.image.subject == self.profile.spec().founder || commit == [0; 32] {
            return Err(ChatError::Authority);
        }
        let epoch = self.epoch.checked_add(1).ok_or_else(dependency)?;
        let body = RemovalBody::new(RemovalSpec {
            group: self.profile.spec().group.clone(),
            subject: old.image.subject,
            commit,
            before_epoch: self.epoch,
            after_epoch: epoch,
            tip_header: old.tip_header,
            frozen_tip: old.frontier.tip(),
        })?;
        let slots: Vec<_> = self
            .slots
            .iter()
            .filter(|slot| slot.image.admission != admission)
            .map(|slot| slot.image)
            .collect();
        let image = self.result_image(&slots, HeaderKind::Removal, body.id(), epoch)?;
        let frame = StateFrame::decode(&image)?;
        let meta = frame.meta();
        let header = HeaderSpec {
            group: meta.group.to_vec(),
            profile: meta.profile,
            issuer: meta.issuer,
            tenure: meta.tenure,
            parent: meta.parent,
            sequence: meta.sequence,
            kind: meta.kind,
            body: meta.body,
            state: frame.commitment(),
        };
        let signing = semantic_spec(&header);
        Ok(UnsignedRemovalStepData {
            body,
            header,
            image,
            signing,
        })
    }
}
