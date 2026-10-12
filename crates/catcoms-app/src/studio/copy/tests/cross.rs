//! Copy between two Flipnotes of one channel (design 6.3 C1'; Review 2, L-1 and L-2).
//!
//! The Index fixture beside this one copies within one document. So it cannot tell a hold on the
//! source from a hold on the destination, and its source can no longer take an overlay job. Here
//! the source X and the destination Y are separate documents, each with a real Closing-overlay
//! branch built by `closing_branch`. The copied item is the title X's branch accepted.
//!
//! - **L-2 (a):** a transfer hold on the DESTINATION refuses the copy at C1, C3 and C4, by message.
//! - **L-2 (b):** a transfer hold on the SOURCE does not. The copy previews `Ready` and applies, and
//!   the source's staged handoff is untouched: still `Prepared`, the same branch, the same ledger.
//!   A hold staged on the source mid-copy costs at most a fresh preview.
//! - **L-1:** an overlay Save that lands on the source between C1 and C3 makes C3 refuse the preview
//!   as changed. C4 checks no source stamp. A Save that touched another element lets the copy apply
//!   the value the source still holds. A Save that replaced the selected value is refused, because
//!   the re-plan no longer resolves the value the echo names.
use super::*;

const X: [u8; 16] = [0x51; 16];
const Y: [u8; 16] = [0x52; 16];
/// The title X's branch accepted, which every copy here carries.
const COPIED: &str = "from X's branch";

fn flipnote(object: [u8; 16]) -> StudioTarget {
    StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object,
    }
}

fn title_op(text: &str) -> Vec<u8> {
    FlipnoteOp::SetHeader(FlipnoteHeader::Title(text.into()))
        .encode()
        .unwrap()
}

struct Cross {
    _hub: std::sync::Arc<Hub>,
    _root: tempfile::TempDir,
    server: Node,
    store: ServerStore,
    x: StudioTarget,
    y: StudioTarget,
    x_close: catcoms_replication::CloseRecord,
    item: StudioRecoveryItem,
    /// Private, so held previews never compete with other tests for the process-wide pool.
    pool: std::sync::Arc<tokio::sync::Semaphore>,
}

impl Cross {
    /// X has a branch whose accepted entry sets the title the copy takes. With `x_open` X's
    /// successor is installed and the branch retained; without it X is still Closing and the branch
    /// can take further Saves. Y is Open with a branch of its own retained, so a transfer hold can
    /// be staged on it.
    fn new(x_open: bool) -> Self {
        let hub = Hub::new();
        let mut server = Server::found(
            hub.join(PeerId::from_u64(1)),
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(ManualClock::new(1000)),
            "copy-cross",
        )
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let mut store = ServerStore::open(root.path(), b"copy-cross", &mut rng()).unwrap();
        let (x, y) = (flipnote(X), flipnote(Y));
        let x_close = closing_branch(
            &mut server,
            &mut store,
            x,
            title_op("X before"),
            title_op("padding"),
            title_op(COPIED),
        );
        if x_open {
            install_successor(&mut server, &mut store, x, &x_close);
        }
        let y_close = closing_branch(
            &mut server,
            &mut store,
            y,
            title_op("Y before"),
            title_op("padding"),
            title_op("Y's own branch"),
        );
        install_successor(&mut server, &mut store, y, &y_close);
        // The value a renderer names is the accepted entry's operation id, which `closing_branch`
        // saved under nonce 3.
        let logical = x.document(&server.group_id()).unwrap();
        let accepted = DomainOp {
            body: title_op(COPIED),
            nonce: [3; 16],
            doc_type: logical.doc_type,
            logical_key: logical.logical_key,
        };
        let item = StudioRecoveryItem::Title {
            value: accepted.id(&server.device_id()),
        };
        Self {
            _hub: hub,
            _root: root,
            server,
            store,
            x,
            y,
            x_close,
            item,
            pool: std::sync::Arc::new(tokio::sync::Semaphore::new(4)),
        }
    }

    fn choice(&self) -> StudioOverlayCopyChoice {
        StudioOverlayCopyChoice {
            destination: self.y,
            item: self.item,
            mode: StudioRecoveryMode::Copy,
        }
    }

    /// C1 and C2.
    async fn plan(&mut self) -> Result<StudioPreparedCopy, AppError> {
        let (choice, pool) = (self.choice(), self.pool.clone());
        let preparation =
            self.server
                .begin_copy_with_pool(&self.store, SERVER, self.x, choice, &pool)?;
        Ok(preparation.plan().await.unwrap())
    }

    /// C3, answered with the preview's disposition.
    fn preview(
        &mut self,
        prepared: StudioPreparedCopy,
    ) -> Result<StudioRecoveryDisposition, AppError> {
        self.server
            .finish_studio_copy_preview(&mut self.store, SERVER, self.x, prepared)
            .map(|preview| preview.value().disposition)
    }

    /// The renderer's echo of a plan. Each new copy takes its own `nonce`: the same nonce and body
    /// as a copy that already landed is that copy's exact retry, and C4 answers it as such.
    fn echo(&self, plan: &StudioOverlayCopyPlan, nonce: u8) -> StudioOverlayCopyApply {
        StudioOverlayCopyApply {
            destination: self.y,
            item: self.item,
            mode: StudioRecoveryMode::Copy,
            epoch_id: plan.epoch_id(),
            expected_projection: plan.fingerprint(),
            nonce: [nonce; 16],
            body: plan.body().cloned().expect("a Ready plan carries a body"),
        }
    }

    /// C4, then the actor's publication of what it returns.
    fn apply(&mut self, echo: StudioOverlayCopyApply) -> Result<bool, AppError> {
        let (request, already) =
            self.server
                .prepare_studio_copy_apply(&mut self.store, SERVER, self.x, echo)?;
        self.server
            .studio_transaction(&mut self.store, SERVER, request)
            .unwrap();
        Ok(already)
    }

    /// Another real Closing-overlay Save onto X's live branch. It must be new work, not an exact
    /// retry the store merely acknowledges, or the cases built on it would test nothing.
    fn save_on_x(&mut self, body: Vec<u8>, nonce: u8) {
        let logical = self.x.document(&self.server.group_id()).unwrap();
        let operation = DomainOp {
            body,
            nonce: [nonce; 16],
            doc_type: logical.doc_type,
            logical_key: logical.logical_key,
        };
        let close = self.x_close.clone();
        let accepted = self.branch(self.x).2;
        closing_save(&mut self.server, &mut self.store, self.x, &close, operation);
        assert_eq!(
            self.branch(self.x).2,
            accepted + 1,
            "precondition: the Save landed on X's branch as new work"
        );
    }

    fn installed_title(&mut self, target: StudioTarget) -> String {
        let store = &mut self.store;
        let title = self
            .server
            .sync
            .with_registry_context(|g, d, _, _| {
                store.with_studio_source(SERVER, g, target, d, |s| {
                    let StudioProjection::Flipnote(art) = s.projection().map_err(invalid)? else {
                        panic!("a Flipnote")
                    };
                    Ok(art.title.map(|t| t.selected.value))
                })
            })
            .unwrap()
            .expect("an installed source");
        title.unwrap_or_default()
    }

    /// Y's installed Open epoch, which an ordinary Save into Y names.
    fn y_epoch(&mut self) -> u128 {
        let (store, y) = (&mut self.store, self.y);
        self.server
            .sync
            .with_registry_context(|g, d, _, _| {
                store.with_studio_source(SERVER, g, y, d, |s| Ok(s.doc_id()))
            })
            .unwrap()
            .expect("Y is installed")
    }

    fn stage_hold(&mut self, target: StudioTarget) {
        let mut b = budget(&mut self.store, &mut self.server);
        let store = &mut self.store;
        self.server
            .sync
            .with_registry_context(|g, d, _, r| {
                store.stage_studio_transfer_hold_for_test(SERVER, g, target, d, 0, r, &mut b);
                Ok::<_, AppError>(())
            })
            .unwrap();
    }

    fn prepared(&self, target: StudioTarget) -> bool {
        let logical = target.document(&self.server.group_id()).unwrap();
        self.store
            .load_epoch_intents_structural(SERVER, &logical)
            .unwrap()
            .handoff_prepared()
    }

    fn branch(&self, target: StudioTarget) -> ([u8; 32], [u8; 32], usize, Vec<u8>) {
        let logical = target.document(&self.server.group_id()).unwrap();
        self.store.studio_branch_identity_for_test(SERVER, &logical)
    }

    /// The target's whole intent ledger, encoded: every intent, not only the branch's.
    fn ledger(&self, target: StudioTarget) -> Vec<u8> {
        let logical = target.document(&self.server.group_id()).unwrap();
        self.store.studio_intent_ledger_for_test(SERVER, &logical)
    }
}

/// L-2 (a): a transfer hold on the destination refuses the copy at every stage that consults it.
/// Every refusal is by message, so no other refusal can stand in for the hold check.
#[tokio::test]
async fn a_cross_document_copy_refuses_while_the_destination_is_held() {
    let mut f = Cross::new(true);
    let before = f.installed_title(f.y);
    let prepared = f.plan().await.unwrap();
    assert_eq!(
        prepared.plan.disposition(),
        StudioRecoveryDisposition::Ready,
        "precondition: the planner's own verdict is Ready"
    );
    let echo = f.echo(&prepared.plan, 0x5b);
    f.stage_hold(f.y);
    assert!(f.prepared(f.y), "precondition: the hold is staged on Y");

    let refused = f
        .preview(prepared)
        .expect_err("C3 must refuse while another document's destination is held")
        .to_string();
    assert!(refused.contains("transfer hold"), "C3 refusal: {refused}");
    let refused = f
        .plan()
        .await
        .expect_err("C1 must refuse while another document's destination is held")
        .to_string();
    assert!(refused.contains("transfer hold"), "C1 refusal: {refused}");
    let refused = f
        .apply(echo)
        .expect_err("C4 must refuse while another document's destination is held")
        .to_string();
    assert!(refused.contains("transfer hold"), "C4 refusal: {refused}");
    assert_eq!(
        f.installed_title(f.y),
        before,
        "a refused copy wrote nothing"
    );
    assert_eq!(
        f.pool.available_permits(),
        4,
        "and keeps no preparation slot"
    );
}

/// L-2 (b): a transfer hold on the SOURCE permits the copy (C1'), and the copy does not touch the
/// handoff it found: X stays `Prepared`, on the same branch, with the same entries, and no intent
/// anywhere in X's record is retired or added.
#[tokio::test]
async fn a_cross_document_copy_from_a_held_source_applies_and_leaves_its_handoff_prepared() {
    let mut f = Cross::new(true);
    f.stage_hold(f.x);
    assert!(f.prepared(f.x), "precondition: the hold is staged on X");
    let branch = f.branch(f.x);
    let ledger = f.ledger(f.x);

    let prepared = f
        .plan()
        .await
        .expect("C1' permits a copy from a held source");
    let echo = f.echo(&prepared.plan, 0x5b);
    assert_eq!(
        f.preview(prepared)
            .expect("C3 permits a copy from a held source"),
        StudioRecoveryDisposition::Ready
    );
    assert!(
        !f.apply(echo).expect("C4 permits a copy from a held source"),
        "nothing had landed yet"
    );
    assert_eq!(f.installed_title(f.y), COPIED, "the copy landed in Y");

    assert!(
        f.prepared(f.x),
        "a copy does not clear the source's Prepared handoff"
    );
    assert_eq!(
        f.branch(f.x),
        branch,
        "nor change anything that records the source's branch"
    );
    assert_eq!(
        f.ledger(f.x),
        ledger,
        "nor retire or add any intent in the source's record"
    );
}

/// A hold staged on the source mid-copy (the review of the cross-document fixture, LOW-2).
///
/// C1' permits a copy from a held source, but staging the hold writes the source's intent record,
/// so the stage that compares that record sees it move:
/// - staged between C1 and C3, C3 refuses the preview as changed, and a fresh preview is `Ready`
///   and applies;
/// - staged between C3 and C4, C4 compares no source stamp, so the copy applies.
///
/// Either way the source stays `Prepared` on the same branch. "Permitted" for a hold staged
/// mid-copy therefore means "after a fresh preview" when the hold lands before C3.
#[tokio::test]
async fn a_hold_staged_on_the_source_mid_copy_costs_at_most_a_fresh_preview() {
    // Between C1 and C3.
    let mut f = Cross::new(true);
    let prepared = f.plan().await.unwrap();
    f.stage_hold(f.x);
    let refused = f
        .preview(prepared)
        .expect_err("C3 must see the source record the hold rewrote")
        .to_string();
    assert!(refused.contains("overlay copy changed"), "{refused}");
    let branch = f.branch(f.x);
    let prepared = f.plan().await.unwrap();
    let echo = f.echo(&prepared.plan, 0x5b);
    assert_eq!(
        f.preview(prepared).unwrap(),
        StudioRecoveryDisposition::Ready
    );
    assert!(!f.apply(echo).unwrap());
    assert_eq!(f.installed_title(f.y), COPIED);
    assert!(f.prepared(f.x) && f.branch(f.x) == branch);

    // Between C3 and C4.
    let mut f = Cross::new(true);
    let prepared = f.plan().await.unwrap();
    let echo = f.echo(&prepared.plan, 0x5b);
    assert_eq!(
        f.preview(prepared).unwrap(),
        StudioRecoveryDisposition::Ready
    );
    f.stage_hold(f.x);
    let branch = f.branch(f.x);
    assert!(
        !f.apply(echo)
            .expect("C4 permits a copy whose source was held after its preview"),
        "nothing had landed yet"
    );
    assert_eq!(f.installed_title(f.y), COPIED);
    assert!(f.prepared(f.x) && f.branch(f.x) == branch);
}

/// L-1, the stamp half. An overlay Save lands on X's live branch while a copy from it is in flight.
///
/// - Between C1 and C3, any Save changes X's stamp, and C3 refuses the preview as changed.
/// - Between C3 and C4, there is no stamp to compare. A Save of another element leaves the selected
///   value as it was, so C4's re-plan matches and the copy applies that value, which is current. A
///   Save that replaces the selected value leaves the echo naming a value X no longer holds, and C4
///   refuses it. Y is unchanged by the refusal.
#[tokio::test]
async fn a_save_landing_on_the_source_mid_copy_is_refused_at_c3_and_at_c4_only_if_it_moved_the_value(
) {
    let mut f = Cross::new(false);
    let fps = |n: u8| {
        FlipnoteOp::SetHeader(FlipnoteHeader::Fps(n))
            .encode()
            .unwrap()
    };

    // Between C1 and C3: any Save.
    let prepared = f.plan().await.unwrap();
    f.save_on_x(fps(12), 0x61);
    let refused = f
        .preview(prepared)
        .expect_err("C3 must refuse a source that a Save moved after C1")
        .to_string();
    assert!(
        refused.contains("overlay copy changed"),
        "C3 refuses a source that moved since C1: {refused}"
    );

    // Between C3 and C4: a Save of another element.
    let prepared = f.plan().await.unwrap();
    let echo = f.echo(&prepared.plan, 0x5b);
    assert_eq!(
        f.preview(prepared).unwrap(),
        StudioRecoveryDisposition::Ready
    );
    f.save_on_x(fps(15), 0x62);
    assert!(!f.apply(echo).unwrap(), "nothing had landed yet");
    assert_eq!(
        f.installed_title(f.y),
        COPIED,
        "the copy applied the value X still holds"
    );

    // Between C3 and C4: a Save that replaces the selected value. Y first gets a title of its own
    // again, so the copy would change it.
    let y_epoch = f.y_epoch();
    f.server
        .studio_transaction(
            &mut f.store,
            SERVER,
            StudioRequest::Apply {
                target: f.y,
                epoch_id: y_epoch,
                nonce: [0x71; 16],
                body: title_op("Y again"),
            },
        )
        .unwrap();
    // A new copy, so a new renderer nonce: under the landed copy's nonce and body this echo would
    // be that copy's exact retry, which C4 answers before it re-plans.
    let prepared = f.plan().await.unwrap();
    let echo = f.echo(&prepared.plan, 0x5c);
    assert_eq!(
        f.preview(prepared).unwrap(),
        StudioRecoveryDisposition::Ready
    );
    f.save_on_x(title_op("X's newer title"), 0x63);
    let refused = f
        .apply(echo)
        .expect_err("C4 must refuse an echo whose value the source replaced")
        .to_string();
    assert!(
        refused.contains("recovery value is not in this version"),
        "C4 refuses an echo naming a value the source no longer holds: {refused}"
    );
    assert_eq!(
        f.installed_title(f.y),
        "Y again",
        "a refused copy wrote nothing"
    );
}
