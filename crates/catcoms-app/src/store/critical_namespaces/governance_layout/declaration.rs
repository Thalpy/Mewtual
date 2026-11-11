//! Both-catalog/seed classification BEFORE every older gate. This handler accepts
//! only an already settled Current3 parent or its exact seeded Current4 successor.
use super::*;

fn summary(
    store: &ServerStore,
    key: &[u8; 32],
    raw: &[u8],
) -> Result<pending_join_codec::Summary, AppError> {
    let plain = Zeroizing::new(unseal(key, &unframe(raw)?)?);
    Ok(store
        .parse_configured_join_catalog(&plain)?
        .summary(*blake3::hash(raw).as_bytes()))
}

impl ServerStore {
    pub(in crate::store) fn prepare_governance_layout_declaration(
        &self,
        fee: &CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Declaration, AppError> {
        let policy = self.governance_layout_policy.as_ref().ok_or_else(refused)?;
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
        let capability = GovernanceCapabilityData::from_original_catalog_data(
            &old_plain,
            &self.chat_vault_domain()?,
        )?;
        let layout = capability.current_layout_data();
        self.check_current_marker_before_old_gates(layout, fee)?;
        {
            let mounted = self.pending_join_catalog.lock().map_err(|_| refused())?;
            let mounted = mounted.as_ref().ok_or_else(refused)?;
            if mounted.pending.is_some() || mounted.current.framed != before.framed {
                return Err(refused());
            }
        }
        let next_plain =
            governance_namespace_codec::prepare_declaration_data(&old_plain, &layout.vault)?;
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
        let seed = codec::seal_seed(&key, &layout.vault, &original, &candidate, &marker, rng)?;
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
        // The large4 transient/repair fee is gone before the old3/2 gates reserve
        // their own large phases. An exact captured marker keeps only its OWN fee.
        let captured = self.inspect_and_repair_governance_declaration()?;
        #[cfg(test)]
        if captured.is_some() {
            after_marker_capture_for_test();
        }
        self.require_no_prior_governance_declaration_debts()?;
        self.open_current_layout_catalog_gate(actual, passphrase, rng)?;
        if let Some(captured) = captured {
            self.install_governance_layout_marker(captured)?;
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
        if let Some((seed_file, seed_raw)) = inventory.seed {
            let plain = codec::open_seed(&key, &seed_raw)?;
            let saved = codec::parse_seed(&key, &layout.vault, &plain)?;
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
