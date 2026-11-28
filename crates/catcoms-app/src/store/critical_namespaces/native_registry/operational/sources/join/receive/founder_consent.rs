//! A separate standing dial authorization for a single original A1 founder.
//! No invitation, network discovery, or legacy peer hint grants this authority.
use super::history_discovery::HistoryContactScope;
use super::*;
use crate::store::persistence::v2_founder_consent::{FounderConsentRoute, FOUNDER_CONSENT_FEE};
use catcoms_discovery::EndpointDialScheduler;
use catcoms_net::{AuthenticatedDirectConnection, MeshConnectionDial};
use catcoms_sync::owner_v2::V2MemberPublicationObservation;
use std::net::IpAddr;
use zeroize::Zeroizing;

type Refusal = V2MemberSaveRefusal;

catcoms_rt::redacted_debug! {
    NativeTextFounderRouteApproval;
    NativeTextFounderRouteAttempt;
    NativeTextFounderRouteGesture;
}

pub(in super::super) struct FounderRouteConsentState {
    generation: watch::Sender<bool>,
    allow_restore: bool,
}

impl Default for FounderRouteConsentState {
    fn default() -> Self {
        Self {
            generation: watch::channel(false).0,
            allow_restore: true,
        }
    }
}

impl FounderRouteConsentState {
    fn observe(&self) -> RequestCancellation {
        RequestCancellation::new(self.generation.subscribe(), None)
    }

    fn stop_restoring(&mut self) {
        self.generation = watch::channel(false).0;
        self.allow_restore = false;
    }
}

/// Exact main-window Remember gesture. A newer mutation ends older gestures.
pub struct NativeTextFounderRouteGesture(NativeTextJoinedContactConsent);

impl NativeTextJoinedRowSelection {
    pub fn capture_founder_route_gesture(&self) -> Result<NativeTextFounderRouteGesture, AppError> {
        if !self.is_current_cached() {
            return Err(refused());
        }
        let state = self
            .original
            .founder_route_consent
            .try_lock()
            .map_err(|_| refused())?;
        Ok(NativeTextFounderRouteGesture(
            NativeTextJoinedContactConsent {
                row: self.clone(),
                signal: state.observe(),
            },
        ))
    }
}

struct ApprovedCandidate {
    gesture: NativeTextFounderRouteGesture,
    host: IpAddr,
    reservation: Zeroizing<Vec<u8>>,
    founder: PeerId,
}

/// Paid original dial with no asynchronous Store or runtime loan.
pub struct NativeTextFounderRouteAttempt {
    contact: ApprovedContact,
    signal: RequestCancellation,
    mount: RequestCancellation,
    transport: MeshConnectionDial,
    candidate: Option<ApprovedCandidate>,
    _invocation: watch::Sender<bool>,
    payment: Arc<JoinFramePayment>,
    _end: retirement::PreparationEnd,
}

/// A current outbound socket for the exact Remember target. Dropping or
/// replacing that socket cannot keep the proof live through another connection.
pub struct NativeTextFounderRouteApproval {
    candidate: ApprovedCandidate,
    outbound: AuthenticatedDirectConnection,
    signal: RequestCancellation,
    _invocation: watch::Sender<bool>,
    _payment: Arc<JoinFramePayment>,
}

impl NativeTextFounderRouteAttempt {
    /// Reconnect using a current standing consent. No address is written here.
    pub async fn connect(self, scheduler: &EndpointDialScheduler) -> Result<(), AppError> {
        if self.candidate.is_some() {
            return Err(refused());
        }
        connect_approved_host(
            self.contact,
            &self.transport,
            scheduler,
            self.signal,
            self.mount,
            self.payment,
        )
        .await
    }

    /// Observe the exact currently authenticated socket, not the aggregate
    /// peer lease that may survive through an inbound or different route.
    pub async fn connect_for_approval(
        self,
        scheduler: &EndpointDialScheduler,
    ) -> Result<NativeTextFounderRouteApproval, AppError> {
        let candidate = self.candidate.ok_or_else(refused)?;
        let (peer, address) = self.contact.target_data();
        let address = address.to_owned();
        let allowed = candidate.gesture.0.constrain(self.signal.clone());
        connect_approved_host(
            self.contact,
            &self.transport,
            scheduler,
            allowed.clone(),
            self.mount.clone(),
            self.payment.clone(),
        )
        .await?;
        let mut signal = allowed;
        let mut mount = self.mount;
        let outbound = tokio::select! {
            biased;
            _ = signal.cancelled() => return Err(refused()),
            _ = mount.cancelled() => return Err(refused()),
            result = tokio::time::timeout(
                Duration::from_secs(3),
                self.transport.observe_authenticated_direct_connection(peer, &address),
            ) => result.map_err(|_| refused())?.map_err(|_| refused())?.ok_or_else(refused)?,
        };
        if signal.is_cancelled() || mount.is_cancelled() || !outbound.is_live() {
            return Err(refused());
        }
        Ok(NativeTextFounderRouteApproval {
            candidate,
            outbound,
            signal: self.signal,
            _invocation: self._invocation,
            _payment: self.payment,
        })
    }
}

type Authority = (
    V2MemberPublicationObservation,
    AuthenticationNegative,
    RequestCancellation,
    CatalogReadReservation,
);

impl<R: CryptoRngCore> NativeTextJoinedSelectionPreparation<R> {
    /// The caller supplies the original accepted invite and an explicitly
    /// approved numeric host. Normal Join and peer hint never call this.
    pub fn prepare_remember_founder_route(
        self,
        selection: &NativeTextJoinedRowSelection,
        invite: &NativeTextInvitePacketData<'_>,
        host: IpAddr,
        gesture: NativeTextFounderRouteGesture,
        caller: RequestCancellation,
    ) -> Result<NativeTextFounderRouteAttempt, Refusal> {
        if !gesture.0.is_current_for(selection) {
            return Err(Refusal::Stale);
        }
        let checked = gesture.0.constrain(caller.clone());
        let (contact, authority) = self.with_member_receiver(selection, &checked, |source| {
            if invite.binding().id_data() != source.registration.invite {
                return Err(Refusal::Stale);
            }
            let authority = source.founder_route_authority()?;
            let route = source
                .store
                .check_a1_consent_route(
                    source.registration,
                    source.warm,
                    invite.reservation_bytes(),
                    host,
                )
                .map_err(|_| Refusal::Stale)?;
            Ok((source.founder_route_contact(route)?, authority))
        })?;
        let candidate = ApprovedCandidate {
            gesture,
            host,
            reservation: Zeroizing::new(invite.reservation_bytes().to_vec()),
            founder: invite.connected_peer_data(),
        };
        self.build_founder_route_attempt(selection, caller, contact, authority, Some(candidate))
    }

    /// The row's native caller orders this exact local mutation against UI
    /// Lock and holds the foreground row permit. Failure disables restores
    /// for the original warm row until another explicit durable decision.
    pub fn remember_founder_route(
        self,
        selection: &NativeTextJoinedRowSelection,
        proof: NativeTextFounderRouteApproval,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), Refusal> {
        if !proof.candidate.gesture.0.is_current_for(selection)
            || proof.signal.is_cancelled()
            || !proof.outbound.is_live()
        {
            return Err(Refusal::Stale);
        }
        self.with_member_receiver(selection, caller, |source| {
            let _fee = source
                .metadata
                .reserve_join_node(FOUNDER_CONSENT_FEE)
                .map_err(|_| Refusal::Busy)?;
            let candidate = &proof.candidate;
            let route = source
                .store
                .check_a1_consent_route(
                    source.registration,
                    source.warm,
                    &candidate.reservation,
                    candidate.host,
                )
                .map_err(|_| Refusal::Stale)?;
            if route.peer != candidate.founder {
                return Err(Refusal::Stale);
            }
            source.founder_route_contact(route)?;
            let mut state = selection
                .original
                .founder_route_consent
                .try_lock()
                .map_err(|_| Refusal::Busy)?;
            if !candidate.gesture.0.is_current_for(selection)
                || proof.signal.is_cancelled()
                || caller.is_cancelled()
                || !proof.outbound.is_live()
            {
                return Err(Refusal::Stale);
            }
            // Admission consumes the gesture and retires all older queued
            // attempts. The original exact socket still guards this write.
            state.stop_restoring();
            source
                .store
                .write_a1_founder_consent(
                    source.registration,
                    source.warm,
                    Some((&candidate.reservation, candidate.host)),
                    || {
                        if caller.is_cancelled()
                            || proof.signal.is_cancelled()
                            || !selection.is_current_cached()
                            || !proof.outbound.is_live()
                        {
                            return Err(refused());
                        }
                        Ok(())
                    },
                    rng,
                )
                .map_err(|_| Refusal::Failed)?;
            state.allow_restore = true;
            Ok(())
        })
    }

    /// Invalidate existing and queued restore attempts before storing clear.
    pub fn forget_founder_route(
        self,
        selection: &NativeTextJoinedRowSelection,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), Refusal> {
        self.with_member_receiver(selection, caller, |source| {
            let _fee = source
                .metadata
                .reserve_join_node(FOUNDER_CONSENT_FEE)
                .map_err(|_| Refusal::Busy)?;
            crate::store::critical_namespaces::joined_pin::require_a1_founder_route_profile(
                source.store,
            )
            .map_err(|_| Refusal::Stale)?;
            let mut state = selection
                .original
                .founder_route_consent
                .try_lock()
                .map_err(|_| Refusal::Busy)?;
            if caller.is_cancelled() || !selection.is_current_cached() {
                return Err(Refusal::Stale);
            }
            state.stop_restoring();
            source
                .store
                .write_a1_founder_consent(
                    source.registration,
                    source.warm,
                    None,
                    || {
                        if caller.is_cancelled() || !selection.is_current_cached() {
                            return Err(refused());
                        }
                        Ok(())
                    },
                    rng,
                )
                .map_err(|_| Refusal::Failed)?;
            Ok(())
        })
    }

    /// Recover only this row's current approval and actual live membership.
    /// A torn or corrupt optional file affects this attempt, not the row.
    pub fn prepare_saved_founder_reconnect(
        self,
        selection: &NativeTextJoinedRowSelection,
        caller: RequestCancellation,
    ) -> Result<Option<NativeTextFounderRouteAttempt>, Refusal> {
        let prepared = self.with_member_receiver(selection, &caller, |source| {
            let mut state = selection
                .original
                .founder_route_consent
                .try_lock()
                .map_err(|_| Refusal::Busy)?;
            if !state.allow_restore {
                return Ok(None);
            }
            let authority = source.founder_route_authority()?;
            let Some(route) = source
                .store
                .read_a1_founder_consent(source.registration, source.warm)
                .map_err(|_| Refusal::Unavailable)?
            else {
                // No standing consent to reconsider on this mounted row.
                // A later explicit durable Remember re-enables the worker.
                state.allow_restore = false;
                return Ok(None);
            };
            let contact = source.founder_route_contact(route)?;
            Ok(Some((contact, authority, state.observe())))
        })?;
        prepared
            .map(|(contact, authority, live)| {
                let caller = HistoryContactScope::constrain(live, caller);
                self.build_founder_route_attempt(selection, caller, contact, authority, None)
            })
            .transpose()
    }

    fn build_founder_route_attempt(
        self,
        selection: &NativeTextJoinedRowSelection,
        caller: RequestCancellation,
        contact: ApprovedContact,
        (observed, authentication, mount, fee): Authority,
        candidate: Option<ApprovedCandidate>,
    ) -> Result<NativeTextFounderRouteAttempt, Refusal> {
        let transport = self
            .original
            .tcp
            .upgrade()
            .ok_or(Refusal::Unavailable)?
            .capture_attached_dial()
            .map_err(|_| Refusal::Unavailable)?;
        let (invocation, live) = watch::channel(false);
        let authority = Arc::new(JoinEffect {
            scope: self.original.scope,
            ui: self.original.command.ui.into_effect_negative(),
            authentication,
            invocation: live,
            caller: caller.clone(),
            row: Some(selection.clone()),
        });
        let signal = observed.constrain(caller.with_authority(authority));
        if signal.is_cancelled() || mount.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        Ok(NativeTextFounderRouteAttempt {
            contact,
            signal,
            mount,
            transport,
            candidate,
            _invocation: invocation,
            payment: Arc::new(JoinFramePayment { _fee: fee }),
            _end: self.original.end.ok_or(Refusal::Unavailable)?,
        })
    }
}

impl<R: CryptoRngCore> ReceiverSource<'_, R> {
    fn founder_route_contact(
        &self,
        route: FounderConsentRoute,
    ) -> Result<ApprovedContact, Refusal> {
        let verifier = self
            .prefix
            .selected_member_verifier()
            .map_err(|_| Refusal::Stale)?;
        let membership = verifier.replay_data();
        let anchor = self
            .node
            .require_review_membership_anchor(
                self.prefix,
                (membership.header().id(), membership.mls_epoch_data()),
            )
            .map_err(|_| Refusal::Stale)?;
        let pinned = pinned_founder(anchor);
        let founder = verifier
            .admission(pinned.admission)
            .map_err(|_| Refusal::Stale)?;
        if !founder.is_open_data()
            || founder.admission_data().spec().subject != pinned.founder
            || *self.founder_hint.get_or_init(|| route.peer) != route.peer
        {
            return Err(Refusal::Stale);
        }
        Ok(ApprovedContact::from_selected_member(
            route.peer,
            pinned.group,
            route.address,
        ))
    }

    fn founder_route_authority(&self) -> Result<Authority, Refusal> {
        let (observed, authentication) = self.effect_data()?;
        let (mount, fee) = self
            .metadata
            .joined_transport_resources(FOUNDER_CONSENT_FEE)
            .map_err(|_| Refusal::Busy)?;
        Ok((observed, authentication, mount, fee))
    }
}
