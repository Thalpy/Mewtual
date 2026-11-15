//! The saved-history reader must never confuse an empty publication queue with
//! absent history. These are genuine signatures and real persisted outboxes.
use super::super::engine::history::{MEMBER_HISTORY_MAX_ITEMS, MEMBER_OUTBOX_HISTORY_FEE};
use super::*;

#[test]
fn member_history_keeps_settled_originals_and_a_fixed_prefix_across_appends_and_reopen() {
    let mut fx = Fixture::new(181);
    let (a1, a2) = (admission(0xa1), admission(0xa2));
    let owner = fx.open();
    let first = fx.sign("historical A1 original", 1);
    let second = fx.sign("current A2 original", 2);
    let (owner, r1) = fx.save(owner, token(1), first.clone(), a1);
    let (owner, r2) = fx.save(owner, token(2), second.clone(), a2);
    let page = owner
        .history_page(&KEY, 0, None, 1, fx.pay(MEMBER_OUTBOX_HISTORY_FEE))
        .unwrap();
    assert_eq!((page.next(), page.end_exclusive()), (1, 2));
    assert_eq!(page.items()[0].action(), &first);
    assert_eq!(page.items()[0].status(), MemberEntryStatus::Pending);
    assert_eq!(page.items()[0].receipt(), &r1);
    let end = page.end_exclusive();
    let old_head = page.selected_head();
    drop(page);

    let mut debt = fx.debt(&owner, a2);
    debt.record_submitted(1).unwrap();
    let owner = fx.checkpoint(owner, debt);
    assert_eq!(owner.pending().unwrap(), 0);
    assert!(fx.debt(&owner, a2).items().is_empty());
    let third = fx.sign("arrived after the first page", 3);
    let (owner, _) = fx.save(owner, token(3), third.clone(), a2);
    let page = owner
        .history_page(&KEY, 1, Some(end), 8, fx.pay(MEMBER_OUTBOX_HISTORY_FEE))
        .unwrap();
    assert_eq!((page.next(), page.end_exclusive()), (2, 2));
    assert_eq!(
        page.items().len(),
        1,
        "new append cannot extend the original scan"
    );
    assert_eq!(page.items()[0].receipt(), &r2);
    assert_eq!(page.items()[0].original_bytes(), second.encode());
    assert_eq!(
        page.items()[0].status(),
        MemberEntryStatus::Settled(DispositionCode::Submitted)
    );
    assert_ne!(page.selected_head(), old_head);
    drop(page);
    let terminal = owner
        .history_page(&KEY, end, Some(end), 8, fx.pay(MEMBER_OUTBOX_HISTORY_FEE))
        .unwrap();
    assert!(terminal.items().is_empty());
    drop(terminal);
    drop(owner);

    let owner = fx.open();
    let before = namespace_files(&owner);
    let page = owner
        .history_page(&KEY, 0, None, 8, fx.pay(MEMBER_OUTBOX_HISTORY_FEE))
        .unwrap();
    assert_eq!((page.next(), page.end_exclusive()), (3, 3));
    assert_eq!(page.context(), owner.context_id());
    assert_eq!(
        page.items().iter().map(|i| i.action()).collect::<Vec<_>>(),
        [&first, &second, &third]
    );
    assert_eq!(
        page.items()[0].status(),
        MemberEntryStatus::Settled(DispositionCode::Held)
    );
    assert_eq!(
        page.items()[1].status(),
        MemberEntryStatus::Settled(DispositionCode::Submitted)
    );
    assert_eq!(page.items()[2].status(), MemberEntryStatus::Pending);
    assert_eq!(
        namespace_files(&owner),
        before,
        "history read performs no recovery/write"
    );
}

#[test]
fn member_history_range_and_terminal_empty_boundaries_are_strict() {
    let mut fx = Fixture::new(182);
    let owner = fx.open();
    let empty = owner
        .history_page(&KEY, 0, None, 8, fx.pay(MEMBER_OUTBOX_HISTORY_FEE))
        .unwrap();
    assert!(empty.items().is_empty());
    assert_eq!((empty.next(), empty.end_exclusive()), (0, 0));
    drop(empty);
    let before = namespace_files(&owner);
    for (start, end, count) in [
        (1, None, 1),
        (0, Some(1), 1),
        (0, None, 0),
        (0, None, MEMBER_HISTORY_MAX_ITEMS + 1),
        (u64::MAX, Some(u64::MAX), 8),
    ] {
        assert!(owner
            .history_page(&KEY, start, end, count, fx.pay(MEMBER_OUTBOX_HISTORY_FEE))
            .is_err());
    }
    assert_eq!(namespace_files(&owner), before);
    let signed = fx.sign("one saved original", 1);
    let (owner, _) = fx.save(owner, token(1), signed, admission(0xa1));
    let empty = owner
        .history_page(&KEY, 1, Some(1), 1, fx.pay(MEMBER_OUTBOX_HISTORY_FEE))
        .unwrap();
    assert!(empty.items().is_empty());
    assert_eq!(empty.next(), 1);
    assert!(owner
        .history_page(&KEY, 2, Some(1), 1, fx.pay(MEMBER_OUTBOX_HISTORY_FEE))
        .is_err());
}

#[test]
fn member_history_rejects_foreign_keys_accounts_and_short_payments_without_mutation() {
    let mut fx = Fixture::new(183);
    let owner = fx.open();
    let signed = fx.sign("private retained original", 1);
    let (owner, _) = fx.save(owner, token(1), signed, admission(0xa1));
    let before = namespace_files(&owner);
    assert!(owner
        .history_page(&[0xff; 32], 0, None, 1, fx.pay(MEMBER_OUTBOX_HISTORY_FEE))
        .is_err());
    let foreign = CriticalCatalogReadPool::new(MEMBER_OUTBOX_HISTORY_FEE).unwrap();
    assert!(owner
        .history_page(
            &KEY,
            0,
            None,
            1,
            foreign.try_reserve(MEMBER_OUTBOX_HISTORY_FEE).unwrap()
        )
        .is_err());
    assert_eq!(foreign.charged_for_test(), 0);
    assert!(owner
        .history_page(&KEY, 0, None, 1, fx.pay(MEMBER_OUTBOX_HISTORY_FEE - 1))
        .is_err());
    assert_eq!(namespace_files(&owner), before);
}

#[test]
fn member_history_missing_or_corrupt_storage_never_returns_a_partial_or_empty_success() {
    let mut fx = Fixture::new(184);
    let owner = fx.open();
    let (owner, _) = fx.save(
        owner,
        token(1),
        fx.sign("valid first original", 1),
        admission(0xa1),
    );
    let (owner, _) = fx.save(
        owner,
        token(2),
        fx.sign("unreadable later original", 2),
        admission(0xa1),
    );
    let selected = owner.clean().unwrap();
    let (entry, data) = owner.debt_entry(&KEY, selected, 1).unwrap();
    let node = owner.reachable_nodes_for_test(&KEY)[0];
    for path in [
        owner.object_path_for_test(ObjectRole::Chunk, data.chunks[0]),
        owner.object_path_for_test(ObjectRole::Entry, entry),
        owner.object_path_for_test(ObjectRole::Node, node),
    ] {
        let original = fs::read(&path).unwrap();
        for missing in [false, true] {
            if missing {
                fs::remove_file(&path).unwrap();
            } else {
                fs::write(&path, b"torn").unwrap();
            }
            let before = namespace_files(&owner);
            let used = fx.pool.charged_for_test();
            assert!(owner
                .history_page(&KEY, 0, None, 8, fx.pay(MEMBER_OUTBOX_HISTORY_FEE))
                .is_err());
            assert_eq!(
                namespace_files(&owner),
                before,
                "failed reader must not repair/erase evidence"
            );
            assert_eq!(
                fx.pool.charged_for_test(),
                used,
                "partial-page scratch releases its payment"
            );
            fs::write(&path, &original).unwrap();
        }
    }
    assert_eq!(
        owner
            .history_page(&KEY, 0, None, 8, fx.pay(MEMBER_OUTBOX_HISTORY_FEE))
            .unwrap()
            .items()
            .len(),
        2
    );
}

#[test]
fn member_history_payment_and_original_bytes_outlive_the_exclusive_owner() {
    let mut fx = Fixture::new(185);
    let owner = fx.open();
    let signed = fx.sign("reader survives owner replacement", 1);
    let expected = signed.encode();
    let (owner, _) = fx.save(owner, token(1), signed, admission(0xa1));
    let used = fx.pool.charged_for_test();
    let page = owner
        .history_page(&KEY, 0, None, 1, fx.pay(MEMBER_OUTBOX_HISTORY_FEE))
        .unwrap();
    assert_eq!(fx.pool.charged_for_test(), used + MEMBER_OUTBOX_HISTORY_FEE);
    drop(owner);
    let retained = fx.pool.charged_for_test();
    assert!(retained >= MEMBER_OUTBOX_HISTORY_FEE);
    assert_eq!(page.items()[0].original_bytes(), expected);
    let replacement = fx.open();
    assert_eq!(page.context(), replacement.context_id());
    drop(replacement);
    assert_eq!(fx.pool.charged_for_test(), retained);
    drop(page);
    assert_eq!(
        fx.pool.charged_for_test(),
        retained - MEMBER_OUTBOX_HISTORY_FEE
    );
}

#[test]
fn member_history_has_an_independent_eight_item_cap_and_preserves_context_isolation() {
    let mut fx = Fixture::new(186);
    let mut owner = fx.open();
    for n in 1..=10 {
        (owner, _) = fx.save(
            owner,
            token(n),
            fx.sign("bounded saved item", n),
            admission(0xa1),
        );
    }
    let first = owner
        .history_page(&KEY, 0, None, 8, fx.pay(MEMBER_OUTBOX_HISTORY_FEE))
        .unwrap();
    assert_eq!(
        (first.items().len(), first.next(), first.end_exclusive()),
        (8, 8, 10)
    );
    let second = owner
        .history_page(
            &KEY,
            first.next(),
            Some(first.end_exclusive()),
            8,
            fx.pay(MEMBER_OUTBOX_HISTORY_FEE),
        )
        .unwrap();
    assert_eq!((second.items().len(), second.next()), (2, 10));
    drop(first);
    drop(second);
    let other_context = context_for(&fx.device, 0x22);
    let other = fx
        .open_full(
            fx.registry.clone(),
            other_context,
            MEMBER_OUTBOX_DEFAULT_QUOTA,
            None,
            true,
        )
        .unwrap();
    let page = other
        .history_page(&KEY, 0, None, 8, fx.pay(MEMBER_OUTBOX_HISTORY_FEE))
        .unwrap();
    assert_ne!(page.context(), owner.context_id());
    assert!(page.items().is_empty());
    assert!(other
        .history_page(&KEY, 0, Some(10), 8, fx.pay(MEMBER_OUTBOX_HISTORY_FEE))
        .is_err());
}
