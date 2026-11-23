//! One explicitly approved direct host for a covered native join request.
use super::*;
use catcoms_discovery::{parse_peer_dial_route, EndpointDialPermit, EndpointDialScheduler};
use catcoms_rt::{DialPermit, DialSubmission};
use std::{net::IpAddr, time::Duration};

pub(in super::super) struct ApprovedContact {
    peer: catcoms_rt::PeerId,
    group: Vec<u8>,
    address: String,
}

impl ApprovedContact {
    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in super::super) fn for_registered_peer_test(
        peer: catcoms_rt::PeerId,
        group: Vec<u8>,
        address: String,
    ) -> Self {
        Self {
            peer,
            group,
            address,
        }
    }

    pub(in super::super) fn from_invite(
        invite: &crate::store::NativeTextInvitePacketData<'_>,
        approved_host: IpAddr,
    ) -> Self {
        Self {
            peer: invite.connected_peer_data(),
            group: invite.binding().spec_data().anchor.group.clone(),
            address: invite.direct_address_data(approved_host),
        }
    }
}

struct JoinDial {
    permit: EndpointDialPermit,
    caller: RequestCancellation,
    mount: RequestCancellation,
    invocation: tokio::sync::watch::Receiver<bool>,
    _payment: Arc<JoinFramePayment>,
}

impl std::fmt::Debug for JoinDial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JoinDial").finish_non_exhaustive()
    }
}

impl DialPermit for JoinDial {
    fn address(&self) -> &str {
        self.permit.address()
    }

    fn commit_if_current(self: Box<Self>) -> Option<String> {
        let Self {
            permit,
            caller,
            mount,
            invocation,
            _payment,
        } = *self;
        let live =
            || !caller.is_cancelled() && !mount.is_cancelled() && invocation.has_changed().is_ok();
        if !live() {
            return None;
        }
        let address = permit.start()?;
        live().then_some(address)
    }
}

impl NativeTextJoinedStorage {
    fn require_contact_request(
        &self,
        invite: &crate::store::NativeTextInvitePacketData<'_>,
        recovered: &mut Vec<u8>,
    ) -> Result<(), AppError> {
        if self.pending.is_some() {
            return self.require_exact_invite(invite).map(|_| ());
        }
        recovered.clear();
        self.index.copy_recovered_join_request(recovered)?;
        Self::require_invite_bytes(recovered, invite)
    }

    /// Use a trusted caller's explicit direct IP approval with the port and
    /// complete peer ID from the exact covered invite. Successful submission
    /// still needs the invited peer's authenticated transport handshake.
    pub async fn connect_accepted_invite_host(
        &self,
        invite: &crate::store::NativeTextInvitePacketData<'_>,
        approved_host: IpAddr,
        transport: &catcoms_net::MeshService,
        scheduler: &EndpointDialScheduler,
        caller: RequestCancellation,
    ) -> Result<(), AppError> {
        let (mount, fee) = self
            .index
            .joined_exchange_resources(4 * MAX_V2_JOIN_REQUEST_PACKET_BYTES + 8192)?;
        let payment = Arc::new(JoinFramePayment { _fee: fee });
        if mount.is_cancelled() || caller.is_cancelled() {
            return Err(refused());
        }
        let mut recovered =
            zeroize::Zeroizing::new(Vec::with_capacity(if self.pending.is_some() {
                0
            } else {
                MAX_V2_JOIN_REQUEST_PACKET_BYTES
            }));
        self.require_contact_request(invite, &mut recovered)?;
        connect_approved_host(
            ApprovedContact::from_invite(invite, approved_host),
            &transport.capture_connection_dial(),
            scheduler,
            caller.clone(),
            mount.clone(),
            payment.clone(),
        )
        .await?;
        self.require_contact_request(invite, &mut recovered)?;
        if caller.is_cancelled() || mount.is_cancelled() {
            return Err(refused());
        }
        Ok(())
    }
}

pub(in super::super) async fn connect_approved_host(
    contact: ApprovedContact,
    transport: &catcoms_net::MeshConnectionDial,
    scheduler: &EndpointDialScheduler,
    mut caller: RequestCancellation,
    mut mount: RequestCancellation,
    payment: Arc<JoinFramePayment>,
) -> Result<(), AppError> {
    if caller.is_cancelled() || mount.is_cancelled() {
        return Err(refused());
    }
    let ApprovedContact {
        peer,
        group,
        address,
    } = contact;
    let route = parse_peer_dial_route(&address, peer.as_bytes()).ok_or_else(refused)?;
    if catcoms_net::addr_is_undialable(&address.parse().map_err(|_| refused())?) {
        return Err(refused());
    }
    if transport
        .connection_lease(peer)
        .is_some_and(|connection| connection.is_live())
    {
        return if caller.is_cancelled() || mount.is_cancelled() {
            Err(refused())
        } else {
            Ok(())
        };
    }
    let mut permits = scheduler.reserve_permits(&group, &[route.endpoint]);
    let permit = permits.pop().ok_or_else(refused)?;
    // Owning the publisher keeps the queued actor command live only for
    // this invocation. A timeout or a dropped invocation revokes it.
    let (_invocation, live) = tokio::sync::watch::channel(false);
    let dial = JoinDial {
        permit,
        caller: caller.clone(),
        mount: mount.clone(),
        invocation: live,
        _payment: payment.clone(),
    };
    let connect = async {
        if transport
            .dial_permit(Box::new(dial))
            .await
            .map_err(|_| refused())?
            == DialSubmission::FailedAfterCommit
        {
            return Err(refused());
        }
        transport
            .wait_for_peer_connected(peer)
            .await
            .map(|_| ())
            .map_err(|_| refused())
    };
    tokio::select! {
        biased;
        _ = caller.cancelled() => return Err(refused()),
        _ = mount.cancelled() => return Err(refused()),
        result = tokio::time::timeout(Duration::from_secs(10), connect) => result.map_err(|_| refused())??,
    }
    if caller.is_cancelled()
        || mount.is_cancelled()
        || transport
            .connection_lease(peer)
            .is_none_or(|connection| !connection.is_live())
    {
        return Err(refused());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joined_contact_refuses_a_queued_dial_after_its_invocation_retires() {
        let identity = catcoms_net::seeded_transport_identity_data(&[7; 32]).unwrap();
        let address = format!("/ip4/127.0.0.1/tcp/5000/p2p/{}", identity.peer_id_data());
        let endpoint = parse_peer_dial_route(&address, identity.phase0_peer_data().as_bytes())
            .unwrap()
            .endpoint;
        let scheduler = EndpointDialScheduler::new(catcoms_discovery::EndpointDialConfig {
            endpoint_limit: 1,
            ..Default::default()
        });
        let endpoints = [endpoint];
        let mut issued = scheduler.reserve_permits(b"original group", &endpoints);
        let (_caller, caller) = tokio::sync::watch::channel(false);
        let (_mount, mount) = tokio::sync::watch::channel(false);
        let (invocation, live) = tokio::sync::watch::channel(false);
        let pool = CriticalCatalogReadPool::new(4096).unwrap();
        let queued = Box::new(JoinDial {
            permit: issued.pop().unwrap(),
            caller: RequestCancellation::new(caller, None),
            mount: RequestCancellation::new(mount, None),
            invocation: live,
            _payment: Arc::new(JoinFramePayment {
                _fee: pool.try_reserve(4096).unwrap(),
            }),
        });
        assert_eq!(queued.address(), address);
        assert!(scheduler
            .reserve_permits(b"original group", &endpoints)
            .is_empty());
        drop(invocation);
        assert!(queued.commit_if_current().is_none());
        assert_eq!(
            scheduler
                .reserve_permits(b"original group", &endpoints)
                .len(),
            1
        );
    }
}
