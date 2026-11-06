//! Actual first-Add successor save. Only the original Current/head writer can call this; a
//! visible authenticated file or decoded operation label never constructs the covering result.
use super::*;

pub(in crate::store::owner_evidence::canonical) struct SavedOwnerSuccessor {
    mount: Arc<()>,
    writer: Arc<()>,
    meta: Meta,
    predecessor: Meta,
}
pub(in crate::store::owner_evidence::canonical) struct OwnerSuccessorInput<'a> {
    pub(in crate::store::owner_evidence::canonical) binding: &'a ChatAdmissionBinding,
    pub(in crate::store::owner_evidence::canonical) expected: &'a Meta,
    pub(in crate::store::owner_evidence::canonical) reservation:
        &'a budget::OwnerOperationReservation,
    pub(in crate::store::owner_evidence::canonical) attempt: [u8; 32],
    pub(in crate::store::owner_evidence::canonical) frozen: &'a [u8],
}
impl SavedOwnerSuccessor {
    pub(in crate::store::owner_evidence::canonical) fn metadata(&self) -> Meta {
        self.meta.clone()
    }
    pub(in crate::store::owner_evidence::canonical) fn validate_install(
        &self,
        head: &OwnerHeadWriteGuard<'_>,
        permit: &ChatAdmissionStoragePermit<'_>,
        binding: &ChatAdmissionBinding,
        writer: &OwnerWriterRegistration,
        predecessor: &Meta,
        digest: [u8; 32],
    ) -> Result<(), AppError> {
        writer.validate_original(head, permit, binding)?;
        if !Arc::ptr_eq(&self.mount, &permit.store().registry_mount())
            || !Arc::ptr_eq(&self.writer, &writer.identity())
            || &self.predecessor != predecessor
            || self.meta.generation != 2
            || self.meta.snapshot != digest
            || head.state.pending.is_some()
            || head.state.current.as_ref() != Some(&self.meta)
            || !permit.vault_format()?.requires_owner_operations()
        {
            return Err(refused());
        }
        Ok(())
    }
}

impl OwnerWriterRegistration {
    /// Original admission is already held. The private caller performs only its validated
    /// install inside this callback; no public arbitrary callback or async writer escape exists.
    pub(in crate::store::owner_evidence::canonical) fn with_first_add_successor<Out>(
        &self,
        permit: &ChatAdmissionStoragePermit<'_>,
        input: OwnerSuccessorInput<'_>,
        rng: &mut impl CryptoRngCore,
        install: impl FnOnce(&OwnerHeadWriteGuard<'_>, SavedOwnerSuccessor) -> Result<Out, AppError>,
    ) -> Result<Out, AppError> {
        let OwnerSuccessorInput {
            binding,
            expected,
            reservation,
            attempt,
            frozen,
        } = input;
        reservation.validate_original(permit, self, expected)?;
        let mut head = self.head.lock()?;
        self.validate_original(&head, permit, binding)?;
        if expected.generation != 1 || frozen.len() > reservation.limits().max_snapshot_bytes {
            return Err(refused());
        }
        let capacity = frozen.len().checked_add(WRAPPER_FEE).ok_or_else(refused)?;
        // Encode only the small fixed wrapper before retained-source classification. A full
        // assembled candidate here would overlap raw+cipher+plain rereads beyond four buffers.
        let mut encoder = Encoder::with_capacity(WRAPPER_FEE);
        encoder
            .put_str("catcoms/owner-canonical-snapshot/v2")
            .map_err(|_| refused())?;
        for field in [
            permit.group(),
            permit.signer().as_slice(),
            expected.profile.as_bytes(),
            permit.domain().as_slice(),
            permit.nonce().as_slice(),
        ] {
            encoder.put_bytes(field).map_err(|_| refused())?;
        }
        encoder.put_u64(permit.revision());
        encoder.put_u64(2);
        encoder.put_bytes(&expected.head).map_err(|_| refused())?;
        encoder.put_u64(expected.generation);
        encoder
            .put_bytes(&expected.snapshot)
            .map_err(|_| refused())?;
        encoder.put_bytes(&attempt).map_err(|_| refused())?;
        encoder.put_u32(u32::try_from(frozen.len()).map_err(|_| refused())?);
        let header = Zeroizing::new(encoder.finish());
        if header.len() > WRAPPER_FEE {
            return Err(refused());
        }
        let mut hash = blake3::Hasher::new();
        hash.update(b"catcoms/owner-head-id/v1\0");
        hash.update(&header);
        hash.update(frozen);
        let meta = Meta {
            generation: 2,
            head: *hash.finalize().as_bytes(),
            snapshot: *blake3::hash(frozen).as_bytes(),
            attempt,
            profile: expected.profile,
        };
        if head.state.current.as_ref() == Some(&meta) && head.state.pending.is_none() {
            return install(
                &head,
                SavedOwnerSuccessor {
                    mount: self.mount.clone(),
                    writer: self.identity(),
                    meta,
                    predecessor: expected.clone(),
                },
            );
        }
        if head.state.current.as_ref() != Some(expected)
            || head
                .state
                .pending
                .as_ref()
                .is_some_and(|pending| pending != &meta)
        {
            return Err(refused());
        }
        // Latch before seal/write. Every failure retains the exact possible successor; neither
        // another request nor the previous ordinary snapshot may overwrite this debt.
        head.state.pending = Some(meta.clone());
        let cap = reservation
            .limits()
            .max_snapshot_bytes
            .checked_add(WRAPPER_FEE + SEALED_FEE)
            .ok_or_else(refused)?;
        let already_written =
            head.settle_operation_record(permit, expected, &meta, &[&header, frozen], cap)?;
        if !already_written {
            // Retained exact sources were either settled above or absent. Assemble/seal only
            // after their reread scratch dropped; this is the original already-frozen Add.
            let mut plain = Zeroizing::new(Vec::with_capacity(capacity));
            plain.extend_from_slice(&header);
            plain.extend_from_slice(frozen);
            let sealed = seal(&permit.store().keys.db_key()?, &plain, rng)?;
            self.validate_original(&head, permit, binding)?;
            reservation.validate_original(permit, self, expected)?;
            permit.store().write_owner_canonical_record(
                permit,
                &head,
                &crate::store::frame(&sealed),
            )?;
        }
        self.validate_original(&head, permit, binding)?;
        reservation.validate_original(permit, self, expected)?;
        head.state.current = Some(meta.clone());
        head.state.pending = None;
        self.validate_original(&head, permit, binding)?;
        install(
            &head,
            SavedOwnerSuccessor {
                mount: self.mount.clone(),
                writer: self.identity(),
                meta,
                predecessor: expected.clone(),
            },
        )
    }
}
