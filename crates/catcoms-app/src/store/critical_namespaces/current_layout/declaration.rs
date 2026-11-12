//! Complete original namespace3 candidate preparation. Actual fixed I/O lives in persistence;
//! no decoded layout or declaration state grants a Current/group/SDK capability.
use super::super::{archive_current_namespace_codec as codec, pending_join_codec as catalog};
use super::*;

impl ServerStore {
    pub(in crate::store) fn prepare_current_layout_declaration(
        &self,
        fee: &CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Declaration, AppError> {
        let policy = self.current_layout_policy.as_ref().ok_or_else(refused)?;
        fee.validate(&policy.archive.pending.pool, codec::DECLARATION_FEE)?;
        let (_file, raw) = self
            .open_pending_join_catalog_slot(Slot::Current, fee)?
            .ok_or_else(refused)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let plain = Zeroizing::new(unseal(&key, &unframe(&raw)?)?);
        let current = self.parse_configured_join_catalog(&plain)?;
        if current.current_layout.is_some() || self.chat_mount.observer().is_cancelled() {
            return Err(refused());
        }
        let archive = current.archive.as_ref().ok_or_else(refused)?;
        let layout = codec::CurrentLayoutData {
            vault: archive.vault,
            root: archive.root,
        };
        self.validate_current_declaration_root(layout)?;
        {
            let mounted = self.pending_join_catalog.lock().map_err(|_| refused())?;
            let mounted = mounted.as_ref().ok_or_else(refused)?;
            if mounted.pending.is_some() || mounted.current.framed != *blake3::hash(&raw).as_bytes()
            {
                return Err(refused());
            }
        }
        let envelope = super::super::catalog_envelope::parse(
            &plain,
            &layout.vault,
            catalog::MAX_CATALOG_PLAIN,
        )?;
        let descriptor = layout.encode_data()?;
        let capacity = plain
            .len()
            .checked_add(12 + descriptor.len())
            .ok_or_else(refused)?;
        if capacity > catalog::MAX_CATALOG_PLAIN || envelope.count >= 16 {
            return Err(refused());
        }
        let mut output = Encoder::with_capacity(capacity);
        output.put_str(catalog::TAG_V2).map_err(|_| refused())?;
        output.put_bytes(&layout.vault).map_err(|_| refused())?;
        output.put_u64(current.generation.checked_add(1).ok_or_else(refused)?);
        output.put_bytes(&current.digest).map_err(|_| refused())?;
        output.put_u32((envelope.count + 1) as u32);
        for index in 0..envelope.count {
            let entry = envelope.entry(index)?;
            output.put_u32(entry.id).put_u32(entry.version);
            output.put_bytes(entry.payload).map_err(|_| refused())?;
        }
        output
            .put_u32(codec::NAMESPACE_ID)
            .put_u32(codec::NAMESPACE_VERSION);
        output.put_bytes(&descriptor).map_err(|_| refused())?;
        let candidate_plain = Zeroizing::new(output.finish());
        let next = self.parse_configured_join_catalog(&candidate_plain)?;
        let expected = current.summary(*blake3::hash(&raw).as_bytes());
        let next = next.summary([0; 32]);
        catalog::successor(&expected, &next)?;
        // Actual original checks precede the FIRST RNG draw; the complete outputs below are
        // cached before any write. No retry or alternate parent may reseal a new candidate.
        self.validate_current_declaration_root(layout)?;
        let candidate = Zeroizing::new(frame(&seal(&key, &candidate_plain, rng)?));
        let mut next = next;
        next.framed = *blake3::hash(&candidate).as_bytes();
        let marker = codec::seal_marker_data(&key, layout, rng)?;
        let seed_plain = codec::encode_declaration_seed_data(
            &key,
            layout,
            expected.framed,
            &marker,
            &candidate,
        )?;
        let seed = codec::seal_declaration_seed_data(&key, &seed_plain, rng)?;
        Ok(Declaration {
            layout,
            parent: DeclarationParent::Original(expected),
            next,
            candidate,
            marker,
            seed,
            seed_file: None,
            stage_file: None,
            marker_file: None,
        })
    }

    /// BEFORE older gates/cleaners: full current+stage+seed classification or an explicit
    /// preserved refusal. The transient debit drops before the old ID2 gate acquires its own;
    /// independent mounted-marker metadata remains paid through the private Store lifetime.
    pub(in crate::store) fn open_current_layout_catalog_gate(
        &self,
        actual_format: catcoms_storage::VaultFormat,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        if actual_format != catcoms_storage::VaultFormat::CriticalNamespaces {
            return Err(refused());
        }
        let layout = self.inspect_and_repair_current_declaration()?;
        self.open_archive_bootstrap_catalog_gate(actual_format, passphrase, rng)?;
        if let Some(layout) = layout {
            self.install_current_layout_marker(layout)?;
        }
        Ok(())
    }

    fn inspect_and_repair_current_declaration(
        &self,
    ) -> Result<Option<codec::CurrentLayoutData>, AppError> {
        let policy = self.current_layout_policy.as_ref().ok_or_else(refused)?;
        let fee = policy
            .archive
            .pending
            .pool
            .try_reserve(codec::DECLARATION_FEE)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let (_current_file, current_raw) = self
            .open_pending_join_catalog_slot(Slot::Current, &fee)?
            .ok_or_else(refused)?;
        let current_plain = Zeroizing::new(unseal(&key, &unframe(&current_raw)?)?);
        let current = self.parse_configured_join_catalog(&current_plain)?;
        let current = current.summary(*blake3::hash(&current_raw).as_bytes());
        drop(current_plain);
        drop(current_raw);
        // Keep complete stage bytes until the independent seed decides the exact closed role.
        // A partial stage is repairable ONLY from that complete authenticated original seed.
        let stage = self.open_pending_join_catalog_slot(Slot::Stage, &fee)?;
        let seed = self.read_current_declaration_seed(&fee)?;
        if let Some((seed_file, seed_raw)) = seed {
            self.require_no_archive_declaration_seed()?;
            if !self.has_owner_operations_intent()? {
                return Err(refused());
            }
            let archive = current.archive.as_ref().ok_or_else(refused)?;
            let layout = codec::CurrentLayoutData {
                vault: archive.vault,
                root: archive.root,
            };
            let seed_plain = codec::open_declaration_seed_data(&key, &seed_raw)?;
            let saved = codec::parse_declaration_seed_data(&key, layout, &seed_plain)?;
            let candidate_plain = Zeroizing::new(unseal(&key, &unframe(saved.catalog_frame)?)?);
            let candidate = self.parse_configured_join_catalog(&candidate_plain)?;
            let next = candidate.summary(*blake3::hash(saved.catalog_frame).as_bytes());
            if next.current_layout != Some(layout)
                || next.archive != current.archive
                || saved.expected_catalog_id == next.framed
            {
                return Err(refused());
            }
            let parent = if current.framed == saved.expected_catalog_id {
                if current.current_layout.is_some() {
                    return Err(refused());
                }
                catalog::successor(&current, &next)?;
                DeclarationParent::Original(current.clone())
            } else if current.framed == next.framed && current.digest == next.digest {
                // Deliberately do NOT synthesize an old Summary/rows/generation. This actual
                // file is already the candidate; only its real current/marker barriers settle.
                if current.current_layout != Some(layout) || next.parent.is_none() {
                    return Err(refused());
                }
                DeclarationParent::AlreadyPromoted {
                    original_frame: saved.expected_catalog_id,
                }
            } else {
                return Err(refused());
            };
            if let Some((_, raw)) = &stage {
                if raw.len() > saved.catalog_frame.len()
                    || saved.catalog_frame[..raw.len()] != raw[..]
                {
                    return Err(refused());
                }
                if matches!(parent, DeclarationParent::AlreadyPromoted { .. })
                    && raw.as_slice() != saved.catalog_frame
                {
                    return Err(refused());
                }
            }
            self.mount_existing_current_declaration_root(archive)?;
            let identity = catcoms_storage::kept::session_id(&seed_file)
                .map_err(|error| AppError::Io(error.to_string()))?;
            let mut declaration = Declaration {
                layout,
                parent,
                next,
                candidate: Zeroizing::new(saved.catalog_frame.to_vec()),
                marker: Zeroizing::new(saved.marker_frame.to_vec()),
                seed: seed_raw,
                seed_file: Some(super::super::pending_join::CreatedStage {
                    file: seed_file,
                    identity: Some(identity),
                }),
                stage_file: None,
                marker_file: None,
            };
            drop(candidate_plain);
            drop(seed_plain);
            drop(stage);
            self.write_current_layout_declaration(&mut declaration, &fee)?;
            return Ok(Some(layout));
        }
        let selected = if let Some((_, stage_raw)) = stage {
            let stage_plain = Zeroizing::new(unseal(&key, &unframe(&stage_raw)?)?);
            let parsed = self.parse_configured_join_catalog(&stage_plain)?;
            let next = parsed.summary(*blake3::hash(&stage_raw).as_bytes());
            if next.digest != current.digest {
                catalog::successor(&current, &next)?;
            }
            if current.current_layout != next.current_layout {
                return Err(refused());
            }
            next
        } else {
            current
        };
        match selected.current_layout {
            Some(layout) => {
                self.require_no_archive_declaration_seed()?;
                if !self.has_owner_operations_intent()? {
                    return Err(refused());
                }
                self.mount_existing_current_declaration_root(
                    selected.archive.as_ref().ok_or_else(refused)?,
                )?;
                self.check_current_marker_before_old_gates(layout, &fee)?;
                Ok(Some(layout))
            }
            None => {
                super::super::current_group::refuse_unsupported_layout(&self.dir)?;
                Ok(None)
            }
        }
    }
}
