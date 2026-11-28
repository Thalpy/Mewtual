//! A real freshly founded and joined group with both review mounts ready.
use super::*;
use crate::store::{NativeTextInvitePacketData, NativeTextJoinedRowSelection};
use catcoms_replication::chat::owner_bodies::ReviewStreamQuery;
use catcoms_rt::OsCryptoRng;
use std::path::Path;

pub(super) fn replace_file_after_review_status<T>(path: &Path, read: impl FnOnce() -> T) -> T {
    use crate::store::persistence::governance_layout::joined::review_decision::AFTER_REVIEW_STATUS_READ;

    let ciphertext = std::fs::read(path).unwrap();
    let parked = path.with_extension("audit-parked");
    let (name, original) = (path.to_path_buf(), parked.clone());
    AFTER_REVIEW_STATUS_READ.with(|hook| {
        hook.set(Some(Box::new(move || {
            std::fs::rename(&name, original).unwrap();
            std::fs::write(name, ciphertext).unwrap();
        })));
    });
    let result = read();
    assert!(parked.is_file(), "the actual decision read reached the cut");
    std::fs::remove_file(path).unwrap();
    std::fs::rename(parked, path).unwrap();
    result
}

pub(super) async fn prove_empty_review_on_original_live_owner(
    (founder, row, founder_vault, founder_budget): (
        &Owner,
        &Row,
        &NativeTextVault,
        &NativeUiInvocationBudget,
    ),
    (member, member_vault, member_budget): (
        &NativeTextJoinedRowSelection,
        &NativeTextVault,
        &NativeUiInvocationBudget,
    ),
    joined: &Owner,
    invite: &NativeTextInvitePacketData<'_>,
    caller: &RequestCancellation,
) {
    let founder_prepare = || {
        founder.capture_member(
            row,
            command(founder_vault, founder_budget),
            founder.policy.prepare_member().unwrap(),
        )
    };
    let member_prepare = || joined.capture_joined_selection(command(member_vault, member_budget));
    let channel = 7;
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let audit_scope = RequestCancellation::new(stopped, None);
    let audit = founder_prepare()
        .audit_founder_held_posts(channel, None, 8, &audit_scope)
        .unwrap();
    audit
        .with_records(|rows, unavailable| {
            assert_eq!(unavailable, 0);
            assert!(rows.is_some_and(|rows| rows.is_empty()));
        })
        .unwrap();
    stop.send(true).unwrap();
    assert!(audit
        .with_publication(|| panic!("retired caller published the warm audit"))
        .is_err());
    drop(audit);

    // The fresh Join already owns a real authenticated connection. Use
    // that route to prove same-session review service.
    let contact = member_prepare()
        .sign_member_contact(member, caller)
        .unwrap();
    // The positive response must go through the original warm founder. A
    // dropped request or generic failure cannot substitute for a checked
    // empty journal, even when the ordinary carrier uses another channel.
    let mut provider = founder_prepare()
        .attach_founder_dependency_provider(channel ^ 1, caller.clone())
        .unwrap();
    let exchange = member_prepare()
        .prepare_review_delivery(
            member,
            invite,
            (&contact, ReviewStreamQuery::Header(1)),
            caller.clone(),
            &mut OsCryptoRng,
        )
        .unwrap();
    let serve = async {
        let incoming = provider
            .next()
            .await
            .expect("the warm founder receives the query");
        let original = founder_prepare();
        tokio::task::spawn_blocking(move || {
            original
                .serve_founder_dependency_request(&provider, incoming)
                .unwrap();
            provider
        })
        .await
        .unwrap()
    };
    let (response, _provider) = tokio::time::timeout(Duration::from_secs(12), async {
        tokio::join!(exchange.receive(), serve)
    })
    .await
    .unwrap();
    member_prepare()
        .with_review_delivery(member, response.unwrap(), caller, |reply| {
            assert!(reply.is_none());
            Ok(())
        })
        .unwrap();
}
