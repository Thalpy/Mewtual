//! Closed LOCAL founder debit/intent DATA. No decoder creates an SDK ticket, RAM scope plan,
//! physical saved result, source permission or Current. All actual file effects live in Store's
//! persistence sibling; only its private-field types may attest their real barriers.
use super::*;
use catcoms_replication::chat::{
    owner_facts::OwnerFactId,
    owner_header::{OwnerBodyId, OwnerHeaderId},
};
use catcoms_storage::private_chat_page::PrivateChatPageId;
use catcoms_sync::owner_v2::{
    BootstrapSourceContextData, BootstrapSourceContextSpec, BootstrapSourceId,
    FounderSnapshotLimits,
};

const DEBIT_TAG: &[u8] = b"catcoms/private-chat-founder-debit/v1\0";
const MARKER_TAG: &[u8] = b"catcoms/private-chat-founder-link/v1\0";
const INTENT_TAG: &[u8] = b"catcoms/private-chat-founder-intent/v1\0";
const SEED_TAG: &[u8] = b"catcoms/private-chat-founder-transition/v1\0";
const VERSION: u16 = 1;
pub(in crate::store) const MAX_DEBIT_PLAIN: usize = 512;
pub(in crate::store) const MAX_DEBIT_FRAME: usize = MAX_DEBIT_PLAIN + 40;
pub(in crate::store) const MAX_MARKER_PLAIN: usize = 1024;
pub(in crate::store) const MAX_TARGET_MARKER_FRAME: usize = MAX_MARKER_PLAIN + 40;
pub(in crate::store) const MAX_INTENT_PLAIN: usize = 1024;
pub(in crate::store) const MAX_INTENT_FRAME: usize = MAX_INTENT_PLAIN + 40;
pub(in crate::store) const MAX_TRANSITION_PLAIN: usize = 4096;
pub(in crate::store) const MAX_TRANSITION_FRAME: usize = MAX_TRANSITION_PLAIN + 40;
pub(in crate::store) const MAX_NAME: usize = 256;
pub(in crate::store) const PERSISTENT_CONTENT_BYTES: u64 = 17_119_058;
pub(in crate::store) const CONTROL_FEE: usize = 256 * 1024;
pub(in crate::store) const MAX_CONTEXT_BYTES: usize = 692;
const _: () = assert!(488 <= MAX_DEBIT_PLAIN && 600 <= MAX_MARKER_PLAIN);
const _: () = assert!(239 + 1 + 36 + 36 + 4 + MAX_CONTEXT_BYTES == 1008);
const _: () = assert!(1008 <= MAX_INTENT_PLAIN);
const _: () = assert!(4 + SEED_TAG.len() + 2 + 4 * 36 + 37 + 2 * (4 + MAX_INTENT_FRAME) == 2366);
const _: () = assert!(2366 <= MAX_TRANSITION_PLAIN);
const _: () = assert!(16 * MAX_TRANSITION_FRAME + 128 * 1024 <= CONTROL_FEE);

macro_rules! record_id {
    ($name:ident, $domain:literal) => {
        /// COMPLETE original sealed local record label; wrapping DATA authenticates nothing.
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub(in crate::store) struct $name([u8; 32]);
        impl $name {
            pub(in crate::store) fn from_bytes(value: [u8; 32]) -> Self {
                Self(value)
            }
            pub(in crate::store) fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
            pub(in crate::store) fn of(frame: &[u8]) -> Self {
                let mut hash = blake3::Hasher::new();
                hash.update($domain);
                hash.update(frame);
                Self(*hash.finalize().as_bytes())
            }
        }
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($name)).finish_non_exhaustive()
            }
        }
    };
}
record_id!(DebitId, b"catcoms/private-chat-founder-debit-id/v1\0");
record_id!(TargetMarkerId, b"catcoms/private-chat-founder-link-id/v1\0");
record_id!(IntentId, b"catcoms/private-chat-founder-intent-id/v1\0");

/// DATA only. Actual expected root comes from persistence's original physical capture.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct InvocationData {
    pub(in crate::store) vault: [u8; 32],
    pub(in crate::store) root: [u8; 32],
    pub(in crate::store) invocation: [u8; 32],
}
impl std::fmt::Debug for InvocationData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FounderInvocationData")
            .finish_non_exhaustive()
    }
}
impl InvocationData {
    /// Fixed path COMPONENT only, including full original root/vault/invocation. The physical
    /// writer still derives its own fixed parent; this is not a caller filename or path API.
    pub(in crate::store) fn name(&self) -> [u8; 64] {
        let mut h = blake3::Hasher::new();
        h.update(b"catcoms/private-chat-founder-invocation/v1\0");
        h.update(&self.vault);
        h.update(&self.root);
        h.update(&self.invocation);
        let mut name = [0; 64];
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for (i, b) in h.finalize().as_bytes().iter().copied().enumerate() {
            name[2 * i] = HEX[(b >> 4) as usize];
            name[2 * i + 1] = HEX[(b & 15) as usize];
        }
        name
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::store) struct DebitData {
    pub(in crate::store) invocation: InvocationData,
    pub(in crate::store) signer: [u8; 32],
    pub(in crate::store) name: String,
    pub(in crate::store) limits: FounderSnapshotLimits,
    pub(in crate::store) io_allowance: u32,
}
impl std::fmt::Debug for DebitData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FounderDebitData").finish_non_exhaustive()
    }
}
impl DebitData {
    pub(in crate::store) fn validate(&self) -> Result<(), AppError> {
        if self.name.is_empty()
            || self.name.len() > MAX_NAME
            || !(1..=8).contains(&self.io_allowance)
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn request_binding(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        h.update(b"catcoms/private-chat-founder-request/v1\0");
        h.update(&self.signer);
        h.update(&(self.name.len() as u32).to_be_bytes());
        h.update(self.name.as_bytes());
        for n in [
            self.limits.max_snapshot_bytes(),
            self.limits.max_mls_bytes(),
            self.limits.max_provider_backing_bytes(),
        ] {
            h.update(&(n as u64).to_be_bytes());
        }
        h.update(&[2, 1, 1]); // supported V2 founding kind + one granted/spent SDK unit
        h.update(&self.io_allowance.to_be_bytes());
        h.update(&PERSISTENT_CONTENT_BYTES.to_be_bytes());
        h.update(&FIXED_FILES.to_be_bytes());
        *h.finalize().as_bytes()
    }
    pub(in crate::store) fn encode(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.validate()?;
        let mut e = Encoder::with_capacity(MAX_DEBIT_PLAIN);
        tag(&mut e, DEBIT_TAG)?;
        put_invocation(&mut e, self.invocation)?;
        put(&mut e, &self.signer)?;
        put(&mut e, self.name.as_bytes())?;
        put_limits(&mut e, self.limits);
        e.put_u8(1).put_u8(1).put_u32(self.io_allowance);
        e.put_u64(PERSISTENT_CONTENT_BYTES).put_u16(FIXED_FILES);
        bounded(e, MAX_DEBIT_PLAIN)
    }
    pub(in crate::store) fn decode(raw: &[u8], expected: InvocationData) -> Result<Self, AppError> {
        let mut d = input(raw, MAX_DEBIT_PLAIN, DEBIT_TAG)?;
        let invocation = get_invocation(&mut d)?;
        let signer = digest(&mut d)?;
        let name = field(&mut d, MAX_NAME)?;
        if name.is_empty() {
            return Err(refused());
        }
        let name = std::str::from_utf8(name).map_err(|_| refused())?;
        let limits = get_limits(&mut d)?;
        if d.get_u8().map_err(|_| refused())? != 1 || d.get_u8().map_err(|_| refused())? != 1 {
            return Err(refused());
        }
        let io_allowance = d.get_u32().map_err(|_| refused())?;
        if !(1..=8).contains(&io_allowance)
            || d.get_u64().map_err(|_| refused())? != PERSISTENT_CONTENT_BYTES
            || d.get_u16().map_err(|_| refused())? != FIXED_FILES
        {
            return Err(refused());
        }
        d.finish().map_err(|_| refused())?;
        if invocation != expected {
            return Err(refused());
        }
        // COMPLETE framing/limits/equalities precede this only variable owned copy.
        Ok(Self {
            invocation,
            signer,
            name: name.to_owned(),
            limits,
            io_allowance,
        })
    }
}

/// Exact unexposed target/planned facts, never a Fresh observation or live reservation.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::store) struct FoundingData {
    pub(in crate::store) group: Vec<u8>,
    pub(in crate::store) signer: [u8; 32],
    pub(in crate::store) fresh_attempt: [u8; 32],
    pub(in crate::store) scope_domain: [u8; 32],
    pub(in crate::store) nonce: [u8; 32],
    pub(in crate::store) revision: u64,
}
impl std::fmt::Debug for FoundingData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FounderPlannedData").finish_non_exhaustive()
    }
}
impl FoundingData {
    pub(in crate::store) fn validate(&self, debit: &DebitData) -> Result<(), AppError> {
        if self.group.is_empty()
            || self.group.len() > 256
            || self.signer != debit.signer
            || self.scope_domain != debit.invocation.vault
            || self.revision != 1
        {
            return Err(refused());
        }
        Ok(())
    }
    fn matches_context(&self, context: &BootstrapSourceContextData) -> bool {
        let c = context.spec_data();
        c.group == self.group
            && c.signer == self.signer
            && c.fresh_attempt == self.fresh_attempt
            && c.scope_domain == self.scope_domain
            && c.scope_nonce == self.nonce
            && c.planned_revision == self.revision
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::store) struct TargetMarkerData {
    pub(in crate::store) invocation: InvocationData,
    pub(in crate::store) debit: DebitId,
    pub(in crate::store) founding: FoundingData,
}
impl std::fmt::Debug for TargetMarkerData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FounderTargetMarkerData")
            .finish_non_exhaustive()
    }
}
impl TargetMarkerData {
    pub(in crate::store) fn encode(
        &self,
        debit: &DebitData,
        id: DebitId,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.validate(debit, id)?;
        let mut e = Encoder::with_capacity(MAX_MARKER_PLAIN);
        tag(&mut e, MARKER_TAG)?;
        put_invocation(&mut e, self.invocation)?;
        put(&mut e, self.debit.as_bytes())?;
        put_founding(&mut e, &self.founding)?;
        bounded(e, MAX_MARKER_PLAIN)
    }
    pub(in crate::store) fn validate(
        &self,
        debit: &DebitData,
        id: DebitId,
    ) -> Result<(), AppError> {
        debit.validate()?;
        self.founding.validate(debit)?;
        if self.invocation != debit.invocation || self.debit != id {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn decode(
        raw: &[u8],
        debit: &DebitData,
        id: DebitId,
    ) -> Result<Self, AppError> {
        let mut d = input(raw, MAX_MARKER_PLAIN, MARKER_TAG)?;
        let invocation = get_invocation(&mut d)?;
        let debit_id = DebitId::from_bytes(digest(&mut d)?);
        let founding = get_founding(&mut d)?;
        d.finish().map_err(|_| refused())?;
        if invocation != debit.invocation || debit_id != id {
            return Err(refused());
        }
        founding.validate(debit)?;
        Ok(Self {
            invocation,
            debit: debit_id,
            founding: founding.owned(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum CloseReason {
    SdkFailed,
    CreatorLost,
    ExplicitClose,
}
#[derive(Clone, PartialEq, Eq)]
pub(in crate::store) enum IntentKind {
    Founding {
        facts: FoundingData,
        marker: TargetMarkerId,
    },
    SourcePrepared {
        context: BootstrapSourceContextData,
        source: BootstrapSourceId,
        marker: TargetMarkerId,
    },
    /// Historical producer-outcome DATA, not a reconstructed runtime SavedFiles/arm. Only
    /// the actual completed writer's separately retained closure may publish this terminal.
    SourceSaved {
        context: BootstrapSourceContextData,
        source: BootstrapSourceId,
        marker: TargetMarkerId,
    },
    ClosedDebit(CloseReason),
    ClosedFounding {
        facts: FoundingData,
        marker: TargetMarkerId,
        reason: CloseReason,
    },
}
#[derive(Clone, PartialEq, Eq)]
pub(in crate::store) struct IntentData {
    pub(in crate::store) invocation: InvocationData,
    pub(in crate::store) debit: DebitId,
    pub(in crate::store) sequence: u64,
    pub(in crate::store) parent: Option<IntentId>,
    pub(in crate::store) io_spent: u32,
    pub(in crate::store) kind: IntentKind,
}
impl std::fmt::Debug for IntentData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FounderIntentData").finish_non_exhaustive()
    }
}
impl IntentData {
    pub(in crate::store) fn decode(
        raw: &[u8],
        debit: &DebitData,
        id: DebitId,
    ) -> Result<Self, AppError> {
        enum Payload<'a> {
            Founding(BorrowedFounding<'a>, TargetMarkerId),
            Source(
                BootstrapSourceContextSpec<'a>,
                BootstrapSourceId,
                TargetMarkerId,
            ),
            SourceSaved(
                BootstrapSourceContextSpec<'a>,
                BootstrapSourceId,
                TargetMarkerId,
            ),
            Closed(CloseReason),
            ClosedFounding(BorrowedFounding<'a>, TargetMarkerId, CloseReason),
        }
        let mut d = input(raw, MAX_INTENT_PLAIN, INTENT_TAG)?;
        let invocation = get_invocation(&mut d)?;
        let debit_id = DebitId::from_bytes(digest(&mut d)?);
        let sequence = d.get_u64().map_err(|_| refused())?;
        let parent = get_parent(&mut d)?;
        let kind = d.get_u8().map_err(|_| refused())?;
        let io_spent = d.get_u32().map_err(|_| refused())?;
        let payload = match kind {
            1 => Payload::Founding(
                get_founding(&mut d)?,
                TargetMarkerId::from_bytes(digest(&mut d)?),
            ),
            2 => {
                if d.get_u8().map_err(|_| refused())? != 0 {
                    return Err(refused());
                }
                let marker = TargetMarkerId::from_bytes(digest(&mut d)?);
                let source = BootstrapSourceId::from_bytes(digest(&mut d)?);
                Payload::Source(
                    decode_context(field(&mut d, MAX_CONTEXT_BYTES)?)?,
                    source,
                    marker,
                )
            }
            3 => Payload::Closed(get_reason(&mut d)?),
            4 => Payload::ClosedFounding(
                get_founding(&mut d)?,
                TargetMarkerId::from_bytes(digest(&mut d)?),
                get_reason(&mut d)?,
            ),
            5 => {
                // Identical bounded payload to SourcePrepared; the closed tag and counters,
                // not any extra caller status/OS identity, distinguish the terminal record.
                if d.get_u8().map_err(|_| refused())? != 0 {
                    return Err(refused());
                }
                let marker = TargetMarkerId::from_bytes(digest(&mut d)?);
                let source = BootstrapSourceId::from_bytes(digest(&mut d)?);
                Payload::SourceSaved(
                    decode_context(field(&mut d, MAX_CONTEXT_BYTES)?)?,
                    source,
                    marker,
                )
            }
            _ => return Err(refused()),
        };
        d.finish().map_err(|_| refused())?;
        if invocation != debit.invocation || debit_id != id || io_spent > debit.io_allowance {
            return Err(refused());
        }
        // All variable framing/counts/limits and EOF precede the bounded group copies below.
        let kind = match payload {
            Payload::Founding(facts, marker) => {
                facts.validate(debit)?;
                IntentKind::Founding {
                    facts: facts.owned(),
                    marker,
                }
            }
            Payload::Source(spec, source, marker) => IntentKind::SourcePrepared {
                context: BootstrapSourceContextData::from_data(spec).map_err(|_| refused())?,
                source,
                marker,
            },
            Payload::SourceSaved(spec, source, marker) => IntentKind::SourceSaved {
                context: BootstrapSourceContextData::from_data(spec).map_err(|_| refused())?,
                source,
                marker,
            },
            Payload::Closed(reason) => IntentKind::ClosedDebit(reason),
            Payload::ClosedFounding(facts, marker, reason) => {
                facts.validate(debit)?;
                IntentKind::ClosedFounding {
                    facts: facts.owned(),
                    marker,
                    reason,
                }
            }
        };
        let data = Self {
            invocation,
            debit: debit_id,
            sequence,
            parent,
            io_spent,
            kind,
        };
        data.validate(debit, id)?;
        Ok(data)
    }
    pub(in crate::store) fn validate(
        &self,
        debit: &DebitData,
        id: DebitId,
    ) -> Result<(), AppError> {
        debit.validate()?;
        if self.invocation != debit.invocation
            || self.debit != id
            || self.io_spent > debit.io_allowance
        {
            return Err(refused());
        }
        match &self.kind {
            IntentKind::Founding { facts, .. } => {
                facts.validate(debit)?;
                if self.sequence != 1 || self.parent.is_some() || self.io_spent != 0 {
                    return Err(refused());
                }
            }
            IntentKind::SourcePrepared { context, .. }
            | IntentKind::SourceSaved { context, .. } => {
                let c = context.spec_data();
                let terminal = matches!(&self.kind, IntentKind::SourceSaved { .. });
                let base: u64 = if terminal { 3 } else { 2 };
                if c.vault != debit.invocation.vault
                    || c.signer != debit.signer
                    || c.limits != debit.limits
                    || c.planned_revision != 1
                    || (terminal && self.io_spent == 0)
                    || self.sequence
                        != base
                            .checked_add(u64::from(self.io_spent))
                            .ok_or_else(refused)?
                    || self.parent.is_none()
                {
                    return Err(refused());
                }
            }
            IntentKind::ClosedDebit(_) => {
                if self.sequence != 1 || self.parent.is_some() || self.io_spent != 0 {
                    return Err(refused());
                }
            }
            IntentKind::ClosedFounding { facts, .. } => {
                facts.validate(debit)?;
                if self.sequence != 2 || self.parent.is_none() || self.io_spent != 0 {
                    return Err(refused());
                }
            }
        }
        Ok(())
    }
    pub(in crate::store) fn encode(
        &self,
        debit: &DebitData,
        id: DebitId,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.validate(debit, id)?;
        let mut e = Encoder::with_capacity(MAX_INTENT_PLAIN);
        tag(&mut e, INTENT_TAG)?;
        put_invocation(&mut e, self.invocation)?;
        put(&mut e, self.debit.as_bytes())?;
        e.put_u64(self.sequence);
        put_parent(&mut e, self.parent)?;
        e.put_u8(match &self.kind {
            IntentKind::Founding { .. } => 1,
            IntentKind::SourcePrepared { .. } => 2,
            IntentKind::ClosedDebit(_) => 3,
            IntentKind::ClosedFounding { .. } => 4,
            IntentKind::SourceSaved { .. } => 5,
        });
        e.put_u32(self.io_spent);
        match &self.kind {
            IntentKind::Founding { facts, marker } => {
                put_founding(&mut e, facts)?;
                put(&mut e, marker.as_bytes())?;
            }
            IntentKind::SourcePrepared {
                context,
                source,
                marker,
            }
            | IntentKind::SourceSaved {
                context,
                source,
                marker,
            } => {
                e.put_u8(0);
                put(&mut e, marker.as_bytes())?;
                put(&mut e, source.as_bytes())?;
                let context = encode_context(context)?;
                put(&mut e, &context)?;
            }
            IntentKind::ClosedDebit(reason) => {
                e.put_u8(reason_tag(*reason));
            }
            IntentKind::ClosedFounding {
                facts,
                marker,
                reason,
            } => {
                put_founding(&mut e, facts)?;
                put(&mut e, marker.as_bytes())?;
                e.put_u8(reason_tag(*reason));
            }
        }
        bounded(e, MAX_INTENT_PLAIN)
    }
    /// Exact candidate-parent DATA consistency. Physical selected-current/debt/root/counters
    /// and original source still govern the effect; this helper does not manufacture an arm.
    pub(in crate::store) fn successor(
        &self,
        old: Option<(&Self, IntentId)>,
        debit: &DebitData,
        id: DebitId,
    ) -> Result<(), AppError> {
        self.validate(debit, id)?;
        let Some((old, old_id)) = old else {
            return match &self.kind {
                IntentKind::Founding { .. } | IntentKind::ClosedDebit(_) => Ok(()),
                _ => Err(refused()),
            };
        };
        old.validate(debit, id)?;
        if self.parent != Some(old_id)
            || self.sequence != old.sequence.checked_add(1).ok_or_else(refused)?
        {
            return Err(refused());
        }
        match (&old.kind, &self.kind) {
            (
                IntentKind::Founding { facts, marker },
                IntentKind::SourcePrepared {
                    context,
                    marker: next,
                    ..
                },
            ) if marker == next && facts.matches_context(context) && self.io_spent == 0 => Ok(()),
            (
                IntentKind::Founding { facts, marker },
                IntentKind::ClosedFounding {
                    facts: next,
                    marker: next_marker,
                    ..
                },
            ) if facts == next && marker == next_marker => Ok(()),
            (
                IntentKind::SourcePrepared {
                    context,
                    source,
                    marker,
                },
                IntentKind::SourcePrepared {
                    context: next,
                    source: next_source,
                    marker: next_marker,
                },
            ) if context == next
                && source == next_source
                && marker == next_marker
                && self.io_spent == old.io_spent.checked_add(1).ok_or_else(refused)? =>
            {
                Ok(())
            }
            (
                IntentKind::SourcePrepared {
                    context,
                    source,
                    marker,
                },
                IntentKind::SourceSaved {
                    context: next,
                    source: next_source,
                    marker: next_marker,
                },
            ) if context == next
                && source == next_source
                && marker == next_marker
                && self.io_spent == old.io_spent =>
            {
                // Pure DATA transition only. The physical writer must additionally retain
                // the SAME actual completed run+saved arm BEFORE replacing the top journal.
                // No SourceSaved outgoing transition exists, even with unused I/O allowance.
                Ok(())
            }
            _ => Err(refused()),
        }
    }
    pub(in crate::store) fn matches_marker(
        &self,
        marker: &TargetMarkerData,
        id: TargetMarkerId,
    ) -> Result<(), AppError> {
        if self.invocation != marker.invocation || self.debit != marker.debit {
            return Err(refused());
        }
        match &self.kind {
            IntentKind::Founding {
                facts,
                marker: expected,
            }
            | IntentKind::ClosedFounding {
                facts,
                marker: expected,
                ..
            } if facts == &marker.founding && *expected == id => Ok(()),
            IntentKind::SourcePrepared {
                context,
                marker: expected,
                ..
            }
            | IntentKind::SourceSaved {
                context,
                marker: expected,
                ..
            } if *expected == id && marker.founding.matches_context(context) => Ok(()),
            _ => Err(refused()),
        }
    }
}

fn tag(e: &mut Encoder, tag: &[u8]) -> Result<(), AppError> {
    put(e, tag)?;
    e.put_u16(VERSION);
    Ok(())
}
fn put(e: &mut Encoder, bytes: &[u8]) -> Result<(), AppError> {
    e.put_bytes(bytes).map_err(|_| refused())?;
    Ok(())
}
fn bounded(e: Encoder, max: usize) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if e.len() > max {
        return Err(refused());
    }
    Ok(Zeroizing::new(e.finish()))
}
fn field<'a>(d: &mut Decoder<'a>, max: usize) -> Result<&'a [u8], AppError> {
    let value = d.get_bytes().map_err(|_| refused())?;
    if value.len() > max {
        return Err(refused());
    }
    Ok(value)
}
fn digest(d: &mut Decoder<'_>) -> Result<[u8; 32], AppError> {
    field(d, 32)?.try_into().map_err(|_| refused())
}
fn input<'a>(raw: &'a [u8], max: usize, expected: &[u8]) -> Result<Decoder<'a>, AppError> {
    if raw.is_empty() || raw.len() > max {
        return Err(refused());
    }
    let mut d = Decoder::new(raw);
    if field(&mut d, expected.len())? != expected || d.get_u16().map_err(|_| refused())? != VERSION
    {
        return Err(refused());
    }
    Ok(d)
}
fn put_invocation(e: &mut Encoder, c: InvocationData) -> Result<(), AppError> {
    put(e, &c.vault)?;
    put(e, &c.root)?;
    put(e, &c.invocation)
}
fn get_invocation(d: &mut Decoder<'_>) -> Result<InvocationData, AppError> {
    Ok(InvocationData {
        vault: digest(d)?,
        root: digest(d)?,
        invocation: digest(d)?,
    })
}
fn put_limits(e: &mut Encoder, limits: FounderSnapshotLimits) {
    for n in [
        limits.max_snapshot_bytes(),
        limits.max_mls_bytes(),
        limits.max_provider_backing_bytes(),
    ] {
        e.put_u64(n as u64);
    }
}
fn get_limits(d: &mut Decoder<'_>) -> Result<FounderSnapshotLimits, AppError> {
    let mut limits = [0usize; 3];
    for n in &mut limits {
        *n = usize::try_from(d.get_u64().map_err(|_| refused())?).map_err(|_| refused())?;
    }
    FounderSnapshotLimits::new(limits[0], limits[1], limits[2]).map_err(|_| refused())
}
fn put_founding(e: &mut Encoder, f: &FoundingData) -> Result<(), AppError> {
    for b in [
        f.group.as_slice(),
        &f.signer,
        &f.fresh_attempt,
        &f.scope_domain,
        &f.nonce,
    ] {
        put(e, b)?;
    }
    e.put_u64(f.revision).put_u8(0);
    Ok(())
}
struct BorrowedFounding<'a> {
    group: &'a [u8],
    signer: [u8; 32],
    fresh_attempt: [u8; 32],
    scope_domain: [u8; 32],
    nonce: [u8; 32],
    revision: u64,
}
impl BorrowedFounding<'_> {
    fn validate(&self, debit: &DebitData) -> Result<(), AppError> {
        if self.group.is_empty()
            || self.group.len() > 256
            || self.signer != debit.signer
            || self.scope_domain != debit.invocation.vault
            || self.revision != 1
        {
            return Err(refused());
        }
        Ok(())
    }
    fn owned(self) -> FoundingData {
        FoundingData {
            group: self.group.to_vec(),
            signer: self.signer,
            fresh_attempt: self.fresh_attempt,
            scope_domain: self.scope_domain,
            nonce: self.nonce,
            revision: self.revision,
        }
    }
}
fn get_founding<'a>(d: &mut Decoder<'a>) -> Result<BorrowedFounding<'a>, AppError> {
    let group = field(d, 256)?;
    if group.is_empty() {
        return Err(refused());
    }
    let signer = digest(d)?;
    let fresh_attempt = digest(d)?;
    let scope_domain = digest(d)?;
    let nonce = digest(d)?;
    let revision = d.get_u64().map_err(|_| refused())?;
    if revision != 1 || d.get_u8().map_err(|_| refused())? != 0 {
        return Err(refused());
    }
    Ok(BorrowedFounding {
        group,
        signer,
        fresh_attempt,
        scope_domain,
        nonce,
        revision,
    })
}
fn put_parent(e: &mut Encoder, parent: Option<IntentId>) -> Result<(), AppError> {
    match parent {
        None => {
            e.put_u8(0);
        }
        Some(id) => {
            e.put_u8(1);
            put(e, id.as_bytes())?;
        }
    }
    Ok(())
}
fn get_parent(d: &mut Decoder<'_>) -> Result<Option<IntentId>, AppError> {
    match d.get_u8().map_err(|_| refused())? {
        0 => Ok(None),
        1 => Ok(Some(IntentId::from_bytes(digest(d)?))),
        _ => Err(refused()),
    }
}
fn reason_tag(r: CloseReason) -> u8 {
    match r {
        CloseReason::SdkFailed => 1,
        CloseReason::CreatorLost => 2,
        CloseReason::ExplicitClose => 3,
    }
}
fn get_reason(d: &mut Decoder<'_>) -> Result<CloseReason, AppError> {
    match d.get_u8().map_err(|_| refused())? {
        1 => Ok(CloseReason::SdkFailed),
        2 => Ok(CloseReason::CreatorLost),
        3 => Ok(CloseReason::ExplicitClose),
        _ => Err(refused()),
    }
}

fn encode_context(context: &BootstrapSourceContextData) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let c = context.spec_data();
    let mut e = Encoder::with_capacity(MAX_CONTEXT_BYTES);
    for b in [
        &c.vault[..],
        c.group,
        &c.signer,
        &c.fresh_attempt,
        &c.scope_domain,
        &c.scope_nonce,
    ] {
        put(&mut e, b)?;
    }
    e.put_u64(c.planned_revision);
    for b in [
        c.profile.as_bytes(),
        c.admission.as_bytes(),
        c.header.as_bytes(),
        c.records.as_bytes(),
        c.manifest.as_bytes(),
    ] {
        put(&mut e, b)?;
    }
    put_limits(&mut e, c.limits);
    put(&mut e, &c.snapshot_digest)?;
    e.put_u32(c.records_plaintext_bytes as u32);
    bounded(e, MAX_CONTEXT_BYTES)
}
fn decode_context(raw: &[u8]) -> Result<BootstrapSourceContextSpec<'_>, AppError> {
    if raw.len() > MAX_CONTEXT_BYTES {
        return Err(refused());
    }
    let mut d = Decoder::new(raw);
    let vault = digest(&mut d)?;
    let group = field(&mut d, 256)?;
    let signer = digest(&mut d)?;
    let fresh_attempt = digest(&mut d)?;
    let scope_domain = digest(&mut d)?;
    let scope_nonce = digest(&mut d)?;
    let planned_revision = d.get_u64().map_err(|_| refused())?;
    let profile = OwnerFactId::from_bytes(digest(&mut d)?);
    let admission = OwnerBodyId::from_bytes(digest(&mut d)?);
    let header = OwnerHeaderId::from_bytes(digest(&mut d)?);
    let records = PrivateChatPageId::from_bytes(digest(&mut d)?);
    let manifest = PrivateChatPageId::from_bytes(digest(&mut d)?);
    let limits = get_limits(&mut d)?;
    let snapshot_digest = digest(&mut d)?;
    let records_plaintext_bytes = d.get_u32().map_err(|_| refused())? as usize;
    d.finish().map_err(|_| refused())?;
    if group.is_empty()
        || scope_domain != vault
        || planned_revision != 1
        || records_plaintext_bytes == 0
        || records_plaintext_bytes > 65_536
    {
        return Err(refused());
    }
    Ok(BootstrapSourceContextSpec {
        vault,
        group,
        signer,
        fresh_attempt,
        scope_domain,
        scope_nonce,
        planned_revision,
        profile,
        admission,
        header,
        records,
        manifest,
        limits,
        snapshot_digest,
        records_plaintext_bytes,
    })
}

// The outer record is existing nonce24+ciphertext/tag framing, under a DISTINCT closed key
// per record kind. Hard bounds precede unframe's cipher copy and AEAD output allocation. Each
// caller still pays original raw + copied cipher + plaintext + any retained candidate copies.
fn seal_small(
    key: &[u8; 32],
    domain: &str,
    plain: &[u8],
    max: usize,
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if plain.is_empty() || plain.len() > max {
        return Err(refused());
    }
    let key = Zeroizing::new(blake3::derive_key(domain, key));
    Ok(Zeroizing::new(frame(&seal(&key, plain, rng)?)))
}
fn open_small(
    key: &[u8; 32],
    domain: &str,
    raw: &[u8],
    max: usize,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if raw.len() <= 40 || raw.len() > max + 40 {
        return Err(refused());
    }
    let key = Zeroizing::new(blake3::derive_key(domain, key));
    let plain = Zeroizing::new(unseal(&key, &unframe(raw)?)?);
    if plain.is_empty() || plain.len() > max {
        return Err(refused());
    }
    Ok(plain)
}
pub(in crate::store) fn seal_debit_data(
    key: &[u8; 32],
    data: &DebitData,
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let plain = data.encode()?;
    seal_small(
        key,
        "catcoms/private-chat-founder-debit-key/v1",
        &plain,
        MAX_DEBIT_PLAIN,
        rng,
    )
}
pub(in crate::store) fn open_debit_data(
    key: &[u8; 32],
    expected: InvocationData,
    raw: &[u8],
) -> Result<DebitData, AppError> {
    let plain = open_small(
        key,
        "catcoms/private-chat-founder-debit-key/v1",
        raw,
        MAX_DEBIT_PLAIN,
    )?;
    DebitData::decode(&plain, expected)
}
pub(in crate::store) fn seal_target_marker_data(
    key: &[u8; 32],
    data: &TargetMarkerData,
    debit: &DebitData,
    id: DebitId,
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let plain = data.encode(debit, id)?;
    seal_small(
        key,
        "catcoms/private-chat-founder-link-key/v1",
        &plain,
        MAX_MARKER_PLAIN,
        rng,
    )
}
pub(in crate::store) fn open_target_marker_data(
    key: &[u8; 32],
    debit: &DebitData,
    id: DebitId,
    raw: &[u8],
) -> Result<TargetMarkerData, AppError> {
    let plain = open_small(
        key,
        "catcoms/private-chat-founder-link-key/v1",
        raw,
        MAX_MARKER_PLAIN,
    )?;
    TargetMarkerData::decode(&plain, debit, id)
}
/// Authenticated bounded LOCATOR DATA only, before the caller knows the flat debit name.
/// Complete framing/EOF and actual root/request context precede the returned labels. The
/// caller MUST then independently open that fixed debit and use the full marker decoder
/// above. This cannot prove saved work, source authority, Fresh, a read payment or Current.
pub(in crate::store) fn open_target_marker_locator_data(
    key: &[u8; 32],
    raw: &[u8],
    vault: [u8; 32],
    root: [u8; 32],
    group: &[u8],
    signer: [u8; 32],
) -> Result<(InvocationData, DebitId), AppError> {
    if group.is_empty() || group.len() > 256 {
        return Err(refused());
    }
    let plain = open_small(
        key,
        "catcoms/private-chat-founder-link-key/v1",
        raw,
        MAX_MARKER_PLAIN,
    )?;
    let mut d = input(&plain, MAX_MARKER_PLAIN, MARKER_TAG)?;
    let invocation = get_invocation(&mut d)?;
    let debit = DebitId::from_bytes(digest(&mut d)?);
    let founding = get_founding(&mut d)?;
    d.finish().map_err(|_| refused())?;
    if invocation.vault != vault
        || invocation.root != root
        || founding.group != group
        || founding.signer != signer
        || founding.scope_domain != vault
        || founding.revision != 1
    {
        return Err(refused());
    }
    Ok((invocation, debit))
}
pub(in crate::store) fn seal_intent_data(
    key: &[u8; 32],
    data: &IntentData,
    debit: &DebitData,
    id: DebitId,
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let plain = data.encode(debit, id)?;
    seal_small(
        key,
        "catcoms/private-chat-founder-intent-key/v1",
        &plain,
        MAX_INTENT_PLAIN,
        rng,
    )
}
pub(in crate::store) fn open_intent_data(
    key: &[u8; 32],
    debit: &DebitData,
    id: DebitId,
    raw: &[u8],
) -> Result<IntentData, AppError> {
    let plain = open_small(
        key,
        "catcoms/private-chat-founder-intent-key/v1",
        raw,
        MAX_INTENT_PLAIN,
    )?;
    IntentData::decode(&plain, debit, id)
}

/// Complete SMALL retry DATA, NOT a directory-adoption or SDK/arm capability. The physical
/// caller must open the ACTUAL selected parent/current+stage and independent complete target
/// marker/debit after every writer wait, and call candidate.successor(actual_parent,...) before
/// effects (or prove exact already-candidate repair). Matching a saved parent32 alone is NOT
/// that closed transition check. Empty/partial target may NEVER be adopted from this DATA.
pub(in crate::store) struct TransitionSeedData<'a> {
    pub(in crate::store) expected_parent: Option<IntentId>,
    pub(in crate::store) marker_frame: Option<&'a [u8]>,
    pub(in crate::store) marker: Option<TargetMarkerData>,
    pub(in crate::store) candidate_frame: &'a [u8],
    pub(in crate::store) candidate: IntentData,
}
impl std::fmt::Debug for TransitionSeedData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FounderTransitionSeedData")
            .finish_non_exhaustive()
    }
}
pub(in crate::store) fn encode_transition_seed(
    key: &[u8; 32],
    debit: &DebitData,
    id: DebitId,
    expected_parent: Option<IntentId>,
    marker: Option<&[u8]>,
    candidate: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if candidate.is_empty()
        || candidate.len() > MAX_INTENT_FRAME
        || marker.is_some_and(|bytes| bytes.is_empty() || bytes.len() > MAX_TARGET_MARKER_FRAME)
    {
        return Err(refused());
    }
    let mut e = Encoder::with_capacity(MAX_TRANSITION_PLAIN);
    tag(&mut e, SEED_TAG)?;
    put_invocation(&mut e, debit.invocation)?;
    put(&mut e, id.as_bytes())?;
    put_parent(&mut e, expected_parent)?;
    put(&mut e, marker.unwrap_or_default())?;
    put(&mut e, candidate)?;
    let raw = bounded(e, MAX_TRANSITION_PLAIN)?;
    // Validate the ACTUAL complete encrypted representations, not a separately supplied DTO
    // which could disagree with candidate bytes. This remains DATA consistency only.
    parse_transition_seed_data(key, &raw, debit, id)?;
    Ok(raw)
}
pub(in crate::store) fn seal_transition_seed(
    key: &[u8; 32],
    plain: &[u8],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    seal_small(
        key,
        "catcoms/private-chat-founder-transition-key/v1",
        plain,
        MAX_TRANSITION_PLAIN,
        rng,
    )
}
pub(in crate::store) fn open_transition_seed(
    key: &[u8; 32],
    raw: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    open_small(
        key,
        "catcoms/private-chat-founder-transition-key/v1",
        raw,
        MAX_TRANSITION_PLAIN,
    )
}
pub(in crate::store) fn parse_transition_seed_data<'a>(
    key: &[u8; 32],
    raw: &'a [u8],
    debit: &DebitData,
    id: DebitId,
) -> Result<TransitionSeedData<'a>, AppError> {
    let mut d = input(raw, MAX_TRANSITION_PLAIN, SEED_TAG)?;
    let invocation = get_invocation(&mut d)?;
    let debit_id = DebitId::from_bytes(digest(&mut d)?);
    let expected_parent = get_parent(&mut d)?;
    let marker = field(&mut d, MAX_TARGET_MARKER_FRAME)?;
    let candidate = field(&mut d, MAX_INTENT_FRAME)?;
    d.finish().map_err(|_| refused())?;
    if invocation != debit.invocation || debit_id != id || candidate.is_empty() {
        return Err(refused());
    }
    // Complete outer framing and ALL payload caps precede copied cipher/AEAD/owned group work.
    let candidate_data = open_intent_data(key, debit, id, candidate)?;
    if candidate_data.parent != expected_parent {
        return Err(refused());
    }
    let marker_data = if marker.is_empty() {
        // A pre-group/debit-only negative closure cannot possess a target. This omission does
        // NOT prove no orphan target exists and never instructs the physical caller to delete it.
        if !matches!(candidate_data.kind, IntentKind::ClosedDebit(_)) {
            return Err(refused());
        }
        None
    } else {
        if matches!(candidate_data.kind, IntentKind::ClosedDebit(_)) {
            return Err(refused());
        }
        let data = open_target_marker_data(key, debit, id, marker)?;
        candidate_data.matches_marker(&data, TargetMarkerId::of(marker))?;
        Some(data)
    };
    Ok(TransitionSeedData {
        expected_parent,
        marker_frame: if marker.is_empty() {
            None
        } else {
            Some(marker)
        },
        marker: marker_data,
        candidate_frame: candidate,
        candidate: candidate_data,
    })
}

#[cfg(test)]
#[path = "founder_codec_tests.rs"]
mod tests;
