//! Overlay entries share the ordinary writer, budgets and exact-retry flush barrier.
use super::*;
use catcoms_replication::studio::{
    StudioClosingOverlayBasis, StudioLocalDraft, StudioOverlayState, StudioTarget,
};

impl ServerStore {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn write_studio_overlay_intent(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        target: StudioTarget,
        device: &MlsDevice,
        group: &ServerGroup,
        expected: [u8; 32],
        basis: Option<&StudioClosingOverlayBasis>,
        operation: DomainOp,
        ts: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
        writer: impl FnOnce(&EpochMutation<'_>, &Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&EpochMutation<'_>, &Path, u64) -> Result<(), AppError>,
    ) -> Result<StudioLocalDraft, AppError> {
        if document.server_id != group.group_id()
            || group.member_signature_key(&device.device_id()).as_deref()
                != Some(device.public_key_bytes().as_slice())
        {
            return Err(invalid("intent author is not a current local member"));
        }
        let scope = scope_bytes(server, document)?;
        let mut state = self.checked_epoch_replay_state(server, document, budget, intents)?;
        // Physical size only; the record was authenticated above.
        let old = self
            .read_scoped_intent_plain(&scope)?
            .map(|record| record.physical_bytes);
        let intent = LocalIntent {
            author: device.device_id(),
            operation,
        };
        let op_id = intent.operation.id(&intent.author);
        if let Some(metadata) = &state.overlay {
            if metadata.target() != target {
                return Err(invalid("overlay belongs to another channel"));
            }
        }
        if let Some(overlay) = state.overlay() {
            if overlay.exact_retry(expected, &intent).map_err(invalid)? {
                let view = overlay.read(&state.ledger).map_err(invalid)?;
                self.write_prepared_intents(
                    server, document, state, old, true, rng, budget, intents, writer, sync,
                )?;
                return Ok(view);
            }
        }
        // Equal nonce/body from an ordinary failed Save is not accepted local draft evidence.
        if state.pending().any(|(id, _)| *id == op_id) {
            return Err(invalid("ordinary intent cannot become an accepted overlay"));
        }
        let basis = basis.ok_or_else(|| invalid("new overlay requires a fresh Closing basis"))?;
        if basis.fingerprint() != expected {
            return Err(invalid("Closing overlay basis changed"));
        }
        let mut overlay = state
            .overlay
            .clone()
            .unwrap_or_else(|| StudioOverlayState::new(basis));
        state
            .ledger
            .prepare(intent.author, intent.operation.clone())
            .map_err(invalid)?;
        let view = overlay
            .append(basis, &state.ledger, op_id, ts)
            .map_err(invalid)?;
        // I-3, second half: the protection transfer. These two holds must run BEFORE the write
        // attempt and while the caller's job-owned transient hold is still alive. Each rotates
        // `Protection.generation` first, so a reference scan already in progress cannot install a
        // set that omits these CIDs, and each either adds them to the known set or leaves
        // protection fail-closed unknown. Only that makes it safe for the caller to drop its
        // transient owner once the write attempt returns: a durable record alone does not repair
        // a reference set that a scan installed while the operation was still in flight.
        self.hold_creative(
            &document.server_id,
            overlay
                .overlay()
                .ok_or_else(|| invalid("overlay base missing"))?
                .base_blob_cids()
                .map_err(invalid),
        );
        self.hold_creative_operation(document, &intent.operation);
        state.overlay = Some(overlay);
        self.write_prepared_intents(
            server, document, state, old, false, rng, budget, intents, writer, sync,
        )?;
        Ok(view)
    }
}
