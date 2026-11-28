//! Both-catalog/seed classification BEFORE every older gate. This handler accepts
//! only an already settled Current3 parent or its exact seeded Current4 successor.
use super::*;

pub(in crate::store) struct JoinedCatalogDescriptors<'a> {
    pub(in crate::store) layout: archive_current_namespace_codec::CurrentLayoutData,
    pub(in crate::store) governance:
        governance_namespace_codec::capability::GovernanceCapabilityData,
    pub(in crate::store) shared_history: Option<&'a [u8]>,
    pub(in crate::store) review_hold: Option<&'a [u8]>,
    pub(in crate::store) review_decision: Option<&'a [u8]>,
    pub(in crate::store) review_receive: Option<&'a [u8]>,
    pub(in crate::store) rejoin_declared: bool,
}

fn summary(
    store: &ServerStore,
    key: &[u8; 32],
    raw: &[u8],
) -> Result<pending_join_codec::Summary, AppError> {
    let plain = Zeroizing::new(unseal(key, &unframe(raw)?)?);
    let parsed = if !store.uses_joined_pin_layout()
        && store.governance_layout_policy.as_ref().is_some_and(|p| {
            p.family == governance_namespace_codec::GovernanceCatalogFamily::NativeLifecycle
        }) {
        pending_join_codec::parse_native_governance_family(
            &plain,
            &store.chat_vault_domain()?,
            true,
        )?
    } else {
        store.parse_configured_join_catalog(&plain)?
    };
    Ok(parsed.summary(*blake3::hash(raw).as_bytes()))
}

impl ServerStore {
    /// Classify the bound warm catalog without adopting any physical object.
    pub(in crate::store) fn founder_review_catalog_step(
        &self,
        kind: super::review_hold_namespace::declaration::ReviewDeclarationKind,
    ) -> Result<Option<([u8; 32], bool)>, AppError> {
        use super::review_hold_namespace::declaration::ReviewDeclarationKind;
        let slot = self
            .pending_join_catalog
            .try_lock()
            .map_err(|error| match error {
                std::sync::TryLockError::WouldBlock => AppError::ChatV2RegistryBusy,
                std::sync::TryLockError::Poisoned(_) => refused(),
            })?;
        let mounted = slot.as_ref().ok_or_else(refused)?;
        let current = &mounted.current;
        if mounted.pending.is_some()
            || mounted.networks.iter().any(Option::is_some)
            || current.rejoin_payload_digest.is_some()
            || current.review_receive_payload_digest.is_some()
        {
            return Err(refused());
        }
        if current.shared_history_payload_digest.is_none() {
            return Ok(None);
        }
        let hold = current.review_hold_payload_digest.is_some();
        let decision = current.review_decision_payload_digest.is_some();
        if (decision || kind == ReviewDeclarationKind::Decision) && !hold {
            return Err(refused());
        }
        Ok(Some((
            current.framed,
            match kind {
                ReviewDeclarationKind::Hold => hold,
                ReviewDeclarationKind::Decision => decision,
            },
        )))
    }

    /// Parse the original declaration's exact bounded candidate under this
    /// mount's configured grammar. The caller owns the read invoice and
    /// must still prove same-inode selection before adopting the summary.
    pub(in crate::store) fn prepare_warm_review_catalog_summary(
        &self,
        key: &[u8; 32],
        candidate: &[u8],
    ) -> Result<pending_join_codec::Summary, AppError> {
        summary(self, key, candidate)
    }

    /// The authenticated original Current already declares the journal.
    /// A damaged optional mount may not acquire a new declaration.
    pub(in crate::store) fn has_declared_review_decision(&self) -> Result<bool, AppError> {
        let catalog = self
            .pending_join_catalog
            .try_lock()
            .map_err(|_| refused())?;
        let selected = catalog.as_ref().ok_or_else(refused)?;
        Ok(selected.current.review_decision_payload_digest.is_some())
    }

    pub(in crate::store) fn parse_joined_pin_catalog(
        &self,
        plain: &[u8],
    ) -> Result<archive_current_namespace_codec::CurrentLayoutData, AppError> {
        Ok(self.parse_joined_catalog_descriptors(plain)?.layout)
    }

    pub(in crate::store) fn parse_joined_catalog_descriptors<'a>(
        &self,
        plain: &'a [u8],
    ) -> Result<JoinedCatalogDescriptors<'a>, AppError> {
        if !self.uses_joined_pin_layout() {
            return Err(refused());
        }
        let parsed = self.parse_configured_join_catalog(plain)?;
        Ok(JoinedCatalogDescriptors {
            layout: parsed.current_layout.ok_or_else(refused)?,
            governance: parsed.governance.ok_or_else(refused)?,
            shared_history: parsed.shared_history_payload,
            review_hold: parsed.review_hold_payload,
            review_decision: parsed.review_decision_payload,
            review_receive: parsed.review_receive_payload,
            rejoin_declared: parsed.rejoin_payload.is_some(),
        })
    }

    pub(in crate::store) fn prepare_governance_layout_declaration(
        &self,
        fee: &CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Declaration, AppError> {
        self.prepare_governance_layout_declaration_with_native(fee, rng, &mut None)
    }
    pub(in crate::store) fn prepare_governance_layout_declaration_with_native(
        &self,
        fee: &CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
        native_slot: &mut Option<
            super::super::native_registry::lifecycle::VerifiedGovernanceNative,
        >,
    ) -> Result<Declaration, AppError> {
        let policy = self.governance_layout_policy.as_ref().ok_or_else(refused)?;
        match (policy.family, native_slot.as_ref()) {
            (governance_namespace_codec::GovernanceCatalogFamily::Standard, None) => {}
            (
                governance_namespace_codec::GovernanceCatalogFamily::NativeLifecycle,
                Some(native),
            ) => native.require_initial_catalog(self)?,
            _ => return Err(refused()),
        }
        fee.validate(&policy.current.archive.pending.pool, codec::DECLARATION_FEE)?;
        policy.require_persistent_capacity()?;
        let inventory = self.read_governance_inventory(fee)?;
        if inventory.stage.is_some()
            || inventory.seed.is_some()
            || inventory.marker.is_some()
            || self.chat_mount.observer().is_cancelled()
        {
            return Err(refused());
        }
        let (initial_current, original) = inventory.current.ok_or_else(refused)?;
        let initial_id = catcoms_storage::kept::session_id(&initial_current)
            .map_err(|e| AppError::Io(e.to_string()))?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let old_plain = Zeroizing::new(unseal(&key, &unframe(&original)?)?);
        let before = self
            .parse_configured_join_catalog(&old_plain)?
            .summary(*blake3::hash(&original).as_bytes());
        if before.governance.is_some() {
            return Err(refused());
        }
        let capability = match policy.family {
            governance_namespace_codec::GovernanceCatalogFamily::Standard => {
                GovernanceCapabilityData::from_original_catalog_data(
                    &old_plain,
                    &self.chat_vault_domain()?,
                )?
            }
            governance_namespace_codec::GovernanceCatalogFamily::NativeLifecycle => {
                GovernanceCapabilityData::from_family_original_catalog_data(
                    &old_plain,
                    &self.chat_vault_domain()?,
                    policy.family,
                )?
            }
        };
        let layout = capability.current_layout_data();
        self.check_current_marker_before_old_gates(layout, fee)?;
        {
            let mounted = self.pending_join_catalog.lock().map_err(|_| refused())?;
            let mounted = mounted.as_ref().ok_or_else(refused)?;
            if mounted.pending.is_some() || mounted.current.framed != before.framed {
                return Err(refused());
            }
        }
        let next_plain = match policy.family {
            governance_namespace_codec::GovernanceCatalogFamily::Standard => {
                governance_namespace_codec::prepare_declaration_data(&old_plain, &layout.vault)?
            }
            governance_namespace_codec::GovernanceCatalogFamily::NativeLifecycle => {
                governance_namespace_codec::prepare_family_declaration_data(
                    &old_plain,
                    &layout.vault,
                    policy.family,
                )?
            }
        };
        let mut next = self
            .parse_configured_join_catalog(&next_plain)?
            .summary([0; 32]);
        // An existing member-outbox ID5 is carried over byte for byte (the codec inserts
        // 4 before it); this second, Summary-level check never lets 4 alter or drop it.
        if next.governance != Some(capability)
            || next.member_outbox != before.member_outbox
            || next.member_outbox_payload_digest != before.member_outbox_payload_digest
        {
            return Err(refused());
        }
        self.require_no_prior_governance_declaration_debts()?;
        self.validate_mounted_current_layout()?;
        // No file write precedes the complete retained candidate. Later retries
        // use exactly these nonce/ciphertexts, never another random preparation.
        let candidate = Zeroizing::new(frame(&seal(&key, &next_plain, rng)?));
        next.framed = *blake3::hash(&candidate).as_bytes();
        let marker = codec::seal_marker(&key, capability, rng)?;
        let seed = match native_slot.as_ref() {
            Some(native) => codec::seal_native_seed(
                &key,
                &layout.vault,
                &original,
                &candidate,
                &marker,
                native.data(),
                rng,
            )?,
            None => codec::seal_seed(&key, &layout.vault, &original, &candidate, &marker, rng)?,
        };
        let initial_current_frame = before.framed;
        Ok(Declaration {
            capability,
            original,
            before,
            candidate,
            next,
            marker,
            seed,
            directory: inventory.directory,
            initial_current: pending_join::CreatedStage {
                file: initial_current,
                identity: Some(initial_id),
            },
            initial_current_frame,
            seed_file: None,
            stage_file: None,
            captured_duplicate_stage: None,
            marker_file: None,
            native: native_slot.take(),
            #[cfg(test)]
            _tail_drop: DeclarationTailDrop,
        })
    }

    pub(in crate::store) fn open_governance_layout_catalog_gate(
        &self,
        actual: VaultFormat,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        if actual != VaultFormat::CriticalNamespaces {
            return Err(refused());
        }
        let joined = if self.uses_joined_pin_layout() {
            Some(self.select_joined_pin_catalog()?)
        } else {
            None
        };
        // An in-progress namespace5 declaration owns the catalog stage slot, which the
        // governance inspector would refuse. With the concrete member handler configured,
        // finish it, abandon a first seed that never authenticated, or preserve and refuse it
        // first; the Current3 gate below re-inspects and installs its marker. Both seeds at
        // once are conflicting debts, except that an unauthenticated member seed beside a
        // governance seed (a predecessor's governance write during the claim window) is
        // abandoned without touching the stage, which is governance's own.
        if self.member_outbox_policy().is_some() && self.member_outbox_seed_present()? {
            drop(self.inspect_and_repair_member_outbox_declaration()?);
        }
        // The large4 transient/repair fee is gone before the old3/2 gates reserve
        // their own large phases. An exact captured marker keeps only its OWN fee.
        let captured = self.inspect_and_repair_governance_declaration()?;
        #[cfg(test)]
        if captured.is_some() {
            after_marker_capture_for_test();
        }
        self.require_no_prior_governance_declaration_debts()?;
        if matches!(self.current_layout_policy.as_ref().and_then(|p|p.native_registry.as_ref()),
            Some(super::super::native_registry::preparation::NativeRegistryHandler::GovernanceOpening(_)))
        {
            // Seed4 repair/abort finished above. Hold native3's actual final
            // catalog and leaves through the COMMON Current/JOIN opener; its
            // one observation is consumed only by open_native afterward.
            self.inspect_and_repair_native_registry_with_joined(joined.as_ref())?;
        }
        self.open_current_layout_catalog_gate(actual, passphrase, rng)?;
        if let Some(captured) = captured {
            self.install_governance_layout_marker(captured)?;
        }
        if let Some(joined) = joined {
            self.retain_joined_pin_selection(joined)?;
        }
        Ok(())
    }

    fn inspect_and_repair_governance_declaration(
        &self,
    ) -> Result<Option<GovernanceMarkerCapture>, AppError> {
        let policy = self.governance_layout_policy.as_ref().ok_or_else(refused)?;
        policy.require_persistent_capacity()?;
        let fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(codec::DECLARATION_FEE)?;
        let inventory = self.read_governance_inventory(&fee)?;
        let (_current_file, current_raw) = inventory.current.as_ref().ok_or_else(refused)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let current = summary(self, &key, current_raw)?;
        let layout = current.current_layout.ok_or_else(refused)?;
        let archive = current.archive.as_ref().ok_or_else(refused)?;
        if policy.family == governance_namespace_codec::GovernanceCatalogFamily::NativeLifecycle {
            if let Some((_, raw)) = inventory.seed.as_ref() {
                let torn = match codec::open_native_seed(&key, raw) {
                    Ok(_) => false,
                    Err(codec::NativeSeedReadError::Unauthenticated(_)) => true,
                    Err(codec::NativeSeedReadError::Indeterminate(_)) if raw.len() < 40 => true,
                    Err(e) => return Err(e.into_error()),
                };
                if torn {
                    let native_inventory=crate::store::persistence::native_registry::lifecycle::governed::NativeGovernanceInventory::capture(self,policy.native_pool.as_ref().ok_or_else(refused)?)?;
                    let proof = super::super::native_registry::lifecycle::verify_governance_native(
                        self,
                        native_inventory,
                        current_raw,
                        None,
                    )?;
                    self.cleanup_native_governance_torn(inventory, proof)?;
                    // Cleanup grants no4: common original3 opening and the same
                    // owning preparation must complete before a fresh declaration.
                    return Ok(None);
                }
            }
        }
        if let Some((seed_file, seed_raw)) = inventory.seed {
            let plain = policy.open_seed(&key, &seed_raw)?;
            let saved = policy.parse_seed(&key, &layout.vault, &plain)?;
            let before = summary(self, &key, saved.original)?;
            let next = summary(self, &key, saved.candidate)?;
            if before.governance.is_some()
                || next.governance != Some(saved.capability)
                || before.current_layout != Some(layout)
                || next.current_layout != Some(layout)
                || before.member_outbox != next.member_outbox
                || before.member_outbox_payload_digest != next.member_outbox_payload_digest
            {
                return Err(refused());
            }
            let promoted = current_raw.as_slice() == saved.candidate;
            if !promoted && current_raw.as_slice() != saved.original {
                return Err(refused());
            }
            let native = match saved.native {
                Some(data) => {
                    let actual=crate::store::persistence::native_registry::lifecycle::governed::NativeGovernanceInventory::capture(self,policy.native_pool.as_ref().ok_or_else(refused)?)?;
                    Some(
                        super::super::native_registry::lifecycle::verify_governance_native(
                            self,
                            actual,
                            saved.original,
                            Some(data),
                        )?,
                    )
                }
                None => None,
            };
            let stage_file = if let Some((file, stage)) = inventory.stage {
                if stage.len() > saved.candidate.len()
                    || saved.candidate[..stage.len()] != stage[..]
                    || (promoted && stage.as_slice() != saved.candidate)
                {
                    return Err(refused());
                }
                Some(pending_join::CreatedStage {
                    identity: Some(
                        catcoms_storage::kept::session_id(&file)
                            .map_err(|e| AppError::Io(e.to_string()))?,
                    ),
                    file,
                })
            } else {
                None
            };
            let marker_file = match inventory.marker {
                Some((file, marker)) => {
                    if marker.len() > saved.marker.len()
                        || saved.marker[..marker.len()] != marker[..]
                        || (promoted && marker.as_slice() != saved.marker)
                    {
                        return Err(refused());
                    }
                    Some(pending_join::CreatedStage {
                        identity: Some(
                            catcoms_storage::kept::session_id(&file)
                                .map_err(|e| AppError::Io(e.to_string()))?,
                        ),
                        file,
                    })
                }
                None if promoted => return Err(refused()),
                None => None,
            };
            let (stage_file, captured_duplicate_stage) = if promoted {
                (None, stage_file)
            } else {
                (stage_file, None)
            };
            #[cfg(test)]
            after_declaration_capture_for_test();
            // BOTH fixed candidates and the complete compound source classified
            // before the first old marker sync or metadata installation.
            self.mount_settled_governance_parent(layout, archive, &inventory.directory, &fee)?;
            let identity = catcoms_storage::kept::session_id(&seed_file)
                .map_err(|e| AppError::Io(e.to_string()))?;
            let initial_current_frame = *blake3::hash(current_raw).as_bytes();
            let (initial_current, _) = inventory.current.ok_or_else(refused)?;
            let initial_id = catcoms_storage::kept::session_id(&initial_current)
                .map_err(|e| AppError::Io(e.to_string()))?;
            let mut declaration = Declaration {
                capability: saved.capability,
                original: Zeroizing::new(saved.original.to_vec()),
                before,
                candidate: Zeroizing::new(saved.candidate.to_vec()),
                next,
                marker: Zeroizing::new(saved.marker.to_vec()),
                seed: seed_raw,
                directory: inventory.directory,
                initial_current: pending_join::CreatedStage {
                    file: initial_current,
                    identity: Some(initial_id),
                },
                initial_current_frame,
                seed_file: Some(pending_join::CreatedStage {
                    file: seed_file,
                    identity: Some(identity),
                }),
                stage_file,
                captured_duplicate_stage,
                marker_file,
                native,
                #[cfg(test)]
                _tail_drop: DeclarationTailDrop,
            };
            drop(plain);
            self.write_governance_layout_declaration(&mut declaration, &fee)?;
            let captured = self.capture_governance_layout_marker(
                declaration.capability,
                declaration.marker_file.as_ref().ok_or_else(refused)?,
            )?;
            return Ok(Some(captured));
        }
        // Without an independently authenticated seed, even a valid new4 stage
        // cannot declare4. Existing4 ordinary JOIN stages preserve immutable4.
        match current.governance {
            None => {
                if inventory.stage.is_some() || inventory.marker.is_some() {
                    return Err(refused());
                }
                self.mount_settled_governance_parent(layout, archive, &inventory.directory, &fee)?;
                Ok(None)
            }
            Some(capability) => {
                if let Some((_, raw)) = inventory.stage {
                    let staged = summary(self, &key, &raw)?;
                    if staged.governance != Some(capability)
                        || staged.governance_payload_digest != current.governance_payload_digest
                    {
                        return Err(refused());
                    }
                    if staged.digest != current.digest {
                        pending_join_codec::successor(&current, &staged)?;
                    }
                }
                let (marker, raw) = inventory.marker.ok_or_else(refused)?;
                codec::require_marker(&key, capability, &raw)?;
                // Capture the SAME probed object across later old-gate waits. This
                // never creates/reinitializes a marker or renews a source lifetime.
                self.mount_settled_governance_parent(layout, archive, &inventory.directory, &fee)?;
                let captured = self.retain_governance_marker_capture(
                    capability,
                    marker,
                    raw,
                    inventory.directory,
                )?;
                Ok(Some(captured))
            }
        }
    }

    fn mount_settled_governance_parent(
        &self,
        layout: archive_current_namespace_codec::CurrentLayoutData,
        archive: &archive_bootstrap::Descriptor,
        directory: &GovernanceDirectoryCapture,
        fee: &CatalogReadReservation,
    ) -> Result<(), AppError> {
        // No ID2-only/new-vault creation path: the original root AND marker3
        // must already exist. All old declaration debts were inventoried first.
        self.mount_existing_current_declaration_root(archive)?;
        self.require_governance_directory_matches_root(directory)?;
        self.check_current_marker_before_old_gates(layout, fee)?;
        self.install_current_layout_marker(layout)?;
        self.require_governance_directory_matches_root(directory)
    }
}
