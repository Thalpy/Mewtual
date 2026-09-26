//! Durable continuing member reachability after authenticated P2P admission.
use super::*;

/// Capture only the local transport's successful outbound Noise listener evidence. The sealed
/// P2P policy is a standing retry obligation, including when this launch sees no usable route.
/// Returns false for legacy/dedicated groups, which retain their prior admission restrictions.
pub(super) async fn persist(
    state: &AppState,
    server: u64,
    instance: u64,
    actor: &ServerActor,
    evidence: Vec<AuthenticatedDialRoute>,
) -> Result<bool, String> {
    if actor.group_mode().await? != catcoms_app::GroupMode::PeerToPeer {
        return Ok(false);
    }
    if !actor.member_mesh_allowed().await? {
        // The immutable mode remains P2P after this device is removed. That historical pin
        // grants no standing authority to the now-inactive local MLS instance.
        actor
            .set_local_reconnect_routes(Vec::new())
            .await
            .map_err(|_| "server stopped".to_string())?;
        return Ok(true);
    }
    let mut peers: HashSet<_> = actor.finalized_member_peers().await?.into_iter().collect();
    let local = state
        .servers
        .lock()
        .await
        .get(&server)
        .filter(|entry| entry.instance == instance)
        .and_then(|entry| entry.mesh.as_ref().map(MeshHandle::local_peer));
    // A close can race the post-admission worker before either side pulled a descriptor. Give
    // this already-proven outbound direction two bounded connected-only opportunities. This
    // also lets an actor leave an earlier reciprocal catch-up wait before the second request.
    let targets: std::collections::BTreeSet<_> = evidence
        .iter()
        .map(|route| route.peer)
        .take(MAX_RECONNECT_ROUTES)
        .collect();
    for peer in targets
        .iter()
        .filter(|peer| !peers.contains(peer))
        .copied()
        .collect::<Vec<_>>()
    {
        // Opposite owners must not occupy both sole actor loops with reciprocal requests. Wait
        // outside the actor on one deterministic side, where it can serve the other's request.
        if local.is_some_and(|local| local > peer) {
            SystemClock.sleep(Duration::from_millis(250)).await;
            if actor.finalized_member_peers().await?.contains(&peer) {
                continue;
            }
        }
        for _ in 0..2 {
            if matches!(actor.finalize_member_connection(peer).await, Ok(true)) {
                break;
            }
        }
    }
    peers = actor.finalized_member_peers().await?.into_iter().collect();
    let observed = select_authenticated_reconnect_routes(evidence, &peers, false);
    // The signed policy and member descriptors must reach disk before the route record can grant
    // restart permission. A failed write keeps the periodic/event obligation outstanding.
    if persist_server_instance(state, server, instance, actor.clone()).await
        != PersistOutcome::Durable
    {
        return Err("member reconnect authority could not finish saving".into());
    }
    if !actor.member_mesh_allowed().await? {
        // Removal can race the first permission read or finalization. The core also checks
        // active local membership at each actual dial, including after this final query.
        actor
            .set_local_reconnect_routes(Vec::new())
            .await
            .map_err(|_| "server stopped".to_string())?;
        return Ok(true);
    }
    let guard = state.store.lock().await;
    let store = guard
        .as_ref()
        .ok_or_else(|| "vault is not mounted".to_string())?;
    let registry = state.servers.lock().await;
    if registry
        .get(&server)
        .is_none_or(|entry| entry.instance != instance)
    {
        return Err("group changed during reconnect finalization".into());
    }
    let mut net = store
        .load_server_net(server)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "network identity is not saved yet".to_string())?;
    let old = net.clone();
    net.reconnect_policy = ReconnectPolicy::MemberMesh;
    if !observed.is_empty() {
        // Refresh observed members while retaining other last-good directions. Seeing no live
        // route is expected when the peer stopped, and can never erase its restart hint.
        let refreshed: HashSet<_> = observed.iter().map(|route| route.peer_id).collect();
        let mut routes = observed;
        routes.extend(
            net.reconnect_routes
                .iter()
                .filter(|route| !refreshed.contains(&route.peer_id))
                .cloned(),
        );
        routes.truncate(MAX_RECONNECT_ROUTES);
        net.reconnect_routes = routes;
        if net
            .pending_recovery_peer
            .is_some_and(|peer| refreshed.contains(&peer))
        {
            net.pending_recovery_peer = None;
            net.pending_recovery_expires_at_ms = 0;
        }
    }
    if net != old {
        store
            .save_server_net(server, &net, &mut OsCryptoRng)
            .map_err(|e| e.to_string())?;
    }
    let routes = net
        .reconnect_routes
        .iter()
        .map(|route| (PeerId::new(route.peer_id), route.address.clone()))
        .collect();
    drop(registry);
    drop(guard);
    actor
        .set_local_reconnect_routes(routes)
        .await
        .map_err(|_| "server stopped".to_string())?;
    Ok(true)
}

/// Complete every available outbound observation before any actor is frozen for orderly close.
/// No renderer data or new connection is needed. Root close coordination owns the outer deadline.
pub(super) async fn before_shutdown(state: &AppState) -> Result<(), String> {
    let entries: Vec<_> = state
        .servers
        .lock()
        .await
        .iter()
        .map(|(id, entry)| (*id, entry.instance, entry.actor.clone(), entry.mesh.clone()))
        .collect();
    for (id, instance, actor, mesh) in entries {
        let Some(mesh) = mesh else {
            continue;
        };
        let evidence = mesh.authenticated_dial_route_evidence();
        persist(state, id, instance, &actor, evidence).await?;
    }
    Ok(())
}
