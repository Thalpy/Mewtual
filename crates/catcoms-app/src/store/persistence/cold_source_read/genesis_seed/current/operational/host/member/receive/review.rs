//! Held review is an original founder's passive audit. The returned page
//! keeps the row/membership revocation and the two independent revisions.
use super::*;
use crate::store::{
    persistence::v2_shared_history::SharedHistoryScope, NativeTextHeldReviewContinuation,
    NativeTextHeldReviewPage,
};
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
