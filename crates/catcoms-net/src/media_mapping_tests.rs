//! Router custody regressions. No sockets or real router state are involved.
use super::*;
use catcoms_rt::ManualClock;
use std::sync::Mutex;

type RouterCalls = Arc<Mutex<Vec<(&'static str, &'static str, u16)>>>;

#[derive(Clone)]
struct Gateway {
    name: &'static str,
    calls: RouterCalls,
    address: Result<IpAddr, String>,
    reject_add: bool,
    slow_add: bool,
    cleanup: Option<Arc<tokio::sync::Semaphore>>,
    slow_address: bool,
    slow_confirmation: bool,
}

impl Gateway {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            calls: Arc::default(),
            address: Ok("8.8.8.8".parse().unwrap()),
            reject_add: false,
            slow_add: false,
            cleanup: None,
            slow_address: false,
            slow_confirmation: false,
        }
    }

    fn record(&self, action: &'static str, port: NonZeroU16) {
        self.calls
            .lock()
            .unwrap()
            .push((self.name, action, port.get()));
    }
}

#[async_trait]
impl MediaGateway for Gateway {
    async fn add_media_port(&self, _: Ipv4Addr, port: NonZeroU16) -> Result<(), String> {
        self.record("add", port);
        if self.slow_add {
            return std::future::pending().await;
        }
        if self.reject_add {
            Err("refused".into())
        } else {
            Ok(())
        }
    }

    async fn external_ip(&self) -> Result<IpAddr, String> {
        if self.slow_address {
            std::future::pending().await
        } else {
            self.address.clone()
        }
    }

    async fn confirmed(&self, _: NonZeroU16) -> bool {
        if self.slow_confirmation {
            std::future::pending().await
        } else {
            true
        }
    }

    async fn remove_media_port(&self, port: NonZeroU16) -> Result<(), String> {
        self.record("remove", port);
        if let Some(gate) = &self.cleanup {
            gate.acquire().await.unwrap().forget();
        }
        Ok(())
    }
}

fn port() -> NonZeroU16 {
    NonZeroU16::new(12_345).unwrap()
}
fn local() -> Ipv4Addr {
    Ipv4Addr::new(192, 168, 1, 20)
}

#[tokio::test]
async fn media_mapping_release_retains_original_gateway_and_external_port() {
    let original = Gateway::new("old-router");
    let new_default = Gateway::new("new-router");
    let (lease, external, confirmed) =
        acquire_upnp_media_lease(original.clone(), local(), port(), &ManualClock::new(0))
            .await
            .unwrap();
    assert_eq!(
        external,
        SocketAddrV4::new("8.8.8.8".parse().unwrap(), port().get())
    );
    assert!(confirmed);
    lease.release().await;
    assert_eq!(
        *original.calls.lock().unwrap(),
        [
            ("old-router", "add", 12_345),
            ("old-router", "remove", 12_345)
        ]
    );
    assert!(new_default.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn media_mapping_bad_address_cleans_acknowledged_add_before_returning_failure() {
    for address in [
        Ok("192.168.1.1".parse().unwrap()),
        Ok("::1".parse().unwrap()),
        Err("query failed".into()),
    ] {
        let gateway = Gateway {
            address,
            ..Gateway::new("router")
        };
        assert!(
            acquire_upnp_media_lease(gateway.clone(), local(), port(), &ManualClock::new(0))
                .await
                .is_err()
        );
        assert_eq!(
            *gateway.calls.lock().unwrap(),
            [("router", "add", 12_345), ("router", "remove", 12_345)]
        );
    }
}

#[tokio::test]
async fn media_mapping_failed_add_does_not_delete_an_unowned_entry() {
    let gateway = Gateway {
        reject_add: true,
        ..Gateway::new("router")
    };
    assert!(
        acquire_upnp_media_lease(gateway.clone(), local(), port(), &ManualClock::new(0))
            .await
            .is_err()
    );
    assert_eq!(*gateway.calls.lock().unwrap(), [("router", "add", 12_345)]);
}

#[tokio::test]
async fn media_mapping_failed_query_retains_custody_until_cleanup_finishes() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let gateway = Gateway {
        address: Err("query failed".into()),
        cleanup: Some(gate.clone()),
        ..Gateway::new("router")
    };
    let clock = ManualClock::new(0);
    let operation = acquire_upnp_media_lease(gateway.clone(), local(), port(), &clock);
    tokio::pin!(operation);
    assert!(futures::poll!(operation.as_mut()).is_pending());
    assert_eq!(gateway.calls.lock().unwrap().len(), 2);
    gate.add_permits(1);
    assert!(operation.await.is_err());
}

#[tokio::test]
async fn media_mapping_address_timeout_still_has_a_separate_cleanup_allowance() {
    let gateway = Gateway {
        slow_address: true,
        ..Gateway::new("router")
    };
    let clock = ManualClock::new(0);
    let operation = acquire_upnp_media_lease(gateway.clone(), local(), port(), &clock);
    tokio::pin!(operation);
    assert!(futures::poll!(operation.as_mut()).is_pending());
    clock.advance_ms(5_000);
    assert!(operation.await.is_err());
    assert_eq!(
        *gateway.calls.lock().unwrap(),
        [("router", "add", 12_345), ("router", "remove", 12_345)]
    );
}

#[tokio::test]
async fn media_mapping_confirmation_timeout_returns_owned_unconfirmed_lease() {
    let gateway = Gateway {
        slow_confirmation: true,
        ..Gateway::new("router")
    };
    let clock = ManualClock::new(0);
    let operation = acquire_upnp_media_lease(gateway.clone(), local(), port(), &clock);
    tokio::pin!(operation);
    assert!(futures::poll!(operation.as_mut()).is_pending());
    clock.advance_ms(5_000);
    let (lease, _, confirmed) = operation.await.unwrap();
    assert!(!confirmed);
    assert_eq!(gateway.calls.lock().unwrap().len(), 1);
    lease.release().await;
    assert_eq!(gateway.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn media_mapping_ambiguous_add_quarantines_without_deleting_an_unowned_entry() {
    let gateway = Gateway {
        slow_add: true,
        ..Gateway::new("router")
    };
    let clock = ManualClock::new(0);
    let operation = acquire_upnp_media_lease(gateway.clone(), local(), port(), &clock);
    tokio::pin!(operation);
    assert!(futures::poll!(operation.as_mut()).is_pending());
    clock.advance_ms(5_000);
    assert!(operation.await.err().unwrap().uncertain);
    assert_eq!(*gateway.calls.lock().unwrap(), [("router", "add", 12_345)]);
}

#[tokio::test]
async fn media_mapping_ambiguous_cleanup_cannot_become_clean_protocol_fallback() {
    let gateway = Gateway {
        address: Err("query failed".into()),
        cleanup: Some(Arc::new(tokio::sync::Semaphore::new(0))),
        ..Gateway::new("router")
    };
    let clock = ManualClock::new(0);
    let operation = acquire_upnp_media_lease(gateway, local(), port(), &clock);
    tokio::pin!(operation);
    assert!(futures::poll!(operation.as_mut()).is_pending());
    clock.advance_ms(5_000);
    assert!(operation.await.err().unwrap().uncertain);
}
