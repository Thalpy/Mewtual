//! Separate response-key custody. Merely signing a Welcome never permits its
//! release: Root must cover its intent before entry and the whole response /
//! installed transition before final publication under actual native guards.
use super::*;
use crate::V2JoinResponseData;
use catcoms_replication::chat::join_bindings::{
    JoinPayloadKindData, SignedWelcomeBindingData, WelcomeBindingSpecData,
};

pub(super) struct PreparedWelcome {
    spec: WelcomeBindingSpecData,
    bytes: Vec<u8>,
    entered: bool,
    raw: Option<[u8; 64]>,
    signed: Option<SignedWelcomeBindingData>,
}

impl<T, R> PreparedV2FounderGovernanceNode<T, R> {
    /// Freeze the exact validated response preimage OUTSIDE final writers from
    /// only this operation's immutable request, actual SDK outputs and signed
    /// materialized Header/Admission. No caller-supplied WelcomeSpec is accepted.
    /// Root MUST durably cover the returned intent before sign_welcome_native.
    pub fn prepare_welcome_data(&mut self) -> Result<(), SyncError> {
        self.pending
            .as_mut()
            .ok_or_else(unavailable)?
            .prepare_welcome()
    }
}

impl PendingAdd {
    pub(super) fn prepare_welcome(&mut self) -> Result<(), SyncError> {
        let p = self;
        if p.welcome.is_some() {
            return Ok(()); // cached historical DATA, including after closure
        }
        if p.phase != Phase::Materialized {
            return Err(unavailable());
        }
        let header = p.header.as_ref().ok_or_else(unavailable)?;
        let body = p.body.as_ref().ok_or_else(unavailable)?;
        let result = p
            .cell
            .as_ref()
            .ok_or_else(unavailable)?
            .completed_result_data()
            .map_err(|_| unavailable())?;
        let spec = WelcomeBindingSpecData {
            invite: p.request.spec.invite,
            anchor: p.request.spec.anchor,
            request_binding: p.request.request_id,
            key_package_wire: p.request.spec.key_package_wire,
            key_package_reference: p.request.spec.key_package_reference,
            admitter: header.spec().issuer,
            add_header: *header.id().as_bytes(),
            admission: *body.admission_id().as_bytes(),
            welcome: JoinPayloadKindData::Welcome
                .commitment_data(result.welcome_data())
                .map_err(|_| unavailable())?,
            policy: p.request.policy,
            public_routing_plan: p.request.public_routing_plan,
            sealed_routing_transfer: JoinPayloadKindData::RoutingTransfer
                .commitment_data(
                    result
                        .sealed_routing_transfer_data()
                        .map_err(|_| unavailable())?,
                )
                .map_err(|_| unavailable())?,
        };
        let bytes = spec.signing_bytes_data().map_err(|_| unavailable())?;
        p.welcome = Some(PreparedWelcome {
            spec,
            bytes,
            entered: false,
            raw: None,
            signed: None,
        });
        Ok(())
    }
}

impl<T, R> PreparedV2FounderGovernanceNode<T, R> {
    /// Exact bounded unsigned DATA for Root's covering WelcomeIntent. This
    /// contains no private routing plaintext, source/key or positive save proof.
    pub fn unsigned_welcome_data(&self) -> Result<(&WelcomeBindingSpecData, &[u8]), SyncError> {
        self.pending
            .as_ref()
            .ok_or_else(unavailable)?
            .unsigned_welcome()
    }

    /// Native key tail with no codec, replay or SDK-graph work. The bounded
    /// signer still needs its independently invoiced signature/error scratch.
    /// Root supplies genuine covering intent/current/source admission. Cache raw64 before any
    /// postverification or late negative, so a lost reply cannot repeat signing.
    pub fn sign_welcome_native(&mut self) -> Result<(), SyncError> {
        self.signing_parts_data()?.1.sign_welcome_native()
    }

    /// Borrow the exact cached key result for Root's RawWelcome record before
    /// fallible materialization. Source or native closure cannot erase it.
    pub fn raw_welcome_signature_data(&self) -> Result<&[u8; 64], SyncError> {
        self.pending.as_ref().ok_or_else(unavailable)?.raw_welcome()
    }

    /// Outside writers, reconstruct and verify using ONLY retained raw64.
    /// Even an error/unwind leaves that raw and the spent key entry owned.
    pub fn materialize_welcome_data(&mut self) -> Result<(), SyncError> {
        self.pending
            .as_mut()
            .ok_or_else(unavailable)?
            .materialize_welcome()
    }
}

impl PendingAdd {
    pub(super) fn unsigned_welcome(&self) -> Result<(&WelcomeBindingSpecData, &[u8]), SyncError> {
        let welcome = self.welcome.as_ref().ok_or_else(unavailable)?;
        Ok((&welcome.spec, &welcome.bytes))
    }

    pub(super) fn raw_welcome(&self) -> Result<&[u8; 64], SyncError> {
        self.welcome
            .as_ref()
            .and_then(|welcome| welcome.raw.as_ref())
            .ok_or_else(unavailable)
    }

    pub(super) fn materialize_welcome(&mut self) -> Result<(), SyncError> {
        let w = self.welcome.as_mut().ok_or_else(unavailable)?;
        if w.signed.is_some() {
            return Ok(());
        }
        let raw = w.raw.ok_or_else(unavailable)?;
        let signed = SignedWelcomeBindingData::sign_using(w.spec.clone(), |bytes| {
            if bytes == w.bytes {
                Ok(raw)
            } else {
                Err(catcoms_replication::chat::ChatError::Dependency)
            }
        })
        .map_err(|_| unavailable())?;
        w.signed = Some(signed);
        Ok(())
    }
}

impl<T, R> PreparedV2FounderGovernanceNode<T, R> {
    /// Cached historical response binding only. Root independently controls
    /// release of this and the exact Welcome/routing payloads after final save.
    pub fn signed_welcome_data(&self) -> Result<&SignedWelcomeBindingData, SyncError> {
        self.pending
            .as_ref()
            .and_then(|p| p.welcome.as_ref())
            .and_then(|w| w.signed.as_ref())
            .ok_or_else(unavailable)
    }

    /// Exact response components from this pending Add's materialized binding
    /// and original SDK completion. Root may seal these for the covering
    /// transaction. Any publication still requires a separately selected head.
    pub fn join_response_data(&self) -> Result<V2JoinResponseData<'_>, SyncError> {
        self.pending
            .as_ref()
            .ok_or_else(unavailable)?
            .join_response()
    }

    /// Simulate a post-key materializer fault without exposing a production
    /// mutable-spec API. Applying it twice restores the SAME original DATA.
    #[cfg(test)]
    pub(super) fn change_welcome_materialization_spec_for_test(&mut self) {
        self.pending
            .as_mut()
            .unwrap()
            .welcome
            .as_mut()
            .unwrap()
            .spec
            .sealed_routing_transfer[0] ^= 1;
    }
}

impl V2FounderAddSigning<'_> {
    pub fn sign_welcome_native(&mut self) -> Result<(), SyncError> {
        if self
            .pending
            .welcome
            .as_ref()
            .ok_or_else(unavailable)?
            .raw
            .is_some()
        {
            return Ok(());
        }
        self.negative.require_live_data()?;
        self.pending.sign_welcome_native(
            &self.seed.body.device,
            self.seed.body.context.spec_data().signer,
        )
    }
}

impl PendingAdd {
    pub(super) fn join_response(&self) -> Result<V2JoinResponseData<'_>, SyncError> {
        let binding = self
            .welcome
            .as_ref()
            .and_then(|welcome| welcome.signed.as_ref())
            .ok_or_else(unavailable)?;
        let result = self
            .cell
            .as_ref()
            .ok_or_else(unavailable)?
            .completed_result_data()
            .map_err(|_| unavailable())?;
        Ok(V2JoinResponseData {
            binding,
            welcome: result.welcome_data(),
            sealed_routing: result
                .sealed_routing_transfer_data()
                .map_err(|_| unavailable())?,
        })
    }

    pub(super) fn sign_welcome_native(
        &mut self,
        device: &MlsDevice,
        signer: [u8; 32],
    ) -> Result<(), SyncError> {
        let p = self;
        let w = p.welcome.as_mut().ok_or_else(unavailable)?;
        if w.raw.is_some() {
            return Ok(());
        }
        if w.entered || w.spec.admitter != signer || !device.retained_ed25519_key_matches(&signer) {
            return Err(unavailable());
        }
        p.welcome_native.enter(
            p.welcome_cut.as_mut().ok_or_else(unavailable)?,
            &p.original,
            NativeChatKeyPurpose::JoinWelcome,
            &mut p.ends,
        )?;
        w.entered = true;
        #[cfg(test)]
        test_cut(TestCut::WelcomeEntered);
        let raw = device
            .sign_retained_ed25519_bounded(
                &signer,
                &w.bytes,
                catcoms_replication::chat::join_bindings::MAX_JOIN_BINDING_BYTES,
            )
            .map_err(|_| unavailable())?;
        w.raw = Some(raw);
        #[cfg(test)]
        test_cut(TestCut::WelcomeRawCached);
        Ok(())
    }
}

impl PreparedWelcome {
    pub(super) fn may_reauthorize_native_data(&self) -> bool {
        !self.entered && self.raw.is_none() && self.signed.is_none()
    }
}
