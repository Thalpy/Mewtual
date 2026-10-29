//! Real kind28 Control7 flights, independent of the Existing Body/Source grammar.
//! All transport work is connected-only and retains the original paid job.
use super::*;
use catcoms_rt::{RequestAuthority, RequestCancellation, SharedRequestKeepalive};
use futures::FutureExt;
use voice_v2::admitted_control::{self as control, Job, Outcome};
#[cfg(test)]
mod tests;

struct Authority {
    route: Arc<dyn RequestAuthority>,
    job: Arc<Job>,
    clock: Arc<dyn Clock>,
    deadline: u64,
}
impl fmt::Debug for Authority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AdmittedControlAuthority { .. }")
    }
}
#[async_trait::async_trait]
impl RequestAuthority for Authority {
    fn authorized(&self) -> bool {
        let mono = self.clock.monotonic_ms();
        // Job samples its ORIGINAL time before checking the actual route gates.
        self.job.authorized() && self.route.authorized() && mono < self.deadline
    }
    async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        futures::future::select_all(vec![
            self.route.revoked().boxed(),
            self.job.revoked().boxed(),
            self.clock.sleep(std::time::Duration::from_millis(
                self.deadline.saturating_sub(self.clock.monotonic_ms()),
            )),
        ])
        .await;
    }
}
pub(crate) struct PendingAdmittedControl<T: MeshTransport> {
    transport: Arc<T>,
    request: Bytes,
    permit: Option<Box<control::wire::Permit>>,
    job: Arc<Job>,
    authority: Arc<Authority>,
    instance: Arc<()>,
    // LAST even for an unexecuted value after Root retirement.
    _custody: SharedRequestKeepalive,
}
pub(crate) struct CompletedAdmittedControl {
    request: Bytes,
    permit: Option<Box<control::wire::Permit>>,
    job: Arc<Job>,
    authority: Arc<Authority>,
    instance: Arc<()>,
    response: Result<Bytes, TransportError>,
    _custody: SharedRequestKeepalive,
}
fn envelope<'a>(
    job: &'a Job,
    permit: Option<&'a control::wire::Permit>,
) -> control::wire::EnvelopeRef<'a> {
    if let Some(permit) = job.helper_permit() {
        control::wire::EnvelopeRef::Delivery(job.object(), permit)
    } else if let Some(permit) = permit {
        control::wire::EnvelopeRef::ToHelper(job.object(), permit)
    } else {
        control::wire::EnvelopeRef::Direct(job.object())
    }
}
fn wrapper_graph<T: MeshTransport>() -> usize {
    // Full preparation/execution overlap, completed Box, shared scheduler
    // variant and both authorities/delegation. Buffer copies/route metadata
    // keep their separate LOWER_BYTES terms. Check before attempt debit.
    2 * std::mem::size_of::<PendingAdmittedControl<T>>()
        + std::mem::size_of::<CompletedAdmittedControl>()
        + std::mem::size_of::<crate::voice_schedule::VoiceFlight>()
        + std::mem::size_of::<Authority>()
        + std::mem::size_of::<control::wire::Permit>()
        + 128
}
impl<T: MeshTransport> PendingAdmittedControl<T> {
    pub(crate) fn peer(&self) -> PeerId {
        self.job.peer()
    }
    pub(crate) async fn execute(
        self,
        signal: tokio::sync::watch::Receiver<bool>,
    ) -> CompletedAdmittedControl {
        let cancellation = RequestCancellation::new(signal, Some(self._custody.clone()))
            .with_authority(self.authority.clone());
        let response = match futures::future::select(
            Box::pin(self.transport.request_connected_cancellable(
                self.job.peer(),
                ProtocolId(RR_PROTOCOL),
                self.request.clone(),
                cancellation,
            )),
            self.authority.revoked().boxed(),
        )
        .await
        {
            futures::future::Either::Left((response, _)) => response,
            futures::future::Either::Right(((), _)) => Err(TransportError::Cancelled),
        }
        .and_then(|bytes| {
            if bytes.len() <= 512 {
                Ok(bytes)
            } else {
                Err(TransportError::NoResponse)
            }
        });
        CompletedAdmittedControl {
            request: self.request,
            permit: self.permit,
            job: self.job,
            authority: self.authority,
            instance: self.instance,
            response,
            _custody: self._custody,
        }
    }
}
impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// The new-family ingress lookup also borrows the native roster without
    /// allocating legacy route-index tables before its verification prepayment.
    pub(super) fn admitted_bound_peer_route(
        &self,
        peer: PeerId,
    ) -> Result<Option<BoundRoute>, VoiceError> {
        let context = self.voice_v2_context(0)?;
        let Some(connection) = self
            .transport
            .connection_lease(peer)
            .filter(|lease| lease.is_live())
        else {
            return Ok(None);
        };
        self.voice_transport_actor()?
            .try_with_current_members(&context, |members| {
                let mut owner = None;
                for (device, record) in &self.peer_records {
                    if record.peer_id == *peer.as_bytes() && members(*device) {
                        if owner.is_some() {
                            return Ok(None);
                        }
                        owner = Some(*device);
                    }
                }
                let Some(device) = owner else {
                    return Ok(None);
                };
                if device == context.local
                    || !connection.is_live()
                    || !self
                        .member_peers
                        .iter()
                        .any(|proof| proof.bound && proof.device == device && proof.peer == peer)
                {
                    return Ok(None);
                }
                Ok(Some(BoundRoute { device, peer }))
            })
    }
    /// Allocation-free unique bound DEVICE lookup. Native membership is borrowed
    /// only after the driver's connection callback has returned; no dial occurs.
    pub(crate) fn admitted_bound_device_route(
        &self,
        device: DeviceId,
        context: &voice_v2::Context,
    ) -> Result<Option<BoundRoute>, VoiceError> {
        let Some(record) = self.peer_records.get(&device) else {
            return Ok(None);
        };
        let peer = PeerId::new(record.peer_id);
        let Some(connection) = self
            .transport
            .connection_lease(peer)
            .filter(|lease| lease.is_live())
        else {
            return Ok(None);
        };
        if !self
            .member_peers
            .iter()
            .any(|proof| proof.bound && proof.device == device && proof.peer == peer)
        {
            return Ok(None);
        }
        self.voice_transport_actor()?
            .try_with_current_members(context, |members| {
                if device == context.local
                    || !members(device)
                    || !connection.is_live()
                    || self
                        .peer_records
                        .iter()
                        .filter(|(id, row)| members(**id) && row.peer_id == *peer.as_bytes())
                        .count()
                        != 1
                {
                    return Ok(None);
                }
                Ok(Some(BoundRoute { device, peer }))
            })
    }
    pub(super) fn prepare_admitted_control_reply(
        &mut self,
        from: PeerId,
        outer: &Outer<'_>,
        ticket: voice_v2::runtime::IngressTicket,
        verification: Arc<voice_v2::runtime::VerificationReplyAuthority>,
        route: Arc<dyn RequestAuthority>,
        work: SharedRequestKeepalive,
    ) -> Option<VoiceTransportReply> {
        let decoded = control::wire::Envelope::decode(outer.inner);
        let mut receipt = None;
        let (status, hash) = match decoded {
            Err(_) => (Status::Refused, [0; 32]),
            Ok(envelope) => {
                let hash = envelope.object().hash();
                let admitted = (|| -> Result<Status, VoiceError> {
                    let m = envelope.object().metadata();
                    let context = self.voice_v2_context(m.scope().room)?;
                    let room = self.voice_room_authority(context.scope.room)?;
                    let actor = self.voice_transport_actor()?;
                    // This repeats exact signed epoch/room/member/route/driver
                    // request binding, without copying the legacy packet path.
                    let disposition = actor.with_control_origin(
                        &ticket,
                        &envelope,
                        &context,
                        |_, _, origin| Ok(origin.disposition()),
                    )?;
                    match disposition {
                        Disposition::Deliver => {
                            receipt = Some(
                                self.admit_admitted_control(&ticket, &envelope, &context, room)?,
                            );
                            Ok(Status::EndpointAdmitted)
                        }
                        Disposition::Forward(_) => {
                            let actor = self.voice_transport_actor()?;
                            let ledger = actor.control()?;
                            actor.prepare_control_members(m.issuer(), m.target(), &context)?;
                            let cached = ledger.cached(m, hash, true)?;
                            let (owner, cached) = match cached {
                                Some(owner) => (owner, true),
                                None => (
                                    ledger.prepare_forward(actor, &envelope, &context, room)?,
                                    false,
                                ),
                            };
                            receipt = Some(
                                ledger
                                    .admit(actor, &ticket, &envelope, &context, &owner, cached)?,
                            );
                            Ok(Status::HelperQueued)
                        }
                    }
                })()
                .unwrap_or(Status::Refused);
                (admitted, hash)
            }
        };
        let body = answer_body(status, hash, outer.inner);
        let transcript = response_transcript(
            &self.group.group_id(),
            outer.key,
            outer.auth,
            from,
            self.transport.local_peer(),
            &body,
        );
        let signature = self.device.sign(&transcript).ok()?;
        VoiceTransportReply::new_admitted(
            encode_signed_commit_resp(&self.device.public_key_bytes(), &signature, &body),
            verification,
            route,
            receipt,
            work,
        )
    }
    pub(crate) fn prepare_admitted_controls(
        &mut self,
        helper: bool,
    ) -> Result<Vec<PendingAdmittedControl<T>>, VoiceError> {
        if wrapper_graph::<T>() > 4096 {
            return Err(VoiceError::Bounds);
        }
        self.reconcile_voice_actor();
        self.prune_voice_routes();
        let Some(ledger) = self.voice_transport_actor()?.control_if_present() else {
            return Ok(Vec::new());
        };
        ledger.collect_expired();
        let mut result = Vec::new();
        for owner in ledger.candidates(helper) {
            if result.len() == 4 {
                break;
            }
            let m = owner.object().metadata();
            let context = self.voice_v2_context(m.scope().room)?;
            if context.scope != m.scope() || context.epoch != m.epoch() {
                continue;
            }
            let (attempts, cursor) = owner.route_cursor();
            let (wall, mono) = owner.samples();
            let helpers = if helper {
                [None, None]
            } else {
                self.admitted_control_helpers(m.scope(), wall, mono, m.target(), cursor)
            };
            let direct = self.admitted_bound_device_route(m.target(), &context)?;
            let mut selected = None;
            // First try the exact target; after a failed direct edge a compatible
            // helper may hedge. Rotate later helpers within the same three tries.
            if helper || attempts == 0 || helpers.iter().all(Option::is_none) {
                selected = direct.map(|route| (route, None));
            }
            if selected.is_none() {
                for helper in helpers.into_iter().flatten() {
                    if let Some(route) = self.admitted_bound_device_route(
                        helper.object().metadata().target(),
                        &context,
                    )? {
                        selected = Some((route, Some(helper)));
                        break;
                    }
                }
            }
            let Some((route, selected_helper)) = selected else {
                continue;
            };
            // Route registry capacity is checked before spending an attempt.
            if self.admitted_route_capacity() == 0 {
                break;
            }
            let job = match ledger.job(&owner, route, selected_helper) {
                Ok(job) => job,
                Err(VoiceError::Capacity | VoiceError::Unauthorized) => continue,
                Err(error) => return Err(error),
            };
            if let Ok(pending) = self.prepare_admitted_request(job, &context) {
                result.push(pending);
            }
        }
        Ok(result)
    }
    fn prepare_admitted_request(
        &mut self,
        job: Job,
        context: &voice_v2::Context,
    ) -> Result<PendingAdmittedControl<T>, VoiceError> {
        let custody = job.keepalive();
        let route = self
            .admitted_bound_device_route(job.target(), context)?
            .ok_or(VoiceError::Unauthorized)?;
        if route.peer != job.peer() {
            return Err(VoiceError::Unauthorized);
        }
        let gate = self.capture_voice_reply_route(route, custody.clone())?;
        let permit = if let Some(helper) = job.selected_helper() {
            let now = self.voice_v2_clock().now_ms();
            Some(Box::new(control::wire::Permit::issue(
                job.object(),
                helper,
                &self.device,
                now,
                job.object()
                    .metadata()
                    .expires()
                    .min(now.saturating_add(voice_v2::MAX_PACKET_LIFETIME_MS)),
            )?))
        } else {
            None
        };
        let inner = envelope(&job, permit.as_deref()).encode();
        let job = Arc::new(job);
        let clock = self.voice_v2_clock();
        let deadline = clock
            .monotonic_ms()
            .checked_add(3000)
            .ok_or(VoiceError::Freshness)?;
        let authority = Arc::new(Authority {
            route: gate,
            job: job.clone(),
            clock,
            deadline,
        });
        let key = self.device.public_key_bytes();
        let mut nonce = [0; 16];
        self.rng.fill_bytes(&mut nonce);
        let auth = RequestAuth {
            ts: self.voice_v2_clock().now_ms(),
            nonce,
            epoch: context.epoch,
        };
        let transcript = catchup_auth_transcript(
            &self.group.group_id(),
            KIND,
            &inner,
            &key,
            auth.ts,
            &auth.nonce,
            auth.epoch,
            Some(&self.local_peer()),
        );
        let signature = self
            .device
            .sign(&transcript)
            .map_err(|_| VoiceError::Signature)?;
        drop(transcript);
        let request = Bytes::from(encode_authed_request_frame(
            KIND,
            &inner,
            &key,
            auth.ts,
            &auth.nonce,
            auth.epoch,
            &signature,
        ));
        if !authority.authorized() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(PendingAdmittedControl {
            transport: self.transport.clone(),
            request,
            permit,
            job,
            authority,
            instance: self.blob_fetch_instance.clone(),
            _custody: custody,
        })
    }
    pub(crate) fn complete_admitted_control(
        &mut self,
        completed: CompletedAdmittedControl,
    ) -> Result<(), CompletedAdmittedControl> {
        self.reconcile_voice_actor();
        let checked = (|| {
            if !Arc::ptr_eq(&completed.instance, &self.blob_fetch_instance)
                || !completed.authority.authorized()
            {
                return Err(VoiceError::Unauthorized);
            }
            completed.job.current(self.voice_transport_actor()?)?;
            let status = match &completed.response {
                Ok(response) => Some(self.verify_admitted_control_ack(&completed, response)?),
                Err(_) => None,
            };
            if !completed.authority.authorized() {
                return Err(VoiceError::Unauthorized);
            }
            completed.job.complete(status);
            Ok(())
        })();
        match checked {
            Err(VoiceError::Capacity | VoiceError::RateLimited) => Err(completed),
            _ => Ok(()),
        }
    }
    fn verify_admitted_control_ack(
        &self,
        completed: &CompletedAdmittedControl,
        response: &[u8],
    ) -> Result<Outcome, VoiceError> {
        let request = Outer::parse(&completed.request)?;
        if response.len() > 512 {
            return Err(VoiceError::Bounds);
        }
        let context = self.voice_v2_context(completed.job.object().metadata().scope().room)?;
        let route = self
            .admitted_bound_device_route(completed.job.target(), &context)?
            .ok_or(VoiceError::Unauthorized)?;
        if route.peer != completed.job.peer() {
            return Err(VoiceError::Unauthorized);
        }
        let mut d = Decoder::new(response);
        let key: &[u8; 32] = d
            .get_bytes()?
            .try_into()
            .map_err(|_| VoiceError::Malformed)?;
        let signature: &[u8; 64] = d
            .get_bytes()?
            .try_into()
            .map_err(|_| VoiceError::Malformed)?;
        let body = d.get_bytes()?;
        d.finish()?;
        if DeviceId::from_public_key_bytes(key) != route.device
            || self.group.member_signature_key(&route.device).as_deref() != Some(key.as_slice())
        {
            return Err(VoiceError::Unauthorized);
        }
        let mut d = Decoder::new(body);
        if d.get_u8()? != 2 {
            return Err(VoiceError::Malformed);
        }
        let status = Status::parse(d.get_u8()?)?;
        let object: [u8; 32] = d
            .get_bytes()?
            .try_into()
            .map_err(|_| VoiceError::Malformed)?;
        let frame_hash: [u8; 32] = d
            .get_bytes()?
            .try_into()
            .map_err(|_| VoiceError::Malformed)?;
        let length = d.get_u32()? as usize;
        d.finish()?;
        if frame_hash != *blake3::hash(request.inner).as_bytes()
            || length != request.inner.len()
            || (object != completed.job.object().hash()
                && (status != Status::Refused || object != [0; 32]))
        {
            return Err(VoiceError::RetryMismatch);
        }
        let ticket = self
            .voice_transport_actor()?
            .admit_transport_verification(route.device, response)?;
        let transcript = response_transcript(
            &self.group.group_id(),
            request.key,
            request.auth,
            self.transport.local_peer(),
            completed.job.peer(),
            body,
        );
        if !verify_with_public_bytes(key, &transcript, signature) {
            return Err(VoiceError::Signature);
        }
        let as_responder = voice_v2::Context {
            local: route.device,
            ..context
        };
        let role = self
            .voice_transport_actor()?
            .try_with_current_members(&context, |members| {
                envelope(&completed.job, completed.permit.as_deref()).authorize(
                    self.device.device_id(),
                    &as_responder,
                    members,
                )
            })?;
        let outcome = match (role, status) {
            (_, Status::Refused) => Outcome::Refused,
            (Disposition::Deliver, Status::EndpointAdmitted) => Outcome::Delivered,
            (Disposition::Forward(_), Status::HelperQueued) => Outcome::Queued,
            _ => return Err(VoiceError::Unauthorized),
        };
        ticket.check_response(self.voice_transport_actor()?, response)?;
        Ok(outcome)
    }
}
