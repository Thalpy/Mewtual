//! Explicit local Studio transactions. The actor supplies live membership while its caller lends
//! the sole mounted vault and lifecycle guards. No Studio packet is gossiped by this module.

use crate::{AppError, Server, ServerStore};
use catcoms_replication::studio::{
    FlipnoteHeader, FlipnoteOp, IndexOp, StudioExpiry, StudioKind, StudioProjection, StudioTarget,
};
use catcoms_replication::{epoch_zero_id, DomainOp};
pub use catcoms_replication::{studio as types, EpochPhase};
use catcoms_rt::{CryptoRngCore, MeshTransport};
use tokio::sync::{oneshot, OwnedMutexGuard};

/// Bounded before entering the actor queue. Bodies are the existing canonical Studio JSON,
/// not renderer-authored Automerge changes. Keep ids/nonces/bodies stable across retries.
pub enum StudioRequest {
    Read {
        target: StudioTarget,
    },
    Apply {
        target: StudioTarget,
        epoch_id: u128,
        nonce: [u8; 16],
        body: Vec<u8>,
    },
    /// Object header first, index second. This is retryable, not a cross-document transaction:
    /// failure can leave an unlisted object. The caller must retry these EXACT fields.
    Create {
        channel: [u8; 16],
        object: [u8; 16],
        nonce: [u8; 16],
        title: String,
        ts: u64,
    },
}
impl std::fmt::Debug for StudioRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioRequest { .. }")
    }
}
impl StudioRequest {
    pub fn target(&self) -> StudioTarget {
        match self {
            Self::Read { target } | Self::Apply { target, .. } => *target,
            Self::Create {
                channel, object, ..
            } => StudioTarget::Flipnote {
                channel: *channel,
                object: *object,
            },
        }
    }
    /// Static bounds/grammar only; identity, causal state and persistence are checked in actor.
    pub fn validate(&self) -> Result<(), AppError> {
        match self {
            Self::Read { .. } => Ok(()),
            Self::Apply {
                target,
                nonce,
                body,
                ..
            } => {
                match target {
                    StudioTarget::Index { .. } => {
                        IndexOp::decode(body).map_err(invalid)?;
                    }
                    StudioTarget::Flipnote { .. } => {
                        match FlipnoteOp::decode(body).map_err(invalid)? {
                            FlipnoteOp::InsertFrame { .. }
                            | FlipnoteOp::RemoveFrame { .. }
                            | FlipnoteOp::ReplaceFrame { .. }
                            | FlipnoteOp::SetHeader(
                                FlipnoteHeader::Title(_) | FlipnoteHeader::Fps(_),
                            ) => {}
                            _ => {
                                return Err(invalid(
                                    "sound, score and export operations are not available yet",
                                ))
                            }
                        }
                    }
                }
                domain(*target, *nonce, body.clone())
                    .encode()
                    .map(|_| ())
                    .map_err(invalid)
            }
            Self::Create { title, ts, .. } => {
                if title.len() > 64 * 1024 {
                    return Err(invalid("Studio title exceeds the operation bound"));
                }
                if *ts > 9_007_199_254_740_991 {
                    return Err(invalid("invalid creation timestamp"));
                }
                let header = FlipnoteOp::SetHeader(FlipnoteHeader::Title(title.clone()))
                    .encode()
                    .map_err(invalid)?;
                domain(self.target(), [0; 16], header)
                    .encode()
                    .map_err(invalid)?;
                // Both complete envelopes must fit BEFORE the object write. The actual creator
                // is supplied later from the live actor; every identity has the same hex width.
                let index = IndexOp::PutObject {
                    object: [0; 16],
                    kind: StudioKind::Flipnote,
                    title: title.clone(),
                    created_by: crate::DeviceId::from_bytes([0; 32]),
                    ts: *ts,
                    expiry: StudioExpiry::Unrecorded,
                }
                .encode()
                .map_err(invalid)?;
                domain(
                    StudioTarget::Index {
                        channel: self.target().channel(),
                    },
                    [0; 16],
                    index,
                )
                .encode()
                .map(|_| ())
                .map_err(invalid)
            }
        }
    }
    pub(crate) fn changes_state(&self) -> bool {
        !matches!(self, Self::Read { .. })
    }
}
fn domain(target: StudioTarget, nonce: [u8; 16], body: Vec<u8>) -> DomainOp {
    let (doc_type, key) = match target {
        StudioTarget::Index { channel } => (catcoms_wire::DocType::StudioIndex, channel),
        StudioTarget::Flipnote { object, .. } => (catcoms_wire::DocType::StudioObject, object),
    };
    DomainOp {
        nonce,
        doc_type,
        logical_key: key.to_vec(),
        body,
    }
}

/// A saved/read projection, not evidence of delivery or owner settlement. An absent object is
/// None; an absent Index reads as an empty epoch-zero view without creating a file.
#[derive(Debug)]
pub struct StudioView {
    pub epoch_id: u128,
    pub epoch: u64,
    pub phase: EpochPhase,
    pub projection: StudioProjection,
}

/// Caller-created, trusted-local custody only. `ordering` MUST retain the native numeric-server,
/// UI commit and registry-incarnation guards until the synchronous transaction ends. No locks
/// may be awaited after the actor is Ready: use fail-fast acquisition and retry on contention.
pub struct StudioVaultLease {
    pub(crate) store: OwnedMutexGuard<Option<ServerStore>>,
    pub(crate) server: u64,
    _ordering: Box<dyn Send>,
}
impl std::fmt::Debug for StudioVaultLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioVaultLease { .. }")
    }
}
impl StudioVaultLease {
    pub fn new(
        store: OwnedMutexGuard<Option<ServerStore>>,
        server: u64,
        ordering: impl Send + 'static,
    ) -> Self {
        Self {
            store,
            server,
            _ordering: Box::new(ordering),
        }
    }
}

/// Two-phase rendezvous: the actor is already in its exclusive Studio arm, so acquiring the
/// vault now cannot block its event producer BEFORE it reaches this arm. Dropping closes it.
#[derive(Debug)]
pub struct StudioReady {
    pub(crate) lease: oneshot::Sender<StudioVaultLease>,
    pub(crate) result: oneshot::Receiver<Result<Option<StudioView>, String>>,
}
impl StudioReady {
    pub async fn execute(self, lease: StudioVaultLease) -> Result<Option<StudioView>, String> {
        self.lease
            .send(lease)
            .map_err(|_| "Studio transaction expired".to_string())?;
        self.result.await.unwrap_or_else(|_| {
            Err("Studio transaction expired, cancelled or server stopped".into())
        })
    }
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Synchronous under actor+vault+lifecycle custody. Full scans are bounded, but accepted-size
    /// latency remains unqualified; this explicit operation is not automatic network serving.
    pub(crate) fn studio_transaction(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        request: StudioRequest,
    ) -> Result<Option<StudioView>, AppError> {
        request.validate()?;
        let target = request.target();
        if !self
            .channels()
            .iter()
            .any(|c| c.id == u128::from_be_bytes(target.channel()))
        {
            return Err(invalid("unknown Studio channel"));
        }
        self.sync.with_registry_context(|group, device, _, _| {
            if group.member_signature_key(&device.device_id()).as_deref()
                != Some(device.public_key_bytes().as_slice())
            {
                return Err(invalid("Studio requires current membership"));
            }
            if let StudioRequest::Apply {
                target: StudioTarget::Index { .. },
                nonce,
                body,
                ..
            } = &request
            {
                let logical = target.document(&group.group_id()).map_err(invalid)?;
                IndexOp::decode_domain(
                    &logical,
                    &domain(target, *nonce, body.clone()),
                    &device.device_id(),
                )
                .map_err(invalid)?;
            }
            if let StudioRequest::Create {
                channel,
                object,
                nonce,
                title,
                ts,
            } = &request
            {
                let initial = domain(
                    target,
                    *nonce,
                    FlipnoteOp::SetHeader(FlipnoteHeader::Title(title.clone()))
                        .encode()
                        .map_err(invalid)?,
                );
                if let Some(held) = store.load_studio_epoch(server, group, target, device)? {
                    if !held.contains_exact_operation(device.device_id(), &initial)? {
                        return Err(invalid("object already exists; use Apply to edit it"));
                    }
                }
                let index = StudioTarget::Index { channel: *channel };
                if let Some(held) = store.load_studio_epoch(server, group, index, device)? {
                    let StudioProjection::Index(projection) = held.projection()? else {
                        return Err(invalid("wrong index type"));
                    };
                    if projection.objects.contains_key(object)
                        || projection.overflow.contains_key(object)
                        || projection.deleted_objects.contains_key(object)
                        || projection.tombstones.contains_key(object)
                    {
                        let put = domain(
                            index,
                            *nonce,
                            IndexOp::PutObject {
                                object: *object,
                                kind: StudioKind::Flipnote,
                                title: title.clone(),
                                created_by: device.device_id(),
                                ts: *ts,
                                expiry: StudioExpiry::Unrecorded,
                            }
                            .encode()
                            .map_err(invalid)?,
                        );
                        if !held.contains_exact_operation(device.device_id(), &put)? {
                            return Err(invalid("object id already occurs in the index"));
                        }
                    }
                }
            }
            Ok(())
        })?;
        // Replacing refresh MUST precede transient pre-holds. The later accounting scan stays
        // reference-neutral, or it could erase a PIX hold before its intent reaches disk.
        if !store.creative_references_known() {
            // Failed discovery keeps deletion disabled. It must not prevent reading an otherwise
            // healthy document; mutation accounting independently refuses unsafe disk state.
            let _ = store.creative_pinned_cids();
        }
        if let StudioRequest::Apply {
            target,
            nonce,
            body,
            ..
        } = &request
        {
            let logical = target.document(&self.group_id()).map_err(invalid)?;
            store.hold_creative_operation(&logical, &domain(*target, *nonce, body.clone()));
        }
        // New local pixel references require exact already-held bytes in this mounted vault.
        // No network fetch or silent reference to a placeholder hash. The pre-hold above and
        // durable write hooks protect these bytes from same-mount cache reclamation; this
        // validation alone does not protect against filesystem damage or external deletion.
        if let StudioRequest::Apply {
            target: StudioTarget::Flipnote { .. },
            body,
            ..
        } = &request
        {
            match FlipnoteOp::decode(body).map_err(invalid)? {
                FlipnoteOp::InsertFrame { cid, bytes, .. }
                | FlipnoteOp::ReplaceFrame { cid, bytes, .. } => {
                    let mut blobs = store.blob_store(&hex::encode(self.group_id()))?;
                    let cid = crate::Cid::from_bytes(cid);
                    let pixels = blobs.get_bounded(&cid, bytes as usize)?.ok_or_else(|| {
                        invalid("publish the frame PIX before saving its reference")
                    })?;
                    if pixels.len() as u64 != bytes {
                        return Err(invalid("frame byte declaration differs from PIX"));
                    }
                    crate::creative::validate_pix(&pixels)?;
                    if pixels[4] != 191 || pixels[5] != 143 {
                        return Err(invalid("Flipnote frames must be 192x144"));
                    }
                    blobs.put_staged(&pixels)?;
                    if !blobs.promote_staged_bounded(&cid, pixels.len())? {
                        return Err(invalid("frame promotion failed"));
                    }
                }
                _ => {}
            }
        }
        // MLS/device/tenure needed to reopen must be durable before a new Studio source names
        // them. Native holds the same numeric-id and incarnation guards as ordinary snapshots.
        if request.changes_state() {
            let snapshot = self.snapshot()?;
            self.sync
                .with_registry_context(|_, _, _, rng| store.save_server(server, &snapshot, rng))?;
        }
        self.sync
            .with_registry_context(|group, device, clock, rng| {
                if group.member_signature_key(&device.device_id()).as_deref()
                    != Some(device.public_key_bytes().as_slice())
                {
                    return Err(invalid("Studio requires current membership"));
                }
                let read = |store: &ServerStore, target| -> Result<Option<StudioView>, AppError> {
                    if let Some(state) = store.load_studio_epoch(server, group, target, device)? {
                        return Ok(Some(StudioView {
                            epoch_id: state.doc_id(),
                            epoch: state.epoch(),
                            phase: state.phase(),
                            projection: state.projection()?,
                        }));
                    }
                    if matches!(target, StudioTarget::Index { .. }) {
                        let empty = catcoms_replication::studio::StudioEpoch::new(
                            group,
                            target,
                            device.device_id(),
                        )
                        .map_err(invalid)?;
                        return Ok(Some(StudioView {
                            epoch_id: empty.doc_id(),
                            epoch: 0,
                            phase: EpochPhase::Open,
                            projection: empty.projection().map_err(invalid)?,
                        }));
                    }
                    Ok(None)
                };
                if matches!(request, StudioRequest::Read { .. }) {
                    return read(store, target);
                }
                let mut scan = store.scan_epoch_storage_with_studio()?;
                while !scan.step()?.complete {}
                let inventory = scan.finish()?;
                let mut budget = store.studio_storage_budget(server, group, &inventory)?;
                let mut apply =
                    |target: StudioTarget, epoch_id, nonce, body| -> Result<(), AppError> {
                        let logical = target.document(&group.group_id()).map_err(invalid)?;
                        let op = DomainOp {
                            nonce,
                            doc_type: logical.doc_type,
                            logical_key: logical.logical_key,
                            body,
                        };
                        // Ciphertext is deliberately not sent: automatic sharing is gate 3. The durable
                        // full signed log and intent, rather than an in-memory packet, retain the edit.
                        store.edit_studio_epoch(
                            server,
                            group,
                            target,
                            epoch_id,
                            device,
                            op,
                            clock.now_ms(),
                            rng,
                            &mut budget,
                        )?;
                        Ok(())
                    };
                match request {
                    StudioRequest::Apply {
                        target,
                        epoch_id,
                        nonce,
                        body,
                    } => apply(target, epoch_id, nonce, body)?,
                    StudioRequest::Create {
                        channel,
                        object,
                        nonce,
                        title,
                        ts,
                    } => {
                        let object_target = StudioTarget::Flipnote { channel, object };
                        apply(
                            object_target,
                            epoch_zero_id(catcoms_wire::DocType::StudioObject, &object),
                            nonce,
                            FlipnoteOp::SetHeader(FlipnoteHeader::Title(title.clone()))
                                .encode()
                                .map_err(invalid)?,
                        )?;
                        apply(
                            StudioTarget::Index { channel },
                            epoch_zero_id(catcoms_wire::DocType::StudioIndex, &channel),
                            nonce,
                            IndexOp::PutObject {
                                object,
                                kind: StudioKind::Flipnote,
                                title,
                                created_by: device.device_id(),
                                ts,
                                expiry: StudioExpiry::Unrecorded,
                            }
                            .encode()
                            .map_err(invalid)?,
                        )?;
                    }
                    StudioRequest::Read { .. } => unreachable!(),
                }
                read(store, target)
            })
    }
}
fn invalid(error: impl std::fmt::Display) -> AppError {
    AppError::Invalid(format!("Studio: {error}"))
}

#[cfg(test)]
mod tests;
