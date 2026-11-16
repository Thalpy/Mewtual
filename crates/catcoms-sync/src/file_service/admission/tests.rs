use super::*;
use crate::file_service::read::MAX_SOURCE_CHUNK;
use crate::file_service::wire::{GrantFields, Window as FileWindow};
use catcoms_mls::MlsDevice;
use catcoms_rt::ManualClock;
use catcoms_storage::Cid;

struct Fixture {
    c: MlsDevice,
    b: MlsDevice,
    a: MlsDevice,
    d: MlsDevice,
}
impl Fixture {
    fn new() -> Self {
        Self {
            c: MlsDevice::generate().unwrap(),
            b: MlsDevice::generate().unwrap(),
            a: MlsDevice::generate().unwrap(),
            d: MlsDevice::generate().unwrap(),
        }
    }
    fn peer(device: &MlsDevice, n: u64) -> ServicePeer {
        ServicePeer {
            device: device.device_id(),
            peer: PeerId::from_u64(n),
        }
    }
    fn raw(
        &self,
        helper: &MlsDevice,
        helper_peer: u64,
        nonce: u8,
        created: u64,
        max: usize,
    ) -> Vec<u8> {
        GrantFields {
            group: b"g",
            epoch: 7,
            helper: helper.device_id(),
            helper_peer: PeerId::from_u64(helper_peer),
            requester_peer: PeerId::from_u64(1),
            window: FileWindow {
                cid: Cid::of(b"ciphertext"),
                max_cipher_bytes: max as u32,
                offset: 0,
                length: 1,
            },
            sources: &[self.a.device_id()],
            created_ms: created,
            nonce: [nonce; 16],
        }
        .issue(&self.c)
        .unwrap()
    }
    fn helper(
        &self,
        runtime: &AdmissionRuntime,
        raw: &[u8],
    ) -> Result<VerifiedGrant, AdmissionError> {
        let ticket = runtime.admit_ingress(
            ServiceScope {
                group: b"g",
                epoch: 7,
                local: Self::peer(&self.b, 2),
            },
            Self::peer(&self.c, 1),
            Role::Helper,
            raw,
        )?;
        runtime.verify(ticket, raw)
    }
    fn source(
        &self,
        runtime: &AdmissionRuntime,
        helper: &MlsDevice,
        peer: u64,
        raw: &[u8],
    ) -> Result<VerifiedGrant, AdmissionError> {
        let grant = Grant::parse(raw).unwrap();
        let delivery =
            Delivery::issue(&grant, self.a.device_id(), PeerId::from_u64(3), helper).unwrap();
        let ticket = runtime.admit_ingress(
            ServiceScope {
                group: b"g",
                epoch: 7,
                local: Self::peer(&self.a, 3),
            },
            Self::peer(helper, peer),
            Role::Source,
            &delivery,
        )?;
        runtime.verify(ticket, &delivery)
    }
}
fn runtime() -> (Arc<ManualClock>, Arc<AdmissionRuntime>) {
    let clock = Arc::new(ManualClock::new(1000));
    (clock.clone(), Arc::new(AdmissionRuntime::new(clock)))
}
fn work(runtime: &AdmissionRuntime, device: DeviceId) -> u64 {
    runtime
        .state
        .lock()
        .unwrap()
        .work_device
        .rows
        .get(&(b"g".as_slice().into(), device))
        .map_or(0, |row| row.used)
}

#[test]
fn helper_and_source_admit_the_same_original_once_each_and_charge_aggregate_work_once() {
    let (_, r) = runtime();
    let f = Fixture::new();
    let raw = f.raw(&f.b, 2, 1, 1000, 100);
    let b = f.helper(&r, &raw).unwrap();
    r.preflight(&b).unwrap();
    r.commit(&b).unwrap();
    let a = f.source(&r, &f.b, 2, &raw).unwrap();
    r.preflight(&a).unwrap();
    r.commit(&a).unwrap();
    assert_eq!(r.state.lock().unwrap().replay.len(), 2);
    assert_eq!(work(&r, f.c.device_id()), b.work_bytes + a.work_bytes);
    assert_eq!(work(&r, f.b.device_id()), a.work_bytes);
    assert_eq!(
        r.state.lock().unwrap().work_process.rows[&()].used,
        b.work_bytes + a.work_bytes
    );
    let replay = f.helper(&r, &raw).unwrap();
    assert_eq!(r.commit(&replay), Err(AdmissionError::Replay));
    let mut changed = GrantFields {
        group: b"g",
        epoch: 7,
        helper: f.b.device_id(),
        helper_peer: PeerId::from_u64(2),
        requester_peer: PeerId::from_u64(1),
        window: b.grant().window(),
        sources: &[f.a.device_id()],
        created_ms: 1000,
        nonce: [1; 16],
    };
    changed.window.cid = Cid::of(b"other");
    let changed = changed.issue(&f.c).unwrap();
    assert_eq!(
        r.commit(&f.helper(&r, &changed).unwrap()),
        Err(AdmissionError::NonceChanged)
    );
}

#[test]
fn invalid_signatures_spend_only_the_actual_sender_ingress_allowance() {
    let (_, r) = runtime();
    let f = Fixture::new();
    let mut raw = f.raw(&f.b, 2, 1, 1000, 100);
    *raw.last_mut().unwrap() ^= 1;
    for _ in 0..8 {
        assert!(matches!(f.helper(&r, &raw), Err(AdmissionError::Signature)));
    }
    assert!(matches!(f.helper(&r, &raw), Err(AdmissionError::Busy)));
    let state = r.state.lock().unwrap();
    assert!(state.replay.is_empty());
    assert!(state.work_device.rows.is_empty());
    assert_eq!(
        state.ingress_device.rows[&(b"g".as_slice().into(), f.c.device_id())].used,
        8
    );
}

#[test]
fn byte_refusal_is_atomic_for_both_identities_all_aggregates_and_the_nonce() {
    for exhaust_original in [true, false] {
        let (_, r) = runtime();
        let f = Fixture::new();
        let donor = Fixture {
            c: MlsDevice::generate().unwrap(),
            b: f.b.duplicate().unwrap(),
            a: f.a.duplicate().unwrap(),
            d: f.d.duplicate().unwrap(),
        };
        let spending = if exhaust_original { &f } else { &donor };
        for nonce in 0..7 {
            let raw = spending.raw(&f.b, 2, nonce, 1000, MAX_SOURCE_CHUNK);
            r.commit(&spending.source(&r, &f.b, 2, &raw).unwrap())
                .unwrap();
        }
        let original = work(&r, f.c.device_id());
        let helper = if exhaust_original { &f.d } else { &f.b };
        let peer = if exhaust_original { 4 } else { 2 };
        let helper_used = work(&r, helper.device_id());
        let aggregate = r.state.lock().unwrap().work_process.rows[&()].used;
        let raw = f.raw(helper, peer, 8, 1000, MAX_SOURCE_CHUNK);
        let proof = f.source(&r, helper, peer, &raw).unwrap();
        assert_eq!(r.preflight(&proof), Err(AdmissionError::Busy));
        assert_eq!(r.commit(&proof), Err(AdmissionError::Busy));
        assert_eq!(
            work(&r, helper.device_id()),
            helper_used,
            "the fitting identity must not be partially charged"
        );
        assert_eq!(work(&r, f.c.device_id()), original);
        assert_eq!(
            r.state.lock().unwrap().work_process.rows[&()].used,
            aggregate
        );
        assert_eq!(r.state.lock().unwrap().replay.len(), 7);
    }
}

#[test]
fn process_and_group_ingress_pressure_leave_another_group_its_share() {
    let (clock, r) = runtime();
    let local = ServicePeer {
        device: DeviceId::from_bytes([200; 32]),
        peer: PeerId::from_u64(200),
    };
    let sender = |n| ServicePeer {
        device: DeviceId::from_bytes([n; 32]),
        peer: PeerId::from_u64(n as u64),
    };
    for group in [b"one".as_slice(), b"two".as_slice()] {
        for device in [1, 2] {
            for _ in 0..8 {
                r.admit_ingress(
                    ServiceScope {
                        group,
                        epoch: 7,
                        local,
                    },
                    sender(device),
                    Role::Helper,
                    &[29],
                )
                .unwrap();
            }
        }
        assert!(matches!(
            r.admit_ingress(
                ServiceScope {
                    group,
                    epoch: 7,
                    local
                },
                sender(3),
                Role::Helper,
                &[29]
            ),
            Err(AdmissionError::Busy)
        ));
    }
    assert!(matches!(
        r.admit_ingress(
            ServiceScope {
                group: b"three",
                epoch: 7,
                local
            },
            sender(3),
            Role::Helper,
            &[29]
        ),
        Err(AdmissionError::Busy)
    ));
    clock.advance_ms(1000);
    r.admit_ingress(
        ServiceScope {
            group: b"three",
            epoch: 7,
            local,
        },
        sender(3),
        Role::Helper,
        &[29],
    )
    .unwrap();
}

#[test]
fn racing_verified_admissions_recheck_and_publish_exactly_one_work_transaction() {
    let (_, r) = runtime();
    let f = Fixture::new();
    let raw = f.raw(&f.b, 2, 1, 1000, 100);
    let first = f.helper(&r, &raw).unwrap();
    let second = f.helper(&r, &raw).unwrap();
    let bytes = first.work_bytes;
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let handles = [first, second]
        .into_iter()
        .map(|proof| {
            let runtime = r.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                runtime.preflight(&proof).unwrap();
                barrier.wait();
                runtime.commit(&proof)
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| **result == Err(AdmissionError::Replay))
            .count(),
        1
    );
    assert_eq!(work(&r, f.c.device_id()), bytes);
    assert_eq!(r.state.lock().unwrap().replay.len(), 1);
}

#[test]
fn protective_full_tables_refuse_without_eviction_or_partial_work() {
    for full_group in [true, false] {
        let (_, r) = runtime();
        let f = Fixture::new();
        let raw = f.raw(&f.b, 2, 200, 1000, 100);
        let proof = f.helper(&r, &raw).unwrap();
        let count = if full_group {
            MAX_REPLAY_GROUP
        } else {
            MAX_REPLAY_PROCESS
        };
        {
            let mut state = r.state.lock().unwrap();
            for i in 0..count {
                let mut nonce = [0; 16];
                nonce[..8].copy_from_slice(&(i as u64).to_be_bytes());
                state.replay.insert(
                    ReplayKey {
                        group: if full_group {
                            b"g".as_slice().into()
                        } else {
                            b"other".as_slice().into()
                        },
                        local: f.b.device_id(),
                        role: Role::Helper,
                        original: f.c.device_id(),
                        nonce,
                    },
                    ReplayRow {
                        hash: [3; 32],
                        until: 41000,
                    },
                );
            }
        }
        assert_eq!(r.commit(&proof), Err(AdmissionError::Busy));
        let state = r.state.lock().unwrap();
        assert_eq!(state.replay.len(), count);
        assert!(state.work_device.rows.is_empty());
        assert!(state.work_process.rows.is_empty());
    }
    let (_, r) = runtime();
    let f = Fixture::new();
    let raw = f.raw(&f.b, 2, 1, 1000, 100);
    let proof = f.source(&r, &f.b, 2, &raw).unwrap();
    {
        let mut state = r.state.lock().unwrap();
        for i in 0..MAX_DEVICES - 1 {
            let mut bytes = [0; 32];
            bytes[..8].copy_from_slice(&(i as u64).to_be_bytes());
            state.work_device.rows.insert(
                (b"fill".as_slice().into(), DeviceId::from_bytes(bytes)),
                Window {
                    until: 2000,
                    used: 0,
                },
            );
        }
    }
    assert_eq!(r.commit(&proof), Err(AdmissionError::Busy));
    assert_eq!(
        r.state.lock().unwrap().work_device.rows.len(),
        MAX_DEVICES - 1
    );
    assert!(r.state.lock().unwrap().replay.is_empty());
}

#[test]
fn strict_future_freshness_and_exact_replay_expiry_never_reopen_a_grant() {
    let clock = Arc::new(ManualClock::new(0));
    let r = AdmissionRuntime::new(clock.clone());
    let f = Fixture::new();
    let future = f.raw(&f.b, 2, 1, 19999, 100);
    r.commit(&f.helper(&r, &future).unwrap()).unwrap();
    let boundary = f.raw(&f.b, 2, 2, 20000, 100);
    assert!(matches!(
        f.helper(&r, &boundary),
        Err(AdmissionError::Unfresh)
    ));
    clock.set_ms(39998);
    assert_eq!(
        r.commit(&f.helper(&r, &future).unwrap()),
        Err(AdmissionError::Replay)
    );
    clock.set_ms(39999);
    assert!(matches!(
        f.helper(&r, &future),
        Err(AdmissionError::Unfresh)
    ));
    clock.set_ms(40000);
    assert!(matches!(
        f.helper(&r, &future),
        Err(AdmissionError::Unfresh)
    ));
    assert!(r.state.lock().unwrap().replay.is_empty());
}

#[test]
fn effective_wall_advances_from_forward_high_water_and_overflow_fails_closed() {
    let (clock, r) = runtime();
    assert_eq!(r.request_time_ms().unwrap(), 1000);
    clock.set_wall_ms(100000);
    assert_eq!(r.request_time_ms().unwrap(), 100000);
    clock.set_wall_ms(1000);
    clock.advance_ms(1000);
    assert_eq!(r.request_time_ms().unwrap(), 101000);
    clock.set_wall_ms(u64::MAX - 5);
    assert_eq!(r.request_time_ms().unwrap(), u64::MAX - 5);
    clock.set_ms(3000);
    assert_eq!(r.request_time_ms(), Err(AdmissionError::Clock));
    clock.set_wall_ms(1000);
    assert_eq!(r.request_time_ms(), Err(AdmissionError::Clock));
}

#[derive(Debug)]
struct LockedClock {
    clock: ManualClock,
    runtime: Mutex<Option<std::sync::Weak<AdmissionRuntime>>>,
}
impl LockedClock {
    fn assert_locked(&self) {
        let runtime = self
            .runtime
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .upgrade()
            .unwrap();
        assert!(
            matches!(
                runtime.state.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ),
            "clock sampling MUST be inside admission serialization"
        );
    }
}
impl Clock for LockedClock {
    fn monotonic_ms(&self) -> u64 {
        self.assert_locked();
        self.clock.monotonic_ms()
    }
    fn now_ms(&self) -> u64 {
        self.assert_locked();
        self.clock.now_ms()
    }
    fn sleep(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        self.clock.sleep(duration)
    }
}
#[test]
fn concurrent_clock_observations_sample_only_inside_the_same_state_lock() {
    let clock = Arc::new(LockedClock {
        clock: ManualClock::new(1000),
        runtime: Mutex::new(None),
    });
    let r = Arc::new(AdmissionRuntime::new(clock.clone()));
    *clock.runtime.lock().unwrap() = Some(Arc::downgrade(&r));
    // First exercise the lock oracle without another thread that could accidentally hold it.
    assert_eq!(r.request_time_ms().unwrap(), 1000);
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let r = r.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..128 {
                    r.request_time_ms().unwrap();
                }
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().unwrap();
    }
    assert!(!r.state.lock().unwrap().time.poisoned);
}

#[test]
fn exact_frame_scope_deadline_and_runtime_incarnation_are_fenced() {
    let (clock, r) = runtime();
    let (_, other) = runtime();
    let f = Fixture::new();
    let raw = f.raw(&f.b, 2, 1, 1000, 100);
    let scope = || ServiceScope {
        group: b"g",
        epoch: 7,
        local: Fixture::peer(&f.b, 2),
    };
    let ticket = r
        .admit_ingress(scope(), Fixture::peer(&f.c, 1), Role::Helper, &raw)
        .unwrap();
    let other_frame = f.raw(&f.b, 2, 2, 1000, 100);
    assert!(matches!(
        r.verify(ticket, &other_frame),
        Err(AdmissionError::Scope)
    ));
    let ticket = r
        .admit_ingress(scope(), Fixture::peer(&f.c, 1), Role::Helper, &raw)
        .unwrap();
    assert!(matches!(
        other.verify(ticket, &raw),
        Err(AdmissionError::Scope)
    ));
    let proof = f.helper(&r, &raw).unwrap();
    assert_eq!(other.preflight(&proof), Err(AdmissionError::Scope));
    clock.advance_ms(EFFECT_MS);
    assert_eq!(r.commit(&proof), Err(AdmissionError::Expired));
    assert!(r.state.lock().unwrap().replay.is_empty());
}

#[test]
fn process_runtime_replacement_and_epoch_scope_changes_do_not_reset_replay() {
    assert!(Arc::ptr_eq(
        &AdmissionRuntime::process(),
        &AdmissionRuntime::process()
    ));
    let (_, r) = runtime();
    let f = Fixture::new();
    let raw = f.raw(&f.b, 2, 1, 1000, 100);
    r.commit(&f.helper(&r, &raw).unwrap()).unwrap();
    let ticket = r
        .admit_ingress(
            ServiceScope {
                group: b"g",
                epoch: 8,
                local: Fixture::peer(&f.b, 2),
            },
            Fixture::peer(&f.c, 1),
            Role::Helper,
            &raw,
        )
        .unwrap();
    assert!(matches!(r.verify(ticket, &raw), Err(AdmissionError::Scope)));
    // A replacement native scope returning to old epoch cannot erase the runtime's row.
    assert_eq!(
        r.commit(&f.helper(&r, &raw).unwrap()),
        Err(AdmissionError::Replay)
    );
}
