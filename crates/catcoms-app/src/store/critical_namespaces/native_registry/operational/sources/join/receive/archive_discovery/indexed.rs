//! Ordinary indexed relay retains two bounded offers per approved peer.
//! The selected Head is held by the actual mount; each paid page keeps only
//! its exact-root negative and eight locator rows, never an archive catalogue.
use super::*;
use crate::store::persistence::v2_shared_history::{
    SelectedSharedHistoryRelayPage, SharedHistoryRelayPageError,
};

// Page/cursor work and retained rows are independently charged by the archive
// account. This metadata debit pays Arc headers, the bounded locator vector,
// filtered IDs, retry copy and the temporary/retained wrapper overlap.
const OFFER_FEE: usize = 2 * size_of::<IndexedOffer>()
    + 4 * V2_ARCHIVE_HISTORY_PAGE_SIZE * size_of::<ActionId>()
    + 2 * V2_ARCHIVE_HISTORY_PAGE_SIZE * size_of::<ArchiveRelayEntry>()
    + 1024;

struct Offered<T> {
    request: Option<V2ArchiveHistoryCursor>,
    next: Option<V2ArchiveHistoryCursor>,
    value: T,
}

/// Two exact keys, constant lookup and no token search proportional to archive
/// size. A delayed request older than these windows must restart explicitly.
struct OfferWindow<T> {
    current: Option<Offered<T>>,
    previous: Option<Offered<T>>,
}
impl<T> Default for OfferWindow<T> {
    fn default() -> Self {
        Self {
            current: None,
            previous: None,
        }
    }
}
impl<T> OfferWindow<T> {
    fn get(&self, request: Option<V2ArchiveHistoryCursor>) -> Option<&Offered<T>> {
        self.current
            .iter()
            .chain(self.previous.iter())
            .find(|offer| offer.request == request)
    }
    fn push(&mut self, offer: Offered<T>) {
        self.previous = self.current.take();
        self.current = Some(offer);
    }
}

struct IndexedOffer {
    page: SelectedSharedHistoryRelayPage,
    actions: Vec<ActionId>,
    _fee: CatalogReadReservation,
}

pub(super) struct IndexedSweep {
    peer: PeerId,
    admission: OwnerBodyId,
    connection: ConnectionLease,
    approval: RequestCancellation,
    scope: RelayScope,
    seed: Zeroizing<[u8; 32]>,
    sequence: u64,
    offers: OfferWindow<Arc<IndexedOffer>>,
    due: u64,
}
impl IndexedSweep {
    pub(super) fn expired(&self, scope: RelayScope, now: u64) -> bool {
        self.due <= now
            || self.scope != scope
            || !self.connection.is_live()
            || self.approval.is_cancelled()
    }
    fn same_origin(&self, request: &RelayRequest) -> bool {
        self.peer == request.peer
            && self.admission == request.admission
            && self.connection.is_same(&request.connection)
            && self.scope == request.scope
    }
    fn next_token(&self, sequence: u64) -> V2ArchiveHistoryCursor {
        V2ArchiveHistoryCursor::from_bytes(
            *blake3::keyed_hash(&self.seed, &sequence.to_be_bytes()).as_bytes(),
        )
    }
}

pub(super) struct IndexedPlan {
    sweep: usize,
    requested: Option<V2ArchiveHistoryCursor>,
    next: Option<V2ArchiveHistoryCursor>,
    offer: Arc<IndexedOffer>,
}
impl IndexedPlan {
    pub(super) fn require_current(
        &self,
        store: &ServerStore,
        scope: SharedHistoryScope,
    ) -> Result<(), V2MemberSaveRefusal> {
        store
            .require_selected_shared_history_relay_page(scope, &self.offer.page)
            .map_err(page_error)
    }

    pub(super) fn wire_page(&self) -> Result<V2ArchiveHistoryPage, V2MemberSaveRefusal> {
        V2ArchiveHistoryPage::new(self.offer.actions.clone(), self.next)
            .map_err(|_| V2MemberSaveRefusal::Failed)
    }
}

fn page_error(error: SharedHistoryRelayPageError) -> V2MemberSaveRefusal {
    match error {
        SharedHistoryRelayPageError::Stale => V2MemberSaveRefusal::Stale,
        SharedHistoryRelayPageError::Unavailable(AppError::ChatV2ArchiveBusy) => {
            V2MemberSaveRefusal::Busy
        }
        SharedHistoryRelayPageError::Unavailable(_) => V2MemberSaveRefusal::Failed,
    }
}

impl ArchiveRelayState {
    pub(super) fn finish_indexed_page(
        &mut self,
        plan: &IndexedPlan,
        now: u64,
    ) -> Result<(), V2MemberSaveRefusal> {
        let sweep = self.indexed_sweeps[plan.sweep]
            .as_mut()
            .ok_or(V2MemberSaveRefusal::Stale)?;
        let offered = sweep
            .offers
            .get(plan.requested)
            .ok_or(V2MemberSaveRefusal::Stale)?;
        if !Arc::ptr_eq(&offered.value, &plan.offer)
            || offered.next != plan.next
            || !sweep.connection.is_live()
            || sweep.approval.is_cancelled()
        {
            return Err(V2MemberSaveRefusal::Stale);
        }
        sweep.due = now.saturating_add(CURSOR_IDLE_MS);
        Ok(())
    }
}

impl<R: CryptoRngCore> ReceiverSource<'_, R> {
    pub(super) fn prepare_indexed_relay_page(
        &self,
        selection: &NativeTextJoinedRowSelection,
        request: RelayRequest,
        rng: &mut impl CryptoRngCore,
        authorized: impl Fn() -> bool,
    ) -> Result<IndexedPlan, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if !authorized() || !request.connection.is_live() || request.approval.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        let mut relay = selection
            .original
            .archive_relay
            .try_lock()
            .map_err(|_| Refusal::Busy)?;
        relay.expire(request.scope, request.now);
        let index = relay
            .indexed_sweeps
            .iter()
            .position(|slot| {
                slot.as_ref()
                    .is_some_and(|sweep| sweep.peer == request.peer)
            })
            .or_else(|| relay.indexed_sweeps.iter().position(Option::is_none))
            .ok_or(Refusal::Busy)?;
        let slot = &mut relay.indexed_sweeps[index];
        let same = slot
            .as_ref()
            .is_some_and(|sweep| sweep.same_origin(&request));
        if !same && request.cursor.is_some() {
            return Err(Refusal::Stale);
        }
        // A Start can restart a stale root, or a window older than the two
        // retained offers. A continuation never rebases onto a newer Head.
        let cached = if same {
            slot.as_ref()
                .and_then(|sweep| sweep.offers.get(request.cursor))
        } else {
            None
        };
        let cached_exists = cached.is_some();
        let cached_current = cached.map(|offer| {
            self.store.require_selected_shared_history_relay_page(
                self.history_scope(request.scope.channel),
                &offer.value.page,
            )
        });
        let cached_stale = match cached_current {
            Some(Err(SharedHistoryRelayPageError::Stale)) => true,
            Some(Err(error)) => return Err(page_error(error)),
            _ => false,
        };
        if request.cursor.is_none() && (!same || !cached_exists || cached_stale) {
            let mut seed = Zeroizing::new([0; 32]);
            rng.fill_bytes(seed.as_mut());
            *slot = Some(IndexedSweep {
                peer: request.peer,
                admission: request.admission,
                connection: request.connection.clone(),
                approval: request.approval.clone(),
                scope: request.scope,
                seed,
                sequence: 0,
                offers: OfferWindow::default(),
                due: request.now.saturating_add(CURSOR_IDLE_MS),
            });
        } else if cached_stale {
            *slot = None;
            return Err(Refusal::Stale);
        }
        let sweep = slot.as_mut().ok_or(Refusal::Stale)?;
        if !sweep.same_origin(&request) {
            return Err(Refusal::Stale);
        }
        if let Some(cached) = sweep.offers.get(request.cursor) {
            let actions =
                self.indexed_relay_actions(request.scope.channel, &cached.value.page, &authorized)?;
            // Never silently narrow or enlarge a page after a failed body read,
            // revocation or delegation change. It remains the exact prior offer.
            if actions != cached.value.actions {
                *slot = None;
                return Err(Refusal::Stale);
            }
            return Ok(IndexedPlan {
                sweep: index,
                requested: request.cursor,
                next: cached.next,
                offer: cached.value.clone(),
            });
        }
        let previous = match request.cursor {
            None => None,
            Some(cursor) => Some(
                &sweep
                    .offers
                    .current
                    .as_ref()
                    .filter(|offer| offer.next == Some(cursor))
                    .ok_or(Refusal::Stale)?
                    .value
                    .page,
            ),
        };
        // Pay all dynamic offer metadata before the page or ID allocations.
        let fee = self
            .metadata
            .reserve_join_node(OFFER_FEE)
            .map_err(|_| Refusal::Busy)?;
        let page = self
            .store
            .selected_shared_history_relay_page(self.history_scope(request.scope.channel), previous)
            .map_err(page_error)?;
        let actions = self.indexed_relay_actions(request.scope.channel, &page, &authorized)?;
        let sequence = sweep.sequence.checked_add(1).ok_or(Refusal::Failed)?;
        let next = page.has_more_data().then(|| sweep.next_token(sequence));
        let offer = Arc::new(IndexedOffer {
            page,
            actions,
            _fee: fee,
        });
        // Cache before response encoding/return. A lost response can retry the
        // identical offered IDs without rescanning earlier pages or redrawing.
        sweep.sequence = sequence;
        sweep.offers.push(Offered {
            request: request.cursor,
            next,
            value: offer.clone(),
        });
        Ok(IndexedPlan {
            sweep: index,
            requested: request.cursor,
            next,
            offer,
        })
        // The relay mutex is released here, before any SDK sealing or native
        // response publication. No RuntimeLoan is retained across an await.
    }

    fn indexed_relay_actions(
        &self,
        channel: u128,
        page: &SelectedSharedHistoryRelayPage,
        authorized: impl Fn() -> bool,
    ) -> Result<Vec<ActionId>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let archive = self.history_scope(channel);
        self.store
            .require_selected_shared_history_relay_page(archive, page)
            .map_err(page_error)?;
        let entries = page.relay_entries_data().collect::<Vec<_>>();
        let mut actions = Vec::with_capacity(V2_ARCHIVE_HISTORY_PAGE_SIZE);
        let verifier = self
            .prefix
            .selected_member_verifier()
            .map_err(|_| Refusal::Stale)?;
        self.store
            .visit_selected_shared_history_relay_page(
                archive,
                &entries,
                || authorized().then_some(()).ok_or_else(refused),
                |entry, action| {
                    if !authorized() {
                        return Err(refused());
                    }
                    if verifier.observe_live_member(entry.admission).is_ok()
                        && verifier
                            .admission(entry.admission)
                            .and_then(|author| {
                                author.admission_data().matches_action_delegation(action)
                            })
                            .is_ok()
                    {
                        actions.push(entry.action);
                    }
                    Ok(())
                },
            )
            .map_err(|_| Refusal::Failed)?;
        if !authorized() {
            return Err(Refusal::Cancelled);
        }
        self.store
            .require_selected_shared_history_relay_page(archive, page)
            .map_err(page_error)?;
        Ok(actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_archive_relay_window_keeps_only_current_previous_and_exact_empty_progress() {
        let cursor = |n| V2ArchiveHistoryCursor::from_bytes([n; 32]);
        let mut window = OfferWindow::default();
        window.push(Offered {
            request: None,
            next: Some(cursor(1)),
            value: Vec::<ActionId>::new(),
        });
        assert!(window.get(None).unwrap().value.is_empty());
        assert_eq!(window.get(None).unwrap().next, Some(cursor(1)));
        window.push(Offered {
            request: Some(cursor(1)),
            next: Some(cursor(2)),
            value: vec![ActionId::from_bytes([3; 32])],
        });
        assert!(window.get(None).is_some());
        window.push(Offered {
            request: Some(cursor(2)),
            next: None,
            value: vec![ActionId::from_bytes([4; 32])],
        });
        assert!(window.get(None).is_none());
        assert_eq!(
            window.get(Some(cursor(1))).unwrap().value,
            vec![ActionId::from_bytes([3; 32])]
        );
        assert_eq!(window.get(Some(cursor(2))).unwrap().next, None);
        assert!(window.get(Some(cursor(5))).is_none());
    }

    #[test]
    fn indexed_archive_relay_healthy_compatibility_work_keeps_immediate_turns() {
        assert_eq!(
            compatibility_turn(Ok(false), true),
            Ok(CompatibilityTurn::More)
        );
        assert_eq!(
            compatibility_turn(Ok(false), false),
            Ok(CompatibilityTurn::More)
        );
        assert_eq!(
            compatibility_turn(Ok(true), true),
            Ok(CompatibilityTurn::Complete)
        );
        assert_eq!(
            compatibility_turn(Err(AppError::ChatV2ArchiveBusy), true),
            Ok(CompatibilityTurn::Deferred)
        );
        assert_eq!(
            compatibility_turn(Err(refused()), false),
            Err(V2MemberSaveRefusal::Failed)
        );
    }

    #[test]
    fn indexed_archive_relay_origin_expiry_and_cancellation_never_adopt_a_replacement() {
        // Pure metadata test, not an authorization fixture. The mounted Store
        // tests independently exercise the real retained selector and files.
        let scope = RelayScope {
            head: OwnerHeaderId::from_bytes([1; 32]),
            epoch: 2,
            channel: 3,
        };
        let connection = ConnectionLease::default();
        let (cancel, signal) = watch::channel(false);
        let approval = RequestCancellation::new(signal, None);
        let request = RelayRequest {
            peer: PeerId::from_u64(4),
            admission: OwnerBodyId::from_bytes([5; 32]),
            connection: connection.clone(),
            approval: approval.clone(),
            scope,
            cursor: None,
            now: 7,
        };
        let sweep = IndexedSweep {
            peer: request.peer,
            admission: request.admission,
            connection: connection.clone(),
            approval,
            scope,
            seed: Zeroizing::new([6; 32]),
            sequence: 0,
            offers: OfferWindow::default(),
            due: 10,
        };
        assert!(sweep.same_origin(&request));
        assert!(!sweep.expired(scope, 9));
        assert!(sweep.expired(scope, 10));
        for foreign in [
            RelayRequest {
                peer: PeerId::from_u64(8),
                ..request.clone_for_test()
            },
            RelayRequest {
                admission: OwnerBodyId::from_bytes([8; 32]),
                ..request.clone_for_test()
            },
            RelayRequest {
                connection: ConnectionLease::default(),
                ..request.clone_for_test()
            },
            RelayRequest {
                scope: RelayScope { epoch: 3, ..scope },
                ..request.clone_for_test()
            },
            RelayRequest {
                scope: RelayScope {
                    channel: 4,
                    ..scope
                },
                ..request.clone_for_test()
            },
            RelayRequest {
                scope: RelayScope {
                    head: OwnerHeaderId::from_bytes([8; 32]),
                    ..scope
                },
                ..request.clone_for_test()
            },
        ] {
            assert!(!sweep.same_origin(&foreign));
        }
        assert_ne!(sweep.next_token(1), sweep.next_token(2));
        cancel.send_replace(true);
        assert!(sweep.expired(scope, 9));
        connection.revoke();
        assert!(sweep.expired(scope, 9));
    }

    // Keep the production request non-Clone. Tests explicitly duplicate only
    // the already-held negative handles and immutable metadata above.
    impl RelayRequest {
        fn clone_for_test(&self) -> Self {
            Self {
                peer: self.peer,
                admission: self.admission,
                connection: self.connection.clone(),
                approval: self.approval.clone(),
                scope: self.scope,
                cursor: self.cursor,
                now: self.now,
            }
        }
    }
}
