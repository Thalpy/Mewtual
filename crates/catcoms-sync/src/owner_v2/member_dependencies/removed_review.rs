//! Founder-only review request data over the bounded dependency connection.
//! The native host checks the original founder route, approved holder, and
//! selected Remove. The group exporter alone authenticates none of those.
use super::archive_history::{
    decode_page, encode_page, get_cursor, put_cursor, V2ArchiveHistoryCursor, V2ArchiveHistoryPage,
    V2_ARCHIVE_HISTORY_PAGE_SIZE,
};
use super::*;
use catcoms_replication::chat::{owner_header::OwnerHeaderId, Body};

catcoms_rt::redacted_debug! {
    PreparedV2RemovedReviewRequest;
    OpenedV2RemovedReviewRequest;
    V2RemovedReviewOriginal;
    V2RemovedReviewPage;
    V2RemovedReviewQuery;
    V2RemovedReviewTarget;
}

const REQUEST: &[u8] = b"mewtual/chat/removed-review-request/v2\0";
const PAGE: &[u8] = b"mewtual/chat/removed-review-page/v2\0";
const ORIGINAL: &[u8] = b"mewtual/chat/removed-review-original/v2\0";
const REQUEST_KEY: &str = "mewtual/chat/removed-review-request-key/v2";
const REPLY_KEY: &str = "mewtual/chat/removed-review-reply-key/v2";
const REQUEST_ID: &str = "mewtual/chat/removed-review-request-id/v2";
const REQUEST_BODY: usize = 4 * 36 + 8 + 1;
const MAX_REQUEST: usize = REQUEST.len() + OVERHEAD + REQUEST_BODY;
const MAX_PAGE: usize =
    PAGE.len() + OVERHEAD + 36 + 4 + 1 + (V2_ARCHIVE_HISTORY_PAGE_SIZE + 1) * 36;
const MAX_ORIGINAL: usize = ORIGINAL.len() + OVERHEAD + archive_relay::MAX_PADDED_BYTES;
const _: () = assert!(MAX_REQUEST < catcoms_rt::CHAT_V2_DEPENDENCY_REQUEST_MAX_BYTES);
const _: () = assert!(MAX_PAGE <= catcoms_rt::CHAT_V2_DEPENDENCY_RESPONSE_MAX_BYTES);
const _: () = assert!(MAX_ORIGINAL <= catcoms_rt::CHAT_V2_DEPENDENCY_RESPONSE_MAX_BYTES);

/// Exact retired admission and the governing Remove that closed it. The
/// same device after a later Add has a different Admission.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct V2RemovedReviewTarget {
    pub admission: OwnerFactId,
    pub removal: OwnerHeaderId,
    pub epoch: u64,
}

/// A fetch names the exact inventory page that offered the original. A
/// holder must resolve the cursor and prove the offer before reading it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum V2RemovedReviewQuery {
    Page(Option<V2ArchiveHistoryCursor>),
    Original {
        offer: V2ArchiveHistoryCursor,
        action: ActionId,
    },
}

/// A snapshot page with a provider-issued offer token, even for the first
/// page. The token for fetching from this page is separate from the cursor
/// that advances the sweep.
pub struct V2RemovedReviewPage {
    offer: V2ArchiveHistoryCursor,
    page: V2ArchiveHistoryPage,
}

impl V2RemovedReviewPage {
    pub fn new(
        offer: V2ArchiveHistoryCursor,
        page: V2ArchiveHistoryPage,
    ) -> Result<Self, SyncError> {
        if page.continuation() == Some(offer) {
            return Err(SyncError::Malformed);
        }
        Ok(Self { offer, page })
    }

    pub fn offer(&self) -> V2ArchiveHistoryCursor {
        self.offer
    }

    pub fn actions(&self) -> &[ActionId] {
        self.page.actions()
    }

    pub fn continuation(&self) -> Option<V2ArchiveHistoryCursor> {
        self.page.continuation()
    }
}

struct Binding {
    scope: V2MemberEnvelopeScopeData,
    route: V2MemberDependencyRoute,
    founder: OwnerFactId,
    target: V2RemovedReviewTarget,
    query: V2RemovedReviewQuery,
    request: [u8; 32],
}

pub struct PreparedV2RemovedReviewRequest {
    binding: Binding,
    wire: Vec<u8>,
}

impl PreparedV2RemovedReviewRequest {
    pub fn wire_data(&self) -> &[u8] {
        &self.wire
    }

    pub fn target(&self) -> V2RemovedReviewTarget {
        self.binding.target
    }
}

pub struct OpenedV2RemovedReviewRequest(Binding);

impl OpenedV2RemovedReviewRequest {
    pub fn recognizes(wire: &[u8]) -> bool {
        wire.len() <= MAX_REQUEST
            && Decoder::new(wire)
                .get_bytes()
                .is_ok_and(|tag| tag == REQUEST)
    }

    pub fn target(&self) -> V2RemovedReviewTarget {
        self.0.target
    }

    pub fn query(&self) -> V2RemovedReviewQuery {
        self.0.query
    }
}

/// An authenticated holder reply. Only the original bytes leave this type.
/// The receiver must rederive the selected Remove and withholding evidence
/// before custody. It yields no ordinary receive or projection capability.
pub struct V2RemovedReviewOriginal(OpenedV2MemberEnvelopeData);

impl V2RemovedReviewOriginal {
    pub fn original_data(&self) -> &[u8] {
        self.0.original_data()
    }
}

struct Endpoint<'a> {
    group: &'a ServerGroup,
    device: &'a MlsDevice,
    scope: V2MemberEnvelopeScopeData,
    route: V2MemberDependencyRoute,
    founder: OwnerFactId,
}

impl Endpoint<'_> {
    fn key(&self, purpose: &'static str) -> Result<Zeroizing<[u8; 32]>, SyncError> {
        require_route(self.route)?;
        let key = member_exporter_key(self.group, self.device, &self.scope, purpose)?;
        // The exporter covers group, Profile, governing head, epoch and
        // channel. Ordered transport peers and founder Admission are
        // independently supplied by both ends.
        let mut route = blake3::Hasher::new_keyed(&key);
        route.update(self.route.requester.as_bytes());
        route.update(self.route.provider.as_bytes());
        route.update(self.founder.as_bytes());
        Ok(Zeroizing::new(*route.finalize().as_bytes()))
    }

    fn reply_key(&self, binding: &Binding) -> Result<Zeroizing<[u8; 32]>, SyncError> {
        if self.scope != binding.scope
            || self.route != binding.route
            || self.founder != binding.founder
        {
            return Err(SyncError::JoinRejected);
        }
        let key = self.key(REPLY_KEY)?;
        Ok(Zeroizing::new(
            *blake3::keyed_hash(&key, &binding.request).as_bytes(),
        ))
    }

    fn binding(
        &self,
        target: V2RemovedReviewTarget,
        query: V2RemovedReviewQuery,
        wire: &[u8],
    ) -> Result<Binding, SyncError> {
        if target.admission == self.founder || target.epoch > self.scope.epoch {
            return Err(SyncError::JoinRejected);
        }
        Ok(Binding {
            scope: self.scope,
            route: self.route,
            founder: self.founder,
            target,
            query,
            request: blake3::derive_key(REQUEST_ID, wire),
        })
    }

    fn prepare(
        &self,
        target: V2RemovedReviewTarget,
        query: V2RemovedReviewQuery,
        rng: &mut impl CryptoRngCore,
    ) -> Result<PreparedV2RemovedReviewRequest, SyncError> {
        let key = self.key(REQUEST_KEY)?;
        let mut body = Encoder::with_capacity(REQUEST_BODY);
        data(body.put_bytes(target.admission.as_bytes()))?;
        data(body.put_bytes(target.removal.as_bytes()))?;
        body.put_u64(target.epoch);
        match query {
            V2RemovedReviewQuery::Page(cursor) => {
                body.put_u8(0);
                put_cursor(&mut body, cursor)?;
            }
            V2RemovedReviewQuery::Original { offer, action } => {
                body.put_u8(1);
                data(body.put_bytes(offer.as_bytes()))?;
                data(body.put_bytes(action.as_bytes()))?;
            }
        }
        let wire = seal_frame(&key, REQUEST, &Zeroizing::new(body.finish()), rng)?;
        let binding = self.binding(target, query, &wire)?;
        Ok(PreparedV2RemovedReviewRequest { binding, wire })
    }

    fn open(&self, wire: &[u8]) -> Result<OpenedV2RemovedReviewRequest, SyncError> {
        let key = self.key(REQUEST_KEY)?;
        let plain = open_frame(&key, parse_frame(wire, REQUEST, MAX_REQUEST)?)?;
        let mut body = Decoder::new(&plain);
        let target = V2RemovedReviewTarget {
            admission: OwnerFactId::from_bytes(digest(&mut body)?),
            removal: OwnerHeaderId::from_bytes(digest(&mut body)?),
            epoch: data(body.get_u64())?,
        };
        let query = match data(body.get_u8())? {
            0 => V2RemovedReviewQuery::Page(get_cursor(&mut body)?),
            1 => V2RemovedReviewQuery::Original {
                offer: V2ArchiveHistoryCursor::from_bytes(digest(&mut body)?),
                action: ActionId::from_bytes(digest(&mut body)?),
            },
            _ => return Err(SyncError::Malformed),
        };
        data(body.finish())?;
        self.binding(target, query, wire)
            .map(OpenedV2RemovedReviewRequest)
    }
}

impl V2FounderMemberDependencyAccess<'_> {
    /// The live native caller separately checks that the provider is an
    /// approved current holder on this original connection.
    pub fn prepare_removed_review(
        &self,
        provider: PeerId,
        target: V2RemovedReviewTarget,
        query: V2RemovedReviewQuery,
        rng: &mut impl CryptoRngCore,
    ) -> Result<PreparedV2RemovedReviewRequest, SyncError> {
        self.removed_endpoint(provider)?.prepare(target, query, rng)
    }

    pub fn open_removed_review_page(
        &self,
        provider: PeerId,
        request: &PreparedV2RemovedReviewRequest,
        wire: &[u8],
    ) -> Result<V2RemovedReviewPage, SyncError> {
        let endpoint = self.removed_endpoint(provider)?;
        let page = open_page(endpoint, &request.binding, wire)?;
        self.removed_endpoint(provider)?;
        Ok(page)
    }

    pub fn open_removed_review_original(
        &self,
        provider: PeerId,
        request: &PreparedV2RemovedReviewRequest,
        wire: &[u8],
    ) -> Result<V2RemovedReviewOriginal, SyncError> {
        let endpoint = self.removed_endpoint(provider)?;
        let original = open_original(endpoint, &request.binding, wire)?;
        self.removed_endpoint(provider)?;
        Ok(original)
    }

    fn removed_endpoint(&self, provider: PeerId) -> Result<Endpoint<'_>, SyncError> {
        if !self.observation.is_current_cached() {
            return Err(SyncError::JoinRejected);
        }
        Ok(Endpoint {
            group: self.group,
            device: self.device,
            scope: self.scope,
            route: V2MemberDependencyRoute {
                requester: self.local,
                provider,
            },
            founder: self.member.admission_data().admission_id(),
        })
    }
}

impl history::V2JoinedMemberHistoryAccess<'_> {
    /// The holder must check the requester's pinned founder Admission and
    /// original invite peer/connection before serving the requested target.
    pub fn open_removed_review_request(
        &self,
        requester: PeerId,
        founder: OwnerFactId,
        wire: &[u8],
    ) -> Result<OpenedV2RemovedReviewRequest, SyncError> {
        self.removed_endpoint(requester, founder).open(wire)
    }

    pub fn seal_removed_review_page(
        &self,
        (requester, founder): (PeerId, OwnerFactId),
        request: &OpenedV2RemovedReviewRequest,
        page: &V2RemovedReviewPage,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Vec<u8>, SyncError> {
        let endpoint = self.removed_endpoint(requester, founder);
        require_progress(&request.0, page)?;
        let key = endpoint.reply_key(&request.0)?;
        let mut body = Encoder::with_capacity(MAX_PAGE - PAGE.len() - OVERHEAD);
        data(body.put_bytes(page.offer.as_bytes()))?;
        data(body.put_bytes(&encode_page(&page.page)?))?;
        seal_frame(&key, PAGE, &Zeroizing::new(body.finish()), rng)
    }

    pub fn seal_removed_review_original(
        &self,
        (requester, founder): (PeerId, OwnerFactId),
        request: &OpenedV2RemovedReviewRequest,
        exact: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<Vec<u8>, SyncError> {
        let V2RemovedReviewQuery::Original { action, .. } = request.0.query else {
            return Err(SyncError::Malformed);
        };
        let endpoint = self.removed_endpoint(requester, founder);
        let key = endpoint.reply_key(&request.0)?;
        let target = request.target();
        let checked =
            super::super::member_envelope::checked_action(exact, &self.scope, target.admission)?;
        require_create(&checked, action)?;
        let plain = archive_relay::encode_original(&self.scope, action, target.admission, exact)?;
        seal_frame(&key, ORIGINAL, &plain, rng)
    }

    fn removed_endpoint(&self, requester: PeerId, founder: OwnerFactId) -> Endpoint<'_> {
        Endpoint {
            group: self.group,
            device: self.device,
            scope: self.scope,
            route: V2MemberDependencyRoute {
                requester,
                provider: self.local,
            },
            founder,
        }
    }
}

fn open_page(
    endpoint: Endpoint<'_>,
    binding: &Binding,
    wire: &[u8],
) -> Result<V2RemovedReviewPage, SyncError> {
    let key = endpoint.reply_key(binding)?;
    let plain = open_frame(&key, parse_frame(wire, PAGE, MAX_PAGE)?)?;
    let mut body = Decoder::new(&plain);
    let offer = V2ArchiveHistoryCursor::from_bytes(digest(&mut body)?);
    let page = decode_page(data(body.get_bytes())?)?;
    data(body.finish())?;
    let page = V2RemovedReviewPage::new(offer, page)?;
    require_progress(binding, &page)?;
    Ok(page)
}

fn open_original(
    endpoint: Endpoint<'_>,
    binding: &Binding,
    wire: &[u8],
) -> Result<V2RemovedReviewOriginal, SyncError> {
    let V2RemovedReviewQuery::Original { action, .. } = binding.query else {
        return Err(SyncError::Malformed);
    };
    let key = endpoint.reply_key(binding)?;
    let plain = open_frame(&key, parse_frame(wire, ORIGINAL, MAX_ORIGINAL)?)?;
    let original = archive_relay::open_original(&endpoint.scope, &plain)?;
    if original.admission_data() != binding.target.admission {
        return Err(SyncError::JoinRejected);
    }
    require_create(original.action_data(), action)?;
    Ok(V2RemovedReviewOriginal(original))
}

fn require_create(action: &SignedAction, id: ActionId) -> Result<(), SyncError> {
    if action.id() != id || !matches!(action.spec().body, Body::Create { .. }) {
        return Err(SyncError::JoinRejected);
    }
    Ok(())
}

fn require_progress(binding: &Binding, page: &V2RemovedReviewPage) -> Result<(), SyncError> {
    let V2RemovedReviewQuery::Page(cursor) = binding.query else {
        return Err(SyncError::Malformed);
    };
    if Some(page.offer()) == cursor
        || (page.continuation().is_some() && page.continuation() == cursor)
    {
        return Err(SyncError::Malformed);
    }
    Ok(())
}

fn digest(input: &mut Decoder<'_>) -> Result<[u8; 32], SyncError> {
    data(input.get_bytes())?
        .try_into()
        .map_err(|_| SyncError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_replication::chat::{
        ActionDraft, ActionSpec, BasisId, GrantId, Realm, MAX_ACTION_BYTES,
    };
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    struct Pair {
        founder: MlsDevice,
        holder: MlsDevice,
        founder_group: ServerGroup,
        holder_group: ServerGroup,
        scope: V2MemberEnvelopeScopeData,
        route: V2MemberDependencyRoute,
        founder_admission: OwnerFactId,
        target: V2RemovedReviewTarget,
    }

    impl Pair {
        fn new() -> Self {
            let founder = MlsDevice::generate().unwrap();
            let holder = MlsDevice::generate().unwrap();
            let mut founder_group = ServerGroup::create(&founder).unwrap();
            let group = founder_group.group_id();
            let package = holder.key_package_for_invite(&group, [1; 16]).unwrap();
            let added = founder_group.add_member(&founder, package).unwrap();
            let holder_group = ServerGroup::join(&holder, &added.welcome).unwrap();
            let scope = V2MemberEnvelopeScopeData {
                group: group.try_into().unwrap(),
                profile: OwnerFactId::from_bytes([2; 32]),
                governing: OwnerHeaderId::from_bytes([3; 32]),
                epoch: founder_group.epoch(),
                channel: 42,
            };
            Self {
                founder,
                holder,
                founder_group,
                holder_group,
                scope,
                route: V2MemberDependencyRoute {
                    requester: PeerId::from_u64(4),
                    provider: PeerId::from_u64(5),
                },
                founder_admission: OwnerFactId::from_bytes([6; 32]),
                target: V2RemovedReviewTarget {
                    admission: OwnerFactId::from_bytes([7; 32]),
                    removal: OwnerHeaderId::from_bytes([8; 32]),
                    epoch: scope.epoch,
                },
            }
        }

        fn requester(&self) -> Endpoint<'_> {
            Endpoint {
                group: &self.founder_group,
                device: &self.founder,
                scope: self.scope,
                route: self.route,
                founder: self.founder_admission,
            }
        }

        fn provider(&self) -> history::V2JoinedMemberHistoryAccess<'_> {
            history::V2JoinedMemberHistoryAccess {
                group: &self.holder_group,
                device: &self.holder,
                scope: self.scope,
                local: self.route.provider,
            }
        }

        fn identity(&self) -> (PeerId, OwnerFactId) {
            (self.route.requester, self.founder_admission)
        }

        fn original(&self, signer: &MlsDevice, body: Body) -> SignedAction {
            let public = signer.public_key_bytes().try_into().unwrap();
            let draft = ActionDraft::new(
                ActionSpec {
                    realm: Realm::new(&self.scope.group, self.scope.channel).unwrap(),
                    basis: BasisId::from_bytes([9; 32]),
                    grant: GrantId::from_bytes([10; 32]),
                    nonce: [11; 16],
                    presented_ms: 1,
                    parents: vec![],
                    body,
                },
                public,
            )
            .unwrap();
            let signature = signer
                .sign_retained_ed25519_bounded(
                    &public,
                    draft.signing_bytes_data(),
                    MAX_ACTION_BYTES,
                )
                .unwrap();
            draft.complete_raw64(signature).unwrap()
        }
    }

    #[test]
    fn removed_review_page_is_bound_to_the_founder_route_remove_and_exact_request() {
        let pair = Pair::new();
        let provider = pair.provider();
        let (requester, founder) = pair.identity();
        let mut rng = ChaCha20Rng::seed_from_u64(42);
        let cursor = V2ArchiveHistoryCursor::from_bytes([12; 32]);
        let query = V2RemovedReviewQuery::Page(Some(cursor));
        let request = pair
            .requester()
            .prepare(pair.target, query, &mut rng)
            .unwrap();
        assert!(OpenedV2RemovedReviewRequest::recognizes(
            request.wire_data()
        ));
        assert!(provider
            .open_archive_page_request(requester, request.wire_data())
            .is_err());
        assert!(provider
            .open_removed_review_request(
                requester,
                OwnerFactId::from_bytes([13; 32]),
                request.wire_data()
            )
            .is_err());
        assert!(provider
            .open_removed_review_request(PeerId::from_u64(17), founder, request.wire_data())
            .is_err());
        let mut other_channel = pair.provider();
        other_channel.scope.channel += 1;
        assert!(other_channel
            .open_removed_review_request(requester, founder, request.wire_data())
            .is_err());
        let opened = provider
            .open_removed_review_request(requester, founder, request.wire_data())
            .unwrap();
        assert!(opened.query() == query);
        assert!(opened.target() == pair.target);
        let offer = V2ArchiveHistoryCursor::from_bytes([19; 32]);
        let stagnant = V2RemovedReviewPage::new(
            offer,
            V2ArchiveHistoryPage::new(Vec::new(), Some(cursor)).unwrap(),
        )
        .unwrap();
        assert!(provider
            .seal_removed_review_page(pair.identity(), &opened, &stagnant, &mut rng)
            .is_err());
        let next = V2ArchiveHistoryCursor::from_bytes([14; 32]);
        let page = V2RemovedReviewPage::new(
            offer,
            V2ArchiveHistoryPage::new(Vec::new(), Some(next)).unwrap(),
        )
        .unwrap();
        let wire = provider
            .seal_removed_review_page(pair.identity(), &opened, &page, &mut rng)
            .unwrap();
        let accepted = open_page(pair.requester(), &request.binding, &wire).unwrap();
        assert!(accepted.offer() == offer);
        assert!(accepted.actions().is_empty());
        assert!(accepted.continuation() == Some(next));

        let first = pair
            .requester()
            .prepare(pair.target, V2RemovedReviewQuery::Page(None), &mut rng)
            .unwrap();
        let first_opened = provider
            .open_removed_review_request(requester, founder, first.wire_data())
            .unwrap();
        let actions = (1..=8)
            .map(|index| ActionId::from_bytes([index; 32]))
            .collect::<Vec<_>>();
        let full = V2RemovedReviewPage::new(
            offer,
            V2ArchiveHistoryPage::new(actions.clone(), Some(next)).unwrap(),
        )
        .unwrap();
        let first_wire = provider
            .seal_removed_review_page(pair.identity(), &first_opened, &full, &mut rng)
            .unwrap();
        assert_eq!(first_wire.len(), MAX_PAGE);
        let first_page = open_page(pair.requester(), &first.binding, &first_wire).unwrap();
        assert_eq!(first_page.actions(), actions);
        assert!(first_page.offer() == offer);
        assert!(open_page(pair.requester(), &request.binding, &first_wire).is_err());
        let unrelated = pair
            .requester()
            .prepare(
                V2RemovedReviewTarget {
                    removal: OwnerHeaderId::from_bytes([15; 32]),
                    ..pair.target
                },
                query,
                &mut rng,
            )
            .unwrap();
        assert!(open_page(pair.requester(), &unrelated.binding, &wire).is_err());
        let retry = pair
            .requester()
            .prepare(pair.target, query, &mut rng)
            .unwrap();
        assert!(open_page(pair.requester(), &retry.binding, &wire).is_err());
        let mut damaged = wire;
        *damaged.last_mut().unwrap() ^= 1;
        assert!(open_page(pair.requester(), &request.binding, &damaged).is_err());
    }

    #[test]
    fn removed_review_original_preserves_its_exact_signed_bytes_and_stays_out_of_ordinary_receive()
    {
        let pair = Pair::new();
        let provider = pair.provider();
        let (requester, founder) = pair.identity();
        let author = MlsDevice::generate().unwrap();
        let exact = pair.original(
            &author,
            Body::Create {
                text: "A late original".into(),
                reply_to: None,
            },
        );
        let mut rng = ChaCha20Rng::seed_from_u64(43);
        let query = V2RemovedReviewQuery::Original {
            offer: V2ArchiveHistoryCursor::from_bytes([12; 32]),
            action: exact.id(),
        };
        let request = pair
            .requester()
            .prepare(pair.target, query, &mut rng)
            .unwrap();
        assert_eq!(request.wire_data().len(), MAX_REQUEST);
        let opened = provider
            .open_removed_review_request(requester, founder, request.wire_data())
            .unwrap();
        assert!(opened.query() == query);
        assert!(provider
            .seal_removed_review_original(pair.identity(), &opened, b"wrong bytes", &mut rng)
            .is_err());
        let bytes = exact.encode();
        let wire = provider
            .seal_removed_review_original(pair.identity(), &opened, &bytes, &mut rng)
            .unwrap();
        let recovered = open_original(pair.requester(), &request.binding, &wire).unwrap();
        assert_eq!(recovered.original_data(), bytes);
        assert!(open_page(pair.requester(), &request.binding, &wire).is_err());
        let ordinary = PreparedV2MemberDependencyRequest::prepare(
            &pair.founder_group,
            &pair.founder,
            pair.scope,
            pair.route,
            exact.id(),
            &mut rng,
        )
        .unwrap();
        assert!(ordinary
            .open_reply(
                &pair.founder_group,
                &pair.founder,
                &pair.scope,
                pair.route,
                &wire
            )
            .is_err());
        let from_rejoin = pair
            .requester()
            .prepare(
                V2RemovedReviewTarget {
                    admission: OwnerFactId::from_bytes([16; 32]),
                    ..pair.target
                },
                query,
                &mut rng,
            )
            .unwrap();
        assert!(open_original(pair.requester(), &from_rejoin.binding, &wire).is_err());
        let changed_offer = pair
            .requester()
            .prepare(
                pair.target,
                V2RemovedReviewQuery::Original {
                    offer: V2ArchiveHistoryCursor::from_bytes([17; 32]),
                    action: exact.id(),
                },
                &mut rng,
            )
            .unwrap();
        assert!(open_original(pair.requester(), &changed_offer.binding, &wire).is_err());
    }
}
