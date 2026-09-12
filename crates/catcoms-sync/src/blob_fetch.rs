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
            },
        })
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
