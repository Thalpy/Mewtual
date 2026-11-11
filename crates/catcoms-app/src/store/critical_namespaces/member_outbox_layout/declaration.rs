//! Namespace5 declaration: warm preparation, the shared wrapper retry step, and the cold
//! both-catalog/seed inspector that runs BEFORE every older gate or cleaner.
//!
//! The handler accepts an already settled Current3 parent (optionally with governance4) or
//! its exact seeded successor. It never requires, creates or implies governance4.
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

/// Governance4, when present, must be carried byte for byte by a namespace5 declaration.
fn same_governance(
    before: &pending_join_codec::Summary,
    next: &pending_join_codec::Summary,
) -> bool {
    before.governance == next.governance
        && before.governance_payload_digest == next.governance_payload_digest
}

fn stage_of(file: File) -> Result<pending_join::CreatedStage, AppError> {
    Ok(pending_join::CreatedStage {
        identity: Some(
            catcoms_storage::kept::session_id(&file).map_err(|e| AppError::Io(e.to_string()))?,
        ),
        file,
    })
}

impl ServerStore {
    pub(in crate::store) fn prepare_member_outbox_layout_declaration(
        &self,
        fee: &CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Declaration, AppError> {
        let policy = self.member_outbox_policy().ok_or_else(refused)?;
        policy.require_capacity()?;
        fee.validate(
            self.member_outbox_declaration_pool()?,
            codec::DECLARATION_FEE,
        )?;
        let inventory = self.read_member_outbox_inventory(fee)?;
        if inventory.stage.is_some()
            || inventory.seed.is_some()
            || inventory.marker.is_some()
            || inventory.parent.is_some()
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
        if before.member_outbox.is_some() {
            return Err(refused());
        }
        let capability = MemberOutboxCapabilityData::from_original_catalog_data(
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
            member_outbox_namespace_codec::prepare_declaration_data(&old_plain, &layout.vault)?;
        let mut next = self
            .parse_configured_join_catalog(&next_plain)?
            .summary([0; 32]);
        if next.member_outbox != Some(capability) || !same_governance(&before, &next) {
            return Err(refused());
        }
        self.require_no_prior_member_outbox_declaration_debts()?;
        self.validate_mounted_current_layout()?;
        // No file write precedes the complete retained candidate. Later retries use exactly
        // these nonce/ciphertexts, never another random preparation.
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
            root: inventory.root,
            initial_current: pending_join::CreatedStage {
                file: initial_current,
                identity: Some(initial_id),
            },
            initial_current_frame,
            seed_file: None,
            stage_file: None,
            captured_duplicate_stage: None,
            marker_file: None,
            parent: None,
        })
    }

    /// The opener wrapper's step: declare and mount namespace5 when this opener carries the
    /// concrete handler and it is not mounted yet. The large declaration fee is reserved only
    /// here (after the caller has drained its own declaration debt) and both owning slots
    /// stay with the caller across failed retries, so no RNG/reseal follows a cached
    /// declaration. On success both slots are drained outside every writer.
    pub(in crate::store) fn retry_member_outbox_layout_declaration(
        &self,
        declaration: &mut Option<Box<Declaration>>,
        declaration_fee: &mut Option<CatalogReadReservation>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let Some(policy) = self.member_outbox_policy() else {
            return Ok(());
        };
        policy.require_capacity()?;
        if self.chat_mount.observer().is_cancelled() {
            return Err(refused());
        }
        if self
            .member_outbox_layout
            .lock()
            .map_err(|_| refused())?
            .is_none()
        {
            if declaration_fee.is_none() {
                *declaration_fee = Some(
                    self.member_outbox_declaration_pool()?
                        .try_reserve(codec::DECLARATION_FEE)?,
                );
            }
            if declaration.is_none() {
                let prepared = self.prepare_member_outbox_layout_declaration(
                    declaration_fee.as_ref().ok_or_else(refused)?,
                    rng,
                )?;
                *declaration = Some(Box::new(prepared));
            }
            let owned = declaration.as_mut().ok_or_else(refused)?;
            self.write_member_outbox_layout_declaration(
                owned,
                declaration_fee.as_ref().ok_or_else(refused)?,
            )?;
            // The durable catalog now IS the candidate: move the in-memory summary first, so a
            // later capture/install failure cannot leave it stale once the mount exists.
            {
                let mut current = self.pending_join_catalog.lock().map_err(|_| refused())?;
                let current = current.as_mut().ok_or_else(refused)?;
                if current.pending.is_some()
                    || (current.current.framed != owned.before.framed
                        && current.current.framed != owned.next.framed)
                {
                    return Err(refused());
                }
                current.current = owned.next.clone();
            }
            // The parent capture moves into the marker capture. If that fails, a retry
            // re-proves the existing directory; it never re-creates it because the catalog
            // is already promoted.
            let parent = owned.parent.take().ok_or_else(refused)?;
            let marker = self.capture_member_outbox_layout_marker(
                owned.capability,
                owned.marker_file.as_ref().ok_or_else(refused)?,
                parent,
            )?;
            self.install_member_outbox_layout_marker(marker)?;
        }
        self.validate_mounted_member_outbox_layout()?;
        // Exact selection/marker/parent barriers are complete. The large original/candidate/
        // seed copies drop outside all writers BEFORE any later funded phase.
        drop(declaration.take());
        drop(declaration_fee.take());
        self.validate_mounted_member_outbox_layout()
    }

    /// Opener gate step for an opener WITH the concrete handler, run before every older
    /// inspector/cleaner. Returns the captured marker to install after the older gates, or
    /// `None` when namespace5 is not declared. Unknown, partial or conflicting state is
    /// preserved and refused.
    pub(in crate::store) fn inspect_and_repair_member_outbox_declaration(
        &self,
    ) -> Result<Option<MemberOutboxMarkerCapture>, AppError> {
        let policy = self.member_outbox_policy().ok_or_else(refused)?;
        policy.require_capacity()?;
        let fee = self
            .member_outbox_declaration_pool()?
            .try_reserve(codec::DECLARATION_FEE)?;
        if refuse_unsupported_storage(&self.dir).is_ok() {
            // No seed, marker or parent of any kind: namespace5 can only be undeclared. This
            // is also the path for a vault still below Current3 (no `chat-owner-v2` root or
            // ID3 yet), which must not be refused merely because the handler is configured.
            // A catalog that nevertheless names ID5 lost its marker: preserved and refused.
            if !self.member_outbox_undeclared_current(&fee)? {
                return Err(refused());
            }
            return Ok(None);
        }
        let inventory = self.read_member_outbox_inventory(&fee)?;
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
            if before.member_outbox.is_some()
                || next.member_outbox != Some(saved.capability)
                || before.current_layout != Some(layout)
                || next.current_layout != Some(layout)
                || !same_governance(&before, &next)
            {
                return Err(refused());
            }
            let promoted = current_raw.as_slice() == saved.candidate;
            if !promoted && current_raw.as_slice() != saved.original {
                return Err(refused());
            }
            let stage_file = match inventory.stage {
                Some((file, stage)) => {
                    if stage.len() > saved.candidate.len()
                        || saved.candidate[..stage.len()] != stage[..]
                        || (promoted && stage.as_slice() != saved.candidate)
                    {
                        return Err(refused());
                    }
                    Some(stage_of(file)?)
                }
                None => None,
            };
            let mut marker_complete = false;
            let marker_file = match inventory.marker {
                Some((file, marker)) => {
                    if marker.len() > saved.marker.len()
                        || saved.marker[..marker.len()] != marker[..]
                        || (promoted && marker.as_slice() != saved.marker)
                    {
                        return Err(refused());
                    }
                    marker_complete = marker.as_slice() == saved.marker;
                    Some(stage_of(file)?)
                }
                None if promoted => return Err(refused()),
                None => None,
            };
            // The parent is created only after a complete marker, and after the commit point
            // it must already exist. Any other combination is not this declaration's state.
            if (promoted && inventory.parent.is_none())
                || (inventory.parent.is_some() && !marker_complete)
            {
                return Err(refused());
            }
            // While a seed exists no outbox namespace can have been created, so a non-empty
            // parent (for example a seed replayed after the outbox was used) is refused here,
            // BEFORE any recovery write touches the stage, seed or marker.
            if inventory.parent.is_some() && !self.member_outbox_parent_is_empty()? {
                return Err(refused());
            }
            let (stage_file, captured_duplicate_stage) = if promoted {
                (None, stage_file)
            } else {
                (stage_file, None)
            };
            // BOTH fixed candidates, the complete compound source, the marker and the parent
            // were classified before the first old marker sync or metadata installation.
            self.mount_settled_member_outbox_parent(layout, archive, &inventory.root, &fee)?;
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
                root: inventory.root,
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
                parent: inventory.parent,
            };
            drop(plain);
            self.write_member_outbox_layout_declaration(&mut declaration, &fee)?;
            let parent = declaration.parent.take().ok_or_else(refused)?;
            let captured = self.capture_member_outbox_layout_marker(
                declaration.capability,
                declaration.marker_file.as_ref().ok_or_else(refused)?,
                parent,
            )?;
            return Ok(Some(captured));
        }
        // Without an independently authenticated seed, even a complete valid ID5 stage cannot
        // declare namespace5. Existing-5 ordinary JOIN stages must preserve it exactly.
        match current.member_outbox {
            None => {
                // Orphan member artifacts without a declared namespace or a seed are preserved
                // and refused. A stage is left to the older inspectors, which already refuse
                // anything that is not an authenticated valid ordinary successor (including
                // an empty legacy fence left by an interrupted first seed).
                if inventory.marker.is_some() || inventory.parent.is_some() {
                    return Err(refused());
                }
                Ok(None)
            }
            Some(capability) => {
                if let Some((_, raw)) = inventory.stage {
                    let staged = summary(self, &key, &raw)?;
                    if staged.member_outbox != Some(capability)
                        || staged.member_outbox_payload_digest
                            != current.member_outbox_payload_digest
                    {
                        return Err(refused());
                    }
                    if staged.digest != current.digest {
                        pending_join_codec::successor(&current, &staged)?;
                    }
                }
                let (marker, raw) = inventory.marker.ok_or_else(refused)?;
                codec::require_marker(&key, capability, &raw)?;
                let parent = inventory.parent.ok_or_else(refused)?;
                // Capture the SAME probed objects across later old-gate waits. This never
                // creates/reinitializes a marker or directory.
                self.mount_settled_member_outbox_parent(layout, archive, &inventory.root, &fee)?;
                let captured = self.retain_member_outbox_marker_capture(
                    capability,
                    marker,
                    raw,
                    inventory.root,
                    parent,
                )?;
                Ok(Some(captured))
            }
        }
    }

    /// Whether the authenticated current catalog does NOT name namespace5. Used only when no
    /// member artifact exists. A missing catalog or any decode failure is an error, never
    /// "undeclared". Reads through the same fixed-slot reader as the ID3 inspector; the
    /// member declaration fee is on that same pool and covers its read invoice.
    fn member_outbox_undeclared_current(
        &self,
        fee: &CatalogReadReservation,
    ) -> Result<bool, AppError> {
        let (_file, raw) = self
            .open_pending_join_catalog_slot(Slot::Current, fee)?
            .ok_or_else(refused)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let plain = Zeroizing::new(unseal(&key, &unframe(&raw)?)?);
        Ok(self
            .parse_configured_join_catalog(&plain)?
            .member_outbox
            .is_none())
    }

    fn mount_settled_member_outbox_parent(
        &self,
        layout: archive_current_namespace_codec::CurrentLayoutData,
        archive: &archive_bootstrap::Descriptor,
        root: &MemberOutboxDirectoryCapture,
        fee: &CatalogReadReservation,
    ) -> Result<(), AppError> {
        // No ID2-only/new-vault creation path: the original root AND marker3 must already
        // exist. Every other declaration debt was inventoried first.
        self.mount_existing_current_declaration_root(archive)?;
        self.require_member_outbox_root_matches(root)?;
        self.check_current_marker_before_old_gates(layout, fee)?;
        self.install_current_layout_marker(layout)?;
        self.require_member_outbox_root_matches(root)
    }
}
