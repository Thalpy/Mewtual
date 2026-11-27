//! Serve exact retired Create originals only to the pinned founder on its
//! current authenticated connection. Ordinary archive requests never use
//! the separate candidate inventory or its offer tokens.
use super::archive_discovery::{RelayScope, CURSOR_IDLE_MS};
use super::dependencies::{ReplyPayment, ScopeAuthority, REPLY_FEE};
use super::*;
use crate::store::persistence::v2_shared_history::{ArchiveRelayEntry, MAX_SHARED_HISTORY_RECORDS};
use catcoms_net::NativeChatMemberDependencyIncoming;
use catcoms_replication::chat::owner_facts::OwnerFactId;
use catcoms_replication::chat::owner_profile::AdmissionOrigin;
use catcoms_rt::ConnectionLease;
use catcoms_sync::owner_v2::{
    V2ArchiveHistoryCursor, V2ArchiveHistoryPage, V2RemovedReviewPage, V2RemovedReviewQuery,
    V2RemovedReviewTarget, V2_ARCHIVE_HISTORY_PAGE_SIZE,
};
use zeroize::Zeroizing;

type Refusal = V2MemberSaveRefusal;
const PAGE_SIZE: usize = V2_ARCHIVE_HISTORY_PAGE_SIZE;
const MAX_PAGES: usize = MAX_SHARED_HISTORY_RECORDS.div_ceil(PAGE_SIZE);

struct ReviewCatalogue {
    entries: Vec<ArchiveRelayEntry>,
    scope: RelayScope,
    _fee: CatalogReadReservation,
}

struct Sweep {
    target: V2RemovedReviewTarget,
    founder: OwnerFactId,
    connection: ConnectionLease,
    catalogue: Arc<ReviewCatalogue>,
    range: std::ops::Range<usize>,
    seed: Zeroizing<[u8; 32]>,
    offered: [bool; MAX_SHARED_HISTORY_RECORDS],
    issued: [bool; MAX_PAGES],
    due: u64,
    finished: bool,
}

impl Sweep {
    fn token(&self, offer: bool, start: usize) -> V2ArchiveHistoryCursor {
        let mut label = [0; 9];
        label[0] = u8::from(offer);
        label[1..].copy_from_slice(&(start as u64).to_be_bytes());
        V2ArchiveHistoryCursor::from_bytes(*blake3::keyed_hash(&self.seed, &label).as_bytes())
    }

    fn start(&self, offer: bool, token: V2ArchiveHistoryCursor) -> Option<usize> {
        let first = self.range.start + if offer { 0 } else { PAGE_SIZE };
        (first..self.range.end)
            .step_by(PAGE_SIZE)
            .find(|&start| self.token(offer, start) == token)
    }

    fn page(&self, start: usize) -> PagePlan {
        let end = (start + PAGE_SIZE).min(self.range.end);
        let issued = self.issued[(start - self.range.start) / PAGE_SIZE];
        PagePlan {
            catalogue: self.catalogue.clone(),
            range: start..end,
            offer: self.token(true, start),
            next: (end < self.range.end).then(|| self.token(false, end)),
            previous: issued.then(|| {
                std::array::from_fn(|offset| start + offset < end && self.offered[start + offset])
            }),
        }
    }

    fn exact(&self, offer: V2ArchiveHistoryCursor, action: ActionId) -> Option<ArchiveRelayEntry> {
        let start = self.start(true, offer)?;
        self.page(start).range.find_map(|index| {
            let entry = self.catalogue.entries[index];
            (self.offered[index]
                && entry.admission == self.target.admission
                && entry.action == action)
                .then_some(entry)
        })
    }
}

#[derive(Default)]
pub(super) struct ReviewRelayState {
    catalogue: Option<Arc<ReviewCatalogue>>,
    sweep: Option<Sweep>,
}

struct ReviewRequest {
    scope: RelayScope,
    target: V2RemovedReviewTarget,
    founder: OwnerFactId,
    query: V2RemovedReviewQuery,
    connection: ConnectionLease,
    now: u64,
}

struct PagePlan {
    catalogue: Arc<ReviewCatalogue>,
    range: std::ops::Range<usize>,
    offer: V2ArchiveHistoryCursor,
    next: Option<V2ArchiveHistoryCursor>,
    previous: Option<[bool; PAGE_SIZE]>,
}

enum Plan {
    Page(PagePlan),
    Original(ArchiveRelayEntry),
}

impl ReviewRelayState {
    pub(super) fn replace_catalogue(
        &mut self,
        mut entries: Vec<ArchiveRelayEntry>,
        scope: RelayScope,
        fee: CatalogReadReservation,
    ) {
        entries.sort_unstable_by_key(|entry| (entry.admission, entry.action));
        self.catalogue = Some(Arc::new(ReviewCatalogue {
            entries,
            scope,
            _fee: fee,
        }));
    }

    pub(super) fn expire(&mut self, scope: RelayScope, now: u64) {
        if self
            .catalogue
            .as_ref()
            .is_some_and(|list| list.scope != scope)
        {
            self.catalogue = None;
        }
        if self.sweep.as_ref().is_some_and(|sweep| {
            sweep.catalogue.scope != scope || sweep.due <= now || !sweep.connection.is_live()
        }) {
            self.sweep = None;
        }
    }

    fn prepare(
        &mut self,
        request: &ReviewRequest,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Plan, Refusal> {
        self.expire(request.scope, request.now);
        if !request.connection.is_live() {
            return Err(Refusal::Cancelled);
        }
        let same = self.sweep.as_ref().is_some_and(|sweep| {
            sweep.target == request.target
                && sweep.founder == request.founder
                && sweep.connection.is_same(&request.connection)
        });
        let latest = self
            .catalogue
            .as_ref()
            .filter(|list| list.scope == request.scope);
        let refreshed = self.sweep.as_ref().is_some_and(|sweep| {
            sweep.finished && latest.is_some_and(|next| !Arc::ptr_eq(next, &sweep.catalogue))
        });
        if request.query == V2RemovedReviewQuery::Page(None) && (!same || refreshed) {
            let catalogue = latest.ok_or(Refusal::Unavailable)?.clone();
            // Each page scans only the requested Admission. Its cursor
            // count cannot reveal other retired authors' archive sizes.
            let start = catalogue
                .entries
                .partition_point(|entry| entry.admission < request.target.admission);
            let end = catalogue
                .entries
                .partition_point(|entry| entry.admission <= request.target.admission);
            let mut seed = Zeroizing::new([0; 32]);
            rng.fill_bytes(seed.as_mut());
            self.sweep = Some(Sweep {
                target: request.target,
                founder: request.founder,
                connection: request.connection.clone(),
                catalogue,
                range: start..end,
                seed,
                offered: [false; MAX_SHARED_HISTORY_RECORDS],
                issued: [false; MAX_PAGES],
                due: request.now.saturating_add(CURSOR_IDLE_MS),
                finished: false,
            });
        } else if !same {
            return Err(Refusal::Stale);
        }
        let sweep = self.sweep.as_ref().ok_or(Refusal::Stale)?;
        match request.query {
            V2RemovedReviewQuery::Page(None) => Ok(Plan::Page(sweep.page(sweep.range.start))),
            V2RemovedReviewQuery::Page(Some(cursor)) => Ok(Plan::Page(
                sweep.page(sweep.start(false, cursor).ok_or(Refusal::Stale)?),
            )),
            V2RemovedReviewQuery::Original { offer, action } => Ok(Plan::Original(
                sweep.exact(offer, action).ok_or(Refusal::Stale)?,
            )),
        }
    }

    fn finish_page(
        &mut self,
        plan: &PagePlan,
        actions: &[ActionId],
        now: u64,
    ) -> Result<(), Refusal> {
        let sweep = self.sweep.as_mut().ok_or(Refusal::Stale)?;
        if !Arc::ptr_eq(&sweep.catalogue, &plan.catalogue)
            || sweep.token(true, plan.range.start) != plan.offer
            || !sweep.connection.is_live()
        {
            return Err(Refusal::Stale);
        }
        let page = (plan.range.start - sweep.range.start) / PAGE_SIZE;
        let includes = |index: usize| {
            actions
                .binary_search(&sweep.catalogue.entries[index].action)
                .is_ok()
        };
        if sweep.issued[page] {
            // A temporarily unreadable original must not block the
            // continuation. Leave the first issued authority unchanged.
            if plan
                .range
                .clone()
                .any(|index| includes(index) && !sweep.offered[index])
            {
                return Err(Refusal::Stale);
            }
        } else {
            // The founder may already know this token even if the retry
            // is lost. Bind the first issued list for the whole sweep.
            for index in plan.range.clone() {
                sweep.offered[index] = includes(index);
            }
            sweep.issued[page] = true;
        }
        sweep.finished |= plan.next.is_none();
        sweep.due = now.saturating_add(CURSOR_IDLE_MS);
        Ok(())
    }
}

impl<R: CryptoRngCore> NativeTextJoinedSelectionPreparation<R> {
    pub(super) fn serve_removed_review_request(
        self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        incoming: NativeChatMemberDependencyIncoming,
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), Refusal> {
        let waiting = incoming.cancellation_authority();
        let (reply, observed, authentication, fee) =
            self.with_member_receiver(selection, &caller, |source| {
                let (peer, wire) = incoming.current_data().ok_or(Refusal::Cancelled)?;
                if source.founder != Some(peer) || !waiting.authorized() {
                    return Err(Refusal::Stale);
                }
                let connection = incoming.observe_connection().ok_or(Refusal::Cancelled)?;
                let verifier = source
                    .prefix
                    .selected_member_verifier()
                    .map_err(|_| Refusal::Stale)?;
                let replay = verifier.replay_data();
                let profile = replay.profile().spec();
                let founder = replay
                    .slots()
                    .find(|slot| slot.image.subject == profile.founder)
                    .map(|slot| OwnerFactId::from_bytes(*slot.image.admission.as_bytes()))
                    .ok_or(Refusal::Stale)?;
                let selected = verifier.admission(founder).map_err(|_| Refusal::Stale)?;
                if selected.admission_data().spec().origin
                    != (AdmissionOrigin::Founder {
                        creation: profile.creation,
                    })
                {
                    return Err(Refusal::Stale);
                }
                verifier
                    .observe_live_member(founder)
                    .map_err(|_| Refusal::Stale)?;
                let fee = source
                    .metadata
                    .reserve_join_node(REPLY_FEE)
                    .map_err(|_| Refusal::Busy)?;
                let access = source
                    .node
                    .member_history_access(source.prefix, channel)
                    .map_err(|_| Refusal::Stale)?;
                let opened = access
                    .open_removed_review_request(peer, founder, wire)
                    .map_err(|_| Refusal::Failed)?;
                let target = opened.target();
                let author = source
                    .prefix
                    .retired_history_data()
                    .iter()
                    .find(|author| author.admission_id_data() == target.admission)
                    .ok_or(Refusal::Stale)?;
                if verifier
                    .removed_review_coordinates(author)
                    .map_err(|_| Refusal::Stale)?
                    != (target.removal, target.epoch)
                {
                    return Err(Refusal::Stale);
                }
                let request = ReviewRequest {
                    scope: source.relay_scope(channel)?,
                    target,
                    founder,
                    query: opened.query(),
                    connection,
                    now: source.node.member_history_time_ms(),
                };
                let plan = selection
                    .original
                    .archive_relay
                    .try_lock()
                    .map_err(|_| Refusal::Busy)?
                    .review
                    .prepare(&request, rng)?;
                let current = || {
                    if waiting.authorized()
                        && request.connection.is_live()
                        && !caller.is_cancelled()
                    {
                        Ok(())
                    } else {
                        Err(refused())
                    }
                };
                current().map_err(|_| Refusal::Cancelled)?;
                let archive = source.history_scope(channel);
                let reply = match plan {
                    Plan::Page(plan) => {
                        let entries = plan.catalogue.entries[plan.range.clone()]
                            .iter()
                            .copied()
                            .enumerate()
                            .filter(|(offset, entry)| {
                                entry.admission == target.admission
                                    && plan.previous.is_none_or(|offered| offered[*offset])
                            })
                            .map(|(_, entry)| entry)
                            .collect::<Vec<_>>();
                        let mut actions = Vec::with_capacity(entries.len());
                        source
                            .store
                            .visit_selected_shared_history_relay_page(
                                archive,
                                &entries,
                                current,
                                |entry, action| {
                                    if verifier
                                        .require_removed_review_create(author, action)
                                        .is_ok()
                                    {
                                        actions.push(entry.action);
                                    }
                                    Ok(())
                                },
                            )
                            .map_err(|_| Refusal::Failed)?;
                        let page = V2ArchiveHistoryPage::new(actions, plan.next)
                            .and_then(|page| V2RemovedReviewPage::new(plan.offer, page))
                            .map_err(|_| Refusal::Failed)?;
                        let reply = access
                            .seal_removed_review_page((peer, founder), &opened, &page, rng)
                            .map_err(|_| Refusal::Failed)?;
                        current().map_err(|_| Refusal::Cancelled)?;
                        selection
                            .original
                            .archive_relay
                            .try_lock()
                            .map_err(|_| Refusal::Busy)?
                            .review
                            .finish_page(
                                &plan,
                                page.actions(),
                                source.node.member_history_time_ms(),
                            )?;
                        reply
                    }
                    Plan::Original(entry) => {
                        let retained = source
                            .store
                            .read_selected_shared_history_record(archive, entry.action)
                            .map_err(|_| Refusal::Failed)?
                            .ok_or(Refusal::Stale)?;
                        if retained.admission != target.admission {
                            return Err(Refusal::Stale);
                        }
                        verifier
                            .require_removed_review_create(author, retained.original.action())
                            .map_err(|_| Refusal::Stale)?;
                        access
                            .seal_removed_review_original(
                                (peer, founder),
                                &opened,
                                retained.original.original_bytes(),
                                rng,
                            )
                            .map_err(|_| Refusal::Failed)?
                    }
                };
                current().map_err(|_| Refusal::Cancelled)?;
                let (observed, authentication) = source.effect_data()?;
                Ok((reply, observed, authentication, fee))
            })?;
        if !waiting.authorized() {
            return Err(Refusal::Cancelled);
        }
        let (signal, invocation) =
            self.dependency_signal(selection, caller, authentication, observed);
        incoming
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::critical_namespaces::read_pool::CriticalCatalogReadPool;
    use catcoms_replication::chat::owner_header::OwnerHeaderId;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn removed_review_relay_requires_offered_action_and_original_sweep_connection() {
        let scope = RelayScope {
            head: OwnerHeaderId::from_bytes([1; 32]),
            epoch: 2,
            channel: 3,
        };
        let author = OwnerFactId::from_bytes([4; 32]);
        let target = V2RemovedReviewTarget {
            admission: author,
            removal: scope.head,
            epoch: scope.epoch,
        };
        let pool = CriticalCatalogReadPool::new(16 * 1024).unwrap();
        let mut state = ReviewRelayState::default();
        let action = |index| ActionId::from_bytes([index; 32]);
        state.replace_catalogue(
            (1..=9)
                .map(|index| ArchiveRelayEntry {
                    action: action(index),
                    admission: author,
                })
                .chain((10..=26).map(|index| ArchiveRelayEntry {
                    action: action(index),
                    admission: OwnerFactId::from_bytes([3; 32]),
                }))
                .collect(),
            scope,
            pool.try_reserve(4096).unwrap(),
        );
        let mut rng = ChaCha20Rng::seed_from_u64(7);
        let connection = ConnectionLease::default();
        let mut request = ReviewRequest {
            scope,
            target,
            founder: OwnerFactId::from_bytes([5; 32]),
            query: V2RemovedReviewQuery::Page(None),
            connection: connection.clone(),
            now: 6,
        };
        let Plan::Page(first) = state.prepare(&request, &mut rng).unwrap() else {
            panic!("first query must prepare a page");
        };
        assert_eq!(first.range, 17..25);
        request.query = V2RemovedReviewQuery::Original {
            offer: first.offer,
            action: action(1),
        };
        assert!(state.prepare(&request, &mut rng).is_err());
        state.finish_page(&first, &[action(1)], 8).unwrap();
        let Plan::Original(exact) = state.prepare(&request, &mut rng).unwrap() else {
            panic!("the exact offer must select one record");
        };
        assert_eq!(exact.action, action(1));
        request.query = V2RemovedReviewQuery::Page(None);
        let Plan::Page(retry) = state.prepare(&request, &mut rng).unwrap() else {
            panic!("a retry must prepare the original page");
        };
        assert!(retry.offer == first.offer);
        assert_eq!(
            retry.previous,
            Some([true, false, false, false, false, false, false, false])
        );
        assert!(state
            .finish_page(&retry, &[action(1), action(2)], 8)
            .is_err());
        // The first reply can be lost while one original becomes unreadable.
        // The retry still covers the same continuation.
        assert!(retry.next == first.next);
        state.finish_page(&retry, &[], 8).unwrap();
        for (offer, id) in [(first.offer, action(2)), (first.next.unwrap(), action(1))] {
            request.query = V2RemovedReviewQuery::Original { offer, action: id };
            assert!(state.prepare(&request, &mut rng).is_err());
        }
        request.query = V2RemovedReviewQuery::Page(first.next);
        let Plan::Page(last) = state.prepare(&request, &mut rng).unwrap() else {
            panic!("the continuation should find the last page for this Admission");
        };
        assert_eq!(last.range, 25..26);
        assert!(last.next.is_none());
        state.finish_page(&last, &[action(9)], 9).unwrap();
        request.query = V2RemovedReviewQuery::Original {
            offer: first.offer,
            action: action(1),
        };
        assert!(state.prepare(&request, &mut rng).is_ok());
        request.target.admission = OwnerFactId::from_bytes([6; 32]);
        assert!(state.prepare(&request, &mut rng).is_err());
        request.target = target;
        request.connection = ConnectionLease::default();
        assert!(state.prepare(&request, &mut rng).is_err());
        request.connection = connection;
        request.scope.head = OwnerHeaderId::from_bytes([8; 32]);
        assert!(state.prepare(&request, &mut rng).is_err());
    }
}
