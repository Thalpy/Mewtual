//! Transports for the internet-exposed infra nodes: metered TCP, and the **TCP/443 WebSocket**
//! listener that is rung 4 of the connectivity ladder.
//!
//! ## Why 443
//!
//! Corporate, university and guest networks routinely allow outbound TCP only to 80 and 443, and
//! frequently only to 443 with something that looks like TLS on it. On such a network **every
//! rung of the ladder fails identically**, with the same unactionable timeout: direct dials,
//! hole punching, the rendezvous noticeboard and the relay all ride arbitrary high ports. One
//! listener on `/ip4/0.0.0.0/tcp/443/tls/ws` passes almost every egress filter and every
//! transparent proxy that inspects but does not terminate TLS, and it costs the operator one
//! address and one certificate.
//!
//! ## `/ws` versus `/tls/ws`, honestly
//!
//! `libp2p-websocket` will only *listen* on a TLS WebSocket address if it has been given a server
//! certificate (`framed::Config::set_tls_config`), and a dialing client validates that
//! certificate against the **public web PKI** roots. So:
//!
//! - `/tls/ws` needs a real CA-issued certificate for a real DNS name, and clients must dial the
//!   node by that name (`/dns4/<name>/tcp/443/tls/ws`). A self-signed certificate does not work:
//!   the client's rustls config trusts webpki roots only.
//! - `/ws` (plain WebSocket on port 443) needs nothing, and still defeats a **port**-based egress
//!   filter. It does not defeat a proxy that expects a TLS ClientHello on 443.
//!
//! Both are offered; the operator picks by supplying a certificate or not. Confidentiality does
//! not depend on the choice either way: libp2p runs Noise inside the WebSocket, and everything
//! above that is MLS ciphertext. The outer TLS exists purely to look like the web.

use std::path::Path;

use libp2p::core::muxing::StreamMuxerBox;
use libp2p::core::transport::Boxed;
use libp2p::core::upgrade::Version;
use libp2p::{noise, tcp, yamux, PeerId, Transport};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};

use crate::admission::addr_prefix;
use crate::metering::ByteMeters;
use crate::NetError;

/// The error type the swarm builder's `with_other_transport` accepts from a transport
/// constructor. `libp2p` only implements its `TryIntoTransport` conversion for this exact boxed
/// error, so the constructors below speak it and callers translate back to [`NetError`].
pub(crate) type BuildError = Box<dyn std::error::Error + Send + Sync>;

fn boxed_err(e: NetError) -> BuildError {
    Box::new(e)
}

/// A loaded server-side TLS configuration for the WebSocket listener.
///
/// Deliberately opaque: it wraps `libp2p::websocket::tls::Config`, whose constructor panics on a
/// malformed key. [`load_ws_tls_pem`] is the only way to build one, and it validates first, so an
/// operator typo is an error message instead of an abort.
#[derive(Clone, Debug)]
pub struct WsTlsConfig(libp2p::websocket::tls::Config);

/// Load a PEM certificate chain and private key for the TCP/443 `/tls/ws` listener.
///
/// `cert_pem` is the full chain (leaf first), exactly as a public CA issues it; `key_pem` is the
/// matching private key (PKCS#8, PKCS#1 or SEC1). Both are parsed and validated here so that a
/// bad path or a mismatched pair fails at startup with a message naming the file, rather than
/// half way through the first join attempt of the first user behind a corporate firewall.
pub fn load_ws_tls_pem(cert_pem: &Path, key_pem: &Path) -> Result<WsTlsConfig, NetError> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert_pem)
        .map_err(|e| NetError::Build(format!("reading {}: {e}", cert_pem.display())))?
        .collect::<Result<_, _>>()
        .map_err(|e| NetError::Build(format!("parsing {}: {e}", cert_pem.display())))?;
    if certs.is_empty() {
        return Err(NetError::Build(format!(
            "{} contains no CERTIFICATE blocks",
            cert_pem.display()
        )));
    }
    let key = PrivateKeyDer::from_pem_file(key_pem)
        .map_err(|e| NetError::Build(format!("reading {}: {e}", key_pem.display())))?;

    let ws_key = libp2p::websocket::tls::PrivateKey::new(key.secret_der().to_vec());
    let ws_certs = certs
        .into_iter()
        .map(|c| libp2p::websocket::tls::Certificate::new(c.as_ref().to_vec()));
    let cfg = libp2p::websocket::tls::Config::new(ws_key, ws_certs).map_err(|e| {
        NetError::Build(format!(
            "the certificate in {} and the key in {} are not a usable server pair: {e}",
            cert_pem.display(),
            key_pem.display()
        ))
    })?;
    Ok(WsTlsConfig(cfg))
}

/// TCP + Noise + yamux, with every byte counted against the remote peer.
///
/// The muxer is boxed before metering and again after: the inner box is what makes the metering
/// wrapper's `Unpin` bounds hold without `unsafe`, and the outer box is what the swarm builder
/// wants. Two vtable hops per poll on an infra node that exists to move other people's bytes is
/// a price worth paying for knowing how many bytes those are.
/// `v4_bits`/`v6_bits` must be the same masks [`crate::admission::Admission`] is configured with.
/// The per-prefix counter is what the shed path denies on (a `PeerId` is a free keypair, a prefix
/// costs an attacker addresses), so a meter keyed on a different mask than the denier would charge
/// one bucket and punish another.
pub(crate) fn metered_tcp_transport(
    key: &libp2p::identity::Keypair,
    meters: &ByteMeters,
    v4_bits: u8,
    v6_bits: u8,
) -> Result<Boxed<(PeerId, StreamMuxerBox)>, BuildError> {
    let meters = meters.clone();
    let noise = noise::Config::new(key).map_err(|e| boxed_err(NetError::Build(e.to_string())))?;
    Ok(tcp::tokio::Transport::new(tcp::Config::default())
        .upgrade(Version::V1Lazy)
        .authenticate(noise)
        .multiplex(yamux::Config::default())
        .map(move |(peer, muxer), endpoint| {
            let prefix = addr_prefix(endpoint.get_remote_address(), v4_bits, v6_bits);
            (
                peer,
                StreamMuxerBox::new(meters.meter(peer, prefix, StreamMuxerBox::new(muxer))),
            )
        })
        .boxed())
}

/// WebSocket (optionally TLS) + Noise + yamux, metered the same way.
///
/// Adding this transport does **not** open a port: it only teaches the swarm to understand `/ws`
/// and `/tls/ws` addresses. The listener is opened by the operator passing such an address, which
/// is what keeps rung 4 opt-in (binding 443 needs privilege on Linux).
pub(crate) fn metered_ws_transport(
    key: &libp2p::identity::Keypair,
    meters: &ByteMeters,
    tls: Option<WsTlsConfig>,
    v4_bits: u8,
    v6_bits: u8,
) -> Result<Boxed<(PeerId, StreamMuxerBox)>, BuildError> {
    let meters = meters.clone();
    let noise = noise::Config::new(key).map_err(|e| boxed_err(NetError::Build(e.to_string())))?;
    let mut ws = libp2p::websocket::Config::new(tcp::tokio::Transport::new(tcp::Config::default()));
    if let Some(WsTlsConfig(cfg)) = tls {
        ws.set_tls_config(cfg);
    }
    Ok(ws
        .upgrade(Version::V1Lazy)
        .authenticate(noise)
        .multiplex(yamux::Config::default())
        .map(move |(peer, muxer), endpoint| {
            let prefix = addr_prefix(endpoint.get_remote_address(), v4_bits, v6_bits);
            (
                peer,
                StreamMuxerBox::new(meters.meter(peer, prefix, StreamMuxerBox::new(muxer))),
            )
        })
        .boxed())
}

/// Whether `addr` is a WebSocket listen address (`/ws`, `/wss` or `/tls/ws`).
pub fn is_websocket_addr(addr: &libp2p::Multiaddr) -> bool {
    use libp2p::multiaddr::Protocol;
    addr.iter()
        .any(|p| matches!(p, Protocol::Ws(_) | Protocol::Wss(_)))
}

/// Whether `addr` binds a wildcard ("any") IP, i.e. `0.0.0.0` or `::`.
///
/// A wildcard bind is correct for *listening* and catastrophic for *advertising*: a relay that
/// hands `0.0.0.0` to a client inside a reservation has told it to dial nothing at all, and the
/// client's circuit listener closes with an error the user experiences as a plain timeout.
pub fn is_wildcard_addr(addr: &libp2p::Multiaddr) -> bool {
    use libp2p::multiaddr::Protocol;
    addr.iter().any(|p| match p {
        Protocol::Ip4(v4) => v4.is_unspecified(),
        Protocol::Ip6(v6) => v6.is_unspecified(),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::Multiaddr;

    #[test]
    fn wildcard_and_websocket_addresses_are_recognised() {
        let wild: Multiaddr = "/ip4/0.0.0.0/tcp/4000".parse().unwrap();
        let wild6: Multiaddr = "/ip6/::/tcp/4000".parse().unwrap();
        let real: Multiaddr = "/ip4/198.51.100.7/tcp/4000".parse().unwrap();
        assert!(is_wildcard_addr(&wild));
        assert!(is_wildcard_addr(&wild6));
        assert!(!is_wildcard_addr(&real));
        // Loopback is not a wildcard: it is undialable from outside, but it is exactly right for
        // the loopback and in-process test harnesses, which must keep working unchanged.
        assert!(!is_wildcard_addr(&"/ip4/127.0.0.1/tcp/0".parse().unwrap()));

        assert!(is_websocket_addr(
            &"/ip4/0.0.0.0/tcp/443/tls/ws".parse().unwrap()
        ));
        assert!(is_websocket_addr(
            &"/ip4/0.0.0.0/tcp/443/ws".parse().unwrap()
        ));
        assert!(!is_websocket_addr(&real));
    }
}
