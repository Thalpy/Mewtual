//! Loans only actual confirmation output capacity. A matching serialized header alone is not
//! authority: the native call/pair/resource/packet-table owners must all be the original ones.
use super::*;
use crate::voice_v2::{Context, EncryptedHeader, EncryptedKind};

impl Workflow {
    pub(in crate::voice_v2) fn outgoing_confirmation_slot(
        &self,
        resources: &Arc<Resources>,
        context: &Context,
        call: &CallLease,
        pair: &CallLease,
        slots: &crate::voice_v2::packets::PacketSlots,
        header: &EncryptedHeader,
    ) -> Result<(slots::Slot, u64, CallLease), VoiceError> {
        let closed = self
            .closed
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let correct_role = if self.binding.low {
            matches!(
                header.kind,
                EncryptedKind::ConfirmHello | EncryptedKind::ConfirmFinish
            )
        } else {
            header.kind == EncryptedKind::ConfirmProof
        };
        if *closed
            || !self.live.is_live()
            || !self.binding.live()
            || !Arc::ptr_eq(resources, &self.resources)
            || context.scope != self.binding.scope
            || context.epoch != self.binding.epoch
            || context.local != self.binding.local
            || !call.is_same(&self.binding.call)
            || !pair.is_same(&self.binding.pair)
            || !correct_role
            || header.offer.is_some()
            || header.handshake != self.binding.handshake
            || header.source_join != self.binding.local_join
            || header.source_key_ref != self.binding.local_key
            || header.target != self.binding.peer
            || header.target_join != self.binding.peer_join
            || header.target_key_ref != self.binding.peer_key
            || header.low_to_high != self.binding.low
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok((
            self.packets.outgoing_slot(slots)?,
            self.binding.deadline,
            self.live.clone(),
        ))
    }
}
