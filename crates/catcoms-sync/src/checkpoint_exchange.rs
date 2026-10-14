//! Typed logical checkpoint scopes. Registry and Studio keep distinct wire kinds and signature
//! domains, but share discovery queues, rate debt and retained request/seed capacity.
use super::*;
use catcoms_replication::{
    registry::{registry_document, RegistryProjection},
    studio::{FlipnoteFrameProjection, ScoreProjection, StudioIndexProjection, StudioTarget},
    LogicalDocument, VerifiedCheckpoint, VerifiedReceipt,
};

/// A Studio query's scope selector, NOT a wire DocType or a key-derivation input. The high-bit
/// score selector makes old peers refuse instead of interpreting a score request as art. The
/// existing signed request/response transcript binds this exact field, with no downgrade retry.
pub(crate) fn studio_scope_tag(target: StudioTarget) -> u16 {
    match target {
        StudioTarget::Index { .. } => 15,
        StudioTarget::Flipnote { .. } => 16,
        StudioTarget::Score { .. } => 0x8010,
    }
}
pub(crate) fn studio_scope_from_tag(
    tag: u16,
    channel: [u8; 16],
    object: [u8; 16],
) -> Result<StudioTarget, SyncError> {
    match tag {
        15 if object == [0; 16] => Ok(StudioTarget::Index { channel }),
        16 => Ok(StudioTarget::Flipnote { channel, object }),
        0x8010 => Ok(StudioTarget::Score { channel, object }),
        _ => Err(SyncError::Malformed),
    }
}

/// Trusted local choice, not a proof that the document exists or belongs to this channel.
/// The source and the expected-hash seed's typed projection independently check that binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CheckpointTarget {
    Registry(u8),
    Studio(StudioTarget),
}
impl CheckpointTarget {
    pub fn document(self, group: &[u8]) -> Result<LogicalDocument, SyncError> {
        Ok(match self {
            Self::Registry(bucket) => registry_document(group, bucket)?,
            Self::Studio(target) => target.document(group)?,
        })
    }
    pub(super) fn doc_type(self) -> DocType {
        match self {
            Self::Registry(_) => DocType::DocRegistry,
            Self::Studio(StudioTarget::Index { .. }) => DocType::StudioIndex,
            Self::Studio(StudioTarget::Flipnote { .. } | StudioTarget::Score { .. }) => {
                DocType::StudioObject
            }
        }
    }
    pub(super) fn bucket(self) -> Result<u8, SyncError> {
        match self {
            Self::Registry(bucket) => Ok(bucket),
            Self::Studio(_) => Err(SyncError::Malformed),
        }
    }
    pub(super) fn head_kind(self) -> u8 {
        match self {
            Self::Registry(_) => KIND_RECEIPT_HEAD,
            Self::Studio(_) => KIND_STUDIO_HEAD,
        }
    }
    pub(super) fn seed_kind(self) -> u8 {
        match self {
            Self::Registry(_) => KIND_REGISTRY_SEED,
            Self::Studio(_) => KIND_STUDIO_SEED,
        }
    }
    pub(super) fn head_domain(self) -> &'static str {
        match self {
            Self::Registry(_) => "catcoms/receipt-head-response/v1",
            Self::Studio(_) => "catcoms/studio-head-response/v1",
        }
    }
    pub(super) fn seed_domain(self) -> &'static str {
        match self {
            Self::Registry(_) => "catcoms/registry-seed-response/v1",
            Self::Studio(_) => "catcoms/studio-seed-response/v1",
        }
    }
    pub(super) fn verify_seed(
        self,
        receipt: &VerifiedReceipt,
        raw: &[u8],
    ) -> Result<VerifiedCheckpoint, SyncError> {
        Ok(match self {
            Self::Registry(bucket) => RegistryProjection::verify_checkpoint(receipt, bucket, raw)?,
            Self::Studio(StudioTarget::Index { .. }) => {
                StudioIndexProjection::verify_checkpoint(receipt, raw)?
            }
            Self::Studio(StudioTarget::Flipnote { channel, .. }) => {
                FlipnoteFrameProjection::verify_checkpoint(receipt, channel, raw)?
            }
            Self::Studio(StudioTarget::Score { channel, .. }) => {
                ScoreProjection::verify_checkpoint(receipt, channel, raw)?
            }
        })
    }
}
