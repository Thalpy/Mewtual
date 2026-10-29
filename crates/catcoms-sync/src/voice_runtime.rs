//! Ephemeral product/native actor attachment. This enables no wire kind or browser protocol.
//! Preparation can happen while an actor starts/restores or remains network-active during UI
//! lock. Only its final native registration and exact captured unlock ticket can activate V2.

use super::*;
use voice_v2::directory::{DeletionReader, Directory, DirectoryReader};
use voice_v2::helper::RoomAuthority;
use voice_v2::runtime::{ActorOwner, ActorRegistration, Runtime, SessionTicket, View};
use voice_v2::{CallLease, VoiceError, WatchLease};

pub(super) struct VoiceRuntime {
    clock: Arc<dyn Clock>,
    pub(super) resources: Arc<voice_v2::resources::Resources>,
    directory_doc: u128,
    pub(super) directory: Directory,
    candidates: Vec<ActorOwner>,
    pub(super) current: Option<ActorOwner>,
    pub(super) routes: voice_transport::RouteOwners,
    pub(super) endpoints: voice_endpoint::EndpointOwners,
    next_collection: u64,
    #[cfg(test)]
    view_builds: u64,
    #[cfg(test)]
    directory_reads: u64,
}
impl VoiceRuntime {
    pub(super) fn observe_document(
        &mut self,
        doc_type: DocType,
        doc_id: u128,
        doc: &AutoCommit,
        change: Option<&automerge::Change>,
    ) {
        if doc_type == DocType::ChannelIndex && doc_id == self.directory_doc {
            let _ = self.directory.observe_change(Some(doc), change);
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Terminal per-Root teardown before stop acknowledgement or any blocked close-event await.
    /// This closes original actors, rooms, local routes and revision subscribers synchronously.
    /// Do not revoke the shared process Runtime or clear group helper/source ledgers: other
    /// servers still use them, and external admitted action/input/lower owners must retain their
    /// actual charges until they drain. A cancelled freeze must not call this terminal method.
    pub fn retire_voice_actor(&mut self) {
        drop(self.voice_runtime.take());
    }
    pub(super) fn voice_transport_actor(&self) -> Result<&ActorOwner, VoiceError> {
        self.voice_runtime
            .as_ref()
            .and_then(|voice| voice.current.as_ref())
            .ok_or(VoiceError::Unauthorized)
    }
    /// Attach another inactive owner to this exact retained actor. Native captures the ticket
    /// before authorization awaits, then registers the returned capability only at its final
    /// exact native entry/session commit. Dropping an unregistered reply revokes that candidate.
    /// Mounted unlock uses this same path; it never revives the old permanently revoked owner.
    pub fn prepare_voice_actor(
        &mut self,
        runtime: &Runtime,
        ticket: &SessionTicket,
        directory_doc: u128,
        reader: DirectoryReader,
        deletions: DeletionReader,
    ) -> Result<ActorRegistration, VoiceError> {
        let group = voice_v2::Scope::for_group(&self.group.group_id(), None, 0).group;
        if let Some(voice) = &mut self.voice_runtime {
            if !Arc::ptr_eq(&voice.resources, &runtime.resources())
                || voice.directory_doc != directory_doc
            {
                return Err(VoiceError::Unauthorized);
            }
            voice.candidates.retain(ActorOwner::viable);
            if voice.candidates.len() >= 8 {
                return Err(VoiceError::Capacity);
            }
        }
        if self.voice_runtime.is_some() {
            let (owner, registration) =
                runtime.prepare_actor(group, self.device.device_id(), ticket)?;
            self.voice_runtime
                .as_mut()
                .ok_or(VoiceError::Unauthorized)?
                .candidates
                .push(owner);
            self.observe_voice_directory();
            return Ok(registration);
        }
        // This already-fallible Root attachment knows the exact shared pool and
        // group. Prepay all registry backing/inline metadata before constructing
        // VoiceRuntime; failed Actor preparation drops Vec before plain funding.
        let routes = voice_transport::RouteOwners::new(&runtime.resources(), group)?;
        let (owner, registration) =
            runtime.prepare_actor(group, self.device.device_id(), ticket)?;
        let voice = VoiceRuntime {
            clock: runtime.clock(),
            resources: runtime.resources(),
            directory_doc,
            directory: Directory::new(group, runtime.resources(), reader, deletions),
            candidates: vec![owner],
            current: None,
            routes,
            endpoints: voice_endpoint::EndpointOwners::default(),
            next_collection: runtime.clock().monotonic_ms().saturating_add(1000),
            #[cfg(test)]
            view_builds: 0,
            #[cfg(test)]
            directory_reads: 0,
        };
        self.voice_runtime = Some(voice);
        self.observe_voice_directory();
        Ok(registration)
    }

    /// Synchronous control-transition observation, including a mutation followed by Err. Reusing
    /// the MLS observation seam closes the driver gap before an actor/network await. It does not
    /// replace the packet/pair membership checks at each eventual protocol effect.
    pub(super) fn reconcile_voice_actor(&mut self) {
        // Networking remains active while the desktop is locked. Its independent generation
        // still observes every actual MLS/policy transition, including an error after a merge.
        self.reconcile_member_maintenance();
        let Some(voice) = &mut self.voice_runtime else {
            return;
        };
        voice.candidates.retain(ActorOwner::viable);
        if let Some(index) = voice.candidates.iter().position(ActorOwner::installed) {
            voice.current = Some(voice.candidates.remove(index));
        }
        if voice.current.as_ref().is_some_and(|owner| !owner.viable()) {
            voice.current = None;
        }
        let eligible = self.voice_v2_context(0).is_ok();
        #[cfg(test)]
        {
            self.voice_runtime.as_mut().unwrap().view_builds += 1;
        }
        let view = View {
            epoch: self.group.epoch(),
            members: self.group.member_device_ids().into_iter().collect(),
            policy: self.group_policy.as_ref().map(GroupPolicy::digest),
            eligible,
            // Shared with voice_v2_context: the local VoiceRouting switch can only narrow this.
            indirect: self.voice_indirect_allowed(),
        };
        if let Some(voice) = &mut self.voice_runtime {
            if let Some(owner) = &mut voice.current {
                // Registered inactive owners wait for their captured native unlock commit; stale
                // restored state waits for MLS control recovery. Neither result grants effects.
                let _ =
                    owner.sync_view_retiring(view, |member| voice.endpoints.retire_member(member));
            }
            voice.endpoints.prune();
        }
    }

    pub(super) fn observe_voice_directory(&mut self) {
        if let Some(voice) = &mut self.voice_runtime {
            #[cfg(test)]
            {
                voice.directory_reads += 1;
            }
            let doc = self
                .docs
                .get(&(DocType::ChannelIndex, voice.directory_doc))
                .map(EncryptedDoc::doc);
            // Failed/oversized reads already revoke all directory authority. Keep diagnosis
            // redacted and let the next valid observation reopen only fresh room incarnations.
            if let Err(error) = voice.directory.observe(doc) {
                tracing::debug!(%error,"voice directory unavailable");
            }
        }
    }
    pub(super) fn observe_voice_document(&mut self, doc_type: DocType, doc_id: u128) {
        if doc_type == DocType::ChannelIndex
            && self
                .voice_runtime
                .as_ref()
                .is_some_and(|voice| voice.directory_doc == doc_id)
        {
            self.observe_voice_directory();
        }
    }
    pub fn voice_v2_clock(&self) -> Arc<dyn Clock> {
        self.voice_runtime
            .as_ref()
            .map(|voice| voice.clock.clone())
            .unwrap_or_else(|| self.clock.clone())
    }
    /// Actor-local cache collection is independent of current V2 permission and never waits for
    /// a WebView consumer. The actor merges this absolute deadline into its existing fair wake.
    pub fn voice_housekeeping_deadline(&self) -> Option<u64> {
        self.voice_runtime
            .as_ref()
            .map(|voice| voice.next_collection)
    }
    /// Keep the deadline paired with its own Clock. Trusted integrations/tests may give the
    /// ordinary server clock a different monotonic origin from the process voice runtime.
    pub fn voice_housekeeping_wake(&self) -> Option<(Arc<dyn Clock>, u64)> {
        self.voice_runtime
            .as_ref()
            .map(|voice| (voice.clock.clone(), voice.next_collection))
    }
    pub fn collect_expired_voice(&mut self) {
        // Idle/unjoined Roots can retain completed verification rows. Return
        // their actual sole-owner metadata before any callback or shared-lane
        // reservation, including while another Root needs the process Crypto cap.
        self.prune_voice_routes();
        let Some(voice) = &mut self.voice_runtime else {
            return;
        };
        let now = voice.clock.monotonic_ms();
        if now < voice.next_collection {
            return;
        }
        // Existing quiet-network housekeeping needs no fresh RPC or allocation.
        // It also reclaims canceled preparations in OTHER rooms under full pressure.
        // Original externally held aliases continue to retain their original payment.
        if let Some(actor) = &voice.current {
            actor.collect_closed_children();
        }
        for candidate in &voice.candidates {
            candidate.collect_closed_children();
        }
        voice.directory.collect_all_closed_children();
        voice.endpoints.collect_expired(voice.clock.as_ref());
        voice.next_collection = now.saturating_add(1000);
    }
    pub fn voice_room_authority(&mut self, room: u128) -> Result<RoomAuthority, VoiceError> {
        self.reconcile_voice_actor();
        self.observe_voice_directory();
        let context = self.voice_v2_context(room)?;
        let voice = self
            .voice_runtime
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        voice
            .current
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?
            .helper()?;
        voice.directory.authority(context.scope)
    }
    pub fn voice_call_lease(&mut self, room: u128) -> Result<CallLease, VoiceError> {
        self.reconcile_voice_actor();
        self.observe_voice_directory();
        let context = self.voice_v2_context(room)?;
        let voice = self
            .voice_runtime
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        let owner = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        voice.directory.call_lease(owner, context.scope)
    }
    pub fn voice_watch_lease(&mut self, room: u128) -> Result<WatchLease, VoiceError> {
        self.reconcile_voice_actor();
        self.observe_voice_directory();
        let context = self.voice_v2_context(room)?;
        let voice = self
            .voice_runtime
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        let owner = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        voice.directory.watch_lease(owner, context.scope)
    }

    /// Production asynchronous callers receive affine custody, rather than an unowned clone.
    pub fn voice_call_grant(&mut self, room: u128) -> Result<voice_v2::CallGrant, VoiceError> {
        self.reconcile_voice_actor();
        self.observe_voice_directory();
        let scope = self.voice_v2_context(room)?.scope;
        let voice = self
            .voice_runtime
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        let owner = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        voice.directory.call_grant(owner, scope)
    }
    /// Native endpoint installation must recognize the captured actual issuer, not just accept
    /// the grant's public scope. Recheck after every asynchronous native authorization step.
    pub fn validate_voice_call_grant(
        &mut self,
        grant: &voice_v2::CallGrant,
    ) -> Result<(), VoiceError> {
        self.require_existing_voice_call(grant)?;
        self.reconcile_voice_actor();
        self.observe_voice_directory();
        if self.voice_v2_context(grant.scope().room)?.scope != grant.scope() {
            return Err(VoiceError::Scope);
        }
        let voice = self
            .voice_runtime
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        voice
            .directory
            .validate_call_grant(self.voice_transport_actor()?, grant)
    }

    /// Validate an already adopted native effect without rebuilding the whole roster or
    /// directory on its small worker payment. Actual MLS/policy/document mutation hooks
    /// still reconcile/revoke synchronously, including prefix-applying failures; do not
    /// replace those hooks with this method. If reconciliation is not current, comparing
    /// the ACTUAL MLS epoch/policy/local leaf with the paid Actor view refuses effects.
    /// This neither refreshes an old grant nor adopts a replacement directory incarnation.
    pub(super) fn validate_current_voice_call_grant(
        &self,
        grant: &voice_v2::CallGrant,
    ) -> Result<(), VoiceError> {
        self.require_existing_voice_call(grant)?;
        let context = self.voice_v2_context(grant.scope().room)?;
        if context.scope != grant.scope() {
            return Err(VoiceError::Scope);
        }
        let actor = self.voice_transport_actor()?;
        actor.try_with_current_members(&context, |_| Ok(()))?;
        self.voice_runtime
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?
            .directory
            .validate_call_grant(actor, grant)
    }
}

#[cfg(test)]
mod tests;
