//! The composite capture copy needs (design 5.2), stage C1.
//!
//! Revision 1 of the design claimed that copy needed nothing beyond the accepted inspection capture
//! and that only the rebuild function changed. That was withdrawn: the inspection capture holds one
//! record's plaintext and stamp and **nothing about a destination**, so a planner that has to check
//! capacity, tombstones and conflicts against where the work is going has no bytes to check them
//! against.
//!
//! So C1 takes a second bounded capture in the **same custody visit under the same preparation
//! permit**: the destination's authenticated Studio source record, and its recovery record if it
//! has one, with the digest and physical size of each. No projection is materialised here. Both
//! projections are built on the detached worker, which is what keeps custody spent on evidence
//! rather than on decoding.
//!
//! Nothing in this module is authority. The captured bytes are a *proposal's* inputs; C3 and C4
//! re-read both records and compare before anything durable happens, because a plan derived from
//! superseded bytes is a stale proposal however well formed it is.
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the copy control actions are this capture's production caller and land next in \
    this scope; its own tests exercise every item here today"
    )
)]
use super::super::epoch_recovery;
use super::*;
use catcoms_crypto::DeviceId;

/// Record identity captured under custody. No key, store handle, `Server` or budget, so it is safe
/// to hold across a detach.
pub(crate) struct StudioDestinationStamp {
    mount: Arc<()>,
    pub(in crate::store) server: u64,
    pub(in crate::store) document: LogicalDocument,
    pub(in crate::store) target: StudioTarget,
    /// The device the capture was taken for. A copy is authored by whoever asked for it, and a
    /// capture taken for one device must not be finished by another.
    actor: DeviceId,
    /// `(blake3 of the authenticated plaintext, physical bytes)`, the same currency contract
    /// `studio_inspection_is_current` uses. Two values rather than one because a record that
    /// changed size without changing its plaintext digest would be a sealing anomaly worth
    /// refusing, not worth tolerating.
    source: (blake3::Hash, u64),
    /// `None` when the destination has no recovery record. Absence is part of the stamp: a
    /// destination that acquires one between the plan and the apply has changed, because a
    /// tombstone in a newly retained version can block a resurrection the plan thought was free.
    recovery: Option<(blake3::Hash, u64)>,
}

impl std::fmt::Debug for StudioDestinationStamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioDestinationStamp { .. }")
    }
}

/// Authenticated destination bytes and their currency stamp.
pub(crate) struct StudioDestinationCapture {
    pub(in crate::store) stamp: StudioDestinationStamp,
    pub(in crate::store) source: Zeroizing<Vec<u8>>,
    pub(in crate::store) recovery: Option<Zeroizing<Vec<u8>>>,
}

impl std::fmt::Debug for StudioDestinationCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioDestinationCapture { .. }")
    }
}

impl ServerStore {
    /// Both destination records, read once under the caller's existing custody and permit.
    ///
    /// The destination's `LogicalDocument` is derived from its target rather than accepted
    /// alongside it, so a request cannot name one channel's target and another channel's document
    /// (C-0: the destination's identity is its complete `LogicalDocument`).
    ///
    /// Decodes nothing. A malformed destination record is the detached worker's problem and
    /// refusing it here would spend custody on work that does not need it.
    pub(crate) fn capture_studio_destination(
        &self,
        server: u64,
        group: &ServerGroup,
        destination: StudioTarget,
        device: &MlsDevice,
    ) -> Result<StudioDestinationCapture, AppError> {
        current_member(group, device)?;
        let document = destination.document(&group.group_id()).map_err(invalid)?;
        let source = self
            .read_studio_record_bounded(
                &scope_bytes(server, &document)?,
                source::MAX_RETAINED_BYTES as usize,
            )?
            .ok_or_else(|| invalid("copy destination has no Studio source"))?;
        let recovery =
            self.read_scoped_recovery_plain(&epoch_recovery::scope_bytes(server, &document)?)?;
        Ok(StudioDestinationCapture {
            stamp: StudioDestinationStamp {
                mount: self.registry_mount(),
                server,
                document,
                target: destination,
                actor: device.device_id(),
                source: (blake3::hash(&source.plain), source.physical_bytes),
                recovery: recovery
                    .as_ref()
                    .map(|r| (blake3::hash(&r.plain), r.physical_bytes)),
            },
            source: source.plain,
            recovery: recovery.map(|r| r.plain),
        })
    }

    /// Re-read **both** destination records and compare digest and physical size.
    ///
    /// Run at C3 and again at C4, alongside the source capture's own
    /// `studio_inspection_is_current`. Three records have to be unchanged for a copy proposal to
    /// still describe reality: the branch it came from, and the destination's two.
    pub(crate) fn studio_destination_is_current(
        &self,
        server: u64,
        group: &ServerGroup,
        destination: StudioTarget,
        device: &MlsDevice,
        stamp: &StudioDestinationStamp,
    ) -> Result<bool, AppError> {
        if !Arc::ptr_eq(&stamp.mount, &self.registry_mount())
            || stamp.server != server
            || stamp.target != destination
            || stamp.actor != device.device_id()
            || stamp.document.server_id != group.group_id()
            || stamp.document != destination.document(&group.group_id()).map_err(invalid)?
        {
            return Ok(false);
        }
        let Some(source) = self.read_studio_record_bounded(
            &scope_bytes(server, &stamp.document)?,
            source::MAX_RETAINED_BYTES as usize,
        )?
        else {
            return Ok(false);
        };
        if (blake3::hash(&source.plain), source.physical_bytes) != stamp.source {
            return Ok(false);
        }
        // Compared as an `Option`, so acquiring a recovery record where there was none, and losing
        // one where there was, are both changes. Reading only "if we captured one" would let a
        // newly retained version's tombstones appear under a plan that never saw them.
        let recovery = self
            .read_scoped_recovery_plain(&epoch_recovery::scope_bytes(server, &stamp.document)?)?
            .map(|r| (blake3::hash(&r.plain), r.physical_bytes));
        Ok(recovery == stamp.recovery)
    }
}
