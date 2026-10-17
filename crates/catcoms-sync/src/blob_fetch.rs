//! Split authenticated blob requests at the network suspension point. The future owns no MLS,
//! file keys, store, or event consumer; completion re-enters the exact originating sync owner.

use super::*;
use catcoms_rt::SharedRequestKeepalive;

/// Independent providers considered for one encrypted variant. Proven live members are preferred;
/// the released explicit-read bootstrap may try known live candidates, but never causes fresh dials.
pub const MAX_BLOB_FETCH_PEERS: usize = 4;

struct FetchContext {
    instance: Arc<()>,
    cid: Cid,
    group: Vec<u8>,
    requester: Vec<u8>,
    provider: Option<DeviceId>,
    auth: RequestAuth,
    max_bytes: usize,
    /// The window this request asked for, when it was a paged fetch.
    ///
    /// Retained so completion can check the answer against the *question*: a page is only
    /// trustworthy as "these bytes, at this offset, of this blob", and the offset it claims must
    /// be the one we asked for rather than one the responder preferred.
    window: Option<u32>,
}

/// One authenticated page of a blob, plus where it sits and whether the blob continues.
#[derive(Debug, Clone)]
pub struct BlobPage {
    /// The page bytes.
    pub bytes: Vec<u8>,
    /// Offset of `bytes` within the whole blob; equal to the offset that was requested.
    pub offset: u32,
    /// Length of the whole blob, as the signer attested it.
    pub total: u32,
    /// Whether the blob continues past this page.
    pub more: bool,
    /// The signing provider's fingerprint.
    pub provider: String,
}

/// What a completed page request turned out to be.
#[derive(Debug)]
pub enum BlobPageOutcome {
    /// An authenticated page.
    Page(Box<BlobPage>),
    /// The provider understands paging and does not hold this blob.
    Absent,
    /// The provider answered empty. On this kind that means it predates paging (or refused the
    /// page under its byte budget), and the caller should fall back to the whole-blob grammar.
    Unsupported,
}

#[cfg(test)]
mod tests;

/// Opaque one-attempt authority prepared by the sync owner. This cannot be cloned/replayed.
pub struct PendingBlobFetch<T: MeshTransport> {
    transport: Arc<T>,
    peer: PeerId,
    request: Vec<u8>,
    context: FetchContext,
}

/// Untrusted response plus the exact request it answers. Only `complete_blob_fetch` may turn it
/// into stored bytes. Retaining accounting here also bounds responses waiting for actor attention.
pub struct CompletedBlobFetch {
    context: FetchContext,
    response: Result<Bytes, TransportError>,
    _keepalive: Option<SharedRequestKeepalive>,
}

impl<T: MeshTransport> fmt::Debug for PendingBlobFetch<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PendingBlobFetch { .. }")
    }
}

impl fmt::Debug for CompletedBlobFetch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CompletedBlobFetch { .. }")
    }
}

impl<T: MeshTransport> PendingBlobFetch<T> {
    /// Perform only network I/O. The caller owns deadline/cancellation signalling and must retain
    /// its own independent capacity for EACH attempt until the transport's actual terminal event.
    pub async fn fetch(self, cancellation: RequestCancellation) -> CompletedBlobFetch {
        let keepalive = cancellation.keepalive();
        let response = self
            .transport
            .request_connected_cancellable(
                self.peer,
                ProtocolId(RR_PROTOCOL),
                Bytes::from(self.request),
                cancellation,
            )
            .await;
        CompletedBlobFetch {
            context: self.context,
            response,
            _keepalive: keepalive,
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// At most four distinct live peers, preferring previously authenticated member responses.
    /// A restored connection may carry file gossip before a new catch-up response; preserve the
    /// existing explicit-read fallback to known live candidates in that case. Such a candidate is
    /// NOT trusted as a member or holder: only its current-member signed response can supply bytes.
    /// Empty/failing blob replies never downgrade unrelated document/membership evidence.
    pub fn blob_fetch_peers(&self) -> Vec<PeerId> {
        let live: HashSet<_> = self
            .transport
            .connection_snapshot()
            .into_iter()
            .map(|row| row.peer)
            .collect();
        let mut peers = Vec::new();
        for proof in self.member_peers.iter().rev() {
            if live.contains(&proof.peer)
                && self.group.contains_device(&proof.device)
                && !peers.contains(&proof.peer)
            {
                peers.push(proof.peer);
                if peers.len() == MAX_BLOB_FETCH_PEERS {
                    break;
                }
            }
        }
        for peer in self.known_peers.iter().rev() {
            if peers.len() == MAX_BLOB_FETCH_PEERS {
                break;
            }
            if live.contains(peer) && !peers.contains(peer) && *peer != self.local_peer() {
                peers.push(*peer);
            }
        }
        peers
    }

    /// The largest page this node is willing to grow to on `peer`'s current path.
    ///
    /// This deliberately does **not** try to name the link. A NAT-punched QUIC connection, which
    /// is what two otherwise-unreachable members end up on and the worst case for a burst, is
    /// indistinguishable from a healthy direct QUIC dial at this seam: libp2p classifies both as
    /// a direct QUIC endpoint. So the requester's page size has to be *measured* by whether
    /// pages actually land, not declared from the path class, and the only thing the path is
    /// consulted for is the ceiling: a relay circuit may have a hard per-circuit data cap under
    /// it, so a fetch over one never grows past the opening size however well it is going.
    pub fn blob_page_ceiling(&self, peer: PeerId) -> usize {
        let relayed_only = self
            .transport
            .connection_snapshot()
            .into_iter()
            .find(|row| row.peer == peer)
            .is_some_and(|row| {
                !row.active.is_empty() && !row.active.iter().any(is_recognized_direct_path)
            });
        if relayed_only {
            MIN_BLOB_PAGE
        } else {
            MAX_BLOB_PAGE
        }
    }

    /// Sign one bounded request under current membership. The caller must separately authorize
    /// this exact blob against its current application manifest before preparation AND completion.
    pub fn prepare_blob_fetch(
        &mut self,
        peer: PeerId,
        cid: Cid,
        max_bytes: usize,
    ) -> Result<PendingBlobFetch<T>, SyncError> {
        if max_bytes > MAX_BOUNDED_BLOB_BYTES || !self.blob_fetch_peers().contains(&peer) {
            return Err(SyncError::Malformed);
        }
        let requester = self.device.public_key_bytes();
        if self
            .group
            .member_signature_key(&self.device.device_id())
            .as_deref()
            != Some(requester.as_slice())
        {
            return Err(SyncError::Unauthorized);
        }
        let provider = self
            .member_peers
            .iter()
            .rev()
            .find(|proof| proof.peer == peer && self.group.contains_device(&proof.device))
            .map(|proof| proof.device);
        let (request, auth) =
            self.build_authed_request(KIND_BLOB_FETCH, &encode_blob_fetch_req(&cid))?;
        Ok(PendingBlobFetch {
            transport: self.transport.clone(),
            peer,
            request,
            context: FetchContext {
                instance: self.blob_fetch_instance.clone(),
                cid,
                group: self.group.group_id(),
                requester,
                provider,
                auth,
                max_bytes,
                window: None,
            },
        })
    }

    /// Sign one **paged** blob request: the same authority and the same one-attempt shape as
    /// [`Self::prepare_blob_fetch`], asking for a window rather than the whole blob.
    ///
    /// `max_bytes` is what the caller would like; the responder clamps it to [`MAX_BLOB_PAGE`],
    /// so this cannot be used to re-create the oversized single response paging exists to
    /// retire. It is accepted at all so a caller on a known-poor path can ask for *less*.
    pub fn prepare_blob_page(
        &mut self,
        peer: PeerId,
        cid: Cid,
        offset: u32,
        max_bytes: usize,
    ) -> Result<PendingBlobFetch<T>, SyncError> {
        let max_bytes = max_bytes.min(MAX_BLOB_PAGE);
        if max_bytes == 0 || !self.blob_fetch_peers().contains(&peer) {
            return Err(SyncError::Malformed);
        }
        let requester = self.device.public_key_bytes();
        if self
            .group
            .member_signature_key(&self.device.device_id())
            .as_deref()
            != Some(requester.as_slice())
        {
            return Err(SyncError::Unauthorized);
        }
        let provider = self
            .member_peers
            .iter()
            .rev()
            .find(|proof| proof.peer == peer && self.group.contains_device(&proof.device))
            .map(|proof| proof.device);
        let (request, auth) = self.build_authed_request(
            KIND_BLOB_PAGE,
            &encode_blob_page_req(&cid, offset, max_bytes as u32),
        )?;
        Ok(PendingBlobFetch {
            transport: self.transport.clone(),
            peer,
            request,
            context: FetchContext {
                instance: self.blob_fetch_instance.clone(),
                cid,
                group: self.group.group_id(),
                requester,
                provider,
                auth,
                max_bytes,
                window: Some(offset),
            },
        })
    }

    /// Authenticate one paged answer without writing anything.
    ///
    /// A page carries no content address of its own, so every guarantee here comes from the
    /// signature: a current member, the provider this request was addressed to, the exact
    /// request (key, timestamp, nonce, epoch), and the exact window. The content address is
    /// checked once, by the caller, over the reassembled whole.
    pub fn authenticate_blob_page(
        &self,
        completed: CompletedBlobFetch,
    ) -> Result<BlobPageOutcome, SyncError> {
        let context = &completed.context;
        let Some(asked_offset) = context.window else {
            return Err(SyncError::Malformed);
        };
        if !Arc::ptr_eq(&self.blob_fetch_instance, &context.instance)
            || self.group.group_id() != context.group
            || self.group.epoch() != context.auth.epoch
            || self.device.public_key_bytes() != context.requester
            || self
                .group
                .member_signature_key(&self.device.device_id())
                .as_deref()
                != Some(context.requester.as_slice())
        {
            return Err(SyncError::Unauthorized);
        }
        let response = completed.response?;
        if response.is_empty() {
            return Ok(BlobPageOutcome::Unsupported);
        }
        let (marker, key, signature, offset, total, page) =
            decode_blob_page_resp(&response, context.max_bytes)?;
        let responder = DeviceId::from_public_key_bytes(key);
        if context
            .provider
            .is_some_and(|provider| responder != provider)
            || !self.group.contains_device(&responder)
        {
            return Err(SyncError::Unauthorized);
        }
        let transcript = blob_page_resp_transcript(
            &context.group,
            &context.requester,
            context.auth.ts,
            &context.auth.nonce,
            context.auth.epoch,
            &context.cid,
            offset,
            total,
            page,
        );
        if !verify_with_public_bytes(key, &transcript, &signature) {
            return Err(SyncError::Malformed);
        }
        // The window is checked against what was asked, not merely against itself. A signature
        // over a *self-consistent* page a requester did not ask for would let a provider skip
        // or repeat a region and only be caught by the final whole-blob hash, after the bytes
        // had already been accumulated.
        if offset != asked_offset {
            return Err(SyncError::Malformed);
        }
        match marker {
            BLOB_PAGE_ABSENT => Ok(BlobPageOutcome::Absent),
            BLOB_PAGE_MORE | BLOB_PAGE_LAST => {
                let end = offset
                    .checked_add(u32::try_from(page.len()).map_err(|_| SyncError::Malformed)?)
                    .ok_or(SyncError::Malformed)?;
                if end > total {
                    return Err(SyncError::Malformed);
                }
                let more = marker == BLOB_PAGE_MORE;
                // A "more" that ends the blob, or a "last" that does not, is an inconsistent
                // answer even though every byte in it is signed. Refusing here keeps the
                // caller's loop terminating on the responder's own claim.
                if more == (end == total) {
                    return Err(SyncError::Malformed);
                }
                // An empty page that claims the blob continues would spin the caller forever.
                if more && page.is_empty() {
                    return Err(SyncError::Malformed);
                }
                Ok(BlobPageOutcome::Page(Box::new(BlobPage {
                    bytes: page.to_vec(),
                    offset,
                    total,
                    more,
                    provider: roles::fingerprint(&responder),
                })))
            }
            _ => Err(SyncError::Malformed),
        }
    }

    /// Consume a network result in its original owner. Membership changes, restored/replaced
    /// owners, wrong signers, oversized responses and replay/substitution all fail before storage.
    /// A successful provider fingerprint attests only to bytes served for this request.
    pub fn complete_blob_fetch(
        &mut self,
        completed: CompletedBlobFetch,
    ) -> Result<Option<String>, SyncError> {
        let Some((blob, provider)) = self.authenticate_blob_fetch(completed)? else {
            return Ok(None);
        };
        self.blobs.put(&blob)?;
        Ok(Some(provider))
    }

    /// Authenticate an exact prepared response without writing it to the ordinary cache. Explicit
    /// kept-copy jobs use this seam to verify file-layer integrity, then write directly into their
    /// pre-reserved disk namespace. Failed/partial copies cannot strand uncapped cache bytes.
    pub fn authenticate_blob_fetch(
        &self,
        completed: CompletedBlobFetch,
    ) -> Result<Option<(Vec<u8>, String)>, SyncError> {
        let context = &completed.context;
        if !Arc::ptr_eq(&self.blob_fetch_instance, &context.instance)
            || self.group.group_id() != context.group
            || self.group.epoch() != context.auth.epoch
            || self.device.public_key_bytes() != context.requester
            || self
                .group
                .member_signature_key(&self.device.device_id())
                .as_deref()
                != Some(context.requester.as_slice())
        {
            return Err(SyncError::Unauthorized);
        }
        let response = completed.response?;
        if response.is_empty() {
            return Ok(None);
        }
        let (key, signature, blob) = decode_blob_response(&response, context.max_bytes)?;
        let responder = DeviceId::from_public_key_bytes(key);
        if context
            .provider
            .is_some_and(|provider| responder != provider)
            || !self.group.contains_device(&responder)
        {
            return Err(SyncError::Unauthorized);
        }
        let transcript = blob_fetch_resp_transcript(
            &context.group,
            &context.requester,
            context.auth.ts,
            &context.auth.nonce,
            context.auth.epoch,
            blob,
        );
        if !verify_with_public_bytes(key, &transcript, &signature) || Cid::of(blob) != context.cid {
            return Err(SyncError::Malformed);
        }
        Ok(Some((blob.to_vec(), roles::fingerprint(&responder))))
    }
}
