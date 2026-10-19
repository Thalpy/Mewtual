//! One-attempt connected maintenance, split at the network wait. Only the originating sync
//! owner authenticates and commits a reply. No MLS/device keys, RNG or event reader leave it.
use super::*;
use catcoms_rt::{RequestAuthority, SharedRequestKeepalive};
use std::sync::{Mutex, Weak};
use tokio::sync::watch;

/// The additive finalization exchange stays distinct from released PEX and role-offer grammars.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberMaintenanceKind {
    /// Released request-bound MLS commit catch-up, including a quiet check at our current epoch.
    /// A missed control broadcast must not require a future data packet to reveal the gap.
    Commits,
    /// Released request-bound peer records; remains unbound endpoint evidence.
    Pex,
    /// Additive policy- and endpoint-bound continuing member exchange.
    Finalize,
    /// Released self-signed, descriptor-bound standing helper role offer.
    Offer,
}

/// State replacement is not evidence of peer failure and must not charge peer-health backoff.
#[derive(Debug, Error)]
pub enum MemberMaintenanceError {
    /// Local authority changed; do not brand the remote peer unhealthy.
    #[error("member maintenance state changed")]
    Superseded,
    /// Current request failed its deadline, transport or validation contract.
    #[error(transparent)]
    Failed(#[from] SyncError),
}

#[derive(Clone, PartialEq, Eq)]
struct View {
    epoch: u64,
    policy: Option<[u8; 32]>,
    active_policy: bool,
    active_member: bool,
}

/// At most eight preparations may retain an exact generation per sync owner. Production runs
/// at most two, but this also bounds standalone callers of the public preparation seam.
#[derive(Default)]
pub(super) struct MaintenanceOwner {
    active: Vec<Weak<Gate>>,
}
impl MaintenanceOwner {
    pub(super) fn disconnected(&self, peer: PeerId) {
        for gate in self.active.iter().filter_map(Weak::upgrade).filter(|gate| {
            gate.peer == peer
                && gate
                    .connection
                    .as_ref()
                    .is_none_or(|lease| !lease.is_live())
        }) {
            gate.revoke();
        }
    }
}
impl Drop for MaintenanceOwner {
    fn drop(&mut self) {
        for gate in self.active.iter().filter_map(Weak::upgrade) {
            gate.revoke();
        }
    }
}
struct Gate {
    peer: PeerId,
    view: View,
    descriptor: Mutex<Option<(DeviceId, [u8; 32])>>,
    live: watch::Sender<bool>,
    clock: Arc<dyn Clock + Send>,
    deadline: u64,
    connection: Option<catcoms_rt::ConnectionLease>,
}
impl fmt::Debug for Gate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MaintenanceAuthority { .. }")
    }
}
impl Gate {
    fn revoke(&self) {
        self.live.send_replace(false);
    }
}
#[async_trait::async_trait]
impl RequestAuthority for Gate {
    fn authorized(&self) -> bool {
        *self.live.borrow()
            && self.clock.monotonic_ms() < self.deadline
            && self.connection.as_ref().is_none_or(|lease| lease.is_live())
    }
    async fn revoked(&self) {
        let mut live = self.live.subscribe();
        loop {
            if !self.authorized() {
                return;
            }
            let remaining = self.deadline.saturating_sub(self.clock.monotonic_ms());
            let connection = async {
                if let Some(connection) = &self.connection {
                    connection.revoked().await;
                } else {
                    std::future::pending::<()>().await;
                }
            };
            match futures::future::select(
                Box::pin(live.changed()),
                Box::pin(async move {
                    futures::future::select(
                        self.clock
                            .sleep(std::time::Duration::from_millis(remaining)),
                        Box::pin(connection),
                    )
                    .await;
                }),
            )
            .await
            {
                futures::future::Either::Left((Ok(()), _)) => {}
                _ => return,
            }
        }
    }
}

struct Context {
    gate: Arc<Gate>,
    instance: Arc<()>,
    kind: MemberMaintenanceKind,
    auth: RequestAuth,
    request_digest: [u8; 32],
    policy: Option<[u8; 32]>,
}
/// Affine request bytes and immutable authority; a caller supplies actual-retirement accounting.
pub struct PendingMemberMaintenance<T: MeshTransport> {
    transport: Arc<T>,
    request: Vec<u8>,
    context: Context,
}
impl<T: MeshTransport> fmt::Debug for PendingMemberMaintenance<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PendingMemberMaintenance { .. }")
    }
}
/// Bounded untrusted answer and its exact context. This is deliberately not cloneable.
pub struct CompletedMemberMaintenance {
    context: Context,
    response: Result<Bytes, TransportError>,
    _custody: Option<SharedRequestKeepalive>,
}
impl fmt::Debug for CompletedMemberMaintenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CompletedMemberMaintenance { .. }")
    }
}
impl<T: MeshTransport> PendingMemberMaintenance<T> {
    /// Uses connected-only submission and the injected absolute deadline. The driver keeps the
    /// supplied custody after this future expires; a timeout never recycles lower occupancy.
    pub async fn execute(self, cancellation: RequestCancellation) -> CompletedMemberMaintenance {
        let custody = cancellation.keepalive();
        let cancellation = cancellation.with_authority(self.context.gate.clone());
        let response = match futures::future::select(
            Box::pin(self.transport.request_connected_cancellable(
                self.context.gate.peer,
                ProtocolId(RR_PROTOCOL),
                Bytes::from(self.request),
                cancellation,
            )),
            Box::pin(self.context.gate.revoked()),
        )
        .await
        {
            futures::future::Either::Left((response, _)) => response,
            futures::future::Either::Right(((), _)) => Err(TransportError::Cancelled),
        };
        // Transport framing has its own larger allocation bound. Refuse oversized answers
        // before retaining them in a completion queue or invoking a decoder.
        let max = match self.context.kind {
            MemberMaintenanceKind::Commits => MAX_CATCHUP_RESPONSE,
            MemberMaintenanceKind::Pex => MAX_PEX_RESPONSE,
            MemberMaintenanceKind::Finalize => member_finalization::MAX_FRAME,
            MemberMaintenanceKind::Offer => MAX_CONTROL_REQUEST,
        };
        let response = response.and_then(|bytes| {
            if bytes.len() <= max {
                Ok(bytes)
            } else {
                Err(TransportError::NoResponse)
            }
        });
        CompletedMemberMaintenance {
            context: self.context,
            response,
            _custody: custody,
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    fn maintenance_view(&self) -> View {
        View {
            epoch: self.group.epoch(),
            policy: self.group_policy_digest(),
            active_policy: self.group_policy_active,
            active_member: self.group.is_active()
                && self.group.contains_device(&self.device.device_id()),
        }
    }
    fn maintenance_descriptor(
        &self,
        peer: PeerId,
        members: &HashSet<DeviceId>,
    ) -> Option<(DeviceId, [u8; 32])> {
        self.peer_records
            .iter()
            .find(|(device, record)| members.contains(device) && record.peer_id == *peer.as_bytes())
            .map(|(device, record)| (*device, peer_descriptor_hash(record)))
    }
    pub(super) fn reconcile_member_maintenance(&mut self) {
        self.reconcile_voice_routes();
        if self.member_maintenance.active.is_empty() {
            return;
        }
        let view = self.maintenance_view();
        // Reconciliation can run on inbound dispatch. Enumerate MLS once, rather than rebuilding
        // its device list inside every descriptor lookup for each of eight retained gates.
        let members: HashSet<_> = self.group.member_device_ids().into_iter().collect();
        // The list is bounded independently of roster size. Do not retain old weak entries.
        let gates: Vec<_> = self
            .member_maintenance
            .active
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        for gate in gates {
            let current = self.maintenance_descriptor(gate.peer, &members);
            let mut expected = gate.descriptor.lock().unwrap();
            // A reciprocal kind26 request can teach the first descriptor while our outbound
            // request is waiting. That is new authenticated knowledge, not replacement. Pin
            // it exactly once; any subsequent change/removal permanently closes this gate.
            let descriptor_changed = if expected.is_some() {
                *expected != current
            } else {
                *expected = current;
                false
            };
            if gate.view != view || descriptor_changed || !self.peer_is_connected(gate.peer) {
                tracing::trace!(local = ?self.local_peer(), peer = ?gate.peer,
                    view_changed = gate.view != view, descriptor_changed,
                    "member maintenance owner superseded");
                gate.revoke();
            }
        }
        self.member_maintenance
            .active
            .retain(|gate| gate.upgrade().is_some_and(|gate| gate.authorized()));
    }

    /// Prepare one connected request, preserving legacy PEX eligibility and the stricter policy
    /// gate on additive member finalization. Each following chain stage must prepare afresh.
    pub fn prepare_member_maintenance(
        &mut self,
        peer: PeerId,
        kind: MemberMaintenanceKind,
    ) -> Result<Option<PendingMemberMaintenance<T>>, SyncError> {
        self.reconcile_member_maintenance();
        if !self.maintenance_view().active_member
            || !self.peer_is_connected(peer)
            || (kind == MemberMaintenanceKind::Finalize && !self.policy_allows_member_mesh())
        {
            return Ok(None);
        }
        if self.member_maintenance.active.len() >= 8 {
            return Err(SyncError::Malformed);
        }
        let connection = self.transport.connection_lease(peer);
        // On capable transports, an absent owner is an actual disconnect rather than an
        // unsupported seam. Do not capture event-driven permission during that race window.
        if self.transport.supports_connection_leases()
            && connection.as_ref().is_none_or(|lease| !lease.is_live())
        {
            return Ok(None);
        }
        let inner = if kind == MemberMaintenanceKind::Commits {
            encode_commit_catchup_req(self.group.epoch())
        } else if kind == MemberMaintenanceKind::Finalize {
            let Some(record) = self.self_record() else {
                return Ok(None);
            };
            member_finalization::body(
                self.group_policy_digest().ok_or(SyncError::Malformed)?,
                self.transport.local_peer(),
                peer,
                record,
            )
        } else {
            Vec::new()
        };
        let tag = match kind {
            MemberMaintenanceKind::Commits => KIND_COMMIT_CATCHUP,
            MemberMaintenanceKind::Pex => KIND_PEX,
            MemberMaintenanceKind::Finalize => member_finalization::KIND_MEMBER_FINALIZE,
            MemberMaintenanceKind::Offer => KIND_SWITCHBOARD_OFFER,
        };
        let (request, auth) = self.build_authed_request(tag, &inner)?;
        if kind == MemberMaintenanceKind::Commits {
            self.stats.commit_catchups_requested += 1;
        }
        let deadline =
            self.clock
                .monotonic_ms()
                .saturating_add(if kind == MemberMaintenanceKind::Finalize {
                    2_000
                } else {
                    3_000
                });
        let gate = Arc::new(Gate {
            peer,
            view: self.maintenance_view(),
            descriptor: Mutex::new(self.maintenance_descriptor(
                peer,
                &self.group.member_device_ids().into_iter().collect(),
            )),
            live: watch::channel(true).0,
            clock: self.clock.clone(),
            deadline,
            connection,
        });
        self.member_maintenance.active.push(Arc::downgrade(&gate));
        Ok(Some(PendingMemberMaintenance {
            transport: self.transport.clone(),
            request,
            context: Context {
                gate,
                instance: self.blob_fetch_instance.clone(),
                kind,
                auth,
                request_digest: *blake3::hash(&inner).as_bytes(),
                policy: self.group_policy_digest(),
            },
        }))
    }

    /// Revalidate before either positive evidence or a negative cache effect. A disconnect,
    /// descriptor replacement, policy activation, removal/re-add or root replacement rejects
    /// the whole result; a released PEX response still establishes only an unbound source.
    pub fn complete_member_maintenance(
        &mut self,
        completed: CompletedMemberMaintenance,
    ) -> Result<bool, MemberMaintenanceError> {
        let context = completed.context;
        self.reconcile_member_maintenance();
        if !Arc::ptr_eq(&context.instance, &self.blob_fetch_instance)
            || !*context.gate.live.borrow()
            || context
                .gate
                .connection
                .as_ref()
                .is_some_and(|lease| !lease.is_live())
            || !self.peer_is_connected(context.gate.peer)
        {
            return Err(MemberMaintenanceError::Superseded);
        }
        if self.clock.monotonic_ms() >= context.gate.deadline {
            return Err(MemberMaintenanceError::Failed(
                TransportError::Timeout(context.gate.peer).into(),
            ));
        }
        let response = completed.response.map_err(SyncError::from)?;
        Ok(match context.kind {
            MemberMaintenanceKind::Commits => self
                .apply_commit_catchup_response(context.gate.peer, context.auth, &response)
                .map(|outcome| matches!(outcome, CommitCatchupOutcome::Verified { .. })),
            MemberMaintenanceKind::Pex => self
                .apply_pex_response(context.gate.peer, &context.auth, response)
                .map(|_| true),
            MemberMaintenanceKind::Offer => {
                self.apply_switchboard_offer_response(context.gate.peer, response)
            }
            MemberMaintenanceKind::Finalize => self.apply_member_finalization_response(
                context.gate.peer,
                &context.auth,
                context.request_digest,
                context.policy.ok_or(SyncError::Malformed)?,
                response,
            ),
        }?)
    }
}
