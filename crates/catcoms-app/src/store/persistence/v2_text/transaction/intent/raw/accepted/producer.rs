//! Exact first-B immutable DAG. Every encrypted output is cached before the bound
//! Scope postcheck; a retry borrows the same nonce/frame and never re-signs/derives.
//! These are DATA only until the paired physical Stage and fresh Commit complete.
use super::*;
use crate::store::persistence::v2_text::wal::{
    TextCompletionWalFrameData, TextDeletionWalFrameData,
};
use catcoms_replication::chat::cold_index::{
    encode_leaf_data, ActionClaimData, ClaimHint, IndexContextData, IndexScopeData,
    ReceiptDigestData, RetentionData,
};
use catcoms_replication::chat::cold_manifest::{
    encode_manifest, ArchiveOperation, ManifestSpec, PageDescriptor, PageRole,
};
use catcoms_replication::chat::cold_records::{encode_record_batch, BatchBinding, RecordSource};
use catcoms_replication::chat::owner_facts::OwnerFactId;

impl PreparedV2TextAcceptedCreate {
    pub(super) fn require_original_candidates(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.metadata.validate_prepaid_account()?;
        self.signature
            .intent
            .retained
            .custody
            .validate_accounts(store)?;
        self.signature.intent.unsigned.validate_original(
            store,
            source,
            &self.signature.intent.retained,
        )?;
        let actual = self
            .signature
            .intent
            .unsigned
            .borrow_accepted_candidate_data(
                &self.signature.intent.retained,
                &self.observe_original(),
            )?;
        if actual.header_data() != &self.parent.header
            || actual.image_data() != self.parent.image.as_slice()
        {
            return Err(refused());
        }
        self.resources()
            .require_bound_original_negative(store, source)
    }
    pub(super) fn retained_token_data(&self, key: &[u8; 32]) -> Result<TokenValueData, AppError> {
        let (frame, id) = self
            .signature
            .intent
            .retained
            .producer_data()
            .retain_frame_data()?
            .ok_or_else(refused)?;
        let plain = open_control_data(key, TextControlKindData::Retain.id_data(frame), frame)?;
        let v = RetainedActionData::decode_data(&plain)?;
        Ok(TokenValueData {
            transaction: v.transaction,
            token: v.token,
            request: v.request,
            retain: id,
            action: v.action,
            original: v.original,
            phase: RequestPhaseData::Retained,
        })
    }
    pub(super) fn prepare_page(
        &self,
        key: &[u8; 32],
        plain: &[u8],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<PreparedPrivateChatPage, AppError> {
        Ok(PreparedPrivateChatPage::prepare(key, plain, rng)?)
    }
    pub(super) fn archive_a_data(&self) -> &TextArchiveData {
        &self.signature.parent.base.archive
    }
    fn operation_data(&self) -> Result<ArchiveOperation, AppError> {
        let v = self
            .signature
            .intent
            .unsigned
            .unsigned_data(&self.signature.intent.retained)?;
        Ok(ArchiveOperation::Append {
            body: v.body.id(),
            admission: v.record.spec().context.admission,
            before: 0,
            after: 1,
        })
    }
    fn receipt_data(&self) -> Result<ReceiptDigestData, AppError> {
        let v = self
            .signature
            .intent
            .unsigned
            .unsigned_data(&self.signature.intent.retained)?;
        let c = self.parent.head.context.spec_data();
        Ok(ReceiptDigestData::from_tuple(
            c.profile,
            OwnerFactId::from_bytes(*c.admission.as_bytes()),
            v.action.id(),
            *v.record.id().as_bytes(),
            0,
            self.parent.header.id(),
        ))
    }
    fn singleton_root(page: PrivateChatPageId, key: [u8; 32]) -> TextRootData {
        TextRootData::Present(NodeRefData {
            page,
            summary: NodeSummaryData {
                height: 0,
                keys: 1,
                min: key,
                max: key,
            },
        })
    }
    /// Account/source checks before each seal and after caching it. Nothing in this
    /// method reads a public network dependency or manufactures an acceptance witness.
    pub(in crate::store) fn prepare_candidates(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        if self.completed.is_some() {
            return self.prepare_completed_candidates(store, source, rng);
        }
        self.metadata.validate_prepaid_account()?;
        self.signature
            .intent
            .retained
            .custody
            .validate_accounts(store)?;
        self.signature.intent.unsigned.validate_original(
            store,
            source,
            &self.signature.intent.retained,
        )?;
        // Preallocate the closed originalB Root-advance state under the NEW B fee.
        // It cannot advance or form a basis until the later genuine SavedB guard.
        let original = self.observe_original();
        self.signature
            .intent
            .unsigned
            .prepare_accepted_advance_data(&self.signature.intent.retained, &original)?;
        self.require_original_candidates(store, source)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        if self.records.is_none() {
            let v = self
                .signature
                .intent
                .unsigned
                .unsigned_data(&self.signature.intent.retained)?;
            let c = self.parent.head.context.spec_data();
            let operation = self.operation_data()?;
            let plain = encode_record_batch(
                BatchBinding {
                    vault: &c.layout.vault,
                    profile: &source.data().profile,
                    role: PageRole::AcceptanceRecords,
                    operation: &operation,
                    append: Some(v.body),
                },
                &[RecordSource::Acceptance(v.record)],
            )
            .map_err(|_| refused())?;
            self.require_original_candidates(store, source)?;
            let page = self.prepare_page(&key, &plain, rng)?;
            self.records = Some(AcceptedPage {
                page,
                plain_bytes: plain.len(),
                entries: 1,
            });
            self.require_original_candidates(store, source)?;
        }
        if self.nodes.is_none() {
            let v = self
                .signature
                .intent
                .unsigned
                .unsigned_data(&self.signature.intent.retained)?;
            let candidate = self
                .signature
                .intent
                .unsigned
                .borrow_accepted_candidate_data(
                    &self.signature.intent.retained,
                    &self.observe_original(),
                )?;
            let plain = catcoms_replication::chat::cold_nodes::encode_node_page(
                &self.parent.head.context.spec_data().layout.vault,
                v.body,
                candidate.append_data(),
            )
            .map_err(|_| refused())?;
            self.require_original_candidates(store, source)?;
            let page = self.prepare_page(&key, &plain, rng)?;
            self.nodes = Some(AcceptedPage {
                page,
                plain_bytes: plain.len(),
                entries: 1,
            });
            self.require_original_candidates(store, source)?;
        }
        if self.header_body.is_none() {
            let v = self
                .signature
                .intent
                .unsigned
                .unsigned_data(&self.signature.intent.retained)?;
            let c = self.parent.head.context.spec_data();
            let operation = self.operation_data()?;
            let plain = encode_record_batch(
                BatchBinding {
                    vault: &c.layout.vault,
                    profile: &source.data().profile,
                    role: PageRole::HeaderBodyRecords,
                    operation: &operation,
                    append: Some(v.body),
                },
                &[
                    RecordSource::Header {
                        header: &self.parent.header,
                        image: &self.parent.image,
                    },
                    RecordSource::Append(v.body),
                ],
            )
            .map_err(|_| refused())?;
            self.require_original_candidates(store, source)?;
            let page = self.prepare_page(&key, &plain, rng)?;
            self.header_body = Some(AcceptedPage {
                page,
                plain_bytes: plain.len(),
                entries: 2,
            });
            self.require_original_candidates(store, source)?;
        }
        if self.manifest.is_none() {
            let r = self.records.as_ref().ok_or_else(refused)?;
            let n = self.nodes.as_ref().ok_or_else(refused)?;
            let h = self.header_body.as_ref().ok_or_else(refused)?;
            let descriptors = [
                (PageRole::AcceptanceRecords, r),
                (PageRole::CompleteNodes, n),
                (PageRole::HeaderBodyRecords, h),
            ]
            .map(|(role, p)| PageDescriptor {
                role,
                page: p.page.id(),
                plaintext_bytes: p.plain_bytes,
                entries: p.entries,
            });
            let c = self.parent.head.context.spec_data();
            let previous = self.archive_a_data().spec_data();
            let plain = encode_manifest(&ManifestSpec {
                vault: c.layout.vault,
                group: c.group,
                profile: c.profile,
                sequence: previous
                    .manifest_sequence
                    .checked_add(1)
                    .ok_or_else(refused)?,
                previous: Some(previous.manifest),
                source_head: Some(self.parent.head.archive),
                before_header: Some(previous.header.id()),
                after_header: self.parent.header.id(),
                operation: self.operation_data()?,
                pages: &descriptors,
            })
            .map_err(|_| refused())?;
            self.require_original_candidates(store, source)?;
            self.manifest = Some(self.prepare_page(&key, &plain, rng)?);
            self.require_original_candidates(store, source)?;
        }
        if self.archive.is_none() {
            let a = TextArchiveData::from_data(TextArchiveSpecData {
                context: self.parent.head.context.id_data()?,
                manifest: self.manifest.as_ref().ok_or_else(refused)?.id(),
                manifest_sequence: self
                    .archive_a_data()
                    .spec_data()
                    .manifest_sequence
                    .checked_add(1)
                    .ok_or_else(refused)?,
                header: &self.parent.header,
                image: &self.parent.image,
                previous: Some(self.parent.head.archive),
                operation: TextArchiveOperationData::Append,
            })?;
            self.require_original_candidates(store, source)?;
            self.archive = Some(a.prepare_frame_data(&key, rng)?);
            self.require_original_candidates(store, source)?;
        }
        if self.actions.is_none() {
            let v = self
                .signature
                .intent
                .unsigned
                .unsigned_data(&self.signature.intent.retained)?;
            let c = self.parent.head.context.spec_data();
            let producer = self.signature.intent.retained.producer_data();
            // The original retention manifest belongs to H_A. Its exact canonical
            // Action bytes are still owned by the SAME SDK custody (never reconstructed
            // from rendered text). Encoding here is bounded and paid before allocation.
            let action_bytes = v.action.encode();
            let claim = ActionClaimData {
                record: *v.record.id().as_bytes(),
                ordinal: 0,
                channel: v.record.spec().channel,
                accepting_header: self.parent.header.id(),
                header_sequence: self.parent.header.spec().sequence,
                append_body: v.body.id(),
                acceptance_manifest: self.manifest.as_ref().ok_or_else(refused)?.id(),
                manifest_sequence: self
                    .archive_a_data()
                    .spec_data()
                    .manifest_sequence
                    .checked_add(1)
                    .ok_or_else(refused)?,
                record_page: self.records.as_ref().ok_or_else(refused)?.page.id(),
                record_row: 0,
                header_body_page: self.header_body.as_ref().ok_or_else(refused)?.page.id(),
                retention: RetentionData::Known {
                    manifest: self.archive_a_data().spec_data().manifest,
                    original: producer.original_action_digest_data()?,
                    bytes: u32::try_from(action_bytes.len()).map_err(|_| refused())?,
                },
                receipt: self.receipt_data()?,
                occurrences: 1,
                hint: ClaimHint::Canonical,
            };
            let plain = encode_leaf_data(
                IndexContextData {
                    vault: c.layout.vault,
                    group: c.group,
                    profile: c.profile,
                    scope: IndexScopeData::Actions(OwnerFactId::from_bytes(
                        *c.admission.as_bytes(),
                    )),
                },
                &[LeafRowData {
                    key: *v.action.id().as_bytes(),
                    value: ValueData::Action(claim),
                }],
                NodePlacementData::Root,
            )
            .map_err(|_| refused())?;
            self.require_original_candidates(store, source)?;
            self.actions = Some(self.prepare_page(&key, &plain, rng)?);
            self.require_original_candidates(store, source)?;
        }
        if self.directory.is_none() {
            let c = self.parent.head.context.spec_data();
            let v = self
                .signature
                .intent
                .unsigned
                .unsigned_data(&self.signature.intent.retained)?;
            let actual = self
                .signature
                .intent
                .unsigned
                .borrow_accepted_candidate_data(
                    &self.signature.intent.retained,
                    &self.observe_original(),
                )?;
            let action_root = Self::singleton_root(
                self.actions.as_ref().ok_or_else(refused)?.id(),
                *v.action.id().as_bytes(),
            );
            let TextRootData::Present(action_root) = action_root else {
                return Err(refused());
            };
            let value = DirectoryData {
                tree: DirectoryTreeData::Present(action_root),
                accepted_leaf_count: 1,
                unique_action_count: 1,
                accepted_root: actual.append_data().new_tip().root,
                own_tip_header: self.parent.header.id(),
                tip_body: v.body.id(),
                prefix_manifest: self.manifest.as_ref().ok_or_else(refused)?.id(),
                prefix_sequence: self
                    .archive_a_data()
                    .spec_data()
                    .manifest_sequence
                    .checked_add(1)
                    .ok_or_else(refused)?,
                prefix_head: ArchiveHeadId::from_bytes(
                    *self
                        .archive
                        .as_ref()
                        .ok_or_else(refused)?
                        .id_data()
                        .as_bytes(),
                ),
            };
            let plain = encode_leaf_data(
                IndexContextData {
                    vault: c.layout.vault,
                    group: c.group,
                    profile: c.profile,
                    scope: IndexScopeData::Directory,
                },
                &[LeafRowData {
                    key: *c.admission.as_bytes(),
                    value: ValueData::Directory(value),
                }],
                NodePlacementData::Root,
            )
            .map_err(|_| refused())?;
            self.require_original_candidates(store, source)?;
            self.directory = Some(self.prepare_page(&key, &plain, rng)?);
            self.require_original_candidates(store, source)?;
        }
        if self.publication.is_none() {
            let TextPendingData::Raw { transaction, .. } = self.parent.head.pending else {
                return Err(refused());
            };
            let p = PublicationRecordData {
                context: self.parent.head.context.id_data()?,
                transaction: transaction.transaction,
                retain: transaction.retain,
                public_header: self.parent.header.id(),
                receipt: self.receipt_data()?,
                archive: ArchiveHeadId::from_bytes(
                    *self
                        .archive
                        .as_ref()
                        .ok_or_else(refused)?
                        .id_data()
                        .as_bytes(),
                ),
                ordinal: 0,
            };
            self.require_original_candidates(store, source)?;
            self.publication = Some(p.prepare_frame_data(&key, rng)?);
            self.require_original_candidates(store, source)?;
        }
        if self.token.is_none() {
            let mut v = self.retained_token_data(&key)?;
            v.phase = RequestPhaseData::Accepted(AcceptedLocatorData {
                header: self.parent.header.id(),
                record_page: self.records.as_ref().ok_or_else(refused)?.page.id(),
                record_row: 0,
                ordinal: 0,
                receipt: self.receipt_data()?,
            });
            let plain = encode_private_leaf_data(
                PrivateIndexContextData {
                    cid: self.parent.head.context.id_data()?,
                    kind: PrivateIndexKindData::Token,
                },
                &[PrivateIndexLeafRowData {
                    key: *v.key_data(self.parent.head.context.id_data()?).as_bytes(),
                    value: PrivateIndexValueData::Token(v),
                }],
                NodePlacementData::Root,
            )
            .map_err(|_| refused())?;
            self.require_original_candidates(store, source)?;
            self.token = Some(self.prepare_page(&key, &plain, rng)?);
            self.require_original_candidates(store, source)?;
        }
        if self.publications.is_none() {
            let TextPendingData::Raw { transaction, .. } = self.parent.head.pending else {
                return Err(refused());
            };
            let v = PublicationValueData {
                transaction: transaction.transaction,
                action: self.signature.record.action,
                receipt: self.receipt_data()?,
                header: self.parent.header.id(),
                publication: TextPublicationIdData::from_bytes(
                    *self
                        .publication
                        .as_ref()
                        .ok_or_else(refused)?
                        .id_data()
                        .as_bytes(),
                ),
                phase: PublicationPhaseData::Eligible,
            };
            let plain = encode_private_leaf_data(
                PrivateIndexContextData {
                    cid: self.parent.head.context.id_data()?,
                    kind: PrivateIndexKindData::Publication,
                },
                &[PrivateIndexLeafRowData {
                    key: PublicationValueData::ordinal_key_data(0),
                    value: PrivateIndexValueData::Publication(v),
                }],
                NodePlacementData::Root,
            )
            .map_err(|_| refused())?;
            self.require_original_candidates(store, source)?;
            self.publications = Some(self.prepare_page(&key, &plain, rng)?);
            self.require_original_candidates(store, source)?;
        }
        if self.candidate.is_none() {
            let p = &self.parent.head;
            let (bytes, count) = self.artifact_totals_data()?;
            if bytes > REMAINING_AFTER_S_BYTES || count > REMAINING_AFTER_S_OBJECTS {
                return Err(refused());
            }
            let v = self.retained_token_data(&key)?;
            let head = TextHeadData {
                context: p.context.clone(),
                generation: p.generation.checked_add(1).ok_or_else(refused)?,
                previous: TextParentData::Operational(TextHeadIdData::of_data(
                    &self.parent.head_frame,
                )),
                archive: ArchiveHeadId::from_bytes(
                    *self
                        .archive
                        .as_ref()
                        .ok_or_else(refused)?
                        .id_data()
                        .as_bytes(),
                ),
                directory: Self::singleton_root(
                    self.directory.as_ref().ok_or_else(refused)?.id(),
                    *p.context.spec_data().admission.as_bytes(),
                ),
                tokens: Self::singleton_root(
                    self.token.as_ref().ok_or_else(refused)?.id(),
                    *v.key_data(p.context.id_data()?).as_bytes(),
                ),
                publications: Self::singleton_root(
                    self.publications.as_ref().ok_or_else(refused)?.id(),
                    PublicationValueData::ordinal_key_data(0),
                ),
                queue_front: 0,
                queue_next: 1,
                pending: TextPendingData::None,
                live_bytes: p.live_bytes.checked_add(bytes).ok_or_else(refused)?,
                live_objects: p.live_objects.checked_add(count).ok_or_else(refused)?,
                selection_attempt: p.selection_attempt.checked_add(1).ok_or_else(refused)?,
            };
            self.require_original_candidates(store, source)?;
            self.candidate = Some(head.prepare_frame_data(&key, rng)?);
            self.require_original_candidates(store, source)?;
        }
        if self.plan.is_none() {
            let previous = RetirementCarryData::decode_data(&self.signature.carry)?;
            if previous.roots_data().len() != 3 || !previous.entries_data().is_empty() {
                return Err(refused());
            }
            let mut roots = previous.roots_data().to_vec();
            roots.push(TextHeadIdData::of_data(&self.parent.head_frame));
            roots.sort_unstable();
            let entries = [
                RetirementEntryData {
                    kind: RetirementKindData::AcceptedIndex,
                    id: *self.parent.directory_root.page.as_bytes(),
                    frame_bytes: u32::try_from(self.parent.directory_frame_bytes)
                        .map_err(|_| refused())?,
                },
                RetirementEntryData {
                    kind: RetirementKindData::TokenIndex,
                    id: *self.parent.token_root.page.as_bytes(),
                    frame_bytes: u32::try_from(self.parent.token_frame.len())
                        .map_err(|_| refused())?,
                },
                RetirementEntryData {
                    kind: RetirementKindData::Intent,
                    id: *self.signature.record.intent.as_bytes(),
                    frame_bytes: u32::try_from(self.signature.parent.intent_frame.len())
                        .map_err(|_| refused())?,
                },
                RetirementEntryData {
                    kind: RetirementKindData::Signature,
                    id: *self
                        .signature
                        .signature
                        .as_ref()
                        .ok_or_else(refused)?
                        .id_data()
                        .as_bytes(),
                    frame_bytes: 405,
                },
            ];
            let plan = RetirementPlanData::from_data(
                self.parent.head.context.id_data()?,
                self.signature.record.transaction,
                TextHeadIdData::of_data(self.candidate.as_ref().ok_or_else(refused)?.frame_data()),
                RetirementCarryData::from_data(&roots, &entries)?,
            )?;
            self.plan = Some(plan.encode_data()?);
            self.require_original_candidates(store, source)?;
        }
        let plan = RetirementPlanData::decode_data(self.plan.as_ref().ok_or_else(refused)?)?;
        if self.completion.is_none() {
            let plain = TextWalFrameData::Completion(TextCompletionWalFrameData {
                context: plan.context_data(),
                transaction: plan.transaction_data(),
                selected: plan.selected_data(),
                plan_commit: plan.id_data()?,
                public_header: self.parent.header.id(),
                reclaimed: plan.carry_data().totals_data()?,
                plan: self.plan.as_ref().ok_or_else(refused)?,
            })
            .encode_data()?;
            self.require_original_candidates(store, source)?;
            self.completion = Some(PreparedTextControlFrameData::prepare_data(
                TextControlKindData::Wal,
                &key,
                &plain,
                rng,
            )?);
            self.require_original_candidates(store, source)?;
        }
        if self.deletion.is_none() {
            let plain = TextWalFrameData::Deletion(TextDeletionWalFrameData {
                context: plan.context_data(),
                transaction: plan.transaction_data(),
                selected: plan.selected_data(),
                plan_commit: plan.id_data()?,
                plan: self.plan.as_ref().ok_or_else(refused)?,
                selected_frame: self.candidate.as_ref().ok_or_else(refused)?.frame_data(),
                completion_frame: self.completion.as_ref().ok_or_else(refused)?.frame_data(),
            })
            .encode_data()?;
            self.require_original_candidates(store, source)?;
            self.deletion = Some(PreparedTextControlFrameData::prepare_data(
                TextControlKindData::Wal,
                &key,
                &plain,
                rng,
            )?);
            self.require_original_candidates(store, source)?;
        }
        if self.wal.is_none() {
            let artifacts = self.artifact_frames_data()?;
            let plain = TextWalFrameData::Work(TextWorkWalFrameData {
                context: plan.context_data(),
                install_parent: TextHeadIdData::of_data(&self.parent.head_frame),
                transaction: plan.transaction_data(),
                public_parent: self.signature.record.public_parent,
                operation: TextWorkWalOperationData::Accept,
                parent_frame: &self.parent.head_frame,
                candidate: plan.selected_data(),
                candidate_frame: self.candidate.as_ref().ok_or_else(refused)?.frame_data(),
                cleanup: TextWorkWalCleanupData::Final {
                    plan_commit: plan.id_data()?,
                    plan: self.plan.as_ref().ok_or_else(refused)?,
                    deletion_frame: self.deletion.as_ref().ok_or_else(refused)?.frame_data(),
                },
                artifacts: TextWalArtifactsData::Borrowed(&artifacts),
            })
            .encode_data()?;
            let TextWalFrameData::Work(w) = TextWalFrameData::decode_data(&plain)? else {
                return Err(refused());
            };
            semantic::verify_original_b_data(self, &key, &w)?;
            self.require_original_candidates(store, source)?;
            self.wal = Some(PreparedTextControlFrameData::prepare_data(
                TextControlKindData::Wal,
                &key,
                &plain,
                rng,
            )?);
            self.require_original_candidates(store, source)?;
        }
        if self.wal.as_ref().ok_or_else(refused)?.frame_data().len() > FIRST_B_WORK_WAL_FRAME_BYTES
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(super) fn artifact_frames_data(&self) -> Result<Vec<TextWalArtifactData<'_>>, AppError> {
        let mut a = Vec::with_capacity(10);
        for (role, p) in [
            (TextArtifactRoleData::Manifest, self.manifest.as_ref()),
            (TextArtifactRoleData::AcceptedIndex, self.actions.as_ref()),
            (TextArtifactRoleData::AcceptedIndex, self.directory.as_ref()),
            (TextArtifactRoleData::TokenIndex, self.token.as_ref()),
            (
                TextArtifactRoleData::PublicationIndex,
                self.publications.as_ref(),
            ),
        ] {
            let p = p.ok_or_else(refused)?;
            a.push(TextWalArtifactData::from_data(
                role,
                *p.id().as_bytes(),
                p.as_bytes(),
            )?);
        }
        for (role, p) in [
            (
                TextArtifactRoleData::AcceptanceRecords,
                self.records.as_ref(),
            ),
            (TextArtifactRoleData::AcceptanceNodes, self.nodes.as_ref()),
            (TextArtifactRoleData::HeaderBody, self.header_body.as_ref()),
        ] {
            let p = p.ok_or_else(refused)?;
            a.push(TextWalArtifactData::from_data(
                role,
                *p.page.id().as_bytes(),
                p.page.as_bytes(),
            )?);
        }
        for (role, p) in [
            (TextArtifactRoleData::Archive, self.archive.as_ref()),
            (TextArtifactRoleData::Publication, self.publication.as_ref()),
        ] {
            let p = p.ok_or_else(refused)?;
            a.push(TextWalArtifactData::from_data(
                role,
                *p.id_data().as_bytes(),
                p.frame_data(),
            )?);
        }
        a.sort_unstable_by_key(|p| (p.role_data(), p.id_data()));
        if a.len() != 10 {
            return Err(refused());
        }
        Ok(a)
    }
    pub(super) fn artifact_totals_data(&self) -> Result<(u128, u128), AppError> {
        let a = self.artifact_frames_data()?;
        let bytes = a.iter().try_fold(0u128, |n, p| {
            n.checked_add(p.frame_data().len() as u128)
                .ok_or_else(refused)
        })?;
        Ok((bytes, a.len() as u128))
    }
}
