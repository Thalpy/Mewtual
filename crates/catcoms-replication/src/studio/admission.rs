//! Typed consumers of P1's existing gated edit/ingest and rollback-safe exact preflight.
//! These core methods do not own the vault, journal intents, publish, or schedule settlement.

use automerge::transaction::Transactable;
use automerge::{AutoCommit, Change, ReadDoc, ScalarValue, ROOT};
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_rt::CryptoRngCore;
use std::cell::RefCell;

use super::*;
use crate::{Admission, EncryptedDoc, EpochGate, EpochPhase, LocalIntent, SealedOp};

pub(super) struct PreparedEdit {
    pub(super) headers: std::collections::BTreeMap<String, ScalarValue>,
    pub(super) entry: (String, Vec<u8>),
}
impl PreparedEdit {
    pub(super) fn write(self, doc: &mut AutoCommit) -> Result<(), automerge::AutomergeError> {
        for (key, value) in self.headers {
            // Equal concurrent headers stay untouched; rewriting would emit cleanup deletes.
            if doc.get(ROOT, &key)?.is_none() {
                doc.put(ROOT, key, value)?;
            }
        }
        doc.put(ROOT, self.entry.0, self.entry.1)?;
        Ok(())
    }
}

/// Caller-selected logical object/channel binding. A registry pointer or UI scalar is not
/// sufficient authority; P1 still binds this target to the signed group and physical epoch.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StudioTarget {
    Index {
        channel: ElementId,
    },
    Flipnote {
        channel: ElementId,
        object: ElementId,
    },
}
impl std::fmt::Debug for StudioTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioTarget { .. }")
    }
}
impl StudioTarget {
    pub fn document(&self, server: &[u8]) -> Result<LogicalDocument, ReplError> {
        match *self {
            Self::Index { channel } => studio_index_document(server, channel),
            Self::Flipnote { object, .. } => flipnote_document(server, object),
        }
    }
    pub fn channel(&self) -> ElementId {
        match *self {
            Self::Index { channel } | Self::Flipnote { channel, .. } => channel,
        }
    }
    pub fn read(
        &self,
        document: &LogicalDocument,
        epoch: u64,
        doc: &AutoCommit,
    ) -> Result<StudioProjection, ReplError> {
        if *document != self.document(&document.server_id)? {
            return Err(ReplError::EpochScope);
        }
        match *self {
            Self::Index { .. } => Ok(StudioProjection::Index(StudioIndexProjection::read(
                document, epoch, doc,
            )?)),
            Self::Flipnote { channel, .. } => Ok(StudioProjection::Flipnote(Box::new(
                FlipnoteFrameProjection::read(document, channel, epoch, doc)?,
            ))),
        }
    }
    pub(super) fn validate(
        &self,
        logical: &LogicalDocument,
        epoch: u64,
        domain: &DomainOp,
        change: &Change,
        before: &AutoCommit,
    ) -> Result<(), ReplError> {
        match *self {
            Self::Index { .. } => validate_index_change(logical, epoch, domain, change, before),
            Self::Flipnote { channel, .. } => {
                validate_frame_change(logical, channel, epoch, domain, change, before)
            }
        }
    }
    /// Caller must seal the intent before calling, then durably save document/log/gate before
    /// publishing the result. Exact retries remain a caller's retained-log reseal operation;
    /// NoChange alone does not attest equal operation bodies or successful persistence.
    #[allow(clippy::too_many_arguments)]
    pub fn edit(
        &self,
        doc: &mut EncryptedDoc,
        gate: &EpochGate,
        device: &MlsDevice,
        group: &ServerGroup,
        rng: &mut impl CryptoRngCore,
        domain: &DomainOp,
        ts: u64,
    ) -> Result<SealedOp, ReplError> {
        if gate.phase() != EpochPhase::Open {
            return Err(ReplError::EpochClosed);
        }
        integer_bound(ts)?;
        let logical = self.document(&group.group_id())?;
        let epoch = gate.epoch();
        let before = doc.doc().clone();
        let projection = self.read(&logical, epoch, &before)?;
        self.local_policy(&projection, domain, &device.device_id())?;
        let prepared = match &projection {
            StudioProjection::Index(_) => {
                index::prepare(&logical, epoch, domain, &device.device_id())?
            }
            StudioProjection::Flipnote(p) => frames::prepare(p, domain, &device.device_id(), ts)?,
        };
        let mut operations = recovery::current_operations(doc)?;
        operations.insert(
            domain.id(&device.device_id()),
            LocalIntent {
                author: device.device_id(),
                operation: domain.clone(),
            },
        );
        doc.edit_domain_preflight_gated(
            &logical,
            gate,
            device,
            group,
            rng,
            domain,
            |staged| prepared.write(staged),
            |domain, change| self.validate(&logical, epoch, domain, change, &before),
            |staged| recovery::preflight(self.read(&logical, epoch, staged)?, &operations),
        )
        .map(|(sealed, _)| sealed)
    }
    /// Remote concurrent edits use the same exact seed/recovery preflight, but NOT the local
    /// editor cap policy. Concurrent valid work may create deterministic overflow that remains
    /// readable/recoverable; it must not be silently dropped as if it had never been received.
    pub fn ingest(
        &self,
        doc: &mut EncryptedDoc,
        gate: &EpochGate,
        sealed: &SealedOp,
        group: &ServerGroup,
        device: &MlsDevice,
    ) -> Result<Admission, ReplError> {
        let logical = self.document(&group.group_id())?;
        let epoch = gate.epoch();
        let before = doc.doc().clone();
        let operations = RefCell::new(recovery::current_operations(doc)?);
        doc.ingest_domain_preflight_gated(
            &logical,
            gate,
            sealed,
            group,
            device,
            |domain, change| {
                self.validate(&logical, epoch, domain, change, &before)?;
                let author = DeviceId::from_bytes(
                    change
                        .actor_id()
                        .to_bytes()
                        .try_into()
                        .map_err(|_| ReplError::EpochAuthority)?,
                );
                operations.borrow_mut().insert(
                    domain.id(&author),
                    LocalIntent {
                        author,
                        operation: domain.clone(),
                    },
                );
                Ok(())
            },
            |staged| recovery::preflight(self.read(&logical, epoch, staged)?, &operations.borrow()),
        )
    }
    fn local_policy(
        &self,
        projection: &StudioProjection,
        domain: &DomainOp,
        author: &DeviceId,
    ) -> Result<(), ReplError> {
        match projection {
            StudioProjection::Index(p) => {
                let op = IndexOp::decode_domain(p.document(), domain, author)?;
                if matches!(op, IndexOp::PutObject { .. })
                    && p.objects.len() + p.overflow.len() >= MAX_INDEX_OBJECTS
                {
                    return Err(ReplError::EpochBound);
                }
            }
            StudioProjection::Flipnote(p) => {
                let op = FlipnoteOp::decode_domain(p.document(), domain)?;
                let total = match op {
                    FlipnoteOp::InsertFrame { bytes, .. } => {
                        if p.timeline.len() >= FLIPNOTE_MAX_FRAMES {
                            return Err(ReplError::EpochBound);
                        }
                        Some(
                            p.declared_frame_bytes
                                .checked_add(bytes)
                                .ok_or(ReplError::EpochBound)?,
                        )
                    }
                    FlipnoteOp::ReplaceFrame { frame, bytes, .. } => {
                        let old = p.frames.get(&frame).ok_or(ReplError::Malformed)?;
                        if p.over_cap.contains_key(&frame) || p.timeline.len() > FLIPNOTE_MAX_FRAMES
                        {
                            return Err(ReplError::EpochBound);
                        }
                        Some(
                            p.declared_frame_bytes
                                .checked_sub(old.pixels.selected.value.bytes)
                                .and_then(|n| n.checked_add(bytes))
                                .ok_or(ReplError::EpochBound)?,
                        )
                    }
                    _ => None,
                };
                if total.is_some_and(|n| n > FLIPNOTE_FRAME_BYTES) {
                    return Err(ReplError::EpochBound);
                }
            }
        }
        Ok(())
    }
}
