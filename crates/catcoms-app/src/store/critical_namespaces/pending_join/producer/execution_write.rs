//! Exact original source execution-DATA writer. Actual runtime tickets remain a separate
//! retained invocation consumer: a saved/repaired DATA result alone cannot recreate a ticket.
use super::super::{execution_payload as payload, execution_state as execution};
use super::*;
use crate::store::chat_admission::predecessor::OriginalJoinExecutionObservation;

#[derive(Clone, Copy)]
pub(super) enum WriteShape {
    Migrate,
    OneSource,
}

impl JournalledCheckpointJoinPreparation {
    /// Persist one exact proposed execution event, or the all-source NotGranted migration.
    /// Caller retains its genuine invocation/target BEFORE calling; retry supplies that SAME
    /// target, never newly samples a nonce/ordinal/grant after an uncertain reply.
    pub(super) fn write_execution_change(
        &mut self,
        store: &ServerStore,
        target: &codec::Metadata,
        input: Option<&execution::Input<'_>>,
        shape: WriteShape,
        observation: Option<OriginalJoinExecutionObservation>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.original
            .pin
            .validate_original(store, self.metadata.group.as_slice())?;
        self.creation.snapshot_limits(store)?;
        let policy = store.pending_join_policy.as_ref().ok_or_else(refused)?;
        policy.execution.as_ref().ok_or_else(refused)?;
        if self
            .transition
            .as_ref()
            .is_some_and(|original| original != target)
        {
            return Err(refused());
        }
        store.retry_pending_join_catalog()?;
        let current = store.pending_join_summary()?;
        let row = current
            .rows
            .iter()
            .flatten()
            .find(|row| row.source == self.metadata.source)
            .ok_or_else(refused)?;
        if row == target {
            // Exact outcome recovery; it does NOT create another runtime ticket or initial
            // grant. The actual retained invocation consumer must identify its original result.
            self.metadata = target.clone();
            self.transition = None;
            self.expected = current;
            return self.confirm_existing(store);
        }
        if row != &self.metadata {
            return Err(refused());
        }
        let write_fee = policy.pool.try_reserve(WRITE_FEE)?;
        let read_fee = policy.pool.try_reserve(READ_FEE)?;
        let (file, raw) = store
            .open_pending_join_catalog_slot(Slot::Current, &read_fee)?
            .ok_or_else(refused)?;
        if *blake3::hash(&raw).as_bytes() != current.framed {
            return Err(refused());
        }
        let key = Zeroizing::new(store.keys.db_key()?);
        let plain = Zeroizing::new(unseal(&key, &unframe(&raw)?)?);
        let parsed = store.parse_configured_join_catalog(&plain)?;
        if parsed.digest != current.digest {
            return Err(refused());
        }
        let candidate_plain =
            encode_execution_change(&parsed, target, input, shape, &store.chat_vault_domain()?)?;
        let candidate_parse = store.parse_configured_join_catalog(&candidate_plain)?;
        let mut next = candidate_parse.summary([0; 32]);
        codec::successor(&current, &next)?;
        if !next.rows.iter().flatten().any(|row| row == target) {
            return Err(refused());
        }
        let encoded = Zeroizing::new(frame(&seal(&key, &candidate_plain, rng)?));
        next.framed = *blake3::hash(&encoded).as_bytes();
        let candidate = PendingCandidate {
            identity: Arc::new(()),
            bytes: encoded,
            next: next.clone(),
            expected: current,
            original: self.original.clone(),
            created: None,
            execution_task: observation,
            received_input: None,
            _fee: write_fee,
        };
        // Complete all large encodings/decryption and release scratch/key/fees BEFORE writers.
        drop(candidate_plain);
        drop(plain);
        drop(raw);
        drop(file);
        drop(read_fee);
        drop(key);
        self.transition = Some(target.clone());
        store.install_pending_join_candidate(candidate)?;
        self.metadata = target.clone();
        self.transition = None;
        self.expected = next;
        Ok(())
    }
}

fn proposed_metadata<'a>(
    original: &'a codec::Metadata,
    target: &'a codec::Metadata,
    shape: &WriteShape,
) -> Result<std::borrow::Cow<'a, codec::Metadata>, AppError> {
    match shape {
        WriteShape::Migrate => {
            if original.execution.is_some() {
                return Err(refused());
            }
            let mut next = original.clone();
            next.execution = Some(execution::State::not_granted());
            if original.source == target.source && &next != target {
                return Err(refused());
            }
            Ok(std::borrow::Cow::Owned(next))
        }
        WriteShape::OneSource if original.source == target.source => {
            Ok(std::borrow::Cow::Borrowed(target))
        }
        WriteShape::OneSource => Ok(std::borrow::Cow::Borrowed(original)),
    }
}

fn encode_execution_change(
    current: &codec::Parsed<'_>,
    target: &codec::Metadata,
    input: Option<&execution::Input<'_>>,
    shape: WriteShape,
    domain: &[u8; 32],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    match (&shape, current.join_version) {
        (WriteShape::Migrate, Some(codec::JOIN_VERSION)) if input.is_none() => {}
        (WriteShape::OneSource, Some(payload::VERSION)) => {}
        _ => return Err(refused()),
    }
    if current.count == 0 {
        return Err(refused());
    }
    let mut size = 4 + payload::TAG.len() + 4usize;
    let mut found = false;
    for row in current.rows.iter().flatten() {
        let metadata = proposed_metadata(&row.metadata, target, &shape)?;
        let next_input = match shape {
            WriteShape::Migrate => None,
            WriteShape::OneSource if row.metadata.source == target.source => input,
            WriteShape::OneSource => row.execution_input.as_ref(),
        };
        size = size
            .checked_add(
                4 + payload::encoded_row_len(&metadata, row.immutable, row.encrypted, next_input)?,
            )
            .ok_or_else(refused)?;
        found |= row.metadata.source == target.source;
    }
    if !found {
        return Err(refused());
    }
    let total = (4 + codec::TAG_V2.len() + 36 + 8 + 36 + 4 + 12usize)
        .checked_add(size)
        .and_then(|length| length.checked_add(archive_suffix_length(current).ok()?))
        .ok_or_else(refused)?;
    if size > codec::MAX_JOIN_PAYLOAD || total > codec::MAX_CATALOG_PLAIN {
        return Err(refused());
    }
    let mut output = Encoder::with_capacity(total);
    output.put_str(codec::TAG_V2).map_err(|_| refused())?;
    output.put_bytes(domain).map_err(|_| refused())?;
    output.put_u64(current.generation.checked_add(1).ok_or_else(refused)?);
    output.put_bytes(&current.digest).map_err(|_| refused())?;
    output.put_u32(1 + u32::from(current.archive.is_some()));
    // The actual execution producer chooses this known version only after configured parsing;
    // outer descriptor/source preservation remains the shared writer's exact contract.
    output.put_u32(codec::JOIN_ID);
    output.put_u32(payload::VERSION);
    output.put_u32(size as u32);
    output.put_str(payload::TAG).map_err(|_| refused())?;
    output.put_u32(current.count as u32);
    for row in current.rows.iter().flatten() {
        let metadata = proposed_metadata(&row.metadata, target, &shape)?;
        let next_input = match shape {
            WriteShape::Migrate => None,
            WriteShape::OneSource if row.metadata.source == target.source => input,
            WriteShape::OneSource => row.execution_input.as_ref(),
        };
        payload::encode_row(
            &mut output,
            &metadata,
            row.immutable,
            row.encrypted,
            next_input,
        )?;
    }
    append_original_archive(&mut output, current)?;
    if output.len() != total {
        return Err(refused());
    }
    Ok(Zeroizing::new(output.finish()))
}

/// Repeated under the actual catalog owner, including physical repair: one genuine retained
/// input may add ONLY its original signed DATA. No grant/debit/regrant, schedule change, source
/// replacement, namespace addition or other-row mutation can borrow this DATA-only permit.
impl PendingCandidate {
    pub(in crate::store) fn validate_received_input_change(&self) -> Result<(), AppError> {
        let received = self.received_input.as_ref().ok_or_else(refused)?;
        codec::successor(&self.expected, &self.next)?;
        if self.expected.count != self.next.count
            || self.expected.archive != self.next.archive
            || self.expected.join_version != Some(payload::VERSION)
            || self.next.join_version != Some(payload::VERSION)
            || self.execution_task.is_some()
        {
            return Err(refused());
        }
        let mut changed = 0;
        for old in self.expected.rows.iter().flatten() {
            let next = self
                .next
                .rows
                .iter()
                .flatten()
                .find(|row| row.source == old.source)
                .ok_or_else(refused)?;
            if old == next {
                continue;
            }
            let old_state = old.execution.ok_or_else(refused)?;
            let next_state = next.execution.ok_or_else(refused)?;
            let mut expected = old.clone();
            expected.execution = Some(next_state);
            if next != &expected
                || execution::successor(
                    &old_state,
                    &next_state,
                    &payload::source_identity(old),
                    old.revision,
                    next.revision,
                )? != execution::Change::Input
                || next_state.input != Some(received_summary(next, received)?)
            {
                return Err(refused());
            }
            changed += 1;
        }
        if changed != 1 {
            return Err(refused());
        }
        Ok(())
    }
}

fn received_summary(
    metadata: &codec::Metadata,
    received: &catcoms_sync::SealedOriginalJoinInput,
) -> Result<execution::InputSummary, AppError> {
    let identity = received.request_identity();
    if received.source_binding() != metadata.binding
        || identity.request_digest() != &metadata.request
        || identity.package_reference() != &metadata.package
        || identity.invite_digest() != &metadata.invite
        || identity.invite_nonce() != &metadata.invite_nonce
    {
        return Err(refused());
    }
    let original = received.arm_data();
    let (nonce, ciphertext) = received.encrypted_parts();
    Ok(execution::InputSummary {
        arm: execution::Arm {
            source: payload::source_identity(metadata),
            grant: original.grant,
            ordinal: original.ordinal,
            scheduling_revision: original.revision,
            activation: original.activation,
            invocation: original.invocation,
            deadline_ms: original.deadline_ms,
        },
        body_length: received.body_length(),
        body_digest: received.body_digest(),
        ciphertext_digest: execution::cipher_digest(nonce, ciphertext),
    })
}

impl JournalledCheckpointJoinPreparation {
    /// Persist actual Sync-held inviter-signed DATA against the WHOLE current envelope. The
    /// original row may have retired or scheduling advanced; immutable source/cipher/pin and
    /// every counter still match. This method creates no fresh task, ticket or SDK completion.
    pub(super) fn persist_received_execution_input(
        &mut self,
        store: &ServerStore,
        received: Arc<catcoms_sync::SealedOriginalJoinInput>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.original
            .pin
            .validate_original(store, self.metadata.group.as_slice())?;
        let policy = store.pending_join_policy.as_ref().ok_or_else(refused)?;
        policy.execution.ok_or_else(refused)?;
        store.retry_pending_join_catalog()?;
        let current = store.pending_join_summary()?;
        let row = current
            .rows
            .iter()
            .flatten()
            .find(|row| row.source == self.metadata.source)
            .ok_or_else(refused)?;
        // Scheduling/counters may advance independently, but the exact sealed original never
        // changes. No newer row/capture/pin is adopted as positive join authority here.
        let mut immutable = self.metadata.clone();
        immutable.revision = row.revision;
        immutable.scheduling = row.scheduling;
        immutable.execution = row.execution;
        if row != &immutable {
            return Err(refused());
        }
        let summary = received_summary(row, &received)?;
        if let Some(existing) = row.execution.and_then(|state| state.input) {
            if existing != summary {
                return Err(refused());
            }
            // Same admitted outcome retry. Cross actual opened file/root barrier again, using
            // only physical DATA permission, never the original now-retired row permission.
            let opened = store.inspect_pending_join_catalog()?;
            return store.with_received_join_input_capture(&self.original.capture, |permit| {
                let mounted = store.pending_join_catalog.lock().map_err(|_| refused())?;
                let mounted = mounted.as_ref().ok_or_else(refused)?;
                permit.validate_capture(&self.original.capture)?;
                self.original
                    .pin
                    .validate_original(store, self.metadata.group.as_slice())?;
                if mounted.pending.is_some()
                    || mounted.current.digest != current.digest
                    || opened.selected.digest != current.digest
                {
                    return Err(refused());
                }
                store.settle_pending_join_catalog(&opened)
            });
        }
        let mut target = row.clone();
        let mut state = target.execution.ok_or_else(refused)?;
        state.input = Some(summary);
        target.execution = Some(state);
        let (nonce, ciphertext) = received.encrypted_parts();
        let input = execution::Input {
            summary,
            nonce,
            ciphertext,
        };
        let write_fee = policy.pool.try_reserve(WRITE_FEE)?;
        let read_fee = policy.pool.try_reserve(READ_FEE)?;
        let (file, raw) = store
            .open_pending_join_catalog_slot(Slot::Current, &read_fee)?
            .ok_or_else(refused)?;
        if *blake3::hash(&raw).as_bytes() != current.framed {
            return Err(refused());
        }
        let key = Zeroizing::new(store.keys.db_key()?);
        let plain = Zeroizing::new(unseal(&key, &unframe(&raw)?)?);
        let parsed = store.parse_configured_join_catalog(&plain)?;
        if parsed.digest != current.digest {
            return Err(refused());
        }
        let candidate_plain = encode_execution_change(
            &parsed,
            &target,
            Some(&input),
            WriteShape::OneSource,
            &store.chat_vault_domain()?,
        )?;
        let candidate_parse = store.parse_configured_join_catalog(&candidate_plain)?;
        let mut next = candidate_parse.summary([0; 32]);
        let encoded = Zeroizing::new(frame(&seal(&key, &candidate_plain, rng)?));
        next.framed = *blake3::hash(&encoded).as_bytes();
        let candidate = PendingCandidate {
            identity: Arc::new(()),
            bytes: encoded,
            next,
            expected: current,
            original: self.original.clone(),
            created: None,
            execution_task: None,
            received_input: Some(received.clone()),
            _fee: write_fee,
        };
        candidate.validate_received_input_change()?;
        drop(candidate_plain);
        drop(plain);
        drop(raw);
        drop(file);
        drop(read_fee);
        drop(key);
        store.install_pending_join_candidate(candidate)
    }
}
