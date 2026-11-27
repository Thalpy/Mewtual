//! Bounded selected-archive inventory. Only the holder can resolve a
//! continuation. An offered ActionId confers no right to open an original.
use super::history::V2JoinedMemberHistoryAccess;
use super::*;

catcoms_rt::redacted_debug! {
    V2ArchiveHistoryCursor;
    V2ArchiveHistoryPage;
    PreparedV2ArchiveHistoryRequest;
    OpenedV2ArchiveHistoryRequest;
}

const REQUEST_TAG: &[u8] = b"mewtual/chat/archive-history-request/v2\0";
const REPLY_TAG: &[u8] = b"mewtual/chat/archive-history-reply/v2\0";
const REQUEST_KEY: &str = "mewtual/chat/archive-history-request-key/v2";
const REPLY_KEY: &str = "mewtual/chat/archive-history-reply-key/v2";
const REQUEST_ID: &str = "mewtual/chat/archive-history-request-id/v2";
pub const V2_ARCHIVE_HISTORY_PAGE_SIZE: usize = 8;
const MAX_REQUEST: usize = REQUEST_TAG.len() + OVERHEAD + 3 * 36;
const MAX_REPLY: usize = REPLY_TAG.len() + OVERHEAD + 1 + (V2_ARCHIVE_HISTORY_PAGE_SIZE + 1) * 36;
const _: () = assert!(MAX_REQUEST < catcoms_rt::CHAT_V2_DEPENDENCY_REQUEST_MAX_BYTES);
const _: () = assert!(MAX_REPLY <= catcoms_rt::CHAT_V2_DEPENDENCY_RESPONSE_MAX_BYTES);

/// A provider-issued continuation. Its bytes contain no raw archive position
/// or filtered ActionId. Passing one to a decoder grants no authority.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct V2ArchiveHistoryCursor([u8; 32]);

impl V2ArchiveHistoryCursor {
    pub fn from_bytes(value: [u8; 32]) -> Self {
        Self(value)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

pub struct V2ArchiveHistoryPage {
    actions: Vec<ActionId>,
    next: Option<V2ArchiveHistoryCursor>,
}

impl V2ArchiveHistoryPage {
    pub fn new(
        actions: Vec<ActionId>,
        next: Option<V2ArchiveHistoryCursor>,
    ) -> Result<Self, SyncError> {
        if actions.len() > V2_ARCHIVE_HISTORY_PAGE_SIZE
            || actions.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(SyncError::Malformed);
        }
        Ok(Self { actions, next })
    }

    pub fn actions(&self) -> &[ActionId] {
        &self.actions
    }

    pub fn continuation(&self) -> Option<V2ArchiveHistoryCursor> {
        self.next
    }
}

struct Binding {
    scope: V2MemberEnvelopeScopeData,
    route: V2MemberDependencyRoute,
    cursor: Option<V2ArchiveHistoryCursor>,
    exact: [u8; 32],
}

impl Binding {
    fn reply_key(
        &self,
        access: &V2JoinedMemberHistoryAccess<'_>,
        route: V2MemberDependencyRoute,
    ) -> Result<Zeroizing<[u8; 32]>, SyncError> {
        if self.scope != access.scope || self.route != route {
            return Err(SyncError::Malformed);
        }
        let key = member_exporter_key(access.group, access.device, &self.scope, REPLY_KEY)?;
        Ok(Zeroizing::new(
            *blake3::keyed_hash(&key, &self.exact).as_bytes(),
        ))
    }

    fn require_progress(&self, page: &V2ArchiveHistoryPage) -> Result<(), SyncError> {
        if page.next.is_some() && page.next == self.cursor {
            return Err(SyncError::Malformed);
        }
        Ok(())
    }
}

pub struct PreparedV2ArchiveHistoryRequest {
    binding: Binding,
    wire: Vec<u8>,
}

impl PreparedV2ArchiveHistoryRequest {
    pub fn wire_data(&self) -> &[u8] {
        &self.wire
    }
}

pub struct OpenedV2ArchiveHistoryRequest(Binding);

impl OpenedV2ArchiveHistoryRequest {
    /// Dispatch by tag only, before authenticated open and contact checks.
    pub fn recognizes(wire: &[u8]) -> bool {
        wire.len() <= MAX_REQUEST
            && Decoder::new(wire)
                .get_bytes()
                .is_ok_and(|tag| tag == REQUEST_TAG)
    }

    pub fn cursor(&self) -> Option<V2ArchiveHistoryCursor> {
        self.0.cursor
    }
}

impl V2JoinedMemberHistoryAccess<'_> {
    pub fn prepare_archive_page(
        &self,
        provider: PeerId,
        cursor: Option<V2ArchiveHistoryCursor>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<PreparedV2ArchiveHistoryRequest, SyncError> {
        let route = V2MemberDependencyRoute {
            requester: self.local,
            provider,
        };
        require_route(route)?;
        let key = member_exporter_key(self.group, self.device, &self.scope, REQUEST_KEY)?;
        let mut body = Encoder::with_capacity(3 * 36);
        data(body.put_bytes(route.requester.as_bytes()))?;
        data(body.put_bytes(route.provider.as_bytes()))?;
        put_cursor(&mut body, cursor)?;
        let wire = seal_frame(&key, REQUEST_TAG, &Zeroizing::new(body.finish()), rng)?;
        let binding = Binding {
            scope: self.scope,
            route,
            cursor,
            exact: blake3::derive_key(REQUEST_ID, &wire),
        };
        Ok(PreparedV2ArchiveHistoryRequest { wire, binding })
    }

    pub fn open_archive_page_request(
        &self,
        requester: PeerId,
        wire: &[u8],
    ) -> Result<OpenedV2ArchiveHistoryRequest, SyncError> {
        let route = V2MemberDependencyRoute {
            requester,
            provider: self.local,
        };
        require_route(route)?;
        let frame = parse_frame(wire, REQUEST_TAG, MAX_REQUEST)?;
        let key = member_exporter_key(self.group, self.device, &self.scope, REQUEST_KEY)?;
        let plain = open_frame(&key, frame)?;
        let mut body = Decoder::new(&plain);
        if data(body.get_bytes())? != route.requester.as_bytes()
            || data(body.get_bytes())? != route.provider.as_bytes()
        {
            return Err(SyncError::Malformed);
        }
        let cursor = get_cursor(&mut body)?;
        data(body.finish())?;
        Ok(OpenedV2ArchiveHistoryRequest(Binding {
            scope: self.scope,
            route,
            cursor,
            exact: blake3::derive_key(REQUEST_ID, wire),
        }))
    }

    pub fn seal_archive_page(
        &self,
        requester: PeerId,
        request: &OpenedV2ArchiveHistoryRequest,
        page: &V2ArchiveHistoryPage,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Vec<u8>, SyncError> {
        request.0.require_progress(page)?;
        let route = V2MemberDependencyRoute {
            requester,
            provider: self.local,
        };
        let key = request.0.reply_key(self, route)?;
        seal_frame(&key, REPLY_TAG, &encode_page(page)?, rng)
    }

    pub fn open_archive_page_reply(
        &self,
        provider: PeerId,
        request: &PreparedV2ArchiveHistoryRequest,
        wire: &[u8],
    ) -> Result<V2ArchiveHistoryPage, SyncError> {
        let route = V2MemberDependencyRoute {
            requester: self.local,
            provider,
        };
        let frame = parse_frame(wire, REPLY_TAG, MAX_REPLY)?;
        let key = request.binding.reply_key(self, route)?;
        let page = decode_page(&open_frame(&key, frame)?)?;
        request.binding.require_progress(&page)?;
        Ok(page)
    }
}

pub(super) fn encode_page(page: &V2ArchiveHistoryPage) -> Result<Zeroizing<Vec<u8>>, SyncError> {
    let mut body = Encoder::with_capacity(MAX_REPLY - OVERHEAD - REPLY_TAG.len());
    body.put_u8(page.actions.len() as u8);
    for action in &page.actions {
        data(body.put_bytes(action.as_bytes()))?;
    }
    put_cursor(&mut body, page.next)?;
    Ok(Zeroizing::new(body.finish()))
}

pub(super) fn decode_page(plain: &[u8]) -> Result<V2ArchiveHistoryPage, SyncError> {
    let mut body = Decoder::new(plain);
    let count = usize::from(data(body.get_u8())?);
    if count > V2_ARCHIVE_HISTORY_PAGE_SIZE {
        return Err(SyncError::Malformed);
    }
    let mut actions = Vec::with_capacity(count);
    for _ in 0..count {
        actions.push(ActionId::from_bytes(
            data(body.get_bytes())?
                .try_into()
                .map_err(|_| SyncError::Malformed)?,
        ));
    }
    let next = get_cursor(&mut body)?;
    data(body.finish())?;
    V2ArchiveHistoryPage::new(actions, next)
}

pub(super) fn put_cursor(
    body: &mut Encoder,
    cursor: Option<V2ArchiveHistoryCursor>,
) -> Result<(), SyncError> {
    data(body.put_bytes(cursor.as_ref().map_or(&[][..], |cursor| cursor.as_bytes())))?;
    Ok(())
}

pub(super) fn get_cursor(
    body: &mut Decoder<'_>,
) -> Result<Option<V2ArchiveHistoryCursor>, SyncError> {
    let bytes = data(body.get_bytes())?;
    if bytes.is_empty() {
        return Ok(None);
    }
    Ok(Some(V2ArchiveHistoryCursor::from_bytes(
        bytes.try_into().map_err(|_| SyncError::Malformed)?,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owner_v2::V2MemberHistoryCursor;
    use catcoms_replication::chat::owner_header::OwnerHeaderId;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn archive_history_binds_the_opaque_cursor_to_the_exact_request() {
        let reader = MlsDevice::generate().unwrap();
        let holder = MlsDevice::generate().unwrap();
        let mut reader_group = ServerGroup::create(&reader).unwrap();
        let group = reader_group.group_id();
        let package = holder.key_package_for_invite(&group, [1; 16]).unwrap();
        let added = reader_group.add_member(&reader, package).unwrap();
        let holder_group = ServerGroup::join(&holder, &added.welcome).unwrap();
        let scope = V2MemberEnvelopeScopeData {
            group: group.try_into().unwrap(),
            profile: OwnerFactId::from_bytes([2; 32]),
            governing: OwnerHeaderId::from_bytes([3; 32]),
            epoch: reader_group.epoch(),
            channel: 42,
        };
        let reader_peer = PeerId::from_u64(4);
        let holder_peer = PeerId::from_u64(5);
        let reader = V2JoinedMemberHistoryAccess {
            group: &reader_group,
            device: &reader,
            scope,
            local: reader_peer,
        };
        let holder = V2JoinedMemberHistoryAccess {
            group: &holder_group,
            device: &holder,
            scope,
            local: holder_peer,
        };
        let mut rng = ChaCha20Rng::seed_from_u64(6);
        let cursor = V2ArchiveHistoryCursor::from_bytes([7; 32]);
        let request = reader
            .prepare_archive_page(holder_peer, Some(cursor), &mut rng)
            .unwrap();
        assert!(OpenedV2ArchiveHistoryRequest::recognizes(
            request.wire_data()
        ));
        assert!(holder
            .open_request(reader_peer, request.wire_data())
            .is_err());
        let own = reader
            .prepare_request(holder_peer, V2MemberHistoryCursor::first(), &mut rng)
            .unwrap();
        assert!(holder
            .open_archive_page_request(reader_peer, own.wire_data())
            .is_err());
        let opened = holder
            .open_archive_page_request(reader_peer, request.wire_data())
            .unwrap();
        assert_eq!(opened.cursor(), Some(cursor));
        let ids = (1..=V2_ARCHIVE_HISTORY_PAGE_SIZE as u8)
            .map(|index| ActionId::from_bytes([index; 32]))
            .collect::<Vec<_>>();
        assert!(V2ArchiveHistoryPage::new(ids[..1].to_vec(), Some(cursor)).is_ok());
        assert!(V2ArchiveHistoryPage::new(vec![ids[0], ids[0]], None).is_err());
        let nonprogressing = V2ArchiveHistoryPage::new(ids.clone(), Some(cursor)).unwrap();
        assert!(holder
            .seal_archive_page(reader_peer, &opened, &nonprogressing, &mut rng)
            .is_err());
        let next = V2ArchiveHistoryCursor::from_bytes([8; 32]);
        let page = V2ArchiveHistoryPage::new(ids.clone(), Some(next)).unwrap();
        let response = holder
            .seal_archive_page(reader_peer, &opened, &page, &mut rng)
            .unwrap();
        let accepted = reader
            .open_archive_page_reply(holder_peer, &request, &response)
            .unwrap();
        assert_eq!(accepted.actions(), ids);
        assert_eq!(accepted.continuation(), Some(next));
        let new_request = reader
            .prepare_archive_page(holder_peer, Some(cursor), &mut rng)
            .unwrap();
        assert!(reader
            .open_archive_page_reply(holder_peer, &new_request, &response)
            .is_err());
        assert!(reader
            .open_archive_page_reply(PeerId::from_u64(9), &request, &response)
            .is_err());
        let mut tampered = response;
        *tampered.last_mut().unwrap() ^= 1;
        assert!(reader
            .open_archive_page_reply(holder_peer, &request, &tampered)
            .is_err());
    }
}
