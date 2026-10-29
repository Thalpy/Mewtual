//! Trusted inactive call-profile activation, before key delegation. These entry
//! points install no packet producer; separate admitted presence requires the
//! actual selected NativeActive winner and the canonical native key tuple.
use super::*;
use voice_v2::call_activation::{ActivationObserver, PreparedAdmittedCall, RootSelectedCall};

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Same preparation through the paid Actor envelope. Refuse a foreign pool or
    /// scope before allocating/issuing any admitted child.
    pub fn prepare_paid_admitted_voice_call(
        &mut self,
        room: u128,
        payment: &voice_v2::call_activation::ActivationRpcPermit,
    ) -> Result<PreparedAdmittedCall, VoiceError> {
        self.reconcile_voice_actor();
        self.observe_voice_directory();
        let context = self.voice_v2_context(room)?;
        let voice = self
            .voice_runtime
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        if !payment.matches_scope(&voice.resources, context.scope) {
            return Err(VoiceError::Scope);
        }
        self.prepare_admitted_voice_call(room)
    }
    /// Prepare one fresh, inert issuer-owned pair. Native must preserve this exact
    /// pair through its candidate CAS and all uncertain replies; no Root selection yet.
    pub fn prepare_admitted_voice_call(
        &mut self,
        room: u128,
    ) -> Result<PreparedAdmittedCall, VoiceError> {
        self.reconcile_voice_actor();
        self.observe_voice_directory();
        let context = self.voice_v2_context(room)?;
        let voice = self
            .voice_runtime
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        let actor = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        PreparedAdmittedCall::prepare(
            &voice.directory,
            voice.resources.clone(),
            self.blob_fetch_instance.clone(),
            actor,
            &context,
        )
    }

    /// Called only by the canonical paid native activation task while it owns the
    /// ORIGINAL UI guard. Selection is inert; key discovery requires a later,
    /// separately checked NativeActive admitted-presence publication.
    pub fn select_admitted_voice_call(
        &mut self,
        grant: &Arc<CallGrant>,
        original: &ActivationObserver,
    ) -> Result<RootSelectedCall, VoiceError> {
        self.reconcile_voice_actor();
        self.observe_voice_directory();
        let context = self.voice_v2_context(original.scope().room)?;
        let voice = self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        if original.scope() != context.scope
            || !original.belongs_to(&voice.resources, &self.blob_fetch_instance)
            || !original.matches_grant(grant)
            || original.is_closed()
        {
            return Err(VoiceError::Unauthorized);
        }
        let actor = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        voice.directory.validate_call_grant(actor, grant)?;
        actor.try_with_current_members(&context, |_| Ok(()))?;
        let source = actor.source()?;
        voice.endpoints.prune();
        if let Some(residence) = &voice.endpoints.activation {
            if residence.original().is_same(original) {
                return original.selected_receipt();
            }
            if !residence.original().is_closed() {
                return Err(VoiceError::Unauthorized);
            }
        }
        // A healthy Existing call cannot be upgraded in place, even with no pair.
        if voice
            .endpoints
            .participant
            .as_ref()
            .is_some_and(|p| p.lease.is_live())
        {
            return Err(VoiceError::Unauthorized);
        }
        let (residence, selected) = original.select()?;
        source.retire_existing_endpoint_sources();
        // No await or second Root turn can interleave this install. A native close
        // AFTER the winning CAS may install a CLOSED residence; its real marker still
        // has to return. It never becomes a guessed never-entered disposition.
        let old = voice.endpoints.activation.replace(residence);
        if let Some(old) = &old {
            let observer = old.original();
            voice
                .directory
                .forget_closed_child(observer.scope(), observer.original_lease());
            actor.forget_closed_child(observer.original_lease());
        }
        self.pending_call_signals.clear();
        // All Group/Policy guards have returned. Do not bury the last Root alias in
        // a scope that could run its actual destructor while those guards are held.
        drop(old);
        Ok(selected)
    }

    /// Exact negative cleanup, including after UI/epoch/call revocation. It neither
    /// consults a current positive grant nor restarts an Actor or closes a successor.
    pub fn clear_admitted_voice_call(
        &mut self,
        original: &ActivationObserver,
    ) -> Result<(), VoiceError> {
        if !Arc::ptr_eq(&self.blob_fetch_instance, original.root_instance()) {
            return Err(VoiceError::Unauthorized);
        }
        original.close();
        let removed = self.voice_runtime.as_mut().and_then(|voice| {
            voice
                .directory
                .forget_closed_child(original.scope(), original.original_lease());
            if let Some(actor) = &voice.current {
                actor.forget_closed_child(original.original_lease());
            }
            if voice
                .endpoints
                .activation
                .as_ref()
                .is_some_and(|held| held.original().is_same(original))
            {
                voice.endpoints.activation.take()
            } else {
                None
            }
        });
        drop(removed); // actual unique residence marker, outside every admission guard
        Ok(())
    }

    pub(crate) fn has_admitted_voice_activation(&self) -> bool {
        self.voice_runtime.as_ref().is_some_and(|voice| {
            voice
                .endpoints
                .activation
                .as_ref()
                .is_some_and(|held| !held.original().is_closed())
        })
    }
    pub(crate) fn require_existing_voice_call(&self, grant: &CallGrant) -> Result<(), VoiceError> {
        if grant.lease().is_admitted() || self.has_admitted_voice_activation() {
            Err(VoiceError::Unauthorized)
        } else {
            Ok(())
        }
    }
}
