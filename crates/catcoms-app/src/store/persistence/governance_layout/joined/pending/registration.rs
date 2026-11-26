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
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let bytes = original
            .iter()
            .try_fold(32 * 1024usize, |total, (_, raw)| {
                total.checked_add(raw.len()).ok_or_else(denied)
            })?;
        let fee = policy.current.archive.pending.pool.try_reserve(bytes)?;
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
}

impl JoinedPendingIo {
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
