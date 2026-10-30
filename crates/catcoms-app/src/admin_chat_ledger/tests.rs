use super::*;
use catcoms_mls::MlsDevice;
use catcoms_replication::op::SignedOp;
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use tokio::sync::watch;

type Node = Server<MemNetwork, ChaCha20Rng>;
async fn node(clock: &ManualClock) -> Node {
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(700)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(700),
        Box::new(clock.clone()),
        "ledger",
    )
    .unwrap();
    // The actual desktop bootstrap opens the directory before Create channel posts to it.
    server.open_channel_index().await.unwrap();
    server
}
fn scope(budget: Arc<AdminReadBudget>) -> (AdminReadScope, watch::Sender<bool>) {
    let (owner, observer) = watch::channel(false);
    (
        AdminReadScope::new(RequestCancellation::new(observer, None), budget),
        owner,
    )
}
fn page(
    node: &mut Node,
    scope: &AdminReadScope,
    cursor: Option<&str>,
    limit: usize,
) -> AdminLedgerPage {
    for _ in 0..4097 {
        match node.admin_chat_ledger_page(scope, cursor, limit) {
            Ok(page) => return page,
            Err(AdminLedgerError::Busy) => {}
            Err(error) => panic!("unexpected ledger read: {error:?}"),
        }
    }
    panic!("finite captured prefix did not complete")
}

#[tokio::test]
async fn retained_originals_survive_actual_edit_delete_and_exact_detail_reads() {
    let mut node = node(&ManualClock::new(1000)).await;
    let channel = node.create_channel("chat").await.unwrap().id;
    node.send_message(channel, "first original").await.unwrap();
    let id = node.messages(channel)[0].id.clone();
    node.edit_message(channel, &id, "second original")
        .await
        .unwrap();
    node.edit_message(channel, &id, "third original")
        .await
        .unwrap();
    node.delete_message(channel, &id).await.unwrap();
    assert!(node.messages(channel).is_empty());
    let (scope, _owner) = scope(AdminReadBudget::new(8 * 1024 * 1024));
    let page = page(&mut node, &scope, None, 32);
    assert!(
        page.incomplete,
        "original attribution is not full graph/typed authority"
    );
    assert_eq!(page.known_row_count, 4);
    assert_eq!(page.rows.len(), 4);
    assert!(page.next_cursor.is_none());
    let mut originals = Vec::new();
    for row in &page.rows {
        let detail = node
            .admin_chat_ledger_detail(&scope, &row.row_id, &page.revision, 0)
            .unwrap();
        assert!(detail.next_offset.is_none());
        let op = SignedOp::decode(&detail.frame_bytes).unwrap();
        assert!(op.verify());
        assert_eq!(detail.signature, op.signature);
        assert!(matches!(detail.decoded, LegacyChangeInspection::Decoded(_)));
        originals.push(
            serde_json::to_string(&automerge::Change::from_bytes(op.delta).unwrap().decode())
                .unwrap(),
        );
    }
    assert!(originals.iter().any(|s| s.contains("first original")));
    assert!(originals.iter().any(|s| s.contains("second original")));
    assert!(originals.iter().any(|s| s.contains("third original")));
    assert!(originals.iter().any(|s| s.contains("del")));
}

#[tokio::test]
async fn full_32_row_page_has_same_channel_cursor_and_new_appends_do_not_starve_snapshot() {
    let mut node = node(&ManualClock::new(1000)).await;
    let channel = node.create_channel("chat").await.unwrap().id;
    for i in 0..40 {
        node.send_message(channel, &format!("original{i}"))
            .await
            .unwrap();
    }
    let (scope, _owner) = scope(AdminReadBudget::new(8 * 1024 * 1024));
    let first = page(&mut node, &scope, None, 32);
    assert_eq!(first.rows.len(), 32);
    assert_eq!(first.known_row_count, 40);
    let cursor = first
        .next_cursor
        .as_deref()
        .expect("same-channel look-ahead");
    node.send_message(channel, "after capture").await.unwrap();
    let next = page(&mut node, &scope, Some(cursor), 32);
    assert_eq!(next.revision, first.revision);
    assert_eq!(next.known_row_count, 40);
    assert_eq!(next.rows.len(), 8);
    assert!(next.next_cursor.is_none());
    let ids: std::collections::BTreeSet<_> = first
        .rows
        .iter()
        .chain(&next.rows)
        .map(|row| &row.row_id)
        .collect();
    assert_eq!(ids.len(), 40);
    let refreshed = page(&mut node, &scope, None, 32);
    assert_ne!(refreshed.revision, first.revision);
    assert_eq!(refreshed.known_row_count, 41);
    assert!(matches!(
        node.admin_chat_ledger_detail(&scope, &first.rows[0].row_id, &first.revision, 0),
        Err(AdminLedgerError::Stale)
    ));
}

#[tokio::test]
async fn new_channel_during_build_does_not_restart_finite_original_inventory() {
    let mut node = node(&ManualClock::new(1000)).await;
    let a = node.create_channel("a").await.unwrap().id;
    for i in 0..20 {
        node.send_message(a, &format!("a{i}")).await.unwrap();
    }
    let (scope, _owner) = scope(AdminReadBudget::new(8 * 1024 * 1024));
    assert!(matches!(
        node.admin_chat_ledger_page(&scope, None, 32),
        Err(AdminLedgerError::Busy)
    ));
    let b = node.create_channel("b").await.unwrap().id;
    node.send_message(b, "new channel").await.unwrap();
    let captured = page(&mut node, &scope, None, 32);
    assert_eq!(captured.known_row_count, 20);
    assert!(captured.rows.iter().all(|row| row.channel == a));
    let refreshed = page(&mut node, &scope, None, 32);
    assert_eq!(refreshed.known_row_count, 21);
    assert!(refreshed
        .rows
        .windows(2)
        .all(|pair| pair[0].row_id < pair[1].row_id));
}

#[tokio::test]
async fn original_scope_retirement_precedes_malformed_cursor_and_old_unlock_never_revives_it() {
    let mut node = node(&ManualClock::new(1000)).await;
    let channel = node.create_channel("chat").await.unwrap().id;
    node.send_message(channel, "private original")
        .await
        .unwrap();
    let budget = AdminReadBudget::new(8 * 1024 * 1024);
    let (old, owner) = scope(budget.clone());
    let first = page(&mut node, &old, None, 32);
    assert!(budget.charged_bytes() > 0);
    owner.send_replace(true);
    assert!(matches!(
        node.admin_chat_ledger_page(&old, Some("invalid"), 0),
        Err(AdminLedgerError::SessionUnavailable)
    ));
    assert_eq!(budget.charged_bytes(), 0);
    let (fresh, _fresh_owner) = scope(budget);
    let newer = page(&mut node, &fresh, None, 32);
    assert_ne!(first.revision, newer.revision);
    assert!(matches!(
        node.admin_chat_ledger_detail(&old, "bad", "bad", 0),
        Err(AdminLedgerError::SessionUnavailable)
    ));
}

#[tokio::test]
async fn cache_quota_is_aggregate_and_capacity_refusal_preserves_all_originals() {
    let clock = ManualClock::new(1000);
    let mut first = node(&clock).await;
    let channel = first.create_channel("chat").await.unwrap().id;
    first.send_message(channel, "keep this").await.unwrap();
    let fee = CACHE_BASE_BYTES + CHANNEL_BYTES + ORIGINAL_ROW_BYTES;
    let budget = AdminReadBudget::new(fee);
    let (scope, _owner) = scope(budget.clone());
    let read = page(&mut first, &scope, None, 32);
    assert_eq!(budget.charged_bytes(), fee);
    let mut second = node(&clock).await;
    let before = second
        .sync
        .doc(DocType::Channel, channel)
        .map(|doc| doc.op_count());
    assert!(matches!(
        second.admin_chat_ledger_page(&scope, None, 32),
        Err(AdminLedgerError::Capacity)
    ));
    assert_eq!(
        second
            .sync
            .doc(DocType::Channel, channel)
            .map(|doc| doc.op_count()),
        before
    );
    assert_eq!(
        first
            .sync
            .doc(DocType::Channel, channel)
            .unwrap()
            .op_count(),
        1
    );
    assert!(first
        .admin_chat_ledger_detail(&scope, &read.rows[0].row_id, &read.revision, 0)
        .is_ok());
    drop(first);
    assert_eq!(budget.charged_bytes(), 0);
}

#[tokio::test]
async fn expiry_and_policy_change_stale_original_detail_without_reconstructing_messages() {
    let clock = ManualClock::new(1000);
    let mut node = node(&clock).await;
    let channel = node.create_channel("chat").await.unwrap().id;
    node.send_message(channel, "retained").await.unwrap();
    let (scope, _owner) = scope(AdminReadBudget::new(8 * 1024 * 1024));
    let first = page(&mut node, &scope, None, 32);
    clock.advance_ms(VIEW_TTL_MS);
    assert!(matches!(
        node.admin_chat_ledger_detail(&scope, &first.rows[0].row_id, &first.revision, 0),
        Err(AdminLedgerError::Stale)
    ));
    node.sync
        .open_channel(DocType::MemberRoles, ROLES_DOC)
        .await
        .unwrap();
    let next = page(&mut node, &scope, None, 32);
    // A real changed local admin generation invalidates a view even if message counts agree.
    node.sync
        .set_admin(&catcoms_crypto::DeviceId::from_bytes([99; 32]), false)
        .await
        .unwrap();
    assert!(matches!(
        node.admin_chat_ledger_detail(&scope, &next.rows[0].row_id, &next.revision, 0),
        Err(AdminLedgerError::Stale)
    ));
    assert_eq!(node.messages(channel)[0].text, "retained");
}

#[test]
fn fixed_reference_grammar_rejects_ambiguous_channels_and_hash_spellings() {
    let hash = "ab".repeat(32);
    assert!(parse_row(&format!("legacy:0:{hash}")).is_ok());
    for id in [
        "",
        "00",
        "01",
        "+1",
        "-1",
        "1 ",
        "340282366920938463463374607431768211456",
    ] {
        assert!(parse_row(&format!("legacy:{id}:{hash}")).is_err(), "{id:?}");
    }
    assert!(parse_row(&format!("legacy:1:{}", hash.to_uppercase())).is_err());
    assert!(parse_row(&format!("legacy:1:{hash}:extra")).is_err());
}

#[tokio::test]
async fn decimal_prefix_channels_page_in_exact_row_id_order_without_omission() {
    let mut node = node(&ManualClock::new(1000)).await;
    // Public channel documents can have any u128 namespace; hashed names usually hide this
    // prefix-order case, but retained/imported namespaces still need exact ASCII pagination.
    for id in [1, 10, 100, 2] {
        node.open_channel(id).await.unwrap();
        node.send_message(id, "original").await.unwrap();
    }
    let (scope, _owner) = scope(AdminReadBudget::new(8 * 1024 * 1024));
    let first = page(&mut node, &scope, None, 2);
    assert_eq!(first.known_row_count, 4);
    assert_eq!(first.rows.len(), 2);
    let last = page(&mut node, &scope, first.next_cursor.as_deref(), 2);
    assert_eq!(last.rows.len(), 2);
    assert!(last.next_cursor.is_none());
    let rows: Vec<_> = first.rows.iter().chain(&last.rows).collect();
    assert!(rows.windows(2).all(|pair| pair[0].row_id < pair[1].row_id));
    assert_eq!(
        rows.iter().map(|row| row.channel).collect::<Vec<_>>(),
        [100, 10, 1, 2]
    );
}

#[tokio::test]
async fn many_empty_sources_pay_actual_inline_index_layout_before_allocation() {
    let mut node = node(&ManualClock::new(1000)).await;
    for id in 0..64 {
        node.open_channel(id).await.unwrap();
    }
    assert!(CHANNEL_BYTES >= std::mem::size_of::<ChannelIndex>() + 47);
    // The old512B estimate admitted this cache even though every empty source still held
    // space for a resumable inline Hasher. Quota refusal leaves both graph and logs intact.
    let budget = AdminReadBudget::new(CACHE_BASE_BYTES + 64 * 512);
    let (scope, _owner) = scope(budget.clone());
    assert!(matches!(
        node.admin_chat_ledger_page(&scope, None, 32),
        Err(AdminLedgerError::Capacity)
    ));
    assert_eq!(budget.charged_bytes(), 0);
    assert!(node.admin_chat_ledger.view.is_none());
    assert_eq!(node.sync.channel_ids().len(), 64);
    assert!(node.sync.document_versions().all(|(_, count)| count == 0));
}

#[tokio::test]
async fn actual_actor_releases_quiet_views_on_scope_retirement_and_idle_expiry() {
    let clock = ManualClock::new(1000);
    let mut server = node(&clock).await;
    let channel = server.create_channel("chat").await.unwrap().id;
    server.send_message(channel, "original").await.unwrap();
    let budget = AdminReadBudget::new(8 * 1024 * 1024);
    let (old, old_owner) = scope(budget.clone());
    let (actor, _events, task) = crate::spawn(server);
    let read = loop {
        match actor.admin_chat_ledger_page(old.clone(), None, 32).await {
            Ok(page) => break page,
            Err(AdminLedgerError::Busy) => {}
            Err(error) => panic!("actual Actor read: {error:?}"),
        }
    };
    assert_eq!(read.rows.len(), 1);
    assert!(budget.charged_bytes() > 0);
    old_owner.send_replace(true);
    // No new read is needed to clear a quiet locked Actor's derived cache.
    for _ in 0..4096 {
        if budget.charged_bytes() == 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(budget.charged_bytes(), 0);
    assert!(matches!(
        actor
            .admin_chat_ledger_detail(old, "bad".into(), "bad".into(), 0)
            .await,
        Err(AdminLedgerError::SessionUnavailable)
    ));
    let (fresh, _fresh_owner) = scope(budget.clone());
    let fresh_page = loop {
        match actor.admin_chat_ledger_page(fresh.clone(), None, 32).await {
            Ok(page) => break page,
            Err(AdminLedgerError::Busy) => {}
            Err(error) => panic!("fresh Actor read: {error:?}"),
        }
    };
    assert_ne!(fresh_page.revision, read.revision);
    assert!(budget.charged_bytes() > 0);
    // Wall-clock correction cannot extend the idle lease. It uses the same injected
    // monotonic Clock as the actual Actor wake, including after backwards wall time.
    clock.set_wall_ms(0);
    clock.advance_ms(VIEW_TTL_MS);
    for _ in 0..4096 {
        if budget.charged_bytes() == 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(budget.charged_bytes(), 0);
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn actual_joined_admin_can_read_and_revocation_precedes_old_or_malformed_references() {
    let hub = Hub::new();
    let clock = ManualClock::new(1000);
    let owner_peer = PeerId::from_u64(701);
    let mut owner = Server::found(
        hub.join(owner_peer),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(701),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    owner.sync.subscribe_control().await.unwrap();
    owner.open_roles().await.unwrap();
    let invite = owner.mint_invite([17; 16], 60_000, vec![]).unwrap();
    let (joined, tick) = tokio::join!(
        Server::join(
            hub.join(PeerId::from_u64(702)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(702),
            Box::new(clock),
            "member",
            owner_peer,
            &invite
        ),
        owner.sync_once(),
    );
    tick.unwrap();
    let mut member = joined.unwrap();
    member.open_roles().await.unwrap();
    member.open_channel(5).await.unwrap();
    member.send_message(5, "retained original").await.unwrap();
    let (scope, _scope_owner) = scope(AdminReadBudget::new(8 * 1024 * 1024));
    assert_eq!(member.my_role(), crate::Role::Member);
    assert!(matches!(
        member.admin_chat_ledger_page(&scope, Some("malformed"), 0),
        Err(AdminLedgerError::NotAuthorized)
    ));
    assert!(matches!(
        member.admin_chat_ledger_detail(&scope, "bad", "bad", 0),
        Err(AdminLedgerError::NotAuthorized)
    ));
    let device = member.device_id;
    owner.set_admin(&device, true).await.unwrap();
    // The wall-clock timeout only bounds a broken test; all policy/revision time comes from
    // the injected Clock. Drive actual signed role gossip, never assign a fixture role boolean.
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while member.my_role() != crate::Role::Admin {
            member.sync_once().await.unwrap();
        }
    })
    .await
    .expect("signed admin grant did not arrive");
    let admitted = page(&mut member, &scope, None, 32);
    assert_eq!(admitted.rows.len(), 1);
    assert!(member
        .admin_chat_ledger_detail(&scope, &admitted.rows[0].row_id, &admitted.revision, 0)
        .is_ok());
    owner.set_admin(&device, false).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while member.my_role() != crate::Role::Member {
            member.sync_once().await.unwrap();
        }
    })
    .await
    .expect("signed admin revocation did not arrive");
    assert!(matches!(
        member.admin_chat_ledger_page(&scope, Some("malformed"), 0),
        Err(AdminLedgerError::NotAuthorized)
    ));
    assert!(matches!(
        member.admin_chat_ledger_detail(&scope, &admitted.rows[0].row_id, &admitted.revision, 0),
        Err(AdminLedgerError::NotAuthorized)
    ));
    assert!(member.admin_chat_ledger.view.is_none());
    owner.set_admin(&device, true).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while member.my_role() != crate::Role::Admin {
            member.sync_once().await.unwrap();
        }
    })
    .await
    .expect("fresh signed admin grant did not arrive");
    assert!(matches!(
        member.admin_chat_ledger_detail(&scope, &admitted.rows[0].row_id, &admitted.revision, 0),
        Err(AdminLedgerError::Stale)
    ));
    let fresh = page(&mut member, &scope, None, 32);
    assert_ne!(fresh.revision, admitted.revision);
}

#[tokio::test]
async fn stale_reset_uses_actual_captured_revision_and_never_bypasses_session_gate() {
    let mut node = node(&ManualClock::new(1000)).await;
    node.open_channel(5).await.unwrap();
    node.send_message(5, "original").await.unwrap();
    let (scope, owner) = scope(AdminReadBudget::new(8 * 1024 * 1024));
    let first = page(&mut node, &scope, None, 32);
    assert!(matches!(
        node.admin_chat_ledger_detail(&scope, &first.rows[0].row_id, &"ab".repeat(16), 0),
        Err(AdminLedgerError::Stale)
    ));
    let revision = node.admin_chat_ledger_revision(&scope).unwrap();
    assert_ne!(revision, first.revision);
    // Capture contains no copied original or signature result yet; the next actual page
    // completes THIS view, rather than returning a separately guessed reset token.
    assert!(!node.admin_chat_ledger.view.as_ref().unwrap().ready);
    let reset = page(&mut node, &scope, None, 32);
    assert_eq!(reset.revision, revision);
    owner.send_replace(true);
    assert_eq!(
        node.admin_chat_ledger_revision(&scope),
        Err(AdminLedgerError::SessionUnavailable)
    );
    assert!(node.admin_chat_ledger.view.is_none());
}
