//! Native's short canonical-owner borrow captures only finite original work.
//! All storage, source issuance, entry upgrades and runtime writers are deferred
//! until that native guard and its affine Sources borrow have left.
use super::*;
use std::sync::Weak;

/// Caller pays BEFORE borrowing native's original collection. The real runtime
/// metadata debit becomes the new entry's debit, or leaves with unused inputs.
/// Equal limits/configuration do not substitute for the original pool identity.
pub struct NativeTextCreationBudget<T, R> {
    metadata: Arc<RuntimeMetadata>,
    _types: PhantomData<fn() -> (T, R)>,
}

#[cfg(test)]
mod tests;
impl NativeTextSourcePolicy {
    pub fn prepare_creation<T: MeshTransport, R: CryptoRngCore>(
        &self,
    ) -> Result<NativeTextCreationBudget<T, R>, AppError> {
        if runtime_invoice::<T, R>().is_none_or(|size| size > ENTRY_BYTES) {
            return Err(refused());
        }
        Ok(NativeTextCreationBudget {
            metadata: Arc::new(RuntimeMetadata {
                pool: self.pool.clone(),
                _debit: self.pool.try_reserve(ENTRY_BYTES)?,
            }),
            _types: PhantomData,
        })
    }
}

/// Infallible owning capture, INCLUDING refusal. This contains no Sources,
/// Entry or source issuer. Never destroy even a refused capture under a native
/// guard: its caller inputs, UI metadata, payment or SourceWork may be last.
pub struct NativeTextCreationPreparation<T: MeshTransport, R: CryptoRngCore> {
    scope: Arc<Scope>,
    book: Weak<SourceBook<T, R>>,
    expected: NativeTextSourcePolicy,
    work: Option<SourceWork>,
    command: Option<NativeTextSourceCommand>,
    request: Option<NativeTextCreationRequest>,
    inputs: Option<NativeTextCreationInputs<R>>,
    budget: NativeTextCreationBudget<T, R>,
    refusal: Option<NativeTextCreationRefusal>,
    // LAST: no empty retirement snapshot may overtake real capture custody.
    _end: Option<retirement::PreparationEnd>,
    // Even an unadmitted/refused capture can own a Weak allocation tail. Keep
    // its original fixed CONTROL payment after that Weak and actual End.
    _control: Arc<retirement::Control>,
}
impl<T, R> std::fmt::Debug for NativeTextCreationBudget<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeTextCreationBudget { .. }")
    }
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for NativeTextCreationPreparation<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeTextCreationPreparation { .. }")
    }
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextSources<T, R> {
    /// Guard-safe, atomic/Arc-only capture. Admission is linearized against the
    /// SAME packed CLOSED+count used by retirement, before issuing SourceWork.
    /// No pool lock, book/Entry upgrade, IO, source issuance or SDK call here.
    pub fn capture_creation(
        &self,
        command: NativeTextSourceCommand,
        request: NativeTextCreationRequest,
        inputs: NativeTextCreationInputs<R>,
        budget: NativeTextCreationBudget<T, R>,
    ) -> NativeTextCreationPreparation<T, R> {
        let end = self.retirement.enter_preparation();
        let refusal = if !self.policy.pool.same_pool_cached(&budget.metadata.pool)
            || self.require_command(&command).is_err()
        {
            Some(NativeTextCreationRefusal::Unavailable)
        } else if end.is_none() {
            Some(NativeTextCreationRefusal::Busy)
        } else {
            None
        };
        let work = if refusal.is_none() {
            self.work.for_paid_runtime(&budget.metadata)
        } else {
            None
        };
        NativeTextCreationPreparation {
            scope: self.scope.clone(),
            book: Arc::downgrade(&self.entries),
            expected: self.policy.clone(),
            work,
            command: Some(command),
            request: Some(request),
            inputs: Some(inputs),
            budget,
            refusal,
            _end: end,
            _control: self.retirement.clone(),
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextCreationPreparation<T, R> {
    fn require_current(&self) -> Result<(), NativeTextCreationRefusal> {
        if let Some(refusal) = self.refusal {
            return Err(refusal);
        }
        let command = self
            .command
            .as_ref()
            .ok_or(NativeTextCreationRefusal::Unavailable)?;
        if self._end.is_none()
            || !Arc::ptr_eq(&self.scope, &command.scope)
            || !self.scope.is_open()
            || !command.ui.is_current_cached()
            || self.work.as_ref().is_none_or(|work| !work.is_live_cached())
        {
            return Err(NativeTextCreationRefusal::Unavailable);
        }
        Ok(())
    }

    /// Finish only OUTSIDE all native/Root guards. Temporary book/Entry/source
    /// issuer pins leave before either storage IO or the runtime writer. The
    /// resulting finite job has an installed original row and running loan,
    /// which cover the handoff before this preparation End can leave.
    pub fn finish(mut self) -> Result<NativeTextCreationJob<T, R>, NativeTextCreationRefusal> {
        use NativeTextCreationRefusal as Error;
        self.require_current()?;
        self.budget
            .metadata
            ._debit
            .validate(&self.expected.pool, ENTRY_BYTES)
            .map_err(|_| Error::Unavailable)?;
        let request = self.request.as_ref().ok_or(Error::Unavailable)?;
        let existing = {
            let book = self.book.upgrade().ok_or(Error::Unavailable)?;
            let entries = book.try_lock().map_err(|_| Error::Busy)?;
            entries
                .iter()
                .find(|entry| entry.invocation == request.invocation)
                .cloned()
        }; // no book/issuer may enter the no-entry IO branch below
        let mut candidate = None;
        let selected = if let Some(entry) = existing {
            if entry.request != request.digest {
                return Err(Error::Conflict);
            }
            entry
        } else {
            let work = self.work.as_ref().ok_or(Error::Unavailable)?;
            work.require_native_founder_configuration()
                .map_err(|_| Error::Unavailable)?;
            if work
                .lookup_creation(request.invocation, request.digest)
                .map_err(|_| Error::Storage)?
                .is_some()
            {
                return Err(Error::ExistingRequiresRestore);
            }
            self.require_current()?;
            // Issuing a source/candidate comes AFTER the exact lookup. A
            // candidate never exists during that storage read.
            let metadata = self.budget.metadata.clone();
            let owner = NativeChatSourceOwner::for_trusted_source(
                &self.command.as_ref().unwrap().ui.command,
                metadata.clone(),
            )
            .map_err(|_| Error::Unavailable)?;
            let request = self.request.take().unwrap();
            let inputs = self.inputs.take().unwrap();
            let entry = Arc::new(Entry {
                invocation: request.invocation,
                request: request.digest,
                owner,
                runtime: Arc::new(RuntimeCell {
                    state: Mutex::new(RuntimeSlot {
                        body: Some(Runtime {
                            founder_policy: self.expected.founder,
                            request: Some(request),
                            device_started: false,
                            driver: None,
                            device: None,
                            transport: None,
                            rng: Some(inputs.rng),
                            group_rng: Some(inputs.group_rng),
                            clock: Some(inputs.clock),
                            network: None,
                            work: self.work.take().unwrap(),
                            _metadata: metadata,
                        }),
                        running: false,
                        interrupted: false,
                        poisoned: false,
                        retiring: false,
                        retirement: retirement::BodyState::new(),
                    }),
                    returned: tokio::sync::Notify::new(),
                }),
            });
            candidate = Some(entry);
            let result = {
                let book = self.book.upgrade().ok_or(Error::Unavailable)?;
                let mut entries = book.try_lock().map_err(|_| Error::Busy)?;
                let current = candidate.as_ref().unwrap();
                if !self.scope.is_open() {
                    Err(Error::Unavailable)
                } else if let Some(old) = entries
                    .iter()
                    .find(|old| old.invocation == current.invocation)
                {
                    if old.request != current.request {
                        Err(Error::Conflict)
                    } else {
                        Ok(old.clone())
                    }
                } else if entries.len() >= self.expected.maximum {
                    Err(Error::Busy)
                } else {
                    entries.push(current.clone());
                    Ok(current.clone())
                }
            };
            result?
        };
        let prepared =
            NativeFounderKeys::prepare(&selected.owner, &self.command.as_ref().unwrap().ui.command);
        let cell = selected.runtime.clone();
        drop(selected);
        drop(candidate); // possible losing whole body, OUTSIDE book/native writers
        let keys = prepared.map_err(|_| Error::Unavailable)?;
        #[cfg(test)]
        retirement::tests::after_key_preparation();
        let body = {
            let mut state = cell.state.try_lock().map_err(|_| Error::Busy)?;
            if state.retiring || !self.scope.is_open() {
                return Err(Error::Unavailable);
            }
            if state.running || state.poisoned {
                return Err(Error::Busy);
            }
            let body = state.body.take().ok_or(Error::Unavailable)?;
            state.running = true;
            body
        };
        Ok(NativeTextCreationJob {
            loan: RuntimeLoan {
                body: Some(body),
                cell,
                recorded: false,
            },
            keys,
            command: self.command.take().unwrap(),
        })
    }
}
