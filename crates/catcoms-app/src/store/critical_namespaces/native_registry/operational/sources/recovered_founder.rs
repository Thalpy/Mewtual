//! Recovered role in the original native Source book.
use super::*;
use catcoms_rt::RequestCancellation;

impl<R: CryptoRngCore> NativeTextSources<catcoms_net::MeshService, R> {
    pub(in super::super) fn install_recovered_founder(
        &mut self,
        vault: &NativeTextVault,
        authentication: &crate::store::NativeTextVaultAuthenticationCheck,
        founder: crate::store::NativeTextColdFounderSource,
        inputs: NativeTextCreationInputs<R>,
        caller: &RequestCancellation,
    ) -> Result<NativeTextRuntimeSelection<catcoms_net::MeshService, R>, AppError> {
        let require_current = || {
            if self.scope.is_open()
                && Arc::ptr_eq(&self.scope, &vault.sources.observe())
                && authentication.is_for_original_vault(vault)
                && authentication.is_live_cached()
                && !caller.is_cancelled()
            {
                Ok(())
            } else {
                Err(refused())
            }
        };
        require_current()?;
        let (status, invocation, request) = founder.runtime_labels();
        if self.work.lookup_creation(invocation, request)? != Some(status)
            || runtime_invoice::<catcoms_net::MeshService, R>().is_none_or(|n| n > ENTRY_BYTES)
        {
            return Err(refused());
        }
        let metadata = Arc::new(RuntimeMetadata {
            pool: self.policy.pool.clone(),
            _debit: self.policy.pool.try_reserve(ENTRY_BYTES)?,
        });
        let work = self.work.for_paid_runtime(&metadata).ok_or_else(refused)?;
        let (join_ingress, member_ingress) = founder.runtime_ingress()?;
        let dependency_lane = founder.runtime_dependency_lane()?;
        let owner = NativeChatSourceOwner::for_trusted_authenticated_mount(
            &vault.observe_ui_mount(),
            metadata.clone(),
        )
        .map_err(|_| refused())?;
        let source_close = owner.close_handle();
        let entry = Arc::new(Entry {
            invocation,
            request,
            owner,
            runtime: Arc::new(RuntimeCell {
                state: Mutex::new(RuntimeSlot {
                    body: Some(Runtime {
                        founder_policy: self.policy.founder,
                        request: None,
                        device_started: false,
                        driver: None,
                        recovered_founder: Some(Box::new(founder)),
                        device: None,
                        transport: None,
                        rng: Some(inputs.rng),
                        group_rng: Some(inputs.group_rng),
                        clock: Some(inputs.clock),
                        network: None,
                        selection: None,
                        source_close: source_close.clone(),
                        work,
                        _metadata: metadata.clone(),
                    }),
                    running: false,
                    interrupted: false,
                    poisoned: false,
                    retiring: false,
                    retirement: retirement::BodyState::new(),
                }),
                returned: tokio::sync::Notify::new(),
                #[cfg(test)]
                tail_probe: Mutex::new(None),
            }),
        });
        let selected = NativeTextRuntimeSelection {
            original: Arc::new(selection::Selection {
                scope: self.scope.clone(),
                runtime: Arc::downgrade(&entry.runtime),
                status,
                invocation,
                request,
                join_ingress: Some(join_ingress),
                member_ingress: Some(member_ingress),
                dependency_lane: Some(dependency_lane),
                source_close,
                recovered_founder: true,
                _metadata: metadata,
            }),
        };
        {
            let mut slot = entry.runtime.state.try_lock().map_err(|_| refused())?;
            slot.body.as_mut().ok_or_else(refused)?.selection = Some(selected.clone());
        }
        {
            let mut entries = self.entries.try_lock().map_err(|_| refused())?;
            if !entries.is_empty() {
                return Err(refused());
            }
            require_current()?;
            entries.push(entry);
        }
        Ok(selected)
    }
}
