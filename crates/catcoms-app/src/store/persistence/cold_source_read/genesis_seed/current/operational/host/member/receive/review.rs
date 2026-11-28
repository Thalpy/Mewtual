//! Held review is an original founder's passive audit. The returned page
//! keeps the row/membership revocation and the two independent revisions.
use super::*;
use crate::store::critical_namespaces::native_registry::founder_review_intake::{
    frozen_author, review_ancestry, FounderReviewCommand, FounderReviewIntake, FounderReviewOrigin,
    FounderReviewOutcome, OpenedFounderReview, REPLY_FEE, SWEEP_FEE, VERIFY_FEE, WIRE_FEE,
};
use crate::store::critical_namespaces::native_registry::founder_review_intake::{
    run_founder_review_transaction, FounderReviewSign, WarmFounderReviewKeys,
};
use crate::store::NativeTextInitialReviewRequest;
use crate::store::{
    persistence::{v2_member_outbox::MemberSavedOriginal, v2_shared_history::SharedHistoryScope},
    NativeTextFounderReviewResult, NativeTextHeldReviewContinuation, NativeTextHeldReviewPage,
};
use catcoms_net::NativeChatMemberDependencies;
use catcoms_replication::chat::owner_bodies::ReviewStreamEntry;
use catcoms_replication::chat::FounderGenesisAnchorData;

pub(super) fn read_warm_founder_review(
    store: &ServerStore,
    scope: SharedHistoryScope,
    (pin, verifier, retired): (
        &FounderGenesisAnchorData,
        &V2GeneralPrefixVerifier,
        &[V2FrozenAdmission],
    ),
    (after, limit): (Option<&NativeTextHeldReviewContinuation>, usize),
    signal: RequestCancellation,
    mut require_selected: impl FnMut(Option<(OwnerHeaderId, u64)>) -> Result<(), AppError>,
) -> Result<NativeTextHeldReviewPage, AppError> {
    let mut require = |anchor| {
        if signal.is_cancelled() {
            return Err(refused());
        }
        require_selected(anchor)
    };
    let page = store.read_selected_review_window(
        scope,
        after,
        limit,
        || require(None),
        |record| {
            let author = record.target_data().spec().context.admission;
            let frozen = retired
                .iter()
                .find(|origin| origin.admission_id_data() == author)
                .ok_or_else(refused)?;
            verifier
                .verify_removed_create_record_for_audit(record, frozen)
                .map_err(|_| refused())
        },
    )?;
    require(None)?;
    // A damaged or absent decision journal leaves covered hold evidence
    // readable with the unavailable status, as on a recovered founder.
    let decisions = page.as_ref().and_then(|(window, _)| {
        store
            .read_founder_review_statuses(pin, scope.channel, window.records_data(), |position| {
                require(position.map(|p| (p.membership_header, p.membership_epoch)))
            })
            .ok()
    });
    require(None)?;
    let page = page
        .map(|(prepared, revision)| prepared.finish_files().map(|page| (page, revision)))
        .transpose()?;
    Ok(NativeTextHeldReviewPage::new(page, scope, signal).with_decisions(decisions))
}

/// The whole warm loan remains here only for local preparation or a
/// received response. The transport await owns detached paid data.
pub(super) fn run_warm_removed_review<T: MeshTransport, R: CryptoRngCore>(
    (store, history): (&ServerStore, &FounderHistory<'_>),
    (owner, member, verifier): (
        &V2TextOwnedCurrent<T, R>,
        &V2MemberLane,
        &V2GeneralPrefixVerifier,
    ),
    native: &NativeChatSourceObservation,
    (operation, route): (FounderReviewCommand<'_>, &NativeChatMemberDependencies),
    caller: &RequestCancellation,
    mut require_selected: impl FnMut() -> Result<(), AppError>,
    rng: &mut impl CryptoRngCore,
) -> Result<FounderReviewOutcome, AppError> {
    let mut check = || {
        if caller.is_cancelled() {
            return Err(refused());
        }
        require_selected()
    };
    check()?;
    let pool = &store
        .archive_bootstrap_policy
        .as_ref()
        .ok_or_else(refused)?
        .pending
        .pool;
    let pay = |bytes| pool.try_reserve_classified(bytes, || AppError::ChatV2ArchiveBusy);
    let channel = history.scope.channel;
    let intake = FounderReviewIntake {
        origin: FounderReviewOrigin::Warm(native),
        verifier,
        frozen: &member.retired_history,
    };
    let result = match operation {
        FounderReviewCommand::Start(admission, contact) => {
            let fee = pay(SWEEP_FEE)?;
            let (_, observation) =
                owner.prepare_governed_member_subscription(store, verifier, channel)?;
            FounderReviewOutcome::Sweep(intake.start(
                (channel, admission, *contact),
                route,
                observation,
                caller,
                fee,
            )?)
        }
        FounderReviewCommand::Prepare(sweep, query) => {
            if sweep.channel_data() != channel {
                return Err(refused());
            }
            let fee = pay(WIRE_FEE)?;
            let access = owner.select_governed_dependency_access(store, verifier, channel)?;
            FounderReviewOutcome::Exchange(Box::new(intake.prepare(
                (sweep, query),
                route,
                access,
                caller,
                fee,
                rng,
            )?))
        }
        FounderReviewCommand::Receive(reply) => {
            let (offered_channel, target, _) = reply.scope_data();
            if offered_channel != channel {
                return Err(refused());
            }
            let fee = pay(REPLY_FEE)?;
            let access = owner.select_governed_dependency_access(store, verifier, channel)?;
            let result = match intake.open(reply, access, fee)? {
                OpenedFounderReview::Page(page) => NativeTextFounderReviewResult::Page(page),
                OpenedFounderReview::Original(action, original) => {
                    let _work = pay(VERIFY_FEE)?;
                    let author = frozen_author(intake.frozen, target.admission)?;
                    let mut require_current = || {
                        check()?;
                        intake.require_reply(reply)
                    };
                    let exact = original.original_data();
                    let ancestry = review_ancestry(
                        (store, history.scope),
                        (verifier, intake.frozen),
                        (author, exact),
                        |ledger| member.domain.begin_channel_view(ledger),
                        |id| exact_saved_dependency(store, history, id, rng),
                        &mut require_current,
                    )?;
                    let derive = |bytes: &[u8]| ancestry.evidence(verifier, author, bytes);
                    let evidence = derive(exact)?;
                    let retained = store.retain_removed_create(
                        history.scope,
                        (verifier, &evidence, exact),
                        require_current,
                        derive,
                        rng,
                    )?;
                    if retained.is_some() {
                        NativeTextFounderReviewResult::Held(action)
                    } else {
                        NativeTextFounderReviewResult::AlreadyInHistory(action)
                    }
                }
            };
            intake.require_reply(reply)?;
            FounderReviewOutcome::Received(result)
        }
    };
    check()?;
    Ok(result)
}

fn exact_saved_dependency(
    store: &ServerStore,
    history: &FounderHistory<'_>,
    id: ActionId,
    rng: &mut dyn CryptoRngCore,
) -> Result<Option<MemberSavedOriginal>, V2MemberSaveRefusal> {
    use V2MemberSaveRefusal as Refusal;
    let mount = store
        .member_outbox_mount()
        .map_err(|_| Refusal::Unavailable)?;
    let key = member_outbox_key(store).map_err(|_| Refusal::Unavailable)?;
    history.saved(&mount, &key, id, rng)
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    pub(in crate::store) fn finish_warm_founder_review(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: (NativeTextInitialReviewRequest, bool),
        (keys, cut, ends): WarmFounderReviewKeys<'_>,
        caller: &RequestCancellation,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Option<ReviewStreamEntry>, AppError> {
        keys.preflight(&self.origin)?;
        let store = layout.checked_store()?;
        let governed = self.governed_add.as_deref_mut().ok_or_else(refused)?;
        let member = self.member.as_deref().ok_or_else(refused)?;
        member.validate_accounts(store)?;
        let verifier = member.verifier.as_ref().ok_or_else(refused)?;
        let original = self.owner.borrow_current()?;
        let pin = original_review_anchor(original.source().source());
        let history = FounderHistory::new(
            store,
            verifier,
            original,
            request.0.channel,
            member.admission,
            member.outbox_context(),
        )
        .map_err(|_| refused())?;
        let _ancestry_fee = store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool
            .try_reserve_classified(VERIFY_FEE, || AppError::ChatV2ArchiveBusy)?;
        run_founder_review_transaction(
            (store, history.scope, &pin),
            (verifier, &member.retired_history),
            request,
            |anchor| {
                if caller.is_cancelled() {
                    return Err(refused());
                }
                keys.preflight(&self.origin)?;
                check_governed_member_at(
                    &self.owner,
                    governed,
                    &member.domain,
                    verifier,
                    store,
                    anchor,
                )
            },
            (
                |ledger| member.domain.begin_channel_view(ledger),
                |id, rng| exact_saved_dependency(store, &history, id, rng),
            ),
            |decision, preimage| {
                self.owner.sign_governed_review(
                    store,
                    verifier,
                    FounderReviewSign {
                        decision,
                        preimage,
                        key: (&keys.original, cut, ends),
                    },
                )
            },
            rng,
        )
    }
}
