//! Query the actual protected Root pair, including original-clock race boundaries. These
//! tests create no private pair state and never infer establishment from a crypto result.
use super::*;
use voice_v2::presence::VoicePairConfirmation;

#[tokio::test]
async fn root_pair_confirmation_is_noncreating_exact_and_quiet_under_full_pressure() {
    let (mut root, remote, runtime, _clock, grant) = low_discovery_fixture(1).await;
    let selected = selector(&root, &remote);
    assert_eq!(
        root.voice_pair_confirmation(&grant, selected, [0; 32]),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(state(&root), (0, 0, 0));
    let receipt = root.prepare_voice_pair(&grant, selected).unwrap();
    let mut changes = root.voice_endpoint_changes().unwrap();
    changes.borrow_and_update();
    let resources = runtime.resources();
    let _pressure = resources
        .charge(
            grant.scope().group,
            voice_v2::resources::GROUP_BYTES - resources.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let before = resources.counts();
    for _ in 0..32 {
        assert_eq!(
            root.voice_pair_confirmation(&grant, selected, receipt.handshake),
            Ok(VoicePairConfirmation::Confirming)
        );
    }
    assert_eq!(resources.counts(), before);
    assert_eq!(state(&root), (1, 1, 1));
    assert!(
        !changes.has_changed().unwrap(),
        "polling must not wake itself"
    );
    let foreign = Arc::new(CallGrant::from_lease(grant.lease(), grant.scope()));
    assert_eq!(
        root.voice_pair_confirmation(&foreign, selected, receipt.handshake),
        Err(VoiceError::Unauthorized)
    );
    for changed in [
        PairSelector {
            peer: root.device_id(),
            ..selected
        },
        PairSelector {
            join: [99; 32],
            ..selected
        },
        PairSelector {
            key_ref: [99; 32],
            ..selected
        },
        PairSelector {
            epoch: selected.epoch + 1,
            ..selected
        },
    ] {
        assert_eq!(
            root.voice_pair_confirmation(&grant, changed, receipt.handshake),
            Err(VoiceError::Unauthorized)
        );
        assert_eq!(
            root.voice_pair_confirmation(&grant, selected, receipt.handshake),
            Ok(VoicePairConfirmation::Confirming),
            "a rejected substitution cannot retire the original pin"
        );
    }
    assert_eq!(
        root.voice_pair_confirmation(&grant, selected, [99; 32]),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(resources.counts(), before);
    assert_eq!(state(&root), (1, 1, 1));
    assert!(!changes.has_changed().unwrap());
}

#[tokio::test]
async fn root_pair_confirmation_cannot_adopt_old_pair_across_epoch_or_lock_unlock() {
    let (mut root, remote, runtime, _clock, grant) = low_discovery_fixture(1).await;
    let selected = selector(&root, &remote);
    let receipt = root.prepare_voice_pair(&grant, selected).unwrap();
    let third = catcoms_mls::MlsDevice::generate().unwrap();
    root.with_observed_mls_transition(|node| {
        node.group
            .add_member(&node.device, third.key_package().unwrap())
    })
    .unwrap();
    assert!(root.group.contains_device(&remote.device_id()));
    for selected in [
        selected,
        PairSelector {
            epoch: root.group.epoch(),
            ..selected
        },
    ] {
        assert_eq!(
            root.voice_pair_confirmation(&grant, selected, receipt.handshake),
            Err(VoiceError::Unauthorized)
        );
    }
    runtime.revoke_session();
    attach(&mut root, &runtime);
    let replacement = Arc::new(root.voice_call_grant(1).unwrap());
    root.start_voice_endpoint(replacement.clone(), key(83).join, key(83).public_key)
        .unwrap();
    assert!(root
        .voice_pair_confirmation(&grant, selected, receipt.handshake)
        .is_err());
    assert_eq!(
        root.voice_pair_confirmation(&replacement, selected, receipt.handshake),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(state(&root), (0, 0, 0));
}

#[tokio::test]
async fn root_pair_confirmation_rechecks_every_original_clock_owner_boundary() {
    let ((mut root, remote, _runtime, _clock, grant), probe) = probed().await;
    let selected = selector(&root, &remote);
    let receipt = root.prepare_voice_pair(&grant, selected).unwrap();
    *probe.lock().unwrap() = Probe::default();
    assert_eq!(
        root.voice_pair_confirmation(&grant, selected, receipt.handshake),
        Ok(VoicePairConfirmation::Confirming)
    );
    let samples = probe.lock().unwrap().reads;
    assert!(samples > 1 && samples < 64);
    for owner in ["call", "watch", "room", "session"] {
        for at in 1..=samples {
            let ((mut root, remote, runtime, _clock, grant), probe) = probed().await;
            let selected = selector(&root, &remote);
            let receipt = root.prepare_voice_pair(&grant, selected).unwrap();
            let hook: Box<dyn FnOnce() + Send> = match owner {
                "call" => {
                    let lease = grant.lease();
                    Box::new(move || lease.revoke())
                }
                "watch" => {
                    let watch = root
                        .voice_runtime
                        .as_ref()
                        .unwrap()
                        .endpoints
                        .implicit_watch_for_test()
                        .unwrap();
                    Box::new(move || watch.revoke())
                }
                "room" => {
                    let lease = root.voice_room_authority(1).unwrap().lease;
                    Box::new(move || lease.revoke())
                }
                _ => Box::new(move || runtime.revoke_session()),
            };
            *probe.lock().unwrap() = Probe {
                reads: 0,
                trigger: at,
                hook: Some(hook),
            };
            assert!(
                root.voice_pair_confirmation(&grant, selected, receipt.handshake)
                    .is_err(),
                "{owner} at {at}/{samples}"
            );
            assert!(
                probe.lock().unwrap().hook.is_none(),
                "actual callback reached"
            );
        }
    }
}

#[tokio::test]
async fn root_pair_confirmation_final_time_and_contention_never_renew_original_pair() {
    let ((mut root, remote, _runtime, _clock, grant), probe) = probed().await;
    let selected = selector(&root, &remote);
    let receipt = root.prepare_voice_pair(&grant, selected).unwrap();
    *probe.lock().unwrap() = Probe::default();
    root.voice_pair_confirmation(&grant, selected, receipt.handshake)
        .unwrap();
    let samples = probe.lock().unwrap().reads;
    for policy in [false, true] {
        let ((mut root, remote, runtime, clock, grant), probe) = probed().await;
        let selected = selector(&root, &remote);
        let receipt = root.prepare_voice_pair(&grant, selected).unwrap();
        let actor = root
            .voice_runtime
            .as_ref()
            .unwrap()
            .current
            .as_ref()
            .unwrap();
        let (held, ready) = std::sync::mpsc::sync_channel(0);
        let (release, released) = std::sync::mpsc::channel();
        let hold = actor.native_view_holder_for_test(policy, held, released);
        let (start, started) = std::sync::mpsc::sync_channel(0);
        let holder = std::thread::spawn(move || {
            if started.recv().is_ok() {
                hold();
            }
        });
        *probe.lock().unwrap() = Probe {
            reads: 0,
            trigger: samples,
            hook: Some(Box::new(move || {
                start.send(()).unwrap();
                ready.recv().unwrap();
            })),
        };
        let before = runtime.resources().counts();
        let result = root.voice_pair_confirmation(&grant, selected, receipt.handshake);
        let _ = release.send(());
        holder.join().unwrap();
        assert_eq!(result, Err(VoiceError::Capacity));
        assert!(probe.lock().unwrap().hook.is_none());
        assert_eq!(runtime.resources().counts(), before);
        assert_eq!(
            root.voice_pair_confirmation(&grant, selected, receipt.handshake),
            Ok(VoicePairConfirmation::Confirming)
        );
        *probe.lock().unwrap() = Probe {
            reads: 0,
            trigger: samples,
            hook: Some(Box::new(move || {
                clock.advance_ms(15_001);
            })),
        };
        assert_eq!(
            root.voice_pair_confirmation(&grant, selected, receipt.handshake),
            Err(VoiceError::Freshness)
        );
        assert!(probe.lock().unwrap().hook.is_none());
        // The pure final fence refuses expiry without dropping ledger owners under its
        // native guards. The next ordinary housekeeping pass retires that same pair.
        assert_eq!(
            root.voice_pair_confirmation(&grant, selected, receipt.handshake),
            Err(VoiceError::Freshness)
        );
        assert_eq!(
            root.voice_pair_confirmation(&grant, selected, receipt.handshake),
            Err(VoiceError::Unauthorized)
        );
    }
}

#[tokio::test]
async fn root_pair_confirmation_rejects_current_member_policy_and_signed_room_replacement() {
    for change in ["member", "policy", "room", "end"] {
        let (mut root, remote, _runtime, _clock, grant) = low_discovery_fixture(2).await;
        let selected = selector(&root, &remote);
        let receipt = root.prepare_voice_pair(&grant, selected).unwrap();
        match change {
            "member" => {
                root.with_observed_mls_transition(|node| {
                    node.group.remove_member(&node.device, &remote.device_id())
                })
                .unwrap();
            }
            "policy" => root.set_voice_routing(crate::VoiceRouting::Direct),
            "room" => {
                use automerge::{transaction::Transactable, ROOT};
                root.post(DocType::ChannelIndex, 991, |doc| {
                    doc.delete(ROOT, "2")?;
                    doc.put(ROOT, "2", true)
                })
                .await
                .unwrap();
            }
            _ => root.stop_voice_endpoint(&grant),
        }
        assert!(
            root.voice_pair_confirmation(&grant, selected, receipt.handshake)
                .is_err(),
            "{change}"
        );
    }
}
