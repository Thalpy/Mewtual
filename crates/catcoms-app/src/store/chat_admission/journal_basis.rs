//! Inert exact admission-basis DATA for the critical join journal. Its decoder supplies no
//! reservation, Fresh provenance, writer or newer capture. Final Pending writes must still
//! hold the actual original admission owner and compare the complete current row under it.
use super::*;

/// A bounded DATA copy of the original optional row. No heap is attached to this value.
/// The constructor/decoder never installs it in the admission inventory.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::store) struct JournalBasis {
    original: Option<Record>,
}
pub(in crate::store) const MAX_ENCODED_BASIS: usize = 83;
impl std::fmt::Debug for JournalBasis {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JournalBasis")
            .finish_non_exhaustive()
    }
}

impl JournalBasis {
    pub(in crate::store) fn from_capture(capture: &ChatAdmissionCapture) -> Self {
        Self {
            original: capture.original.clone(),
        }
    }

    /// Fixed bounded framing before an outer journal encoder allocates. Canonical absence has
    /// exactly one byte; no padded/trailing representation compares equal to the same basis.
    pub(in crate::store) fn encoded_len(&self) -> usize {
        if self.original.is_some() {
            MAX_ENCODED_BASIS
        } else {
            1
        }
    }
    pub(in crate::store) fn encode_into(&self, output: &mut Encoder) -> Result<(), AppError> {
        let Some(record) = &self.original else {
            output.put_u8(0);
            return Ok(());
        };
        output.put_u8(1);
        output.put_bytes(&record.nonce).map_err(|_| refused())?;
        output.put_u64(record.revision);
        output.put_u8(match record.phase {
            Phase::Initializing => 0,
            Phase::Current => 1,
            Phase::Retired => 2,
        });
        output.put_u8(match record.origin {
            Origin::Legacy => 0,
            Origin::Fresh => 1,
        });
        output
            .put_bytes(&record.provenance)
            .map_err(|_| refused())?;
        Ok(())
    }

    /// Decode DATA only. A journal cannot create a current row or a new capture by calling it.
    pub(in crate::store) fn decode(bytes: &[u8]) -> Result<Self, AppError> {
        if bytes.is_empty() || bytes.len() > MAX_ENCODED_BASIS {
            return Err(refused());
        }
        let mut input = Decoder::new(bytes);
        let original = match input.get_u8().map_err(|_| refused())? {
            0 => None,
            1 => {
                let nonce = input
                    .get_bytes()
                    .map_err(|_| refused())?
                    .try_into()
                    .map_err(|_| refused())?;
                let revision = input.get_u64().map_err(|_| refused())?;
                let phase = match input.get_u8().map_err(|_| refused())? {
                    0 => Phase::Initializing,
                    1 => Phase::Current,
                    2 => Phase::Retired,
                    _ => return Err(refused()),
                };
                let origin = match input.get_u8().map_err(|_| refused())? {
                    0 => Origin::Legacy,
                    1 => Origin::Fresh,
                    _ => return Err(refused()),
                };
                let provenance = input
                    .get_bytes()
                    .map_err(|_| refused())?
                    .try_into()
                    .map_err(|_| refused())?;
                if revision == 0 {
                    return Err(refused());
                }
                Some(Record {
                    nonce,
                    revision,
                    phase,
                    origin,
                    provenance,
                })
            }
            _ => return Err(refused()),
        };
        input.finish().map_err(|_| refused())?;
        Ok(Self { original })
    }
}

/// Borrow of the already-held ORIGINAL writer. Rechecking this after the catalog wait reads
/// actual captured state without recursively locking the non-reentrant admission mutex.
pub(in crate::store) struct JournalAdmissionPermit<'writer> {
    store: &'writer ServerStore,
    capture: &'writer ChatAdmissionCapture,
    runtime: &'writer Runtime,
}
impl JournalAdmissionPermit<'_> {
    /// Negative original JOIN-task check while this SAME admission writer is held. Call after
    /// the catalog wait immediately before FIRST counter candidate admission. It grants no
    /// resource/unit/scope or permission to recreate a ticket from an already-admitted repair.
    pub(in crate::store) fn require_original_join_execution(
        &self,
        observation: &super::predecessor::OriginalJoinExecutionObservation,
    ) -> Result<(), AppError> {
        self.validate_original()?;
        self.runtime
            .predecessors
            .validate_join_execution(self.capture, observation)
    }

    pub(in crate::store) fn validate_capture(
        &self,
        capture: &ChatAdmissionCapture,
    ) -> Result<(), AppError> {
        if !std::ptr::eq(self.capture, capture) {
            return Err(refused());
        }
        self.validate_original()
    }

    pub(in crate::store) fn validate_original(&self) -> Result<(), AppError> {
        let capture = self.capture;
        if !self.runtime.managed
            || self.runtime.dirty
            || self.runtime.format != Some(catcoms_storage::VaultFormat::CriticalNamespaces)
            || capture.mount.is_cancelled()
            || capture
                .admission
                .as_ref()
                .is_some_and(RequestCancellation::is_cancelled)
            || !Arc::ptr_eq(&capture.mount_identity, &self.store.chat_mount.identity)
            || self
                .runtime
                .table
                .as_ref()
                .ok_or_else(refused)?
                .get(&capture.key)
                != capture.original.as_ref()
        {
            return Err(refused());
        }
        if capture.issued.lock().map_err(|_| refused())?.is_some() {
            return Err(refused());
        }
        Ok(())
    }
}

impl<'writer> JournalAdmissionPermit<'writer> {
    /// Admission-family constructor for a concrete original registration while its actual
    /// Runtime writer is already held. It grants no scope advance or publication capability.
    pub(super) fn for_original(
        store: &'writer ServerStore,
        capture: &'writer ChatAdmissionCapture,
        runtime: &'writer Runtime,
    ) -> Self {
        Self {
            store,
            capture,
            runtime,
        }
    }
}

/// Restrictive physical-mount permit for retaining one already authenticated received INPUT.
/// Unlike JournalAdmissionPermit this deliberately has NO original-row or task authority.
/// The catalog owner must separately prove exactly one Input DATA change before admission;
/// no caller can use this type to admit counters, scopes, scheduling or namespace changes.
pub(in crate::store) struct ReceivedJoinInputPermit<'writer> {
    store: &'writer ServerStore,
    capture: &'writer ChatAdmissionCapture,
    runtime: &'writer Runtime,
}
impl ReceivedJoinInputPermit<'_> {
    pub(in crate::store) fn validate_capture(
        &self,
        capture: &ChatAdmissionCapture,
    ) -> Result<(), AppError> {
        if !std::ptr::eq(self.capture, capture)
            || !self.runtime.managed
            || self.runtime.dirty
            || self.runtime.format != Some(catcoms_storage::VaultFormat::CriticalNamespaces)
            || self.runtime.table.is_none()
            || capture.mount.is_cancelled()
            || !Arc::ptr_eq(&capture.mount_identity, &self.store.chat_mount.identity)
        {
            return Err(refused());
        }
        Ok(())
    }
}

/// Repair-only original physical mount + actual Store pending identity. This ignores an old
/// admission row only AFTER its immutable catalog output was admitted into Store custody. It
/// cannot first-admit a candidate, activate/renew a task, issue a ticket or advance a scope.
pub(in crate::store) struct RetainedJoinDataRepairPermit<'writer> {
    physical: ReceivedJoinInputPermit<'writer>,
    identity:
        &'writer crate::store::critical_namespaces::pending_join::RetainedJoinCandidateIdentity,
}
impl RetainedJoinDataRepairPermit<'_> {
    fn validate_candidate(
        &self,
        candidate: &crate::store::critical_namespaces::pending_join::PendingCandidate,
    ) -> Result<(), AppError> {
        self.physical
            .validate_capture(&candidate.original.capture)?;
        self.identity.validate_candidate(candidate)
    }
}

/// Closed physical-writer dispatch. Received DATA never masquerades as an original task/row
/// permit; only the exact retained input candidate may use its distinct branch.
#[derive(Clone, Copy)]
pub(in crate::store) enum JoinCatalogWritePermit<'permit, 'writer> {
    Original(&'permit JournalAdmissionPermit<'writer>),
    Received(&'permit ReceivedJoinInputPermit<'writer>),
    Retained(&'permit RetainedJoinDataRepairPermit<'writer>),
}
impl JoinCatalogWritePermit<'_, '_> {
    pub(in crate::store) fn require_original_join_execution(
        &self,
        observation: &super::predecessor::OriginalJoinExecutionObservation,
    ) -> Result<(), AppError> {
        match self {
            Self::Original(permit) => permit.require_original_join_execution(observation),
            Self::Received(_) | Self::Retained(_) => Err(refused()),
        }
    }

    pub(in crate::store) fn validate_candidate(
        &self,
        candidate: &crate::store::critical_namespaces::pending_join::PendingCandidate,
    ) -> Result<(), AppError> {
        match self {
            Self::Original(permit) if candidate.received_input.is_none() => {
                permit.validate_capture(&candidate.original.capture)
            }
            Self::Received(permit)
                if candidate.received_input.is_some() && candidate.execution_task.is_none() =>
            {
                permit.validate_capture(&candidate.original.capture)?;
                candidate.validate_received_input_change()
            }
            Self::Retained(permit) => permit.validate_candidate(candidate),
            _ => Err(refused()),
        }
    }
}

impl ServerStore {
    /// Retry ONLY the same Store-owned admitted DATA. Caller captured its opaque identity from
    /// mounted.pending and must compare that SAME object again after catalog wait. A clean
    /// actual retired/later row is allowed; dirty negative inventory must repair independently.
    pub(in crate::store) fn with_retained_join_catalog_repair<T>(
        &self,
        capture: &ChatAdmissionCapture,
        identity: &crate::store::critical_namespaces::pending_join::RetainedJoinCandidateIdentity,
        effect: impl FnOnce(&RetainedJoinDataRepairPermit<'_>) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let runtime = self.chat_mount.locked()?;
        if self.pending_join_policy.is_none() {
            return Err(refused());
        }
        let physical = ReceivedJoinInputPermit {
            store: self,
            capture,
            runtime: &runtime,
        };
        physical.validate_capture(capture)?;
        let permit = RetainedJoinDataRepairPermit { physical, identity };
        {
            // Establish actual admitted custody here, not just at the final physical writer.
            // An identity minted on an uninstalled candidate cannot enter even this callback.
            // Release this subordinate owner before callback/head work; SAME admission still
            // orders all candidate installs, and final settle repeats the exact object check.
            let mounted = self.pending_join_catalog.lock().map_err(|_| refused())?;
            let pending = mounted
                .as_ref()
                .and_then(|mounted| mounted.pending.as_ref())
                .ok_or_else(refused)?;
            permit.validate_candidate(pending)?;
        }
        let result = effect(&permit);
        drop(runtime);
        result
    }

    /// Retain only genuine received INPUT DATA on the SAME physical live mount, including
    /// after its original admission row has been retired. This never returns a task permission.
    pub(in crate::store) fn with_received_join_input_capture<T>(
        &self,
        capture: &ChatAdmissionCapture,
        effect: impl FnOnce(&ReceivedJoinInputPermit<'_>) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let runtime = self.chat_mount.locked()?;
        let permit = ReceivedJoinInputPermit {
            store: self,
            capture,
            runtime: &runtime,
        };
        permit.validate_capture(capture)?;
        let result = effect(&permit);
        drop(runtime);
        result
    }

    /// Serialize a Pending-source write against the ORIGINAL admission capture. This callback
    /// may acquire only the catalog owner after admission; it must not call a Store admission/
    /// format getter, encode/perform large allocations, await, or drop quota owners while this
    /// writer is held. Small existing I/O diagnostics remain possible on the error path.
    /// No positive admission/Current permit escapes. The complete original row is compared,
    /// including retired phase and provenance; equal signer/group/provenance alone is not CAS.
    pub(in crate::store) fn with_original_join_catalog_capture<T>(
        &self,
        capture: &ChatAdmissionCapture,
        effect: impl FnOnce(&JournalAdmissionPermit<'_>) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let runtime = self.chat_mount.locked()?;
        let permit = JournalAdmissionPermit {
            store: self,
            capture,
            runtime: &runtime,
        };
        permit.validate_original()?;
        // The consumer MUST call this same permit's validate_original after obtaining its
        // catalog owner, immediately before final source/parent CAS and the admitted effect.
        let result = effect(&permit);
        drop(runtime);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These are storage DATA fixtures only. A decoded Current/Fresh label supplies no actual
    // admission observer, published owner, mounted journal capture or historical membership.
    #[test]
    fn journal_basis_preserves_complete_optional_row_phase_origin_nonce_revision_and_provenance() {
        let absent = JournalBasis { original: None };
        let mut encoded = Encoder::with_capacity(absent.encoded_len());
        absent.encode_into(&mut encoded).unwrap();
        assert_eq!(encoded.len(), 1);
        assert!(JournalBasis::decode(&encoded.finish()).unwrap() == absent);
        for phase in [Phase::Initializing, Phase::Current, Phase::Retired] {
            for origin in [Origin::Legacy, Origin::Fresh] {
                for revision in [1, u64::MAX] {
                    let basis = JournalBasis {
                        original: Some(Record {
                            nonce: [1; 32],
                            revision,
                            phase,
                            origin,
                            provenance: [2; 32],
                        }),
                    };
                    let mut encoded = Encoder::with_capacity(basis.encoded_len());
                    basis.encode_into(&mut encoded).unwrap();
                    assert_eq!(encoded.len(), MAX_ENCODED_BASIS);
                    assert!(JournalBasis::decode(&encoded.finish()).unwrap() == basis);
                }
            }
        }
    }

    #[test]
    fn journal_basis_malformed_flags_counts_zero_revision_and_noncanonical_absence_refuse() {
        for bytes in [
            vec![],
            vec![2],
            vec![0, 0],
            vec![1],
            vec![0; MAX_ENCODED_BASIS + 1],
        ] {
            assert!(JournalBasis::decode(&bytes).is_err());
        }
        let basis = JournalBasis {
            original: Some(Record {
                nonce: [1; 32],
                revision: 1,
                phase: Phase::Current,
                origin: Origin::Fresh,
                provenance: [2; 32],
            }),
        };
        let mut encoded = Encoder::new();
        basis.encode_into(&mut encoded).unwrap();
        let original = encoded.finish();
        for at in [1usize, 2, 3, 4, 45, 46, 47] {
            let mut bytes = original.clone();
            bytes[at] = 255;
            assert!(JournalBasis::decode(&bytes).is_err());
        }
        let mut zero_revision = original.clone();
        zero_revision[37..45].fill(0);
        assert!(JournalBasis::decode(&zero_revision).is_err());
        for length in 0..original.len() {
            assert!(JournalBasis::decode(&original[..length]).is_err());
        }
    }
}
