//! Shared bounds for private, locally proven restart hints. These limits confer no authority;
//! callers authenticate evidence before retention and recheck membership before each dial.

pub const MAX_RECONNECT_PEERS: usize = 8;
pub const MAX_RECONNECT_ROUTES_PER_PEER: usize = catcoms_rt::MAX_PEER_DIAL_BATCH;
pub const MAX_RECONNECT_RETAINED_ROUTES: usize =
    MAX_RECONNECT_PEERS * MAX_RECONNECT_ROUTES_PER_PEER;
pub const MAX_RECONNECT_ROUTE_BYTES: usize = 512;
/// Per-route overhead: encoded peer/address length prefixes and peer bytes.
pub const RECONNECT_ROUTE_WIRE_OVERHEAD: usize = 4 + 32 + 4;
/// Includes the one-byte route count as well as every encoded route.
pub const MAX_RECONNECT_SERIALIZED_ROUTE_BYTES: usize = 8 * 1024;
/// Stored diversity does not increase the socket work permitted in any one discovery pass.
pub const MAX_RECONNECT_DIAL_PEERS_PER_PASS: usize = 2;
/// The outbound observation ledger plus the previously retained set; not an unbounded scan.
pub const MAX_RECONNECT_ROUTE_CANDIDATES: usize = catcoms_rt::MAX_CONNECTED_PEER_SNAPSHOT
    * catcoms_rt::MAX_PEER_DIAL_BATCH
    + MAX_RECONNECT_RETAINED_ROUTES;

/// Retain one route per peer before transport redundancy. Input order is priority: the native
/// bridge supplies most-recent successful observations, followed by still-current sealed hints.
/// Thus only exceeding the peer or byte budget evicts a different member; refreshing one peer
/// cannot occupy another peer's slot. The encoder and runtime use this identical bounded policy.
pub fn retain_reconnect_routes(
    routes: impl IntoIterator<Item = ([u8; 32], String)>,
    peer_limit: usize,
) -> Vec<([u8; 32], String)> {
    let peer_limit = peer_limit.min(MAX_RECONNECT_PEERS);
    let mut peers: Vec<([u8; 32], Vec<String>)> = Vec::new();
    for (peer, address) in routes.into_iter().take(MAX_RECONNECT_ROUTE_CANDIDATES) {
        if address.len() > MAX_RECONNECT_ROUTE_BYTES {
            continue;
        }
        let position = peers.iter().position(|(existing, _)| *existing == peer);
        let index = match position {
            Some(index) => index,
            None if peers.len() < peer_limit => {
                peers.push((peer, Vec::new()));
                peers.len() - 1
            }
            None => continue,
        };
        let retained = &mut peers[index].1;
        if retained.len() < MAX_RECONNECT_ROUTES_PER_PEER && !retained.contains(&address) {
            retained.push(address);
        }
    }
    let mut retained = Vec::new();
    let mut bytes = 1;
    for route_index in 0..MAX_RECONNECT_ROUTES_PER_PEER {
        for (peer, routes) in &peers {
            let Some(address) = routes.get(route_index) else {
                continue;
            };
            let encoded = RECONNECT_ROUTE_WIRE_OVERHEAD + address.len();
            if bytes + encoded <= MAX_RECONNECT_SERIALIZED_ROUTE_BYTES {
                retained.push((*peer, address.clone()));
                bytes += encoded;
            }
        }
    }
    retained
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_retention_preserves_diversity_before_redundancy_and_bounds_every_dimension() {
        let candidates = (1..=10).flat_map(|peer| {
            (0..3).map(move |route| {
                (
                    [peer; 32],
                    format!("{route}{}", "x".repeat(MAX_RECONNECT_ROUTE_BYTES - 1)),
                )
            })
        });
        let retained = retain_reconnect_routes(candidates, MAX_RECONNECT_PEERS);
        assert_eq!(
            retained[..MAX_RECONNECT_PEERS]
                .iter()
                .map(|(peer, _)| peer[0])
                .collect::<Vec<_>>(),
            (1..=8).collect::<Vec<_>>()
        );
        for peer in 1..=8 {
            assert!(
                retained.iter().filter(|(id, _)| id[0] == peer).count()
                    <= MAX_RECONNECT_ROUTES_PER_PEER
            );
        }
        assert!(
            1 + retained
                .iter()
                .map(|(_, address)| address.len() + RECONNECT_ROUTE_WIRE_OVERHEAD)
                .sum::<usize>()
                <= MAX_RECONNECT_SERIALIZED_ROUTE_BYTES
        );
        assert!(
            retained.len() < MAX_RECONNECT_RETAINED_ROUTES,
            "the independent byte cap must also constrain worst-case descriptors"
        );
        let refreshed = retain_reconnect_routes(
            [
                ([2; 32], "B-new-TCP".into()),
                ([2; 32], "B-new-QUIC".into()),
                ([3; 32], "C-last-good-private".into()),
            ],
            MAX_RECONNECT_PEERS,
        );
        assert_eq!(
            refreshed
                .iter()
                .map(|(peer, _)| peer[0])
                .collect::<Vec<_>>(),
            vec![2, 3, 2]
        );
        assert_eq!(
            retain_reconnect_routes(refreshed, 1).len(),
            MAX_RECONNECT_ROUTES_PER_PEER
        );
    }
}
