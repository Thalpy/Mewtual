//! Retained exact originals for a native joined-registry transaction.
use super::*;
use std::sync::Arc;

type BorrowedFile<'a> = (&'a File, &'a [u8]);
const FILES: usize = 5;
fn names() -> [&'static str; FILES] {
    [
        critical_namespaces::Slot::Current.name(),
        INDEX_NAME,
        PENDING_NAME,
        welcome::ENTRY,
        welcome::OUTCOME,
    ]
}

pub(in crate::store) struct JoinedRegistrationDependency {
    files: [OpenedArchiveObject; FILES],
    directory: GovernanceDirectoryCapture,
    mount: Arc<()>,
    data: NativeTextJoinedRegistrationData,
    reading: std::sync::Mutex<()>,
    _lease: catcoms_storage::VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

impl JoinedRegistrationDependency {
    fn capture(
        store: &ServerStore,
        data: NativeTextJoinedRegistrationData,
        expected_directory: &GovernanceDirectoryCapture,
        original: [BorrowedFile<'_>; FILES],
    ) -> Result<Arc<Self>, AppError> {
        let bytes = registration_fee_bytes(original.map(|(_, bytes)| bytes.len()))?;
        let fee = Self::reserve_capture(store, bytes)?;
        Self::capture_prepaid(store, data, expected_directory, original, fee)
    }

    fn reserve_capture(
        store: &ServerStore,
        bytes: usize,
    ) -> Result<CatalogReadReservation, AppError> {
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        policy.current.archive.pending.pool.try_reserve(bytes)
    }

    fn capture_prepaid(
        store: &ServerStore,
        data: NativeTextJoinedRegistrationData,
        expected_directory: &GovernanceDirectoryCapture,
        original: [BorrowedFile<'_>; FILES],
        fee: CatalogReadReservation,
    ) -> Result<Arc<Self>, AppError> {
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let bytes = registration_fee_bytes(original.map(|(_, raw)| raw.len()))?;
        fee.validate(&policy.current.archive.pending.pool, bytes)?;
        let directory = capture_directory(store)?;
        if directory.identity != expected_directory.identity {
            return Err(denied());
        }
        let mut files = Vec::with_capacity(FILES);
        for ((file, expected), name) in original.into_iter().zip(names()) {
            let path = fixed(store, false, name)?;
            let identity = session_id(file).map_err(io)?;
            let held = read(&path, expected.len())?.ok_or_else(denied)?;
            if session_id(&held.0).map_err(io)? != identity || held.1.as_slice() != expected {
                return Err(denied());
            }
            files.push(held);
        }
        let result = Arc::new(Self {
            files: files.try_into().map_err(|_| denied())?,
            directory,
            mount: store.registry_mount(),
            data,
            reading: std::sync::Mutex::new(()),
            _lease: store._session.retain_for_physical_work(),
            _fee: fee,
        });
        result.require(store)?;
        Ok(result)
    }

    pub(in crate::store) fn data(&self) -> NativeTextJoinedRegistrationData {
        self.data
    }

    /// Used inside native selection barriers. Never acquires a Root, native
    /// index, governance or joined-pin writer to prove these same files.
    pub(in crate::store) fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        let _reading = self.reading.try_lock().map_err(|_| denied())?;
        if !Arc::ptr_eq(&self.mount, &store.registry_mount()) {
            return Err(denied());
        }
        validate_directory(store, &self.directory)?;
        absent(&fixed(store, false, INDEX_STAGE)?)?;
        absent(&fixed(store, false, governance::GOVERNANCE_SEED)?)?;
        absent(&fixed(
            store,
            false,
            critical_namespaces::Slot::Stage.name(),
        )?)?;
        for ((file, bytes), name) in self.files.iter().zip(names()) {
            exact_held(
                &fixed(store, false, name)?,
                file,
                session_id(file).map_err(io)?,
                bytes,
            )?;
        }
        validate_directory(store, &self.directory)
    }

    fn with_review_catalog(
        &self,
        store: &ServerStore,
        original: &OpenedArchiveObject,
        current: &OpenedArchiveObject,
        fee: CatalogReadReservation,
    ) -> Result<Arc<Self>, AppError> {
        let _reading = self.reading.try_lock().map_err(|_| denied())?;
        let [previous, index, pending, welcome, outcome] = &self.files;
        require_original_catalog_copy(previous, original)?;
        if !Arc::ptr_eq(&self.mount, &store.registry_mount()) {
            return Err(denied());
        }
        // Keep the four exact source objects and their original file
        // identities. Only the typed ID12 writer can provide the new Current.
        Self::capture_prepaid(
            store,
            self.data,
            &self.directory,
            [
                (&current.0, &current.1),
                (&index.0, &index.1),
                (&pending.0, &pending.1),
                (&welcome.0, &welcome.1),
                (&outcome.0, &outcome.1),
            ],
            fee,
        )
    }
}

fn registration_fee_bytes(lengths: [usize; FILES]) -> Result<usize, AppError> {
    lengths
        .into_iter()
        .try_fold(32 * 1024usize, |total, bytes| {
            total.checked_add(bytes).ok_or_else(denied)
        })
}

fn require_original_catalog_copy(
    held: &OpenedArchiveObject,
    expected: &OpenedArchiveObject,
) -> Result<(), AppError> {
    if held.1 != expected.1
        || session_id(&held.0).map_err(io)? != session_id(&expected.0).map_err(io)?
    {
        return Err(denied());
    }
    Ok(())
}

impl JoinedPendingIo {
    /// This exact mounted join admitted a review declaration whose physical
    /// outcome the warm owner could not adopt. A retry must reopen it cold.
    pub(in crate::store) fn require_warm_review_catalog(&self) -> Result<(), AppError> {
        if self.review_reopen_required {
            return Err(AppError::ChatV2JoinReviewReopen);
        }
        Ok(())
    }

    /// Install the exact first review receiver while the original warm join
    /// still owns the metadata and no native row selection has been issued.
    pub(in crate::store) fn select_warm_review_receiver(
        &mut self,
        store: &ServerStore,
        original: &mut Arc<JoinedRegistrationDependency>,
        before_write: impl FnOnce() -> Result<(), AppError>,
        rng: &mut dyn catcoms_rt::CryptoRngCore,
    ) -> Result<bool, AppError> {
        use critical_namespaces::pending_join::Settle;

        self.require_warm_review_catalog()?;
        original.require(store)?;
        if self.warm_registration_data(store)? != original.data() || store.has_rejoin_evidence()? {
            return Err(denied());
        }
        let original_frame = *blake3::hash(&self.catalog.1).as_bytes();
        let mut admitted = false;
        let result = (|| {
            let mut funding = None;
            let Some((current, receiver)) = store.cover_warm_review_receiver(
                &self.catalog,
                |candidate_bytes| {
                    let catalog = store
                        .pending_join_catalog
                        .try_lock()
                        .map_err(|_| denied())?;
                    require_idle_catalog(catalog.as_ref().ok_or_else(denied)?, original_frame)?;
                    drop(catalog);
                    let mut lengths = original.files.each_ref().map(|(_, raw)| raw.len());
                    lengths[0] = candidate_bytes;
                    let fee = JoinedRegistrationDependency::reserve_capture(
                        store,
                        registration_fee_bytes(lengths)?,
                    )
                    .map_err(|_| AppError::ChatV2ArchiveBusy)?;
                    let inspect = store
                        .reserve_pending_join_catalog_read()
                        .map_err(|_| AppError::ChatV2ArchiveBusy)?;
                    before_write()?;
                    funding = Some((fee, inspect));
                    admitted = true;
                    Ok(())
                },
                rng,
            )?
            else {
                return Ok(false);
            };

            let (fee, inspect) = funding.ok_or_else(denied)?;
            let opened = store.inspect_prepaid_pending_join_catalog(inspect)?;
            let actual = opened.current.as_ref().ok_or_else(denied)?;
            if !matches!(opened.settle, Settle::None)
                || opened.selected.framed != *blake3::hash(&current.1).as_bytes()
                || session_id(actual).map_err(io)? != session_id(&current.0).map_err(io)?
            {
                return Err(denied());
            }
            let registration = original.with_review_catalog(store, &self.catalog, &current, fee)?;
            let mut catalog_slot = store.pending_join_catalog.lock().map_err(|_| denied())?;
            let catalog = catalog_slot.as_mut().ok_or_else(denied)?;
            let mut index_slot = store.joined_pin_index.lock().map_err(|_| denied())?;
            let index = index_slot.as_mut().ok_or_else(denied)?;
            let mut review = store.review_receive_mount.lock().map_err(|_| denied())?;
            require_idle_catalog(catalog, original_frame)?;
            if review.is_some() {
                return Err(denied());
            }
            // A join recovered before its Welcome can borrow a second held
            // Current from SelectedPinIndex. Preserve that proof's original
            // directory and require the exact same predecessor before moving it.
            let replacement = index
                .cold_catalog
                .as_ref()
                .map(|old| {
                    require_original_catalog_copy(&old.current, &self.catalog)?;
                    if old.directory.identity != self.directory.identity {
                        return Err(denied());
                    }
                    let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
                    old._fee
                        .validate(&policy.current.archive.pending.pool, current.1.len())?;
                    Ok((
                        current.0.try_clone().map_err(io)?,
                        Zeroizing::new(current.1.to_vec()),
                    ))
                })
                .transpose()?;
            index.require_file(store)?;
            require_pending_names(store, index.pending.as_ref())?;
            opened.validate_original(store)?;
            registration.require(store)?;
            receiver.require(store)?;

            // Everything below is an infallible update of the original
            // owners under the same metadata cell. A revoked UI cannot
            // publish the row, but also cannot leave a half-installed owner.
            let retired_cold = index
                .cold_catalog
                .as_mut()
                .zip(replacement)
                .map(|(old, replacement)| std::mem::replace(&mut old.current, replacement));
            catalog.current = opened.selected;
            let retired_current = std::mem::replace(&mut self.catalog, current);
            let retired_registration = std::mem::replace(original, registration);
            *review = Some(receiver);
            drop(review);
            drop(index_slot);
            drop(catalog_slot);
            drop((retired_cold, retired_current, retired_registration));
            Ok(true)
        })();
        self.review_reopen_required = admitted && result.is_err();
        result.map_err(|error| {
            if admitted {
                AppError::ChatV2JoinReviewReopen
            } else {
                error
            }
        })
    }

    pub(in crate::store) fn capture_registration_dependency(
        &self,
        store: &ServerStore,
    ) -> Result<Arc<JoinedRegistrationDependency>, AppError> {
        let data = self.warm_registration_data(store)?;
        let guard = store.joined_pin_index.lock().map_err(|_| denied())?;
        let index = guard.as_ref().ok_or_else(denied)?;
        let pending = self.pending.as_ref().ok_or_else(denied)?;
        let (entry, outcome) = self
            .welcome
            .as_ref()
            .ok_or_else(denied)?
            .registration_files()?;
        let proof = JoinedRegistrationDependency::capture(
            store,
            data,
            &self.directory,
            [
                (&self.catalog.0, &self.catalog.1),
                (&index.selected.0, &index.selected.1),
                (&pending.file, &self.exact),
                entry,
                outcome,
            ],
        )?;
        drop(guard);
        if self.warm_registration_data(store)? != proof.data() {
            return Err(denied());
        }
        proof.require(store)?;
        Ok(proof)
    }
}

fn require_idle_catalog(
    catalog: &critical_namespaces::pending_join::Mounted,
    original_frame: [u8; 32],
) -> Result<(), AppError> {
    if catalog.current.framed != original_frame
        || catalog.pending.is_some()
        || catalog.networks.iter().any(Option::is_some)
    {
        return Err(denied());
    }
    Ok(())
}

impl ColdJoinedSelection {
    pub(in crate::store) fn capture_registration_dependency(
        &self,
        store: &ServerStore,
    ) -> Result<Arc<JoinedRegistrationDependency>, AppError> {
        self.require(store)?;
        let pending = self.index.pending.as_ref().ok_or_else(denied)?;
        let welcome = pending.welcome.as_ref().ok_or_else(denied)?;
        let data = welcome.registration_candidate_data().ok_or_else(denied)?;
        let (request, context) = pending._pending.pending_data();
        let data = NativeTextJoinedRegistrationData::from_selection(
            request.request_bytes_data(),
            context,
            pending._pending.transport_data()?.ok_or_else(denied)?,
            data,
        )?;
        let (entry, outcome) = welcome.registration_files()?;
        let proof = JoinedRegistrationDependency::capture(
            store,
            data,
            &self.directory,
            [
                (&self.current, &self.bytes),
                (&self.index.selected.0, &self.index.selected.1),
                (&pending.original.0, &pending.original.1),
                entry,
                outcome,
            ],
        )?;
        self.require(store)?;
        proof.require(store)?;
        Ok(proof)
    }
}
