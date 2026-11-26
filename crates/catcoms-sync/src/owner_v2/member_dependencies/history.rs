//! Page hints for one provider's original saved history. The caller must prove
//! approval and current admission on the native connection before serving or
//! using a page. These IDs carry no authority to render or accept an action.
use super::*;

catcoms_rt::redacted_debug! {
    PreparedV2MemberHistoryRequest;
    OpenedV2MemberHistoryRequest;
    V2MemberHistoryPage;
    V2JoinedMemberHistoryAccess<'_>;
    V2MemberHistoryEntry;
}

const HISTORY_REQUEST_TAG: &[u8] = b"mewtual/chat/member-history-request/v2\0";
const HISTORY_REPLY_TAG: &[u8] = b"mewtual/chat/member-history-reply/v2\0";
const HISTORY_REQUEST_KEY: &str = "mewtual/chat/member-history-request-key/v2";
const HISTORY_REPLY_KEY: &str = "mewtual/chat/member-history-reply-key/v2";
const HISTORY_REQUEST_ID: &str = "mewtual/chat/member-history-request-id/v2";

pub const V2_MEMBER_HISTORY_PAGE_SPAN: usize = 8;
const HISTORY_REQUEST_BYTES: usize = HISTORY_REQUEST_TAG.len() + OVERHEAD + 2 * 36 + 8 + 1 + 8;
const HISTORY_ENTRY_BYTES: usize = 8 + 36;
const MAX_V2_MEMBER_HISTORY_REPLY_BYTES: usize =
    HISTORY_REPLY_TAG.len() + OVERHEAD + 8 + 1 + V2_MEMBER_HISTORY_PAGE_SPAN * HISTORY_ENTRY_BYTES;
const _: () = assert!(HISTORY_REQUEST_BYTES < catcoms_rt::CHAT_V2_DEPENDENCY_REQUEST_MAX_BYTES);
const _: () =
    assert!(MAX_V2_MEMBER_HISTORY_REPLY_BYTES <= catcoms_rt::CHAT_V2_DEPENDENCY_RESPONSE_MAX_BYTES);

/// One verified joined node's selected channel. The native caller still owns
/// its approved remote admission, connection lease and disk authorization.
pub struct V2JoinedMemberHistoryAccess<'a> {
    pub(crate) group: &'a ServerGroup,
    pub(crate) device: &'a MlsDevice,
    pub(crate) scope: V2MemberEnvelopeScopeData,
    pub(crate) local: PeerId,
}

impl V2JoinedMemberHistoryAccess<'_> {
    pub fn prepare_request(
        &self,
        provider: PeerId,
        cursor: V2MemberHistoryCursor,
        rng: &mut impl CryptoRngCore,
    ) -> Result<PreparedV2MemberHistoryRequest, SyncError> {
        PreparedV2MemberHistoryRequest::prepare(
            self.group,
            self.device,
            self.scope,
            V2MemberDependencyRoute {
                requester: self.local,
                provider,
            },
            cursor,
            rng,
        )
    }

    pub fn open_reply(
        &self,
        provider: PeerId,
        request: &PreparedV2MemberHistoryRequest,
        wire: &[u8],
    ) -> Result<V2MemberHistoryPage, SyncError> {
        request.open_reply(
            self.group,
            self.device,
            &self.scope,
            V2MemberDependencyRoute {
                requester: self.local,
                provider,
            },
            wire,
        )
    }

    pub fn open_request(
        &self,
        requester: PeerId,
        wire: &[u8],
    ) -> Result<OpenedV2MemberHistoryRequest, SyncError> {
        OpenedV2MemberHistoryRequest::open(
            self.group,
            self.device,
            self.scope,
            V2MemberDependencyRoute {
                requester,
                provider: self.local,
            },
            wire,
        )
    }

    pub fn seal_reply(
        &self,
        requester: PeerId,
        request: &OpenedV2MemberHistoryRequest,
        page: &V2MemberHistoryPage,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Vec<u8>, SyncError> {
        request.seal_reply(
            self.group,
            self.device,
            &self.scope,
            V2MemberDependencyRoute {
                requester,
                provider: self.local,
            },
            page,
            rng,
        )
    }
}

/// Provider-local positions in an append-only outbox. A continuation keeps the
/// original end, so new writes cannot extend this sweep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct V2MemberHistoryCursor {
    start: u64,
    end: Option<u64>,
}

impl V2MemberHistoryCursor {
    pub fn first() -> Self {
        Self {
            start: 0,
            end: None,
        }
    }

    fn new(start: u64, end: Option<u64>) -> Result<Self, SyncError> {
        match end {
            Some(end) if start <= end => Ok(Self {
                start,
                end: Some(end),
            }),
            None if start == 0 => Ok(Self::first()),
            _ => Err(SyncError::Malformed),
        }
    }

    pub fn bounds(self) -> (u64, Option<u64>) {
        (self.start, self.end)
    }
}

#[derive(Clone, Copy)]
pub struct V2MemberHistoryEntry {
    pub ordinal: u64,
    pub action: ActionId,
}

/// The slice scanned is derived from the request and its frozen end. Missing
/// entries mean the provider did not advertise them in this slice.
pub struct V2MemberHistoryPage {
    cursor: V2MemberHistoryCursor,
    end: u64,
    next: u64,
    entries: Vec<V2MemberHistoryEntry>,
}

impl V2MemberHistoryPage {
    pub fn new(
        cursor: V2MemberHistoryCursor,
        end: u64,
        entries: Vec<V2MemberHistoryEntry>,
    ) -> Result<Self, SyncError> {
        if cursor.end.is_some_and(|expected| expected != end) || end < cursor.start {
            return Err(SyncError::Malformed);
        }
        let next = end.min(
            cursor
                .start
                .saturating_add(V2_MEMBER_HISTORY_PAGE_SPAN as u64),
        );
        if entries.len() > (next - cursor.start) as usize
            || entries
                .windows(2)
                .any(|pair| pair[0].ordinal >= pair[1].ordinal)
        {
            return Err(SyncError::Malformed);
        }
        for (index, entry) in entries.iter().enumerate() {
            if !(cursor.start..next).contains(&entry.ordinal)
                || entries[..index]
                    .iter()
                    .any(|other| other.action == entry.action)
            {
                return Err(SyncError::Malformed);
            }
        }
        Ok(Self {
            cursor,
            end,
            next,
            entries,
        })
    }

    pub fn entries(&self) -> &[V2MemberHistoryEntry] {
        &self.entries
    }

    pub fn continuation(&self) -> Option<V2MemberHistoryCursor> {
        (self.next < self.end).then_some(V2MemberHistoryCursor {
            start: self.next,
            end: Some(self.end),
        })
    }
}

struct HistoryBinding {
    scope: V2MemberEnvelopeScopeData,
    route: V2MemberDependencyRoute,
    cursor: V2MemberHistoryCursor,
    exact: [u8; 32],
}

impl HistoryBinding {
    fn reply_key(
        &self,
        group: &ServerGroup,
        device: &MlsDevice,
        current: &V2MemberEnvelopeScopeData,
        route: V2MemberDependencyRoute,
    ) -> Result<Zeroizing<[u8; 32]>, SyncError> {
        if self.scope != *current || self.route != route {
            return Err(SyncError::Malformed);
        }
        let base = member_exporter_key(group, device, current, HISTORY_REPLY_KEY)?;
        Ok(Zeroizing::new(
            *blake3::keyed_hash(&base, &self.exact).as_bytes(),
        ))
    }
}

pub struct PreparedV2MemberHistoryRequest {
    binding: HistoryBinding,
    wire: Vec<u8>,
}

impl PreparedV2MemberHistoryRequest {
    fn prepare(
        group: &ServerGroup,
        device: &MlsDevice,
        scope: V2MemberEnvelopeScopeData,
        route: V2MemberDependencyRoute,
        cursor: V2MemberHistoryCursor,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Self, SyncError> {
        require_route(route)?;
        let key = member_exporter_key(group, device, &scope, HISTORY_REQUEST_KEY)?;
        let mut body = Encoder::new();
        data(body.put_bytes(route.requester.as_bytes()))?;
        data(body.put_bytes(route.provider.as_bytes()))?;
        body.put_u64(cursor.start);
        if let Some(end) = cursor.end {
            body.put_u8(1).put_u64(end);
        } else {
            body.put_u8(0);
        }
        let wire = seal_frame(
            &key,
            HISTORY_REQUEST_TAG,
            &Zeroizing::new(body.finish()),
            rng,
        )?;
        Ok(Self {
            binding: HistoryBinding {
                scope,
                route,
                cursor,
                exact: blake3::derive_key(HISTORY_REQUEST_ID, &wire),
            },
            wire,
        })
    }

    pub fn wire_data(&self) -> &[u8] {
        &self.wire
    }

    fn open_reply(
        &self,
        group: &ServerGroup,
        device: &MlsDevice,
        current: &V2MemberEnvelopeScopeData,
        route: V2MemberDependencyRoute,
        wire: &[u8],
    ) -> Result<V2MemberHistoryPage, SyncError> {
        let frame = parse_frame(wire, HISTORY_REPLY_TAG, MAX_V2_MEMBER_HISTORY_REPLY_BYTES)?;
        let key = self.binding.reply_key(group, device, current, route)?;
        let plain = open_frame(&key, frame)?;
        let mut body = Decoder::new(&plain);
        let end = data(body.get_u64())?;
        let count = usize::from(data(body.get_u8())?);
        if count > V2_MEMBER_HISTORY_PAGE_SPAN {
            return Err(SyncError::Malformed);
        }
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let ordinal = data(body.get_u64())?;
            let action = ActionId::from_bytes(
                data(body.get_bytes())?
                    .try_into()
                    .map_err(|_| SyncError::Malformed)?,
            );
            entries.push(V2MemberHistoryEntry { ordinal, action });
        }
        data(body.finish())?;
        V2MemberHistoryPage::new(self.binding.cursor, end, entries)
    }
}

pub struct OpenedV2MemberHistoryRequest(HistoryBinding);

impl OpenedV2MemberHistoryRequest {
    /// Dispatch by tag only. Authentication remains mandatory in `open`.
    pub fn recognizes(wire: &[u8]) -> bool {
        Decoder::new(wire)
            .get_bytes()
            .is_ok_and(|tag| tag == HISTORY_REQUEST_TAG)
    }

    fn open(
        group: &ServerGroup,
        device: &MlsDevice,
        scope: V2MemberEnvelopeScopeData,
        route: V2MemberDependencyRoute,
        wire: &[u8],
    ) -> Result<Self, SyncError> {
        require_route(route)?;
        let frame = parse_frame(wire, HISTORY_REQUEST_TAG, HISTORY_REQUEST_BYTES)?;
        let key = member_exporter_key(group, device, &scope, HISTORY_REQUEST_KEY)?;
        let plain = open_frame(&key, frame)?;
        let mut body = Decoder::new(&plain);
        if data(body.get_bytes())? != route.requester.as_bytes()
            || data(body.get_bytes())? != route.provider.as_bytes()
        {
            return Err(SyncError::Malformed);
        }
        let start = data(body.get_u64())?;
        let end = match data(body.get_u8())? {
            0 => None,
            1 => Some(data(body.get_u64())?),
            _ => return Err(SyncError::Malformed),
        };
        data(body.finish())?;
        Ok(Self(HistoryBinding {
            scope,
            route,
            cursor: V2MemberHistoryCursor::new(start, end)?,
            exact: blake3::derive_key(HISTORY_REQUEST_ID, wire),
        }))
    }

    pub fn cursor(&self) -> V2MemberHistoryCursor {
        self.0.cursor
    }

    fn seal_reply(
        &self,
        group: &ServerGroup,
        device: &MlsDevice,
        current: &V2MemberEnvelopeScopeData,
        route: V2MemberDependencyRoute,
        page: &V2MemberHistoryPage,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Vec<u8>, SyncError> {
        if self.0.cursor != page.cursor {
            return Err(SyncError::Malformed);
        }
        let key = self.0.reply_key(group, device, current, route)?;
        let mut body = Encoder::new();
        body.put_u64(page.end).put_u8(page.entries.len() as u8);
        for entry in &page.entries {
            body.put_u64(entry.ordinal);
            data(body.put_bytes(entry.action.as_bytes()))?;
        }
        seal_frame(&key, HISTORY_REPLY_TAG, &Zeroizing::new(body.finish()), rng)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    fn entry(ordinal: u64, byte: u8) -> V2MemberHistoryEntry {
        V2MemberHistoryEntry {
            ordinal,
            action: ActionId::from_bytes([byte; 32]),
        }
    }

    #[test]
    fn member_history_page_skips_filtered_ordinals_and_freezes_its_end() {
        let first = V2MemberHistoryCursor::first();
        let page = V2MemberHistoryPage::new(first, 19, vec![entry(2, 2)]).unwrap();
        let next = page.continuation().unwrap();
        assert_eq!(next.bounds(), (8, Some(19)));
        assert!(V2MemberHistoryPage::new(next, 20, vec![]).is_err());
        assert!(V2MemberHistoryPage::new(next, 19, vec![entry(7, 7)]).is_err());
        assert!(V2MemberHistoryPage::new(next, 19, vec![entry(16, 16)]).is_err());
        assert!(V2MemberHistoryPage::new(next, 19, vec![entry(9, 9), entry(8, 8)]).is_err());
        assert!(V2MemberHistoryPage::new(next, 19, vec![entry(8, 8), entry(9, 8)]).is_err());
        let empty = V2MemberHistoryPage::new(next, 19, vec![]).unwrap();
        let tail = empty.continuation().unwrap();
        assert_eq!(tail.bounds(), (16, Some(19)));
        assert!(V2MemberHistoryPage::new(tail, 19, vec![entry(18, 18)])
            .unwrap()
            .continuation()
            .is_none());
        assert!(V2MemberHistoryCursor::new(3, Some(2)).is_err());
        assert!(V2MemberHistoryCursor::new(1, None).is_err());
    }

    #[test]
    fn member_history_exchange_binds_exact_request_scope_and_endpoints() {
        let alice = MlsDevice::generate().unwrap();
        let bob = MlsDevice::generate().unwrap();
        let mut alice_group = ServerGroup::create(&alice).unwrap();
        let group = alice_group.group_id();
        let package = bob.key_package_for_invite(&group, [81; 16]).unwrap();
        let added = alice_group.add_member(&alice, package).unwrap();
        let bob_group = ServerGroup::join(&bob, &added.welcome).unwrap();
        let scope = V2MemberEnvelopeScopeData {
            group: group.try_into().unwrap(),
            profile: OwnerFactId::from_bytes([82; 32]),
            governing: catcoms_replication::chat::owner_header::OwnerHeaderId::from_bytes([83; 32]),
            epoch: alice_group.epoch(),
            channel: 84,
        };
        let route = V2MemberDependencyRoute {
            requester: PeerId::from_u64(1),
            provider: PeerId::from_u64(2),
        };
        let requester = V2JoinedMemberHistoryAccess {
            group: &alice_group,
            device: &alice,
            scope,
            local: route.requester,
        };
        let provider = V2JoinedMemberHistoryAccess {
            group: &bob_group,
            device: &bob,
            scope,
            local: route.provider,
        };
        let mut rng = ChaCha20Rng::seed_from_u64(85);
        let cursor = V2MemberHistoryCursor::first();
        let request = requester
            .prepare_request(route.provider, cursor, &mut rng)
            .unwrap();
        assert!(OpenedV2MemberHistoryRequest::recognizes(
            request.wire_data()
        ));
        assert!(provider
            .open_request(PeerId::from_u64(3), request.wire_data())
            .is_err());
        assert!(OpenedV2MemberDependencyRequest::open(
            &bob_group,
            &bob,
            scope,
            route,
            request.wire_data()
        )
        .is_err());
        let opened = provider
            .open_request(route.requester, request.wire_data())
            .unwrap();
        let entries = (0..8)
            .map(|ordinal| entry(ordinal, ordinal as u8 + 1))
            .collect();
        let page = V2MemberHistoryPage::new(opened.cursor(), 9, entries).unwrap();
        let reply = provider
            .seal_reply(route.requester, &opened, &page, &mut rng)
            .unwrap();
        assert_eq!(reply.len(), MAX_V2_MEMBER_HISTORY_REPLY_BYTES);
        let recovered = requester
            .open_reply(route.provider, &request, &reply)
            .unwrap();
        assert_eq!(recovered.entries().len(), 8);
        assert_eq!(recovered.continuation().unwrap().bounds(), (8, Some(9)));
        assert!(requester
            .open_reply(PeerId::from_u64(3), &request, &reply)
            .is_err());
        let other_scope = V2JoinedMemberHistoryAccess {
            scope: V2MemberEnvelopeScopeData {
                channel: 86,
                ..scope
            },
            ..requester
        };
        assert!(other_scope
            .open_reply(route.provider, &request, &reply)
            .is_err());
        let later = requester
            .prepare_request(route.provider, cursor, &mut rng)
            .unwrap();
        assert!(requester
            .open_reply(route.provider, &later, &reply)
            .is_err());
        let mut tampered = reply;
        *tampered.last_mut().unwrap() ^= 1;
        assert!(requester
            .open_reply(route.provider, &request, &tampered)
            .is_err());
    }
}
