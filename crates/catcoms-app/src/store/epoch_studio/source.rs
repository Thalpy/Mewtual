//! One mount-owned, moved (never cloned) verified restart unit. This is work reuse, not a
//! persistence owner or an authority cache. Every take authenticates the actual full wrapper.

use super::*;
use catcoms_crypto::DeviceId;

/// Encoded input bound, NOT a resident-heap promise. Existing graph/operation/projection limits
/// still bound the parsed unit, the transient preflight draft, and serialization allocations.
const MAX_RETAINED_BYTES: u64 = 8 * 1024 * 1024;

#[cfg(test)]
thread_local! {
    static FULL_RESTORES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
pub(crate) fn studio_full_restores_for_test() -> usize {
    FULL_RESTORES.get()
}

pub(super) fn restore_unit(
    snapshot: &[u8],
    group: &ServerGroup,
    target: StudioTarget,
    actor: DeviceId,
) -> Result<StudioEpoch, AppError> {
    #[cfg(test)]
    FULL_RESTORES.set(FULL_RESTORES.get() + 1);
    StudioEpoch::restore(snapshot, group, target, actor).map_err(invalid)
}

pub(super) struct SourceVersion {
    mount: Arc<()>,
    server: u64,
    digest: blake3::Hash,
    record: StorageRecord,
    bytes: u64,
}
impl std::fmt::Debug for SourceVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceVersion")
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

pub(in crate::store) struct RetainedSource {
    state: EpochStudioState,
    actor: DeviceId,
    mls_epoch: u64,
}

impl RetainedSource {
    fn matches(
        &self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        actor: DeviceId,
    ) -> bool {
        self.actor == actor
            && self.mls_epoch == group.epoch()
            && self.state.unit.document().server_id == group.group_id()
            && self.state.unit.target() == target
            && self
                .state
                .source
                .as_ref()
                .is_some_and(|s| s.server == server)
    }
}

impl ServerStore {
    /// Explicit view access may rebuild on a cache miss; automatic receive never takes that
    /// large cold fallback. A warm view reauthenticates its wrapper without cloning the graph.
    pub(crate) fn with_studio_source<V>(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        read: impl FnOnce(&EpochStudioState) -> Result<V, AppError>,
    ) -> Result<Option<V>, AppError> {
        current_member(group, device)?;
        let prepared = if self.studio_source_is_warm(server, group, target, device) {
            let held = self.studio_source.take().expect("matched source");
            // Explicit read alone may recover from changed/cold bytes by the normal full load.
            // Never restore the stale slot after an authentication or projection error.
            self.studio_source_bytes_match(&held.state)
                .unwrap_or(false)
                .then_some(held.state)
        } else {
            None
        };
        let state = match prepared {
            Some(state) => Some(state),
            None => self.load_studio_epoch(server, group, target, device)?,
        };
        let Some(state) = state else {
            return Ok(None);
        };
        let view = read(&state)?;
        // Every StudioUpdated also invalidates the channel list. Its independent Index read
        // must not make refresh order evict the large art graph and pause its next packet.
        // Keep only the Index's pure verified footprint, not a second retained graph.
        if matches!(target, StudioTarget::Index { .. })
            && self.studio_source.as_ref().is_some_and(|held| {
                matches!(held.state.unit.target(), StudioTarget::Flipnote { .. })
            })
        {
            self.cache_studio_source_footprint(group, &state);
        } else {
            self.retain_studio_source(group, device, state);
        }
        Ok(Some(view))
    }

    fn studio_source_bytes_match(&self, state: &EpochStudioState) -> Result<bool, AppError> {
        let Some(version) = state.source.as_ref() else {
            return Ok(false);
        };
        if !Arc::ptr_eq(&version.mount, &self.registry_mount()) {
            return Ok(false);
        }
        let scope = scope_bytes(version.server, state.unit.document())?;
        let Some(held) = self.read_studio_record_bounded(&scope, MAX_RETAINED_BYTES as usize)?
        else {
            return Ok(false);
        };
        Ok(held.physical_bytes == version.bytes && blake3::hash(&held.plain) == version.digest)
    }

    /// A cold small Index packet must not evict the explicitly opened large art source. A warm
    /// ingest took the slot, so its successful result can replace it without a second graph.
    pub(crate) fn retain_received_studio_source(
        &mut self,
        group: &ServerGroup,
        device: &MlsDevice,
        state: EpochStudioState,
    ) {
        if self.studio_source.is_none() {
            self.retain_studio_source(group, device, state);
        }
    }
    pub(super) fn studio_source_version(
        &self,
        server: u64,
        unit: &StudioEpoch,
        plain: &[u8],
        bytes: u64,
    ) -> Result<SourceVersion, AppError> {
        let scope = scope_bytes(server, unit.document())?;
        Ok(SourceVersion {
            mount: self.registry_mount(),
            server,
            digest: blake3::hash(plain),
            bytes,
            record: storage_record(
                server,
                unit.document(),
                &scope,
                bytes,
                unit.storage_protocol_bytes().map_err(invalid)?,
            )?,
        })
    }

    /// Transfer a just-verified read or durable result into the sole slot. No graph copy and no
    /// vault key is retained. Oversized sources remain readable/saved, simply not warm for receive.
    pub(crate) fn retain_studio_source(
        &mut self,
        group: &ServerGroup,
        device: &MlsDevice,
        state: EpochStudioState,
    ) {
        self.studio_source = None;
        if !self.cache_studio_source_footprint(group, &state) {
            return;
        }
        self.studio_source = Some(RetainedSource {
            state,
            actor: device.device_id(),
            mls_epoch: group.epoch(),
        });
    }

    /// Metadata-only reuse for a verified result, including an Index refresh beside open art.
    fn cache_studio_source_footprint(
        &mut self,
        group: &ServerGroup,
        state: &EpochStudioState,
    ) -> bool {
        let Some(version) = state.source.as_ref() else {
            return false;
        };
        if version.bytes > MAX_RETAINED_BYTES
            || !Arc::ptr_eq(&version.mount, &self.registry_mount())
            || state.unit.document().server_id != group.group_id()
        {
            return false;
        }
        // Full restore (or checked ingest + successful save) already established this footprint.
        // Future inventory hits STILL authenticate the complete wrapper and reach directory EOF.
        self.inventory_cache.put(
            (EpochRecordKind::Studio, version.record.id),
            version.bytes,
            version.digest,
            version.record,
        );
        true
    }

    /// Pre-I/O service refusal only. A candidate NEVER authorizes reuse before fresh byte checks.
    pub(crate) fn studio_source_is_warm(
        &self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
    ) -> bool {
        self.studio_source
            .as_ref()
            .is_some_and(|s| s.matches(server, group, target, device.device_id()))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ingest_studio_epoch_reusing(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        sealed: &SealedOp,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<(Admission, EpochStudioState), AppError> {
        self.ingest_studio_epoch_reusing_with_io(
            server,
            group,
            target,
            device,
            sealed,
            rng,
            budget,
            atomic_write,
            sync_studio,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store::epoch_studio) fn ingest_studio_epoch_reusing_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        sealed: &SealedOp,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<(Admission, EpochStudioState), AppError> {
        if !self.studio_source_is_warm(server, group, target, device) {
            // Caller applies the cold source rail before selecting this adapter. Repeat here so
            // an internal caller cannot accidentally fall back to a full large reconstruction.
            self.check_studio_receive_source_bound(server, &group.group_id(), target)?;
            return self.ingest_studio_epoch(server, group, target, device, sealed, rng, budget);
        }
        // Consume first: every subsequent failure drops the unit, never retains a mutated or
        // uncertain graph. The caller can explicitly Read/Save to prepare a fresh source.
        let retained = self.studio_source.take().expect("matched source");
        let EpochStudioState { mut unit, source } = retained.state;
        let version = source.expect("retained version");
        current_member(group, device)?;
        if sealed.blob.ciphertext.len() > MAX_INBOUND_CIPHERTEXT {
            return Err(invalid("inbound ciphertext too large"));
        }
        self.enter_studio_budget(server, group, budget)?;
        let checked = (|| {
            let scope = scope_bytes(server, unit.document())?;
            let held = self
                .read_studio_record_bounded(&scope, MAX_RETAINED_BYTES as usize)?
                .ok_or_else(|| invalid("prepared Studio source disappeared"))?;
            if !Arc::ptr_eq(&version.mount, &self.registry_mount())
                || held.physical_bytes != version.bytes
                || blake3::hash(&held.plain) != version.digest
            {
                return Err(invalid("prepared Studio source changed; explicitly reopen"));
            }
            budget
                .storage
                .verify_record(
                    &StorageScope::new(server, &group.group_id()).map_err(invalid)?,
                    version.record.id,
                    Some(version.record),
                )
                .map_err(invalid)?;
            Ok(())
        })();
        if checked.is_err() {
            budget.storage.invalidate();
        }
        checked?;
        let before = Zeroizing::new(unit.snapshot().map_err(invalid)?);
        let admission = unit.ingest(sealed, group, device).map_err(invalid)?;
        let state = self.save_studio_source_reusing(
            server,
            unit,
            Some(version.record),
            &before,
            WritePurpose::Ordinary,
            rng,
            &mut budget.storage,
            writer,
            sync,
            Some(version),
        )?;
        Ok((admission, state))
    }
}

#[cfg(test)]
mod bounds {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    #[test]
    fn studio_source_retention_size_rail_is_inclusive_and_never_clones_a_slot() {
        let root = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(197);
        let mut store = ServerStore::open(root.path(), b"source-rail", &mut rng).unwrap();
        let device = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&device).unwrap();
        let target = StudioTarget::Index { channel: [7; 16] };
        for bytes in [MAX_RETAINED_BYTES, MAX_RETAINED_BYTES + 1] {
            let unit = StudioEpoch::new(&group, target, device.device_id()).unwrap();
            // Private encoded-size ballast only. Actual wrapper/version matching and persisted
            // contents are covered separately; this fixture cannot authorize an absent file.
            let version = store
                .studio_source_version(73, &unit, b"test-only", bytes)
                .unwrap();
            store.retain_studio_source(
                &group,
                &device,
                EpochStudioState {
                    unit,
                    source: Some(version),
                },
            );
            assert_eq!(
                store.studio_source_is_warm(73, &group, target, &device),
                bytes == MAX_RETAINED_BYTES
            );
        }
    }
}
