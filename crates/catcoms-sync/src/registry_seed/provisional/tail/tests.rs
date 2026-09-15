use super::super::seed::tests::{hint, pair, response, retained, seed, target, Node};
use super::*;
use catcoms_replication::registry_epoch::catchup::{
    RegistryOpPage, RegistryPageCursor, REGISTRY_CURSOR_BYTES,
};
use catcoms_replication::studio::catchup::StudioPageProvider;
use catcoms_replication::studio::{FlipnoteHeader, FlipnoteOp, IndexOp, StudioEpoch};
use catcoms_replication::{DomainOp, InheritedCheckpoint, SealedOp};
use catcoms_rt::{Hub, ManualClock, MemNetwork};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

struct Fixture {
    provider: Node,
    client: Node,
    clock: ManualClock,
    target: StudioTarget,
    source: StudioEpoch,
    candidate: PreparedProvisionalStudioSeed,
}
impl Fixture {
    async fn new(art: bool) -> Self {
        let (mut provider, mut client, clock) = pair().await;
        let target = target(art);
        let (receipt, raw) = seed(&provider, target);
        // Construct a real source using a SEPARATE owner receipt. The unconfirmed client only
        // receives the non-owner receipt above; a matching graph grants it no source authority.
        let opening = Receipt::sign(
            receipt.document.clone(),
            0,
            receipt.close_record_hash,
            receipt.seed_change_hash,
            0,
            InheritedCheckpoint::EpochZero,
            &client.device,
        )
        .unwrap();
        let source = StudioEpoch::from_checkpoint(
            &provider.group,
            target,
            provider.device.device_id(),
            opening,
            0,
            raw.bytes(),
        )
        .unwrap();
        let hint = hint(&mut provider, &mut client, target, &receipt).await;
        let pending = client.prepare_provisional_studio_seed(hint).unwrap();
        let completed = response(&mut provider, pending, target, Some(raw.bytes().to_vec())).await;
        let candidate = client
            .complete_provisional_studio_seed(completed)
            .unwrap()
            .unwrap()
            .prepare()
            .unwrap();
        assert!(!candidate.tail_complete());
        Self {
            provider,
            client,
            clock,
            target,
            source,
            candidate,
        }
    }
    fn edit(&mut self, n: u8) -> SealedOp {
        let logical = self
            .target
            .document(&self.provider.group.group_id())
            .unwrap();
        let body = match self.target {
            StudioTarget::Flipnote { .. } => {
                FlipnoteOp::SetHeader(FlipnoteHeader::Title(format!("signed tail {n}")))
                    .encode()
                    .unwrap()
            }
            _ => IndexOp::SetTitle {
                object: [7; 16],
                title: format!("signed tail {n}"),
            }
            .encode()
            .unwrap(),
        };
        self.source
            .edit_or_reseal(
                &self.provider.device,
                &self.provider.group,
                &mut self.provider.rng,
                &DomainOp {
                    doc_type: logical.doc_type,
                    logical_key: logical.logical_key,
                    body,
                    nonce: [n + 32; 16],
                },
                1000,
            )
            .unwrap()
    }
}
async fn answer(
    provider: &mut Node,
    pending: PendingProvisionalStudioTail<MemNetwork>,
    target: StudioTarget,
    doc_id: u128,
    outcome: RegistryPageOutcome,
) -> CompletedProvisionalStudioTail {
    let watch = provider.watch_studio(target, doc_id).unwrap();
    let (completed, ()) = tokio::join!(pending.fetch(), async {
        provider.run_once().await.unwrap();
        provider
            .serve_studio_request(&watch, |_, _, _, _| Ok::<_, ()>(outcome))
            .unwrap()
            .unwrap()
            .unwrap();
    });
    completed
}
fn page(operations: Vec<SealedOp>, next: Option<RegistryPageCursor>) -> RegistryPageOutcome {
    RegistryPageOutcome::Page(RegistryOpPage { operations, next })
}

#[tokio::test]
async fn provisional_tail_preserves_request_and_original_deadlines_through_parser_and_use() {
    for stage in ["request", "lifetime", "parser", "ready"] {
        let mut f = Fixture::new(true).await;
        let op = f.edit(1);
        if stage != "request" {
            f.clock.advance_ms(FETCH_MS - 1_000);
        }
        let pending = f
            .client
            .prepare_provisional_studio_tail(f.candidate)
            .unwrap();
        let completed = answer(
            &mut f.provider,
            pending,
            f.target,
            f.source.doc_id(),
            page(vec![op], None),
        )
        .await;
        if stage == "request" || stage == "lifetime" {
            f.clock.advance_ms(if stage == "request" {
                REQUEST_MS
            } else {
                1_000
            });
            assert!(
                f.client
                    .complete_provisional_studio_tail(completed)
                    .is_err(),
                "late {stage}"
            );
        } else {
            f.clock.advance_ms(999);
            let mut preparation = f
                .client
                .complete_provisional_studio_tail(completed)
                .unwrap()
                .unwrap();
            if stage == "parser" {
                preparation.clock = Arc::new(super::super::seed::tests::ParseDeadlineClock {
                    base: f.clock.clone(),
                    expiry: preparation.hint.expires,
                    reads: std::sync::atomic::AtomicUsize::new(0),
                });
                assert!(preparation.prepare().is_err());
            } else {
                let ready = preparation.prepare().unwrap();
                f.client
                    .with_provisional_studio_seed(&ready, |_| ())
                    .unwrap();
                f.clock.advance_ms(1);
                assert!(f
                    .client
                    .with_provisional_studio_seed(&ready, |_| panic!("extended tail lifetime"))
                    .is_err());
            }
        }
        assert_eq!(retained(&f.client), 0);
    }
}

#[tokio::test]
async fn provisional_tail_pages_use_real_fixed_prefix_without_rebinding_ordinary_watch() {
    for art in [false, true] {
        let mut f = Fixture::new(art).await;
        for n in 1..36 {
            f.edit(n);
        }
        let source_bytes = f.source.snapshot().unwrap();
        let original_doc = f.candidate.hint.watch.doc_id;
        let watch = f
            .provider
            .watch_studio(f.target, f.source.doc_id())
            .unwrap();
        let mut serving = StudioPageProvider::new(
            f.provider.device.device_id(),
            Arc::new(f.clock.clone()),
            &mut f.provider.rng,
        );
        let mut candidate = f.candidate;
        for expected_more in [true, false] {
            let pending = f.client.prepare_provisional_studio_tail(candidate).unwrap();
            let (completed, ()) = tokio::join!(pending.fetch(), async {
                f.provider.run_once().await.unwrap();
                f.provider
                    .serve_studio_request(&watch, |g, d, r, query| {
                        assert!(query.heads.is_empty());
                        assert_eq!(query.seed, f.source.catchup_frontier().seed);
                        serving.page(&f.source, g, d, query, r)
                    })
                    .unwrap()
                    .unwrap()
                    .unwrap();
            });
            assert_eq!(retained(&f.client), 1);
            let preparation = f
                .client
                .complete_provisional_studio_tail(completed)
                .unwrap()
                .unwrap();
            candidate = tokio::task::spawn_blocking(move || preparation.prepare())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(candidate.tail_complete(), !expected_more);
            assert_eq!(candidate.hint.watch.doc_id, original_doc);
            assert!(f.client.studio_watch_is_current(&candidate.hint.watch));
        }
        f.client
            .with_provisional_studio_seed(&candidate, |view| {
                assert_eq!(view.projection, &f.source.projection().unwrap());
                assert_eq!(view.candidate.provider, f.provider.device.device_id());
                assert!(view
                    .candidate
                    .receipt
                    .verify_current_owner(&f.client.group, 0)
                    .is_err());
            })
            .unwrap();
        assert_eq!(f.source.snapshot().unwrap(), source_bytes);
        assert!(f.client.prepare_provisional_studio_tail(candidate).is_err());
        assert_eq!(retained(&f.client), 0);
    }
}

#[tokio::test]
async fn provisional_tail_refuses_nonpage_outcomes_and_nonprogressing_cursors() {
    for bad in ["restart", "checkpoint", "history", "repeated-cursor"] {
        let mut f = Fixture::new(true).await;
        let op = f.edit(1);
        let mut cursor = vec![0; REGISTRY_CURSOR_BYTES];
        cursor[0] = 1;
        if bad == "repeated-cursor" {
            let pending = f
                .client
                .prepare_provisional_studio_tail(f.candidate)
                .unwrap();
            let completed = answer(
                &mut f.provider,
                pending,
                f.target,
                f.source.doc_id(),
                page(
                    vec![op.clone()],
                    Some(RegistryPageCursor::from_bytes(&cursor).unwrap()),
                ),
            )
            .await;
            f.candidate = f
                .client
                .complete_provisional_studio_tail(completed)
                .unwrap()
                .unwrap()
                .prepare()
                .unwrap();
            assert!(!f.candidate.tail_complete());
        }
        let outcome = match bad {
            "restart" => RegistryPageOutcome::Restart,
            "checkpoint" => RegistryPageOutcome::CheckpointRequired,
            "history" => RegistryPageOutcome::HistoricalAuthorizationRequired,
            _ => page(
                vec![op],
                Some(RegistryPageCursor::from_bytes(&cursor).unwrap()),
            ),
        };
        let pending = f
            .client
            .prepare_provisional_studio_tail(f.candidate)
            .unwrap();
        let completed = answer(
            &mut f.provider,
            pending,
            f.target,
            f.source.doc_id(),
            outcome,
        )
        .await;
        let result = f.client.complete_provisional_studio_tail(completed);
        if bad.ends_with("cursor") {
            assert!(result.is_err(), "{bad}");
        } else {
            assert!(result.unwrap().is_none(), "{bad}");
        }
        assert_eq!(retained(&f.client), 0);
    }
}

#[tokio::test]
async fn provisional_tail_rechecks_lifecycle_at_completion_and_after_detached_parse() {
    for stage in ["completed", "preparation", "ready"] {
        for change in [
            "watch",
            "attempt",
            "endpoint",
            "membership",
            "instance",
            "expiry",
        ] {
            let mut f = Fixture::new(true).await;
            let op = f.edit(1);
            let pending = f
                .client
                .prepare_provisional_studio_tail(f.candidate)
                .unwrap();
            let completed = answer(
                &mut f.provider,
                pending,
                f.target,
                f.source.doc_id(),
                page(vec![op], None),
            )
            .await;
            let (completed, preparation, ready) = if stage == "completed" {
                (Some(completed), None, None)
            } else {
                let prep = f
                    .client
                    .complete_provisional_studio_tail(completed)
                    .unwrap()
                    .unwrap();
                if stage == "preparation" {
                    (None, Some(prep), None)
                } else {
                    (None, None, Some(prep.prepare().unwrap()))
                }
            };
            match change {
                "watch" => {
                    f.client.watch_studio(f.target, 99).unwrap();
                }
                "attempt" => {
                    drop(
                        f.client
                            .prepare_checkpoint_head(
                                f.provider.local_peer(),
                                CheckpointTarget::Studio(f.target),
                            )
                            .unwrap(),
                    );
                }
                "endpoint" => {
                    f.client.member_peers.clear();
                }
                "membership" => {
                    let joined = MlsDevice::generate().unwrap();
                    let invite = f.client.mint_invite([55; 16], u64::MAX, vec![]).unwrap();
                    let kp = joined
                        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
                        .unwrap();
                    f.client
                        .admit_now(
                            &invite,
                            &catcoms_mls::serialize_key_package(&kp).unwrap(),
                            f.clock.now_ms(),
                        )
                        .unwrap();
                }
                "instance" => {
                    f.client = Node::restore(
                        &f.client.snapshot().unwrap(),
                        Hub::new().join(PeerId::from_u64(99)),
                        ChaCha20Rng::seed_from_u64(834),
                        Box::new(f.clock.clone()),
                    )
                    .unwrap();
                }
                _ => {
                    f.clock.advance_ms(FETCH_MS);
                }
            }
            if let Some(completed) = completed {
                assert!(
                    f.client
                        .complete_provisional_studio_tail(completed)
                        .is_err(),
                    "{stage}/{change}"
                );
            }
            let ready = match preparation {
                Some(prep) if change == "expiry" => {
                    assert!(prep.prepare().is_err());
                    None
                }
                Some(prep) => Some(prep.prepare().unwrap()),
                None => ready,
            };
            if let Some(ready) = ready {
                assert!(f
                    .client
                    .with_provisional_studio_seed(&ready, |_| panic!("stale {stage}/{change}"))
                    .is_err());
            }
        }
    }
}

#[tokio::test]
async fn provisional_tail_retains_cancelled_lower_transport_and_ready_capacity() {
    for stage in ["cancelled", "completed", "preparation", "ready"] {
        let mut f = Fixture::new(true).await;
        let op = f.edit(1);
        let pending = f
            .client
            .prepare_provisional_studio_tail(f.candidate)
            .unwrap();
        let (completed, preparation, ready) = if stage == "cancelled" {
            let mut future = Box::pin(pending.fetch());
            assert!(futures::poll!(future.as_mut()).is_pending());
            drop(future);
            (None, None, None)
        } else {
            let completed = answer(
                &mut f.provider,
                pending,
                f.target,
                f.source.doc_id(),
                page(vec![op], None),
            )
            .await;
            if stage == "completed" {
                (Some(completed), None, None)
            } else {
                let prep = f
                    .client
                    .complete_provisional_studio_tail(completed)
                    .unwrap()
                    .unwrap();
                if stage == "preparation" {
                    (None, Some(prep), None)
                } else {
                    (None, None, Some(prep.prepare().unwrap()))
                }
            }
        };
        let second = f.client.reserve_provisional_checkpoint_capacity().unwrap();
        let third = f.client.reserve_provisional_checkpoint_capacity().unwrap();
        assert!(
            f.client.reserve_provisional_checkpoint_capacity().is_err(),
            "lost {stage} custody"
        );
        let authoritative = f
            .client
            .prepare_checkpoint_discovery(f.provider.local_peer(), CheckpointTarget::Registry(4))
            .unwrap();
        assert_eq!(retained(&f.client), 4);
        f.clock.advance_ms(FETCH_MS);
        assert!(f.client.reserve_provisional_checkpoint_capacity().is_err());
        if stage == "cancelled" {
            f.provider.run_once().await.unwrap();
        }
        drop((completed, preparation, ready));
        assert!(f.client.reserve_provisional_checkpoint_capacity().is_ok());
        drop((second, third, authoritative));
    }
}
