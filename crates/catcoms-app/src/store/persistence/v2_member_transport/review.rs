//! Shared connected review egress. The original warm or recovered founder
//! supplies the exact selected membership proof for every journal read.
use super::*;
use crate::store::persistence::governance_layout::joined::review_decision::ReviewDelivery;
use crate::store::RetainedBootstrapSourceData;
use catcoms_replication::chat::{
    owner_bodies::ReviewStreamQuery, owner_header::OwnerHeaderId, FounderGenesisAnchorData,
};
use std::cell::RefCell;
use zeroize::Zeroizing;

pub(in crate::store) fn original_review_anchor(
    source: &RetainedBootstrapSourceData,
) -> FounderGenesisAnchorData {
    let genesis = source.data().genesis_public_data();
    let profile = genesis.profile_data();
    let header = genesis.header_data();
    FounderGenesisAnchorData {
        group: profile.spec().group.clone(),
        founder: profile.spec().founder,
        profile: profile.id(),
        tenure: profile.tenure(),
        admission: genesis.admission_data().admission_id(),
        header: header.id(),
        state: header.spec().state,
    }
}

pub(in crate::store) fn prepare_founder_review_delivery_reply(
    store: &ServerStore,
    (pin, verifier, frozen): (
        &FounderGenesisAnchorData,
        &V2GeneralPrefixVerifier,
        &[V2FrozenAdmission],
    ),
    provider: V2FounderMemberDependencyAccess<'_>,
    (incoming, caller): (&NativeChatMemberDependencyIncoming, &RequestCancellation),
    fee: CatalogReadReservation,
    require_selected: impl FnMut(Option<(OwnerHeaderId, u64)>) -> Result<(), AppError>,
    rng: &mut impl CryptoRngCore,
) -> Result<FounderDependencyReply, AppError> {
    let (requester, wire) = incoming.current_data().ok_or_else(refused)?;
    let request = provider.open_review_delivery_request(verifier, pin, requester, wire)?;
    let selected = RefCell::new(require_selected);
    let require = |anchor| {
        if caller.is_cancelled()
            || incoming.current_data().is_none()
            || !request.requester_data().is_live_cached()
        {
            return Err(refused());
        }
        selected.borrow_mut()(anchor)
    };
    let query = request.query_data();
    let response = store.read_public_review(
        pin,
        query,
        |position| {
            require(
                position.map(|position| (position.membership_header, position.membership_epoch)),
            )
        },
        |record| {
            let ReviewStreamQuery::Approval { channel, .. } = query else {
                return Err(refused());
            };
            let held = record.held_data();
            let target = held.target_data().spec();
            if target.channel != channel {
                return Ok(false);
            }
            let exact = record.decision_data().original_data();
            require(Some((exact.remove_header, exact.remove_epoch)))?;
            let author = frozen
                .iter()
                .find(|author| author.admission_id_data() == target.context.admission)
                .ok_or_else(refused)?;
            // The writer proved the reply parents before signing. Serving
            // rechecks the exact frozen post without taking the ordinary
            // archive gate inside ID11. Recipients verify their own parents.
            verifier
                .verify_removed_create_record_for_audit(held, author)
                .map_err(|_| refused())?;
            Ok(true)
        },
    )?;
    let frame = response.map(|reply| match reply {
        ReviewDelivery::Header(entry) => Zeroizing::new(entry.encode()),
        ReviewDelivery::Approval(frame) => frame,
    });
    require(None)?;
    let wire = provider.seal_review_delivery_reply(
        pin,
        requester,
        &request,
        frame.as_deref().map(Vec::as_slice),
        rng,
    )?;
    require(None)?;
    Ok(FounderDependencyReply {
        wire: bytes::Bytes::from(wire),
        observation: provider.into_observation(),
        requester: Some(request.into_requester()),
        fee,
    })
}
