//! Inventory of another member's selected archive. Local maintenance builds
//! the catalogue. Remote requests only borrow eligible IDs for a bounded page.

use super::dependencies::{ReplyPayment, ScopeAuthority, REPLY_FEE};
use super::history_discovery::HistoryContactScope;
use super::removed_review::ReviewRelayState;
use super::*;
use crate::store::persistence::v2_shared_history::{ArchiveRelayEntry, MAX_SHARED_HISTORY_RECORDS};
use catcoms_net::NativeChatMemberDependencyIncoming;
use catcoms_replication::chat::owner_header::OwnerHeaderId;
use catcoms_rt::ConnectionLease;
use catcoms_sync::owner_v2::{
    V2ArchiveHistoryCursor, V2ArchiveHistoryPage, V2_ARCHIVE_HISTORY_PAGE_SIZE,
};
use zeroize::Zeroizing;

const CATALOGUE_FEE: usize = MAX_SHARED_HISTORY_RECORDS * size_of::<ArchiveRelayEntry>() + 8192;
const CATALOGUE_REFRESH_MS: u64 = 20_000;
pub(super) const CURSOR_IDLE_MS: u64 = 300_000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct RelayScope {
    pub(super) head: OwnerHeaderId,
    pub(super) epoch: u64,
    pub(super) channel: u128,
}

struct ArchiveCatalogue {
    entries: Vec<ArchiveRelayEntry>,
    scope: RelayScope,
    created_ms: u64,
    _fee: CatalogReadReservation,
}

struct RelaySweep {
    peer: PeerId,
    admission: OwnerBodyId,
    connection: ConnectionLease,
    approval: RequestCancellation,
    catalogue: Arc<ArchiveCatalogue>,
    seed: Zeroizing<[u8; 32]>,
    due: u64,
    finished: bool,
}

impl RelaySweep {
    fn token(&self, index: usize) -> V2ArchiveHistoryCursor {
        V2ArchiveHistoryCursor::from_bytes(
            *blake3::keyed_hash(&self.seed, &(index as u64).to_be_bytes()).as_bytes(),
        )
    }

    fn start(&self, cursor: Option<V2ArchiveHistoryCursor>) -> Option<usize> {
        match cursor {
            None => Some(0),
            Some(cursor) => (V2_ARCHIVE_HISTORY_PAGE_SIZE..self.catalogue.entries.len())
                .step_by(V2_ARCHIVE_HISTORY_PAGE_SIZE)
                .find(|&index| self.token(index) == cursor),
        }
    }
}

#[derive(Default)]
pub(in crate::store::critical_namespaces::native_registry::operational::sources::join) struct ArchiveRelayState
{
    catalogue: Option<Arc<ArchiveCatalogue>>,
    sweeps: [Option<RelaySweep>; MAX_APPROVED_MEMBER_PEERS],
    pub(super) review: ReviewRelayState,
}

struct RelayRequest {
    peer: PeerId,
    admission: OwnerBodyId,
    connection: ConnectionLease,
    approval: RequestCancellation,
    scope: RelayScope,
    cursor: Option<V2ArchiveHistoryCursor>,
    now: u64,
}

struct PreparedRelayPage {
    sweep: usize,
    catalogue: Arc<ArchiveCatalogue>,
    entries: std::ops::Range<usize>,
    next: Option<V2ArchiveHistoryCursor>,
}

impl ArchiveRelayState {
    fn expire(&mut self, current: RelayScope, now: u64) {
        for slot in &mut self.sweeps {
            if slot.as_ref().is_some_and(|sweep| {
                sweep.due <= now
                    || sweep.catalogue.scope != current
                    || !sweep.connection.is_live()
                    || sweep.approval.is_cancelled()
            }) {
                *slot = None;
            }
        }
    }

    fn prepare_page(
        &mut self,
        request: RelayRequest,
        rng: &mut impl CryptoRngCore,
    ) -> Result<PreparedRelayPage, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if !request.connection.is_live() || request.approval.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        self.expire(request.scope, request.now);
        let index = self
            .sweeps
            .iter()
            .position(|slot| {
                slot.as_ref()
                    .is_some_and(|sweep| sweep.peer == request.peer)
            })
            .or_else(|| self.sweeps.iter().position(Option::is_none))
            .ok_or(Refusal::Busy)?;
        let slot = &mut self.sweeps[index];
        let catalogue = self
            .catalogue
            .as_ref()
            .filter(|value| value.scope == request.scope);
        let same_origin = slot.as_ref().is_some_and(|sweep| {
            sweep.admission == request.admission && sweep.connection.is_same(&request.connection)
        });
        let refresh = same_origin
            && slot.as_ref().is_some_and(|sweep| {
                sweep.finished && catalogue.is_some_and(|new| !Arc::ptr_eq(new, &sweep.catalogue))
            });
        if request.cursor.is_none() && (!same_origin || refresh) {
            let catalogue = catalogue.ok_or(Refusal::Unavailable)?.clone();
            let mut seed = Zeroizing::new([0; 32]);
            rng.fill_bytes(seed.as_mut());
            *slot = Some(RelaySweep {
                peer: request.peer,
                admission: request.admission,
                connection: request.connection,
                approval: request.approval,
                catalogue,
                seed,
                due: request.now.saturating_add(CURSOR_IDLE_MS),
                finished: false,
            });
        } else if !same_origin {
            // A new connection or approval cannot adopt a prior sweep token.
            return Err(Refusal::Stale);
        }
        let sweep = slot.as_ref().ok_or(Refusal::Stale)?;
        let start = sweep.start(request.cursor).ok_or(Refusal::Stale)?;
        let end = (start + V2_ARCHIVE_HISTORY_PAGE_SIZE).min(sweep.catalogue.entries.len());
        let next = (end < sweep.catalogue.entries.len()).then(|| sweep.token(end));
        Ok(PreparedRelayPage {
            sweep: index,
            catalogue: sweep.catalogue.clone(),
            entries: start..end,
            next,
        })
    }

    fn finish_page(
        &mut self,
        page: &PreparedRelayPage,
        now: u64,
    ) -> Result<(), V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let sweep = self.sweeps[page.sweep].as_mut().ok_or(Refusal::Stale)?;
        if !Arc::ptr_eq(&sweep.catalogue, &page.catalogue)
            || !sweep.connection.is_live()
            || sweep.approval.is_cancelled()
        {
            return Err(Refusal::Stale);
        }
        sweep.finished |= page.next.is_none();
        sweep.due = now.saturating_add(CURSOR_IDLE_MS);
        Ok(())
    }
}

impl<R: CryptoRngCore> NativeTextJoinedSelectionPreparation<R> {
    /// Progress at most one local batch of archive index work. This can
    /// settle an existing local WAL, so it is never called by a remote request.
    pub fn maintain_archive_relay_catalogue(
        self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        caller: &RequestCancellation,
    ) -> Result<bool, V2MemberSaveRefusal> {
        self.with_member_receiver(selection, caller, |source| {
            source.maintain_relay_catalogue(selection, channel, caller)
        })
    }

    pub(super) fn serve_archive_history_request(
        self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        request: NativeChatMemberDependencyIncoming,
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let waiting = request.cancellation_authority();
        let (reply, approval, observed, auth, fee) =
            self.with_member_receiver(selection, &caller, |source| {
                let (peer, wire) = request.current_data().ok_or(Refusal::Cancelled)?;
                let connection = request.observe_connection().ok_or(Refusal::Cancelled)?;
                let (admission, approval) = source.require_history_peer(selection, peer)?;
                let fee = source
                    .metadata
                    .reserve_join_node(REPLY_FEE)
                    .map_err(|_| Refusal::Busy)?;
                let access = source
                    .node
                    .member_history_access(source.prefix, channel)
                    .map_err(|_| Refusal::Stale)?;
                let query = access
                    .open_archive_page_request(peer, wire)
                    .map_err(|_| Refusal::Failed)?;
                let scope = source.relay_scope(channel)?;
                let now = source.node.member_history_time_ms();
                let prepared = selection
                    .original
                    .archive_relay
                    .try_lock()
                    .map_err(|_| Refusal::Busy)?
                    .prepare_page(
                        RelayRequest {
                            peer,
                            admission,
                            connection,
                            approval: approval.clone(),
                            scope,
                            cursor: query.cursor(),
                            now,
                        },
                        rng,
                    )?;
                let entries = &prepared.catalogue.entries[prepared.entries.clone()];
                let mut actions = Vec::with_capacity(entries.len());
                let verifier = source
                    .prefix
                    .selected_member_verifier()
                    .map_err(|_| Refusal::Stale)?;
                let still_authorized =
                    || waiting.authorized() && !caller.is_cancelled() && !approval.is_cancelled();
                if !still_authorized() {
                    return Err(Refusal::Cancelled);
                }
                source
                    .store
                    .visit_selected_shared_history_relay_page(
                        source.history_scope(channel),
                        entries,
                        || still_authorized().then_some(()).ok_or_else(refused),
                        |entry, action| {
                            if !still_authorized() {
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
                if !still_authorized() {
                    return Err(Refusal::Cancelled);
                }
                let page = V2ArchiveHistoryPage::new(actions, prepared.next)
                    .map_err(|_| Refusal::Failed)?;
                let reply = access
                    .seal_archive_page(peer, &query, &page, rng)
                    .map_err(|_| Refusal::Failed)?;
                let (observed, auth) = source.effect_data()?;
                if !still_authorized() {
                    return Err(Refusal::Cancelled);
                }
                // A failed record read or failed reply preparation cannot
                // keep an obsolete sweep alive for indefinite retries.
                selection
                    .original
                    .archive_relay
                    .try_lock()
                    .map_err(|_| Refusal::Busy)?
                    .finish_page(&prepared, source.node.member_history_time_ms())?;
                Ok((reply, approval, observed, auth, fee))
            })?;
        if !waiting.authorized() {
            return Err(Refusal::Cancelled);
        }
        let caller = HistoryContactScope::constrain(approval, caller);
        let (signal, invocation) = self.dependency_signal(selection, caller, auth, observed);
        request
            .respond(
                bytes::Bytes::from(reply),
                Arc::new(ScopeAuthority(signal)),
                Arc::new(ReplyPayment {
                    _invocation: invocation,
                    _fee: fee,
                }),
            )
            .map_err(|_| Refusal::Cancelled)
    }
}

impl<R: CryptoRngCore> ReceiverSource<'_, R> {
    pub(super) fn relay_scope(&self, channel: u128) -> Result<RelayScope, V2MemberSaveRefusal> {
        let (head, _, epoch) = self
            .node
            .selected_join_data()
            .map_err(|_| V2MemberSaveRefusal::Stale)?;
        Ok(RelayScope {
            head,
            epoch,
            channel,
        })
    }

    fn maintain_relay_catalogue(
        &self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        caller: &RequestCancellation,
    ) -> Result<bool, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if !self
            .store
            .has_mounted_shared_history()
            .map_err(|_| Refusal::Busy)?
        {
            return Err(Refusal::Unavailable);
        }
        let scope = self.relay_scope(channel)?;
        let now = self.node.member_history_time_ms();
        {
            let mut relay = selection
                .original
                .archive_relay
                .try_lock()
                .map_err(|_| Refusal::Busy)?;
            relay.expire(scope, now);
            relay.review.expire(scope, now);
            if relay.catalogue.as_ref().is_some_and(|value| {
                value.scope == scope && now.saturating_sub(value.created_ms) < CATALOGUE_REFRESH_MS
            }) {
                return Ok(true);
            }
        }
        let archive = self.history_scope(channel);
        if !self
            .store
            .advance_shared_history_relay_index(archive)
            .map_err(|_| Refusal::Failed)?
        {
            return Ok(false);
        }
        let verifier = self
            .prefix
            .selected_member_verifier()
            .map_err(|_| Refusal::Stale)?;
        let fee = self
            .metadata
            .reserve_join_node(CATALOGUE_FEE)
            .map_err(|_| Refusal::Busy)?;
        let mut review = self
            .metadata
            .reserve_join_node(CATALOGUE_FEE)
            .ok()
            .map(|fee| (Vec::with_capacity(MAX_SHARED_HISTORY_RECORDS), fee));
        let mut retired_admissions = Vec::new();
        if review.is_some() {
            retired_admissions.extend(
                self.prefix
                    .retired_history_data()
                    .iter()
                    .filter(|author| verifier.removed_review_coordinates(author).is_ok())
                    .map(|author| author.admission_id_data()),
            );
            retired_admissions.sort_unstable();
        }
        let mut entries = Vec::with_capacity(MAX_SHARED_HISTORY_RECORDS);
        if !self
            .store
            .inspect_shared_history_relay_index(archive, |entry| {
                if verifier.observe_live_member(entry.admission).is_ok() {
                    entries.push(entry);
                } else if let Some((retired, _)) = &mut review {
                    if retired_admissions.binary_search(&entry.admission).is_ok() {
                        retired.push(entry);
                    }
                }
            })
            .map_err(|_| Refusal::Failed)?
        {
            return Ok(false);
        }
        if caller.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        let mut relay = selection
            .original
            .archive_relay
            .try_lock()
            .map_err(|_| Refusal::Busy)?;
        relay.catalogue = Some(Arc::new(ArchiveCatalogue {
            entries,
            scope,
            created_ms: now,
            _fee: fee,
        }));
        if let Some((entries, fee)) = review {
            relay.review.replace_catalogue(entries, scope, fee);
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::critical_namespaces::read_pool::CriticalCatalogReadPool;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn archive_relay_cursor_retries_the_frozen_page_and_refuses_a_new_connection() {
        let scope = RelayScope {
            head: OwnerHeaderId::from_bytes([1; 32]),
            epoch: 2,
            channel: 3,
        };
        let pool = CriticalCatalogReadPool::new(CATALOGUE_FEE * 3).unwrap();
        let catalogue = |count: u8| {
            Arc::new(ArchiveCatalogue {
                entries: (1..=count)
                    .map(|index| ArchiveRelayEntry {
                        action: ActionId::from_bytes([index; 32]),
                        admission: catcoms_replication::chat::owner_facts::OwnerFactId::from_bytes(
                            [4; 32],
                        ),
                    })
                    .collect(),
                scope,
                created_ms: 0,
                _fee: pool.try_reserve(CATALOGUE_FEE).unwrap(),
            })
        };
        let mut state = ArchiveRelayState {
            catalogue: Some(catalogue(9)),
            ..ArchiveRelayState::default()
        };
        let (approval, live) = watch::channel(false);
        let connection = ConnectionLease::default();
        let request = |cursor, connection: &ConnectionLease| RelayRequest {
            peer: PeerId::from_u64(5),
            admission: OwnerBodyId::from_bytes([6; 32]),
            connection: connection.clone(),
            approval: RequestCancellation::new(live.clone(), None),
            scope,
            cursor,
            now: 7,
        };
        let mut rng = ChaCha20Rng::seed_from_u64(8);
        let first = state
            .prepare_page(request(None, &connection), &mut rng)
            .unwrap();
        assert_eq!(first.entries, 0..8);
        let cursor = first.next.unwrap();
        state.finish_page(&first, 7).unwrap();
        state.catalogue = Some(catalogue(12));
        let mut failed_attempt = request(Some(cursor), &connection);
        failed_attempt.now = 11;
        let _unprepared_reply = state.prepare_page(failed_attempt, &mut rng).unwrap();
        assert_eq!(
            state.sweeps[first.sweep].as_ref().unwrap().due,
            7 + CURSOR_IDLE_MS
        );
        for _ in 0..2 {
            let last = state
                .prepare_page(request(Some(cursor), &connection), &mut rng)
                .unwrap();
            assert!(Arc::ptr_eq(&last.catalogue, &first.catalogue));
            assert_eq!(last.entries, 8..9);
            assert!(last.next.is_none());
            state.finish_page(&last, 7).unwrap();
        }
        let updated = state
            .prepare_page(request(None, &connection), &mut rng)
            .unwrap();
        assert_eq!(updated.catalogue.entries.len(), 12);
        let new_cursor = updated.next;
        assert_ne!(new_cursor, Some(cursor));
        assert!(state
            .prepare_page(request(Some(cursor), &connection), &mut rng)
            .is_err());
        let replacement = ConnectionLease::default();
        assert!(state
            .prepare_page(request(new_cursor, &replacement), &mut rng)
            .is_err());
        approval.send_replace(true);
        assert!(state
            .prepare_page(request(new_cursor, &connection), &mut rng)
            .is_err());
    }
}
