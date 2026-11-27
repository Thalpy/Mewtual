//! One authenticated invitation exchange for a covered replacement Pending.
//! There is no active member or Welcome authority at this boundary.
use super::super::super::native_ui::UiEffectNegative;
use super::*;
use crate::store::persistence::governance_layout::joined::rejoin::PreparedRejoinTransport;
use crate::store::{NativeTextInvitePacketData, NativeTextRejoinPendingReceipt};
use catcoms_rt::{Clock, PeerId, RequestAuthority, SystemClock};

const TRANSPORT_CONTROL_BYTES: usize = 4096;

struct RejoinExchangeAuthority {
    ui: UiEffectNegative,
    mount: RequestCancellation,
    caller: RequestCancellation,
    invocation: tokio::sync::watch::Receiver<bool>,
}

catcoms_rt::redacted_debug! {
    RejoinExchangeAuthority;
    NativeTextRejoinExchange;
    NativeTextRejoinDelivery;
}

#[async_trait::async_trait]
impl RequestAuthority for RejoinExchangeAuthority {
    fn authorized(&self) -> bool {
        self.invocation.has_changed().is_ok()
            && self.ui.is_current_cached()
            && !self.mount.is_cancelled()
            && !self.caller.is_cancelled()
    }

    async fn revoked(&self) {
        let mut invocation = self.invocation.clone();
        let mut caller = self.caller.clone();
        let mut mount = self.mount.clone();
        // A native lock can update the original UI atomics without sending
        // a watch value. There is only one outstanding transport loan.
        while self.authorized() {
            tokio::select! {
                _ = invocation.changed() => return,
                _ = caller.cancelled() => return,
                _ = mount.cancelled() => return,
                _ = SystemClock.sleep(std::time::Duration::from_millis(25)) => {},
            }
        }
    }
}

/// A finite, paid request using a single original replacement transport.
/// The renderer still explicitly approves the invited host before dialing.
pub struct NativeTextRejoinExchange {
    _invocation: tokio::sync::watch::Sender<bool>,
    recovered: catcoms_sync::RecoveredV2PendingJoinSource,
    receipt: NativeTextRejoinPendingReceipt,
    cookie: Arc<()>,
    tcp: join_tcp::Loan,
    target: PeerId,
    signal: RequestCancellation,
    mount: RequestCancellation,
    payment: Arc<JoinFramePayment>,
}

impl NativeTextRemovedJoinedVault {
    /// Prepare a transport loan using only a covered replacement Pending in
    /// the exact authenticated retired vault. No native key is entered here.
    pub fn prepare_rejoin_exchange(
        &mut self,
        ui: UiCapture,
        invite: &NativeTextInvitePacketData<'_>,
        token: [u8; 32],
        caller: RequestCancellation,
    ) -> Result<Option<NativeTextRejoinExchange>, AppError> {
        if !self.tcp_drain.is_drained() {
            return Ok(None);
        }
        self.ui.require_capture(&ui)?;
        let require_current = || {
            if caller.is_cancelled() || !self.ui.is_current_cached(&ui) {
                return Err(refused());
            }
            Ok(())
        };
        require_current()?;
        let PreparedRejoinTransport {
            recovered,
            seed,
            local,
            receipt,
            cookie,
            mount,
            transport_lease,
            pool,
            exchange_fee,
        } = self.index.prepare_rejoin_transport(
            (self.registration, self.removal),
            &mut self.rejoin_pending,
            (invite, token),
        )?;
        if self.rejoin_tcp.is_none() {
            self.rejoin_tcp = Some(Arc::new(join_tcp::Transport::paid(
                pool.try_reserve(TRANSPORT_CONTROL_BYTES)?,
            )));
        }
        let tcp = self.rejoin_tcp.as_ref().ok_or_else(refused)?.checkout(
            local,
            || {
                Ok((
                    seed,
                    catcoms_net::NativeTcpPreparation::new(
                        transport_lease,
                        Arc::new(JoinFramePayment {
                            _fee: pool.try_reserve(catcoms_net::NATIVE_TCP_LIFETIME_BYTES)?,
                        }),
                    ),
                ))
            },
            &require_current,
        )?;
        require_current()?;
        let (invocation, observer) = tokio::sync::watch::channel(false);
        let authority = Arc::new(RejoinExchangeAuthority {
            ui: ui.into_effect_negative(),
            mount: mount.clone(),
            caller: caller.clone(),
            invocation: observer,
        });
        Ok(Some(NativeTextRejoinExchange {
            _invocation: invocation,
            recovered,
            receipt,
            cookie,
            tcp,
            target: invite.connected_peer_data(),
            signal: caller.with_authority(authority),
            mount,
            payment: Arc::new(JoinFramePayment { _fee: exchange_fee }),
        }))
    }
}

impl NativeTextRejoinExchange {
    /// Dial only the original invited peer at this explicitly approved IP.
    pub async fn connect_approved_host(
        &self,
        invite: &NativeTextInvitePacketData<'_>,
        host: std::net::IpAddr,
        scheduler: &catcoms_discovery::EndpointDialScheduler,
    ) -> Result<(), AppError> {
        NativeTextJoinedStorage::require_invite_bytes(self.recovered.request_bytes_data(), invite)?;
        connect_approved_host(
            ApprovedContact::from_invite(invite, host),
            &self.tcp.borrow()?.capture_connection_dial(),
            scheduler,
            self.signal.clone(),
            self.mount.clone(),
            self.payment.clone(),
        )
        .await
    }

    /// Receive only from the original connected peer, then verify the exact
    /// signed request and the selected Remove before returning the delivery.
    pub async fn receive(self) -> Result<NativeTextRejoinDelivery, AppError> {
        let delivery = receive_connected_join(
            self.recovered.request_bytes_data(),
            self.target,
            self.tcp.borrow()?,
            self.signal.clone(),
            self.mount.clone(),
            self.payment.clone(),
        )
        .await?;
        self.recovered
            .validate_pinned_join_delivery_data(&delivery.bytes)
            .map_err(|_| refused())?;
        if delivery.scope.is_cancelled() || self.signal.is_cancelled() {
            return Err(refused());
        }
        Ok(NativeTextRejoinDelivery {
            delivery,
            exchange: self,
        })
    }
}

/// Original connected delivery awaiting a separate covered Welcome Entry.
pub struct NativeTextRejoinDelivery {
    delivery: ConnectedJoinDelivery,
    exchange: NativeTextRejoinExchange,
}

impl NativeTextRejoinDelivery {
    pub fn request_digest_data(&self) -> [u8; 32] {
        self.exchange.receipt.request_digest
    }

    /// The current carrier still belongs to the original retired owner.
    /// Entry must also recheck the actual covered file roles.
    pub fn is_for_original_cached(&self, original: &NativeTextRemovedJoinedVault) -> bool {
        !self.delivery.scope.is_cancelled()
            && !self.exchange.signal.is_cancelled()
            && original
                .rejoin_tcp
                .as_ref()
                .is_some_and(|owner| self.exchange.tcp.is_for_original(owner))
            && original
                .rejoin_pending
                .as_ref()
                .is_some_and(|pending| pending.matches_cookie(&self.exchange.cookie))
    }
}
