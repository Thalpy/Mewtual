//! Isolated provider construction, actual WebSocket TLS trust/name validation
//! and DNS. No browser, physical network or total TLS resource bound is claimed.
use super::*;
use catcoms_rt::{Clock, SystemClock};
use std::process::{Command, Stdio};

const CHILD: &str = "MEWTUAL_TEST_EXPLICIT_PROTOCOL_PROVIDERS";

#[tokio::test]
async fn explicit_quic_and_websocket_providers_work_in_both_orders_without_global_install() {
    if let Ok(order) = std::env::var(CHILD) {
        assert!(rustls::crypto::CryptoProvider::get_default().is_none());
        let key = keypair_from_seed([0x71; 32]).unwrap();
        match order.as_str() {
            "quic-first" => {
                let _quic = libp2p::quic::Config::new(&key);
                let _ws = libp2p::websocket::tls::Config::client();
            }
            "ws-first" => {
                let _ws = libp2p::websocket::tls::Config::client();
                let _quic = libp2p::quic::Config::new(&key);
            }
            _ => panic!("unknown isolated construction order"),
        }
        assert!(rustls::crypto::CryptoProvider::get_default().is_none());
        return;
    }
    // The test runner may execute unrelated TLS users on other threads. Only
    // fresh child processes can prove that no earlier initializer hid a default
    // provider dependency. Each child runs exactly this test and no compiler.
    for order in ["quic-first", "ws-first"] {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "protocol_provider_tests::explicit_quic_and_websocket_providers_work_in_both_orders_without_global_install", "--nocapture"])
            .env(CHILD, order)
            .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let clock = SystemClock;
        let started = clock.monotonic_ms();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => {
                    let output = child.wait_with_output().unwrap();
                    assert!(
                        output.status.success(),
                        "{order}: {} {}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    assert!(
                        stdout.contains("running 1 test")
                            && stdout.contains("test result: ok. 1 passed;"),
                        "isolated filter did not execute exactly one test: {stdout}"
                    );
                    break;
                }
                Ok(None) if clock.monotonic_ms().saturating_sub(started) < 10_000 => {
                    clock.sleep(Duration::from_millis(10)).await;
                }
                state => {
                    // Always retire this owned child before reporting a watchdog
                    // or process failure. Never leave a second runner behind.
                    let _ = child.kill();
                    let _ = child.wait_with_output();
                    panic!("isolated provider constructor did not finish: {order}: {state:?}");
                }
            }
        }
    }
}

fn certificate(pem: &[u8]) -> libp2p::websocket::tls::Certificate {
    use rustls_pki_types::pem::PemObject;
    let cert = rustls_pki_types::CertificateDer::from_pem_slice(pem).unwrap();
    libp2p::websocket::tls::Certificate::new(cert.as_ref().to_vec())
}
fn server_config(correct_name: bool) -> libp2p::websocket::tls::Config {
    // Public, deterministic test keys, not credentials. RFC8410 PKCS#8 wraps
    // the supplied Ed25519 seed; the fixtures were signed by test CA seed1.
    // Fixed2020..2100 validity avoids runtime certificate generation/time RNG.
    let mut key = vec![
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ];
    key.extend_from_slice(&[if correct_name { 2 } else { 3 }; 32]);
    let cert = if correct_name {
        include_bytes!("../test-data/protocol-tls/localhost.pem").as_slice()
    } else {
        include_bytes!("../test-data/protocol-tls/wrong-name.pem").as_slice()
    };
    libp2p::websocket::tls::Config::new(
        libp2p::websocket::tls::PrivateKey::new(key),
        [certificate(cert)],
    )
    .unwrap()
}
fn client_config(trust_test_ca: bool) -> libp2p::websocket::tls::Config {
    if !trust_test_ca {
        return libp2p::websocket::tls::Config::client();
    }
    // This explicit additional anchor exists ONLY in this test transport. The
    // production client must continue to use public web-PKI roots exclusively.
    let mut builder = libp2p::websocket::tls::Config::builder();
    builder
        .add_trust(&certificate(include_bytes!(
            "../test-data/protocol-tls/ca.pem"
        )))
        .unwrap();
    builder.finish()
}

#[tokio::test]
async fn actual_websocket_tls_preserves_trust_name_checks_and_localhost_dns() {
    use futures::{
        future::poll_fn,
        io::{AsyncReadExt, AsyncWriteExt},
    };
    use libp2p::core::transport::{DialOpts, PortUse, TransportEvent};
    use std::pin::Pin;
    for (trust, correct_name, dns, accepted) in [
        (true, true, false, true),
        (true, true, true, true),
        (false, true, false, false),
        (true, false, false, false),
    ] {
        let phase = async {
            let mut server = libp2p::websocket::Config::new(libp2p::tcp::tokio::Transport::new(
                libp2p::tcp::Config::default(),
            ));
            server.set_tls_config(server_config(correct_name));
            server
                .listen_on(
                    libp2p::core::transport::ListenerId::next(),
                    "/ip4/127.0.0.1/tcp/0/tls/ws".parse().unwrap(),
                )
                .unwrap();
            let address = loop {
                match poll_fn(|cx| Pin::new(&mut server).poll(cx)).await {
                    TransportEvent::NewAddress { listen_addr, .. } => break listen_addr,
                    TransportEvent::ListenerError { error, .. } => {
                        panic!("TLS listener failed: {error}")
                    }
                    _ => {}
                }
            };
            let port = address
                .iter()
                .find_map(|p| {
                    if let libp2p::multiaddr::Protocol::Tcp(port) = p {
                        Some(port)
                    } else {
                        None
                    }
                })
                .unwrap();
            let target = if dns {
                format!("/dns4/localhost/tcp/{port}/tls/ws")
                    .parse()
                    .unwrap()
            } else {
                address
            };
            let mut client = libp2p::websocket::Config::new(libp2p::tcp::tokio::Transport::new(
                libp2p::tcp::Config::default(),
            ));
            client.set_tls_config(client_config(trust));
            // The production outer-DNS composition is unchanged. Exercise the
            // actual upgraded resolver against localhost, without a remote host.
            let mut client = libp2p::dns::tokio::Transport::system(client).unwrap();
            let outbound = client
                .dial(
                    target,
                    DialOpts {
                        role: libp2p::core::Endpoint::Dialer,
                        port_use: PortUse::New,
                    },
                )
                .unwrap();
            let inbound = async {
                loop {
                    match poll_fn(|cx| Pin::new(&mut server).poll(cx)).await {
                        TransportEvent::Incoming { upgrade, .. } => return upgrade.await,
                        TransportEvent::ListenerError { error, .. } => {
                            panic!("TLS listener failed: {error}")
                        }
                        _ => {}
                    }
                }
            };
            let (outbound, inbound) = futures::join!(outbound, inbound);
            if !accepted {
                assert!(
                    outbound.is_err(),
                    "untrusted CA or wrong name passed real TLS"
                );
                return;
            }
            let mut outbound = outbound.unwrap();
            let mut inbound = inbound.unwrap();
            let send = async {
                outbound.write_all(b"tls checked").await.unwrap();
                outbound.flush().await.unwrap();
                let mut reply = [0; 2];
                outbound.read_exact(&mut reply).await.unwrap();
                assert_eq!(&reply, b"ok");
            };
            let receive = async {
                let mut payload = [0; 11];
                inbound.read_exact(&mut payload).await.unwrap();
                assert_eq!(&payload, b"tls checked");
                inbound.write_all(b"ok").await.unwrap();
                inbound.flush().await.unwrap();
            };
            futures::join!(send, receive);
        };
        tokio::select! {
            _ = phase => {}
            _ = SystemClock.sleep(Duration::from_secs(5)) => panic!("WS TLS phase stalled: trust={trust}, name={correct_name}, dns={dns}")
        }
    }
}
