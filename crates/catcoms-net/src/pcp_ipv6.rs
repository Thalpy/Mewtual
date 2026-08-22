//! Minimal PCP MAP support for IPv6 firewall pinholes.
//!
//! `portmapper` 0.18 is intentionally retained for IPv4 UPnP/PCP/NAT-PMP because it matches the
//! workspace MSRV, but its public client and mapping types are fixed to `Ipv4Addr`. RFC 6887 uses
//! 128-bit address fields and explicitly supports IPv6 firewalls, so this module implements only
//! the missing IPv6 MAP subset. Keeping the packet boundary small makes it possible to validate
//! nonce, protocol, port, address family, lease timing, and interface scoping before an address is
//! ever offered to libp2p.

use std::net::{Ipv6Addr, SocketAddrV6};
use std::num::NonZeroU16;
use std::time::Duration;

use thiserror::Error;

use crate::addr::ipv6_is_pcp_pinhole_candidate;

pub(crate) const SERVER_PORT: u16 = 5351;
pub(crate) const MAX_PACKET_SIZE: usize = 1100;
/// One byte larger than the accepted protocol bound lets a datagram adapter distinguish an
/// oversized UDP packet from a valid maximum-sized response truncated into its receive buffer.
pub(crate) const RECEIVE_BUFFER_SIZE: usize = MAX_PACKET_SIZE + 1;
// Request a short lease to bound stale firewall exposure after a crash. RFC 6887 lets the router
// choose another lifetime; validated responses are honored up to the separate 24-hour sanity cap.
pub(crate) const REQUESTED_LIFETIME_SECONDS: u32 = 5 * 60;
pub(crate) const MAX_ACCEPTED_LIFETIME_SECONDS: u32 = 24 * 60 * 60;

const VERSION: u8 = 2;
const MAP_OPCODE: u8 = 1;
const RESPONSE_BIT: u8 = 0x80;
const HEADER_SIZE: usize = 24;
const MAP_DATA_SIZE: usize = 36;
const MAP_PACKET_SIZE: usize = HEADER_SIZE + MAP_DATA_SIZE;
const MIN_RETRY_GAP_MS: u64 = 4_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MapProtocol {
    Tcp,
    Udp,
}

impl MapProtocol {
    pub(crate) const fn number(self) -> u8 {
        match self {
            Self::Tcp => 6,
            Self::Udp => 17,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MapLease {
    pub(crate) external_ip: Ipv6Addr,
    pub(crate) external_port: NonZeroU16,
    pub(crate) lifetime_seconds: u32,
    pub(crate) epoch: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecodedResponse {
    Announce {
        result_code: u8,
        lifetime_seconds: u32,
        epoch: u32,
    },
    Map {
        result_code: u8,
        lifetime_seconds: u32,
        epoch: u32,
        nonce: [u8; 12],
        protocol: u8,
        internal_port: u16,
        external_port: u16,
        external_ip: Ipv6Addr,
    },
}

impl DecodedResponse {
    pub(crate) const fn epoch(self) -> u32 {
        match self {
            Self::Announce { epoch, .. } | Self::Map { epoch, .. } => epoch,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(crate) enum PcpIpv6Error {
    #[error("no current interface owns IPv6 listener address {0}")]
    NoInterface(Ipv6Addr),
    #[error("interface for IPv6 listener {0} has no usable IPv6 PCP gateway")]
    NoGateway(Ipv6Addr),
    #[error("could not read the operating system's IPv6 route table: {0}")]
    RouteDiscovery(String),
    #[error("malformed PCP response: {0}")]
    Malformed(&'static str),
    #[error("PCP gateway refused MAP: {reason} (code {code}, retry after {retry_after_seconds}s)")]
    ServerResult {
        code: u8,
        reason: &'static str,
        retry_after_seconds: u32,
    },
    #[error("PCP response nonce does not match this mapping")]
    NonceMismatch,
    #[error("PCP response protocol does not match this mapping")]
    ProtocolMismatch,
    #[error("PCP response internal port does not match this mapping")]
    PortMismatch,
    #[error("PCP gateway returned a zero external port")]
    ZeroExternalPort,
    #[error("PCP gateway returned a zero mapping lifetime")]
    ZeroLifetime,
    #[error("PCP gateway returned non-public or non-IPv6 address {0}")]
    NonGlobalExternal(Ipv6Addr),
    #[error("PCP gateway did not answer within the bounded acquisition window")]
    Timeout,
    #[error("PCP IPv6 socket failed: {0}")]
    Io(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InterfaceRoute {
    index: u32,
    local_addresses: Vec<Ipv6Addr>,
    gateway_addresses: Vec<Ipv6Addr>,
}

/// Locate the IPv6 default router on the exact interface that owns `local_ip`.
///
/// Link-local default routers require a zone/scope identifier on every operating system. Using
/// the interface index from the same snapshot also prevents a mapping request for a Wi-Fi privacy
/// address from accidentally being sent through an Ethernet gateway.
pub(crate) fn discover_gateway(local_ip: Ipv6Addr) -> Result<SocketAddrV6, PcpIpv6Error> {
    let addresses = getifs::interface_ipv6_addrs()
        .map_err(|error| PcpIpv6Error::RouteDiscovery(error.to_string()))?;
    let Some(index) = addresses
        .iter()
        .find(|address| address.addr() == local_ip)
        .map(|address| address.index())
    else {
        return Err(PcpIpv6Error::NoInterface(local_ip));
    };
    let routes = getifs::route_ipv6_table()
        .map_err(|error| PcpIpv6Error::RouteDiscovery(error.to_string()))?;
    // The route's native index is the important part here. In particular, Windows has distinct
    // IPv4 and IPv6 interface indices; inferring a zone from the former breaks IPv6-only adapters
    // and tunnels. Only an actual `::/0` route may nominate the PCP server for this source GUA.
    let route = InterfaceRoute {
        index,
        local_addresses: addresses
            .iter()
            .filter(|address| address.index() == index)
            .map(|address| address.addr())
            .collect(),
        gateway_addresses: routes
            .iter()
            .filter(|route| route.index() == index && route.is_default())
            .filter_map(|route| route.gateway())
            .collect(),
    };
    select_gateway(local_ip, &[route])
}

fn select_gateway(
    local_ip: Ipv6Addr,
    routes: &[InterfaceRoute],
) -> Result<SocketAddrV6, PcpIpv6Error> {
    let Some(interface) = routes
        .iter()
        .find(|route| route.local_addresses.contains(&local_ip))
    else {
        return Err(PcpIpv6Error::NoInterface(local_ip));
    };
    let gateway = interface
        .gateway_addresses
        .iter()
        .copied()
        .find(is_unicast_link_local)
        .or_else(|| {
            interface.gateway_addresses.iter().copied().find(|address| {
                !address.is_unspecified()
                    && !address.is_loopback()
                    && !address.is_multicast()
                    && !is_unique_local(address)
            })
        })
        .ok_or(PcpIpv6Error::NoGateway(local_ip))?;
    let scope_id = if is_unicast_link_local(&gateway) {
        interface.index
    } else {
        0
    };
    Ok(SocketAddrV6::new(gateway, SERVER_PORT, 0, scope_id))
}

fn is_unicast_link_local(address: &Ipv6Addr) -> bool {
    address.segments()[0] & 0xffc0 == 0xfe80
}

fn is_unique_local(address: &Ipv6Addr) -> bool {
    address.segments()[0] & 0xfe00 == 0xfc00
}

/// Encode one RFC 6887 MAP request. Suggested address/port are zero for acquisition and the
/// currently assigned values for renewal/recovery.
pub(crate) fn encode_map_request(
    local_ip: Ipv6Addr,
    local_port: NonZeroU16,
    protocol: MapProtocol,
    nonce: [u8; 12],
    lifetime_seconds: u32,
    suggested: Option<(Ipv6Addr, NonZeroU16)>,
) -> [u8; MAP_PACKET_SIZE] {
    let mut packet = [0u8; MAP_PACKET_SIZE];
    packet[0] = VERSION;
    packet[1] = MAP_OPCODE;
    packet[4..8].copy_from_slice(&lifetime_seconds.to_be_bytes());
    packet[8..24].copy_from_slice(&local_ip.octets());
    packet[24..36].copy_from_slice(&nonce);
    packet[36] = protocol.number();
    packet[40..42].copy_from_slice(&local_port.get().to_be_bytes());
    if let Some((external_ip, external_port)) = suggested {
        packet[42..44].copy_from_slice(&external_port.get().to_be_bytes());
        packet[44..60].copy_from_slice(&external_ip.octets());
    }
    packet
}

pub(crate) fn decode_response(packet: &[u8]) -> Result<DecodedResponse, PcpIpv6Error> {
    if packet.len() < HEADER_SIZE || packet.len() > MAX_PACKET_SIZE {
        return Err(PcpIpv6Error::Malformed(
            "packet length is outside PCP bounds",
        ));
    }
    if packet.len() % 4 != 0 {
        return Err(PcpIpv6Error::Malformed(
            "packet length is not a multiple of four bytes",
        ));
    }
    if packet[0] != VERSION {
        return Err(PcpIpv6Error::Malformed("unsupported version"));
    }
    if packet[1] & RESPONSE_BIT == 0 {
        return Err(PcpIpv6Error::Malformed("packet is not a response"));
    }
    let opcode = packet[1] & !RESPONSE_BIT;
    let result_code = packet[3];
    let lifetime_seconds = u32::from_be_bytes(packet[4..8].try_into().expect("four bytes"));
    let epoch = u32::from_be_bytes(packet[8..12].try_into().expect("four bytes"));
    match opcode {
        // RFC 6887 requires clients to ignore response options they do not understand. Base
        // fields remain strictly bound below; any aligned bytes after the base response are
        // therefore intentionally ignored rather than making an otherwise valid lease fail.
        0 => Ok(DecodedResponse::Announce {
            result_code,
            lifetime_seconds,
            epoch,
        }),
        MAP_OPCODE => {
            if packet.len() < MAP_PACKET_SIZE {
                return Err(PcpIpv6Error::Malformed(
                    "MAP response is shorter than 60 bytes",
                ));
            }
            Ok(DecodedResponse::Map {
                result_code,
                lifetime_seconds,
                epoch,
                nonce: packet[24..36].try_into().expect("twelve bytes"),
                protocol: packet[36],
                internal_port: u16::from_be_bytes(packet[40..42].try_into().expect("two bytes")),
                external_port: u16::from_be_bytes(packet[42..44].try_into().expect("two bytes")),
                external_ip: Ipv6Addr::from(
                    <[u8; 16]>::try_from(&packet[44..60]).expect("16 bytes"),
                ),
            })
        }
        _ => Err(PcpIpv6Error::Malformed("unsupported opcode")),
    }
}

pub(crate) fn validate_map_response(
    response: DecodedResponse,
    nonce: [u8; 12],
    protocol: MapProtocol,
    local_port: NonZeroU16,
) -> Result<MapLease, PcpIpv6Error> {
    let DecodedResponse::Map {
        result_code,
        lifetime_seconds,
        epoch,
        nonce: received_nonce,
        protocol: received_protocol,
        internal_port,
        external_port,
        external_ip,
    } = response
    else {
        return Err(PcpIpv6Error::Malformed(
            "ANNOUNCE response received for MAP request",
        ));
    };
    // Error responses are attacker-influenced UDP too. Bind them to this request before using the
    // result code or its retry-after value; accepting a short/unmatched error would let stray PCP
    // traffic suppress a legitimate mapping attempt.
    if received_nonce != nonce {
        return Err(PcpIpv6Error::NonceMismatch);
    }
    if received_protocol != protocol.number() {
        return Err(PcpIpv6Error::ProtocolMismatch);
    }
    if internal_port != local_port.get() {
        return Err(PcpIpv6Error::PortMismatch);
    }
    if result_code != 0 {
        return Err(server_result(result_code, lifetime_seconds));
    }
    let external_port = NonZeroU16::new(external_port).ok_or(PcpIpv6Error::ZeroExternalPort)?;
    if lifetime_seconds == 0 {
        return Err(PcpIpv6Error::ZeroLifetime);
    }
    if external_ip.to_ipv4_mapped().is_some() || !ipv6_is_pcp_pinhole_candidate(&external_ip) {
        return Err(PcpIpv6Error::NonGlobalExternal(external_ip));
    }
    Ok(MapLease {
        external_ip,
        external_port,
        lifetime_seconds: lifetime_seconds.min(MAX_ACCEPTED_LIFETIME_SECONDS),
        epoch,
    })
}

fn server_result(code: u8, retry_after_seconds: u32) -> PcpIpv6Error {
    PcpIpv6Error::ServerResult {
        code,
        reason: match code {
            1 => "unsupported PCP version",
            2 => "operation not authorized",
            3 => "malformed request",
            4 => "unsupported opcode",
            5 => "unsupported mandatory option",
            6 => "malformed option",
            7 => "network failure",
            8 => "insufficient gateway resources",
            9 => "unsupported transport protocol",
            10 => "subscriber mapping quota exceeded",
            11 => "suggested external address unavailable",
            12 => "request source and declared client address differ",
            13 => "too many remote peers",
            _ => "unknown result",
        },
        retry_after_seconds: retry_after_seconds.min(MAX_ACCEPTED_LIFETIME_SECONDS),
    }
}

/// First renewal is uniformly distributed across the RFC's 1/2..5/8 lease window.
pub(crate) fn first_renewal_delay(lifetime_seconds: u32, random: u8) -> Duration {
    let lifetime_ms = u64::from(lifetime_seconds) * 1_000;
    let jitter_window = lifetime_ms / 8;
    let jitter = jitter_window.saturating_mul(u64::from(random)) / u64::from(u8::MAX);
    Duration::from_millis((lifetime_ms / 2).saturating_add(jitter))
}

/// Retry points after an unanswered renewal approach expiry without ever scheduling requests less
/// than four seconds apart. `attempt` 0, 1, and 2 correspond to 3/4, 7/8, and 15/16.
pub(crate) fn next_retry_at_ms(
    granted_at_ms: u64,
    lifetime_seconds: u32,
    previous_attempt_ms: u64,
    attempt: u8,
) -> Option<u64> {
    let lifetime_ms = u64::from(lifetime_seconds) * 1_000;
    let expires_at = granted_at_ms.saturating_add(lifetime_ms);
    let denominator = 1u64.checked_shl(u32::from(attempt).saturating_add(2))?;
    let numerator = denominator.saturating_sub(1);
    let candidate =
        granted_at_ms.saturating_add(lifetime_ms.saturating_mul(numerator) / denominator);
    let candidate = candidate.max(previous_attempt_ms.saturating_add(MIN_RETRY_GAP_MS));
    (candidate < expires_at).then_some(candidate)
}

/// RFC 6887 epoch validation. A backwards jump larger than one second, or server/client elapsed
/// time differing by more than the specified tolerance, indicates likely gateway state loss.
pub(crate) fn epoch_may_have_reset(
    previous_epoch: u32,
    previous_received_ms: u64,
    current_epoch: u32,
    current_received_ms: u64,
) -> bool {
    if current_epoch.saturating_add(1) < previous_epoch {
        return true;
    }
    let client_delta = current_received_ms.saturating_sub(previous_received_ms) / 1_000;
    let server_delta = u64::from(current_epoch.saturating_sub(previous_epoch));
    client_delta.saturating_add(2) < server_delta.saturating_sub(server_delta / 16)
        || server_delta.saturating_add(2) < client_delta.saturating_sub(client_delta / 16)
}

/// Pure lease timer used by the async adapter. Keeping renewal/expiry transitions independent of
/// sockets makes the failure-prone part deterministic under unit tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LeaseSchedule {
    granted_at_ms: u64,
    lifetime_seconds: u32,
    next_send_ms: u64,
    retry_attempt: u8,
}

impl LeaseSchedule {
    pub(crate) fn new(granted_at_ms: u64, lifetime_seconds: u32, random: u8) -> Self {
        Self {
            granted_at_ms,
            lifetime_seconds,
            next_send_ms: granted_at_ms
                .saturating_add(first_renewal_delay(lifetime_seconds, random).as_millis() as u64),
            retry_attempt: 0,
        }
    }

    pub(crate) fn expires_at_ms(self) -> u64 {
        self.granted_at_ms
            .saturating_add(u64::from(self.lifetime_seconds).saturating_mul(1_000))
    }

    pub(crate) fn next_wake_ms(self) -> u64 {
        self.next_send_ms.min(self.expires_at_ms())
    }

    pub(crate) fn is_expired(self, now_ms: u64) -> bool {
        now_ms >= self.expires_at_ms()
    }

    pub(crate) fn renewal_sent(&mut self, now_ms: u64) {
        self.next_send_ms = next_retry_at_ms(
            self.granted_at_ms,
            self.lifetime_seconds,
            now_ms,
            self.retry_attempt,
        )
        .unwrap_or_else(|| self.expires_at_ms());
        self.retry_attempt = self.retry_attempt.saturating_add(1);
    }

    /// Respect a PCP server's retry horizon without changing the deadline of the lease that was
    /// already granted. An error response to a renewal is not a revocation.
    pub(crate) fn defer_retry(&mut self, now_ms: u64, retry_after: Duration) {
        let requested =
            now_ms.saturating_add(retry_after.as_millis().try_into().unwrap_or(u64::MAX));
        self.next_send_ms = requested.min(self.expires_at_ms());
    }

    pub(crate) fn renew_after(&mut self, now_ms: u64, delay: Duration) {
        self.next_send_ms = now_ms
            .saturating_add(delay.as_millis().try_into().unwrap_or(u64::MAX))
            .min(self.expires_at_ms());
    }

    pub(crate) fn renewed(&mut self, now_ms: u64, lifetime_seconds: u32, random: u8) {
        *self = Self::new(now_ms, lifetime_seconds, random);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn success_response(
        nonce: [u8; 12],
        protocol: MapProtocol,
        internal_port: u16,
        external_port: u16,
        external_ip: Ipv6Addr,
        lifetime_seconds: u32,
        epoch: u32,
    ) -> Vec<u8> {
        let mut packet = vec![0u8; MAP_PACKET_SIZE];
        packet[0] = VERSION;
        packet[1] = RESPONSE_BIT | MAP_OPCODE;
        packet[4..8].copy_from_slice(&lifetime_seconds.to_be_bytes());
        packet[8..12].copy_from_slice(&epoch.to_be_bytes());
        packet[24..36].copy_from_slice(&nonce);
        packet[36] = protocol.number();
        packet[40..42].copy_from_slice(&internal_port.to_be_bytes());
        packet[42..44].copy_from_slice(&external_port.to_be_bytes());
        packet[44..60].copy_from_slice(&external_ip.octets());
        packet
    }

    #[test]
    fn map_request_encodes_the_ipv6_client_nonce_and_transport_exactly() {
        let local = "2606:4700::10".parse().unwrap();
        let external = "2606:4700::20".parse().unwrap();
        let port = NonZeroU16::new(22487).unwrap();
        let nonce = [0x5a; 12];
        let packet = encode_map_request(
            local,
            port,
            MapProtocol::Udp,
            nonce,
            3_600,
            Some((external, port)),
        );
        assert_eq!(packet.len(), 60);
        assert_eq!(&packet[..8], &[2, 1, 0, 0, 0, 0, 14, 16]);
        assert_eq!(&packet[8..24], &local.octets());
        assert_eq!(&packet[24..36], &nonce);
        assert_eq!(packet[36], 17);
        assert_eq!(
            u16::from_be_bytes(packet[40..42].try_into().unwrap()),
            22487
        );
        assert_eq!(
            u16::from_be_bytes(packet[42..44].try_into().unwrap()),
            22487
        );
        assert_eq!(&packet[44..60], &external.octets());
    }

    #[test]
    fn map_response_is_bound_to_nonce_protocol_port_and_public_ipv6() {
        let nonce = [7; 12];
        let port = NonZeroU16::new(22487).unwrap();
        let external = "2606:4700::1111".parse().unwrap();
        let response = decode_response(&success_response(
            nonce,
            MapProtocol::Tcp,
            port.get(),
            port.get(),
            external,
            3_600,
            91,
        ))
        .unwrap();
        assert_eq!(
            validate_map_response(response, nonce, MapProtocol::Tcp, port).unwrap(),
            MapLease {
                external_ip: external,
                external_port: port,
                lifetime_seconds: 3_600,
                epoch: 91,
            }
        );

        let wrong_nonce = decode_response(&success_response(
            [8; 12],
            MapProtocol::Tcp,
            port.get(),
            port.get(),
            external,
            3_600,
            91,
        ))
        .unwrap();
        assert_eq!(
            validate_map_response(wrong_nonce, nonce, MapProtocol::Tcp, port),
            Err(PcpIpv6Error::NonceMismatch)
        );
        let wrong_protocol = decode_response(&success_response(
            nonce,
            MapProtocol::Udp,
            port.get(),
            port.get(),
            external,
            3_600,
            91,
        ))
        .unwrap();
        assert_eq!(
            validate_map_response(wrong_protocol, nonce, MapProtocol::Tcp, port),
            Err(PcpIpv6Error::ProtocolMismatch)
        );
        let wrong_port = decode_response(&success_response(
            nonce,
            MapProtocol::Tcp,
            port.get() + 1,
            port.get(),
            external,
            3_600,
            91,
        ))
        .unwrap();
        assert_eq!(
            validate_map_response(wrong_port, nonce, MapProtocol::Tcp, port),
            Err(PcpIpv6Error::PortMismatch)
        );
        for refused in [
            "100::1",
            "2001:1::1",
            "2001:db8::1",
            "2002::1",
            "3fff::1",
            "fd00::1",
            "fe80::1",
            "::ffff:203.0.113.7",
        ] {
            let refused = refused.parse().unwrap();
            let response = decode_response(&success_response(
                nonce,
                MapProtocol::Tcp,
                port.get(),
                port.get(),
                refused,
                3_600,
                91,
            ))
            .unwrap();
            assert!(matches!(
                validate_map_response(response, nonce, MapProtocol::Tcp, port),
                Err(PcpIpv6Error::NonGlobalExternal(_))
            ));
        }
    }

    #[test]
    fn gateway_selection_is_exactly_interface_scoped() {
        let wifi_ip = "2606:4700::10".parse().unwrap();
        let ethernet_ip = "2001:4860::10".parse().unwrap();
        let routes = vec![
            InterfaceRoute {
                index: 4,
                local_addresses: vec![wifi_ip],
                gateway_addresses: vec!["fe80::1".parse().unwrap()],
            },
            InterfaceRoute {
                index: 9,
                local_addresses: vec![ethernet_ip],
                gateway_addresses: vec!["fe80::2".parse().unwrap()],
            },
        ];
        assert_eq!(
            select_gateway(ethernet_ip, &routes).unwrap(),
            SocketAddrV6::new("fe80::2".parse().unwrap(), 5351, 0, 9)
        );
        assert_eq!(
            select_gateway("2606:4700::99".parse().unwrap(), &routes),
            Err(PcpIpv6Error::NoInterface("2606:4700::99".parse().unwrap()))
        );
    }

    #[test]
    fn renewal_and_epoch_rules_are_bounded() {
        assert_eq!(first_renewal_delay(3_600, 0), Duration::from_secs(1_800));
        assert_eq!(
            first_renewal_delay(3_600, u8::MAX),
            Duration::from_secs(2_250)
        );
        assert_eq!(next_retry_at_ms(1_000, 100, 51_000, 0), Some(76_000));
        assert_eq!(next_retry_at_ms(1_000, 8, 6_000, 2), None);
        assert!(epoch_may_have_reset(900, 1_000, 10, 2_000));
        assert!(!epoch_may_have_reset(900, 1_000, 901, 2_000));

        let mut schedule = LeaseSchedule::new(1_000, 100, 0);
        assert_eq!(schedule.next_wake_ms(), 51_000);
        schedule.renewal_sent(51_000);
        assert_eq!(schedule.next_wake_ms(), 76_000);
        schedule.defer_retry(52_000, Duration::from_secs(40));
        assert_eq!(schedule.next_wake_ms(), 92_000);
        assert!(!schedule.is_expired(100_999));
        assert!(schedule.is_expired(101_000));
        schedule.renewed(80_000, 300, u8::MAX);
        assert_eq!(schedule.next_wake_ms(), 267_500);
        assert_eq!(schedule.expires_at_ms(), 380_000);
    }

    #[test]
    fn malformed_and_refused_responses_keep_scoped_details() {
        assert!(matches!(
            decode_response(&[2, RESPONSE_BIT | MAP_OPCODE]),
            Err(PcpIpv6Error::Malformed(_))
        ));
        let mut refused = success_response(
            [9; 12],
            MapProtocol::Tcp,
            22487,
            22487,
            "2606:4700::1".parse().unwrap(),
            60,
            1,
        );
        refused[3] = 2;
        let refused = decode_response(&refused).unwrap();
        assert_eq!(
            validate_map_response(
                refused,
                [9; 12],
                MapProtocol::Tcp,
                NonZeroU16::new(22487).unwrap()
            ),
            Err(PcpIpv6Error::ServerResult {
                code: 2,
                reason: "operation not authorized",
                retry_after_seconds: 60,
            })
        );
        let mut short = vec![0u8; HEADER_SIZE];
        short[0] = VERSION;
        short[1] = RESPONSE_BIT | MAP_OPCODE;
        short[3] = 2;
        assert!(matches!(
            decode_response(&short),
            Err(PcpIpv6Error::Malformed(
                "MAP response is shorter than 60 bytes"
            ))
        ));
        let mut unaligned = success_response(
            [1; 12],
            MapProtocol::Tcp,
            22487,
            22487,
            "2606:4700::1".parse().unwrap(),
            60,
            1,
        );
        unaligned.push(0);
        assert!(decode_response(&unaligned).is_err());
    }

    #[test]
    fn response_options_are_ignored_but_must_fit_aligned_protocol_bounds() {
        let nonce = [3; 12];
        let mut map = success_response(
            nonce,
            MapProtocol::Udp,
            22487,
            22487,
            "2606:4700::1".parse().unwrap(),
            300,
            8,
        );
        // An unknown optional zero-length option occupies its aligned four-byte header. The
        // client does not interpret it, as required for forward-compatible PCP responses.
        map.extend_from_slice(&[64, 0, 0, 0]);
        assert!(matches!(
            decode_response(&map),
            Ok(DecodedResponse::Map { .. })
        ));

        let mut announce = vec![0u8; HEADER_SIZE + 4];
        announce[0] = VERSION;
        announce[1] = RESPONSE_BIT;
        announce[HEADER_SIZE..].copy_from_slice(&[64, 0, 0, 0]);
        assert!(matches!(
            decode_response(&announce),
            Ok(DecodedResponse::Announce { .. })
        ));

        map.resize(MAX_PACKET_SIZE + 4, 0);
        assert!(decode_response(&map).is_err());
    }
}
