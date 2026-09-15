//! Interrupt actual actor-driven takeover at the recovery/successor writer, then reuse the
//! accepted matrix's restart, receipt, pointer, recovery and subsequent-Apply assertions.
//! This models an I/O error followed by loss of volatile state, not process/power-loss testing.
use super::*;
use crate::store::StudioRotationBoundary;
use catcoms_replication::Receipt;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn studio_actor_interrupted_successor_index_recovery_write_resumes_after_restart() {
    cases(
        StudioTarget::Index { channel: channel() },
        StudioRotationBoundary::Recovery,
    )
    .await;
}

#[tokio::test]
async fn studio_actor_interrupted_successor_flipnote_recovery_write_resumes_after_restart() {
    cases(target(), StudioRotationBoundary::Recovery).await;
}

#[tokio::test]
async fn studio_actor_interrupted_successor_index_install_write_resumes_after_restart() {
    cases(
        StudioTarget::Index { channel: channel() },
        StudioRotationBoundary::Successor,
    )
    .await;
}

#[tokio::test]
async fn studio_actor_interrupted_successor_flipnote_install_write_resumes_after_restart() {
    cases(target(), StudioRotationBoundary::Successor).await;
}

async fn cases(target: StudioTarget, boundary: StudioRotationBoundary) {
    for after_write in [false, true] {
        // An installed old-owner checkpoint plus a later frozen close challenges inheritance
        // and recovery. The new owner's receipt is produced solely by ordinary actor work.
        successor_with_interruption(
            target,
            true,
            true,
            Some(Interruption {
                boundary,
                after_write,
            }),
        )
        .await;
    }
}

#[derive(Clone, Copy)]
pub(super) struct Interruption {
    boundary: StudioRotationBoundary,
    after_write: bool,
}

impl Interruption {
    fn successor_written(self) -> bool {
        self.boundary == StudioRotationBoundary::Successor && self.after_write
    }

    pub(super) async fn reach_write(
        self,
        actor: &crate::ServerActor,
        store: &Arc<Mutex<Option<ServerStore>>>,
        verifier: &mut Node,
        clock: &ManualClock,
        target: StudioTarget,
    ) -> (Receipt, Vec<u8>) {
        let logical = target.document(&verifier.group_id()).unwrap();
        let (hit, before) = {
            let mut guard = store.lock().await;
            let held = guard.as_mut().unwrap();
            let before = verifier.sync.with_registry_context(|g, d, _, _| {
                held.load_studio_epoch(SERVER, g, target, d)
                    .unwrap()
                    .unwrap()
            });
            assert_eq!((before.epoch(), before.phase()), (1, EpochPhase::Closing));
            assert_eq!(
                held.load_epoch_recovery(SERVER, &logical)
                    .unwrap()
                    .retained()
                    .len(),
                0
            );
            (
                held.interrupt_studio_rotation_for_test(target, self.boundary, self.after_write),
                before,
            )
        };
        for _ in 0..60 {
            clock.advance_ms(1000);
            joining::step(actor, store).await;
            actor.wait_studio_preparation().await;
            if hit.load(Ordering::SeqCst) {
                break;
            }
        }
        assert!(
            hit.load(Ordering::SeqCst),
            "actor never reached {:?}, after_write={}",
            self.boundary,
            self.after_write
        );
        let guard = store.lock().await;
        let held = guard.as_ref().unwrap();
        let journal = held.load_epoch_owner_receipts(SERVER, &logical).unwrap();
        assert!(
            journal.published().is_none(),
            "failed rotation must not complete availability"
        );
        let receipt = journal
            .pending()
            .expect("decision must precede installation")
            .clone();
        assert_eq!(
            receipt.tenure_start_group_epoch,
            verifier.sync.observed_owner_tenure_start().unwrap()
        );
        assert_eq!(receipt.closed_epoch, 1);
        let close = journal
            .close_for(&receipt)
            .expect("exact close must survive the interruption")
            .encode();
        let state = verifier.sync.with_registry_context(|g, d, _, _| {
            receipt
                .verify_current_owner(g, receipt.tenure_start_group_epoch)
                .unwrap();
            held.load_studio_epoch(SERVER, g, target, d)
                .unwrap()
                .unwrap()
        });
        if self.successor_written() {
            assert_eq!(
                (state.epoch(), state.phase(), state.op_count()),
                (2, EpochPhase::Open, 0)
            );
        } else {
            assert_eq!(state.doc_id(), before.doc_id());
            assert_eq!(state.phase(), EpochPhase::Closing);
            assert_eq!(state.op_count(), before.op_count());
            assert_eq!(state.projection().unwrap(), before.projection().unwrap());
        }
        let recovery = held.load_epoch_recovery(SERVER, &logical).unwrap();
        let recovery_written =
            self.boundary == StudioRotationBoundary::Successor || self.after_write;
        assert_eq!(
            recovery.retained().len(),
            usize::from(recovery_written),
            "recovery must precede the successor write"
        );
        assert!(recovery.staged().is_none());
        assert!(recovery.eviction_pending().unwrap().is_none());
        if recovery_written {
            let recovered = StudioRecovery::from_snapshot(
                recovery.retained().next().unwrap(),
                &logical,
                target.channel(),
            )
            .unwrap();
            assert_eq!(recovered.projection(), &before.projection().unwrap());
        }
        assert_eq!(
            held.load_epoch_intents(SERVER, &logical)
                .unwrap()
                .pending()
                .len(),
            0
        );
        (receipt, close)
    }

    pub(super) fn assert_reopened(
        self,
        read: &crate::studio::StudioView,
        original: &StudioProjection,
        initial_epoch: u64,
        receipt: &Receipt,
    ) {
        let epoch = initial_epoch + u64::from(self.successor_written());
        assert_eq!(read.epoch, epoch);
        assert_eq!(
            read.phase,
            if self.successor_written() {
                EpochPhase::Open
            } else {
                EpochPhase::Closing
            }
        );
        let mut expected = original.clone();
        match &mut expected {
            StudioProjection::Flipnote(art) => art.epoch = epoch,
            StudioProjection::Index(index) => index.epoch = epoch,
        }
        assert_eq!(read.projection, expected);
        if self.successor_written() {
            assert_eq!(
                read.epoch_id,
                catcoms_replication::epoch::epoch_id(
                    receipt.document.doc_type,
                    &receipt.document.logical_key,
                    epoch,
                    &receipt.close_record_hash
                )
            );
        }
    }
}
