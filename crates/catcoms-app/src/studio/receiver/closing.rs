//! Direct Closing edits remain local draft work until the existing automatic handoff accepts
//! them. The actor derives the signed close from the sealed journal; IPC carries retry identity.
use super::*;
use catcoms_replication::studio::{
    StudioOverlayProvenance, StudioOverlaySave, StudioOverlaySaveRefusal,
};

impl StudioReceiver {
    pub(super) fn closing_save_control<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        request: StudioControlRequest,
    ) -> Result<StudioControlResponse, AppError> {
        request.validate()?;
        let target = request.target;
        if !server
            .channels()
            .iter()
            .any(|c| c.id == u128::from_be_bytes(target.channel()))
        {
            return Err(invalid("unknown Studio channel"));
        }
        // Preserve provenance at the route boundary. A valid acceptance of preview-local work
        // cannot be relabelled as a Closing Save by resending it through another command.
        let logical = target.document(&server.group_id()).map_err(invalid)?;
        let state = store.load_epoch_intents_structural(id, &logical)?;
        if state.handoff_metadata().is_some_and(|m| {
            m.overlay().is_some()
                && matches!(m.provenance(), StudioOverlayProvenance::Unconfirmed { .. })
        }) {
            return Ok(closing_refused(
                target,
                StudioOverlaySaveRefusal::OtherKindDraft,
                "the live draft was made on unconfirmed history",
            ));
        }
        drop(state);
        // Keep lookup failure as a value for Save. Terminal retries run before the store opens
        // this attempt, even after the journal/source has disappeared or tenure changed (V8).
        let close = server.sync.with_registry_context(|group, _, _, _| {
            store.saved_studio_closing_record(id, group, target)
        });
        let visit = match request.action {
            StudioControlAction::BeginClosingOverlaySave => {
                let mut budget = unconfirmed::save_budget(server, store, id)?;
                let ticket = match close.and_then(|close| {
                    server.prepare_studio_closing_overlay(store, id, target, &close, &mut budget)
                }) {
                    Ok(ticket) => ticket,
                    Err(error) => return closing_error(target, error),
                };
                return Ok(StudioControlResponse::ClosingOverlaySaveTicket {
                    target,
                    basis: ticket.basis.fingerprint(),
                    branch: ticket.branch,
                });
            }
            StudioControlAction::SaveClosingOverlay(save) => {
                let operation = crate::studio::domain(target, save.nonce, save.body);
                self.save_overlay_attempt(
                    server,
                    store,
                    id,
                    target,
                    close,
                    save.basis,
                    save.branch,
                    operation,
                    None,
                )?
            }
            _ => return Err(invalid("not a Closing draft Save")),
        };
        let outcome = match visit {
            StudioOverlaySaveVisit::Saved(saved) => match *saved {
                StudioOverlaySave::Local(draft) => StudioUnconfirmedSaveOutcome::Saved {
                    basis: draft.basis(),
                    accepted: draft.accepted(),
                },
                StudioOverlaySave::Acknowledged { basis, accepted } => {
                    StudioUnconfirmedSaveOutcome::Saved { basis, accepted }
                }
                StudioOverlaySave::Disposed(manifest) => {
                    StudioUnconfirmedSaveOutcome::Disposed(manifest)
                }
                StudioOverlaySave::HandedOff(outcome) => {
                    StudioUnconfirmedSaveOutcome::HandedOff(outcome)
                }
            },
            StudioOverlaySaveVisit::Refused { reason, message } => {
                StudioUnconfirmedSaveOutcome::Refused { reason, message }
            }
            StudioOverlaySaveVisit::Scheduled => StudioUnconfirmedSaveOutcome::Scheduled,
            StudioOverlaySaveVisit::Busy => StudioUnconfirmedSaveOutcome::Busy,
            StudioOverlaySaveVisit::Paused => StudioUnconfirmedSaveOutcome::Paused,
        };
        self.settlement
            .note(target, StudioSettlementState::RefreshRequired);
        Ok(StudioControlResponse::ClosingOverlaySaved { target, outcome })
    }
}

fn closing_refused(
    target: StudioTarget,
    reason: StudioOverlaySaveRefusal,
    message: &str,
) -> StudioControlResponse {
    StudioControlResponse::ClosingOverlaySaved {
        target,
        outcome: StudioUnconfirmedSaveOutcome::Refused {
            reason,
            message: message.into(),
        },
    }
}
fn closing_error(target: StudioTarget, error: AppError) -> Result<StudioControlResponse, AppError> {
    let StudioOverlaySaveVisit::Refused { reason, message } = refused_save_visit(error)? else {
        unreachable!("refusal converter returns only a refusal")
    };
    Ok(closing_refused(target, reason, &message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_mls::MlsDevice;
    use catcoms_replication::studio::{FlipnoteHeader, FlipnoteOp};
    use catcoms_rt::{Hub, ManualClock, PeerId};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[tokio::test]
    async fn closing_control_schedules_real_local_work_and_exact_retry_survives_missing_close() {
        let mut server = Server::found(
            Hub::new().join(PeerId::from_u64(1)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(923),
            Box::new(ManualClock::new(1000)),
            "owner",
        )
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let mut store = ServerStore::open(
            root.path(),
            b"closing-control",
            &mut ChaCha20Rng::seed_from_u64(924),
        )
        .unwrap();
        let target = StudioTarget::Flipnote {
            channel: crate::channel_id("general").to_be_bytes(),
            object: [0xe1; 16],
        };
        server.sync.with_registry_context(|g, d, _, _| {
            crate::store::studio_closing_control_fixture(&mut store, 7, g, d, target)
        });
        let mut receiver = StudioReceiver::default();
        let ticket = receiver
            .closing_save_control(
                &mut server,
                &mut store,
                7,
                StudioControlRequest {
                    target,
                    action: StudioControlAction::BeginClosingOverlaySave,
                },
            )
            .unwrap();
        let StudioControlResponse::ClosingOverlaySaveTicket { basis, branch, .. } = ticket else {
            panic!("a genuine Closing source/journal must mint a ticket: {ticket:?}")
        };
        let body = FlipnoteOp::SetHeader(FlipnoteHeader::Title("local while rotating".into()))
            .encode()
            .unwrap();
        let request = || StudioControlRequest {
            target,
            action: StudioControlAction::SaveClosingOverlay(Box::new(
                StudioUnconfirmedOverlaySaveRequest {
                    basis,
                    branch,
                    nonce: [0xe2; 16],
                    body: body.clone(),
                },
            )),
        };
        assert!(matches!(
            receiver
                .control(&mut server, &mut store, 7, request())
                .unwrap()
                .2,
            Some(StudioControlResponse::ClosingOverlaySaved {
                outcome: StudioUnconfirmedSaveOutcome::Scheduled,
                ..
            })
        ));
        let job = receiver.detach(&mut server).expect("Closing work detaches");
        let result = job.run(None).await;
        receiver.complete(&mut server, result);
        assert!(matches!(
            receiver
                .control(&mut server, &mut store, 7, request())
                .unwrap()
                .2,
            Some(StudioControlResponse::ClosingOverlaySaved {
                outcome: StudioUnconfirmedSaveOutcome::Saved { accepted: 1, .. },
                ..
            })
        ));
        crate::store::remove_studio_closing_journal_fixture(&store, 7, &server.group_id(), target);
        assert!(
            matches!(
                receiver
                    .control(&mut server, &mut store, 7, request())
                    .unwrap()
                    .2,
                Some(StudioControlResponse::ClosingOverlaySaved {
                    outcome: StudioUnconfirmedSaveOutcome::Saved { accepted: 1, .. },
                    ..
                })
            ),
            "the actual control still answers acceptance after the sealed journal is gone"
        );
        assert!(
            matches!(
                receiver
                    .control(
                        &mut server,
                        &mut store,
                        7,
                        StudioControlRequest {
                            target,
                            action: StudioControlAction::BeginClosingOverlaySave
                        }
                    )
                    .unwrap()
                    .2,
                Some(StudioControlResponse::ClosingOverlaySaved {
                    outcome: StudioUnconfirmedSaveOutcome::Refused { .. },
                    ..
                })
            ),
            "a new ticket cannot borrow the old acceptance as close authority"
        );
        // An accepted retry MUST classify before opening a failed close lookup. This also pins
        // the production wrapper's Result-based authority boundary without fabricating a close.
        let operation = crate::studio::domain(target, [0xe2; 16], body.clone());
        let saved = receiver
            .save_overlay_attempt(
                &mut server,
                &mut store,
                7,
                target,
                Err(invalid("journal disappeared")),
                basis,
                branch,
                operation,
                None,
            )
            .unwrap();
        assert!(
            matches!(saved, StudioOverlaySaveVisit::Saved(saved) if matches!(*saved, StudioOverlaySave::Acknowledged { accepted: 1, .. }))
        );
        let changed = crate::studio::domain(
            target,
            [0xe2; 16],
            FlipnoteOp::SetHeader(FlipnoteHeader::Title("tampered".into()))
                .encode()
                .unwrap(),
        );
        assert!(matches!(
            receiver
                .save_overlay_attempt(
                    &mut server,
                    &mut store,
                    7,
                    target,
                    Err(invalid("journal disappeared")),
                    basis,
                    branch,
                    changed,
                    None
                )
                .unwrap(),
            StudioOverlaySaveVisit::Refused {
                reason: StudioOverlaySaveRefusal::NonceConflict,
                ..
            }
        ));
        let fresh = crate::studio::domain(target, [0xe3; 16], body);
        assert!(
            receiver
                .save_overlay_attempt(
                    &mut server,
                    &mut store,
                    7,
                    target,
                    Err(invalid("journal disappeared")),
                    basis,
                    branch,
                    fresh,
                    None
                )
                .is_err(),
            "a failed close lookup never authorizes new work"
        );
    }
}
