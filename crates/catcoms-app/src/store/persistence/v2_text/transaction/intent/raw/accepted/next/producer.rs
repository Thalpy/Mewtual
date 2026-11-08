//! Exact second founder B DAG. All original outputs are cached once; rewrites
//! consume authenticated root-leaf paths and preserve untouched rows. No split,
//! signer, Header/model setter, quota reacquisition or caller-root path is exposed.
use super::*;
use crate::store::persistence::v2_text::private_index::{
    PrivateIndexNodeFrame, PrivateIndexPathData, PrivateIndexRewriteData,
};
use crate::store::persistence::v2_text::wal::{
    TextCompletionWalFrameData, TextDeletionWalFrameData,
};
use catcoms_replication::chat::cold_index::{
    ActionClaimData, ClaimHint, NodeFrame, NodeKindData, ReceiptDigestData, RetentionData,
};
use catcoms_replication::chat::cold_manifest::{
    encode_manifest, ArchiveOperation, ManifestSpec, PageDescriptor, PageRole,
};
use catcoms_replication::chat::cold_records::{encode_record_batch, BatchBinding, RecordSource};
use catcoms_replication::chat::owner_facts::OwnerFactId;
use catcoms_storage::private_chat_page::open_private_chat_page;

fn only_accepted_leaf(nodes: &[Zeroizing<Vec<u8>>], expected: u64) -> Result<&[u8], AppError> {
    if nodes.len() != 1 {
        return Err(refused());
    }
    let node = NodeFrame::decode(&nodes[0], NodePlacementData::Root).map_err(|_| refused())?;
    if node.kind() != NodeKindData::Leaf
        || node.summary().height != 0
        || node.summary().keys != expected
    {
        return Err(refused());
    }
    Ok(&nodes[0])
}
fn only_private_leaf(nodes: &PrivateIndexRewriteData, expected: u64) -> Result<&[u8], AppError> {
    if nodes.nodes_data().len() != 1 {
        return Err(refused());
    }
    let node = &nodes.nodes_data()[0];
    if node.summary_data().height != 0 || node.summary_data().keys != expected {
        return Err(refused());
    }
    Ok(node.plain_data())
}
impl PreparedV2TextAcceptedCreate {
    fn completed_operation_data(&self) -> Result<ArchiveOperation, AppError> {
        let v = self
            .signature
            .intent
            .unsigned
            .unsigned_data(&self.signature.intent.retained)?;
        Ok(ArchiveOperation::Append {
            body: v.body.id(),
            admission: v.record.spec().context.admission,
            before: self.parent.directory.accepted_leaf_count,
            after: self
                .parent
                .directory
                .accepted_leaf_count
                .checked_add(1)
                .ok_or_else(refused)?,
        })
    }
    fn completed_receipt_data(&self) -> Result<ReceiptDigestData, AppError> {
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
            self.parent.directory.accepted_leaf_count,
            self.parent.header.id(),
        ))
    }
    /// Account/source checks before each seal and after caching it. Nothing in this
    /// method reads a public network dependency or manufactures an acceptance witness.
    pub(in crate::store::persistence::v2_text::transaction::intent::raw::accepted) fn prepare_completed_candidates(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        rng: &mut dyn CryptoRngCore,
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
            let operation = self.completed_operation_data()?;
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
            // A1 creates a leaf only; the genuine 1->2 append creates the new
            // leaf AND its completed parent. The manifest descriptor must report
            // the actual Core output count, not copy the first profile's literal.
            let node_entries = u16::try_from(candidate.append_data().created_nodes().len())
                .map_err(|_| refused())?;
            self.require_original_candidates(store, source)?;
            let page = self.prepare_page(&key, &plain, rng)?;
            self.nodes = Some(AcceptedPage {
                page,
                plain_bytes: plain.len(),
                entries: node_entries,
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
            let operation = self.completed_operation_data()?;
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
                operation: self.completed_operation_data()?,
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
            let producer = self.signature.intent.retained.producer_data();
            // The original retention manifest belongs to H_A. Its exact canonical
            // Action bytes are still owned by the SAME SDK custody (never reconstructed
            // from rendered text). Encoding here is bounded and paid before allocation.
            let action_bytes = v.action.encode();
            let claim = ActionClaimData {
                record: *v.record.id().as_bytes(),
                ordinal: self.parent.directory.accepted_leaf_count,
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
                receipt: self.completed_receipt_data()?,
                occurrences: 1,
                hint: ClaimHint::Canonical,
            };
            let path = self
                .signature
                .intent
                .query
                .borrow_completed_actions_path_data(&self.signature.intent.retained)?;
            let rewritten = path.rewrite_leaf_data(LeafRowData {
                key: *v.action.id().as_bytes(),
                value: ValueData::Action(claim),
            })?;
            let plain = only_accepted_leaf(&rewritten, 2)?;
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
            let action_root = Self::completed_accepted_root_data(
                &key,
                self.actions.as_ref().ok_or_else(refused)?,
                2,
            )?;
            let TextRootData::Present(action_root) = action_root else {
                return Err(refused());
            };
            let value = DirectoryData {
                tree: DirectoryTreeData::Present(action_root),
                accepted_leaf_count: actual.append_data().new_tip().count,
                unique_action_count: self
                    .parent
                    .directory
                    .unique_action_count
                    .checked_add(1)
                    .ok_or_else(refused)?,
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
            let path = self
                .signature
                .intent
                .query
                .borrow_completed_directory_path_data(&self.signature.intent.retained)?;
            let rewritten = path.rewrite_leaf_data(LeafRowData {
                key: *c.admission.as_bytes(),
                value: ValueData::Directory(value),
            })?;
            let plain = only_accepted_leaf(&rewritten, 1)?;
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
                receipt: self.completed_receipt_data()?,
                archive: ArchiveHeadId::from_bytes(
                    *self
                        .archive
                        .as_ref()
                        .ok_or_else(refused)?
                        .id_data()
                        .as_bytes(),
                ),
                ordinal: self.parent.head.queue_next,
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
                ordinal: self.parent.directory.accepted_leaf_count,
                receipt: self.completed_receipt_data()?,
            });
            let plain = self.completed_token_rewrite_data(&key, v)?;
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
                receipt: self.completed_receipt_data()?,
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
            let path = self
                .signature
                .intent
                .query
                .borrow_completed_publication_path_data(&self.signature.intent.retained)?;
            let rewritten = path
                .rewrite_leaf_data(&PrivateIndexLeafRowData {
                    key: PublicationValueData::ordinal_key_data(self.parent.head.queue_next),
                    value: PrivateIndexValueData::Publication(v),
                })
                .map_err(|_| refused())?;
            let plain = only_private_leaf(&rewritten, 2)?;
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
                directory: Self::completed_accepted_root_data(
                    &key,
                    self.directory.as_ref().ok_or_else(refused)?,
                    1,
                )?,
                tokens: Self::completed_private_root_data(
                    &key,
                    self.token.as_ref().ok_or_else(refused)?,
                    PrivateIndexKindData::Token,
                    p.context.id_data()?,
                    2,
                )?,
                publications: Self::completed_private_root_data(
                    &key,
                    self.publications.as_ref().ok_or_else(refused)?,
                    PrivateIndexKindData::Publication,
                    p.context.id_data()?,
                    2,
                )?,
                queue_front: p.queue_front,
                queue_next: p.queue_next.checked_add(1).ok_or_else(refused)?,
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
            self.signature.intent.require_original_completed_carry(
                &RetirementCarryData::decode_data(&self.signature.intent.carry)?,
            )?;
            if previous.roots_data().len() != 3
                || previous.entries_data()
                    != self
                        .signature
                        .intent
                        .retained
                        .completed_retirement_entries_data()?
            {
                return Err(refused());
            }
            let mut roots = previous.roots_data().to_vec();
            roots.push(TextHeadIdData::of_data(&self.parent.head_frame));
            roots.sort_unstable();
            let entries = self.completed_retirement_entries_data()?;
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
            super::semantic::verify_completed_b_data(self, &key, &w)?;
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

    fn completed_token_rewrite_data(
        &self,
        key: &[u8; 32],
        replacement: TokenValueData,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let context = self.parent.head.context.id_data()?;
        let token_key = *replacement.key_data(context).as_bytes();
        let mut path = PrivateIndexPathData::new_data(
            PrivateIndexContextData {
                cid: context,
                kind: PrivateIndexKindData::Token,
            },
            self.parent.token_root,
            token_key,
        )
        .map_err(|_| refused())?;
        path.accept_frame_data(key, &self.parent.token_frame)
            .map_err(|_| refused())?;
        let row = path
            .row_data()
            .map_err(|_| refused())?
            .ok_or_else(refused)?;
        if row.value != PrivateIndexValueData::Token(self.retained_token_data(key)?) {
            return Err(refused());
        }
        let rewrite = path
            .rewrite_leaf_data(&PrivateIndexLeafRowData {
                key: token_key,
                value: PrivateIndexValueData::Token(replacement),
            })
            .map_err(|_| refused())?;
        Ok(Zeroizing::new(only_private_leaf(&rewrite, 2)?.to_vec()))
    }
    pub(super) fn completed_accepted_root_data(
        key: &[u8; 32],
        page: &PreparedPrivateChatPage,
        expected: u64,
    ) -> Result<TextRootData, AppError> {
        let plain =
            open_private_chat_page(key, page.id(), page.as_bytes()).map_err(|_| refused())?;
        let node = NodeFrame::decode(&plain, NodePlacementData::Root).map_err(|_| refused())?;
        if node.kind() != NodeKindData::Leaf
            || node.summary().height != 0
            || node.summary().keys != expected
        {
            return Err(refused());
        }
        Ok(TextRootData::Present(NodeRefData {
            page: page.id(),
            summary: node.summary(),
        }))
    }
    pub(super) fn completed_private_root_data(
        key: &[u8; 32],
        page: &PreparedPrivateChatPage,
        kind: PrivateIndexKindData,
        cid: crate::store::persistence::v2_text::common::TextContextIdData,
        expected: u64,
    ) -> Result<TextRootData, AppError> {
        let plain =
            open_private_chat_page(key, page.id(), page.as_bytes()).map_err(|_| refused())?;
        let node = PrivateIndexNodeFrame::decode(
            &plain,
            PrivateIndexContextData { cid, kind },
            NodePlacementData::Root,
        )
        .map_err(|_| refused())?;
        if node.kind() != NodeKindData::Leaf
            || node.summary().height != 0
            || node.summary().keys != expected
        {
            return Err(refused());
        }
        Ok(TextRootData::Present(NodeRefData {
            page: page.id(),
            summary: node.summary(),
        }))
    }
    pub(in crate::store::persistence::v2_text::transaction::intent::raw::accepted) fn completed_retirement_entries_data(
        &self,
    ) -> Result<Vec<RetirementEntryData>, AppError> {
        let original = self.completed.as_ref().ok_or_else(refused)?;
        let mut entries = self
            .signature
            .intent
            .retained
            .completed_retirement_entries_data()?
            .to_vec();
        entries.extend_from_slice(&[
            original.actions,
            original.publications,
            RetirementEntryData {
                kind: RetirementKindData::AcceptedIndex,
                id: *self.parent.directory_root.page.as_bytes(),
                frame_bytes: u32::try_from(self.parent.directory_frame_bytes)
                    .map_err(|_| refused())?,
            },
            RetirementEntryData {
                kind: RetirementKindData::TokenIndex,
                id: *self.parent.token_root.page.as_bytes(),
                frame_bytes: u32::try_from(self.parent.token_frame.len()).map_err(|_| refused())?,
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
                frame_bytes: u32::try_from(
                    self.signature
                        .signature
                        .as_ref()
                        .ok_or_else(refused)?
                        .frame_data()
                        .len(),
                )
                .map_err(|_| refused())?,
            },
        ]);
        entries.sort_unstable_by_key(|e| (e.kind, e.id));
        if entries.len() != 7 {
            return Err(refused());
        }
        Ok(entries)
    }
}
