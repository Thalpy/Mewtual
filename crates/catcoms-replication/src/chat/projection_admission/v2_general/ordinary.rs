//! Exact ordinary dependency/delegation verification and separate effect issuance.
use super::*;

/// Caller-supplied DATA loan, not a witness. The issuer independently checks exact minimal
/// dependency membership, every real signature/delegation and the opaque selected Admission.
/// It does not recursively expand arbitrary caller coverage or accept unused extras.
pub struct V2OrdinaryDependency<'a> {
    action: &'a SignedAction,
    admission: &'a V2AuthenticatedAdmission,
}
impl<'a> V2OrdinaryDependency<'a> {
    pub fn new(action: &'a SignedAction, admission: &'a V2AuthenticatedAdmission) -> Self {
        Self { action, admission }
    }
}

/// Closed inclusion source. Neither variant accepts a parsed tip, label, boolean or caller
/// 'verified' witness. The actual verifier produced both opaque provenance owners.
pub enum V2GeneralAcceptanceSource<'a> {
    Current(&'a V2AuthenticatedAdmission),
    Frozen(&'a V2FrozenAdmission),
}
redacted_debug!(V2OrdinaryDependency<'_>, V2GeneralAcceptanceSource<'_>);

#[derive(Debug, PartialEq, Eq)]
pub enum V2OrdinaryReceiveStatus {
    WaitingForDependencies(Vec<ActionId>),
    Retained(ActionState),
}

struct LedgerDependencies<'a> {
    missing: Vec<ActionId>,
    exact: Vec<(&'a SignedAction, V2AuthenticatedAdmission)>,
}

impl V2GeneralPrefixVerifier {
    fn require_admission(&self, admission: &V2AuthenticatedAdmission) -> Result<(), ChatError> {
        if !Arc::ptr_eq(&self.identity, &admission.core.identity)
            || admission.core.body.spec().profile != self.math.profile().id()
            || admission.core.body.spec().group != self.math.profile().spec().group
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }
    fn require_view(&self, action: &SignedAction, view: &ProjectionView) -> Result<(), ChatError> {
        self.require_view_spec(action.spec(), view)
    }
    pub(super) fn require_view_spec(
        &self,
        spec: &ActionSpec,
        view: &ProjectionView,
    ) -> Result<(), ChatError> {
        if !view.life.live()
            || view.life.realm != spec.realm.id()
            || !view
                .life
                .group
                .as_ref()
                .is_some_and(|group| Arc::ptr_eq(group, &self.group))
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }

    /// Authenticate ONE exact historical action under this verified governing prefix.
    /// Removal cannot erase these historical MEMBER rights, but this value supplies neither
    /// OPEN nor exact acceptance. The ledger still verifies every dependency's semantic
    /// validity/causal target before an entry becomes Valid. Cross-author moderation has no
    /// approval producer here and therefore refuses rather than borrowing owner certification.
    pub fn verify_action_semantics(
        &self,
        action: &SignedAction,
        admission: &V2AuthenticatedAdmission,
        dependencies: &[V2OrdinaryDependency<'_>],
    ) -> Result<VerifiedAuthorityBasis, ChatError> {
        // Historical issuance still verifies the REAL action signature. An unsigned Draft
        // can reach only the shared private shape/delegation/dependency kernel below.
        self.live()?;
        if !action.verify() {
            return Err(ChatError::Signature);
        }
        let covered = self.verify_unsigned_authority(
            action.spec(),
            action.signer(),
            admission,
            dependencies,
        )?;
        self.live()?;
        Ok(VerifiedAuthorityBasis {
            realm: action.spec().realm.id(),
            basis: action.spec().basis,
            grant: action.spec().grant,
            subject: *action.signer(),
            rights: Rights::MEMBER,
            covered,
            constraint: SemanticConstraint::V2OrdinaryAction {
                action: action.id(),
                profile: self.math.profile().id(),
                admission: admission.core.body.admission_id(),
                tenure: self.math.profile().tenure(),
            },
        })
    }

    // ONLY the signed issuer and actual current-Admission pre-key issuer use this kernel.
    // Dependency actions retain their real signature/delegation checks; no dummy signature,
    // reusable witness, caller coverage extras or conditional verification bypass exists.
    // The actual host separately prepays ordinary_authorization_temporary_layout_bytes()
    // outside writers. Draft/Plan/replay invoices cannot silently substitute for this work.
    pub(super) fn verify_unsigned_authority(
        &self,
        spec: &ActionSpec,
        signer: &[u8; 32],
        admission: &V2AuthenticatedAdmission,
        dependencies: &[V2OrdinaryDependency<'_>],
    ) -> Result<BTreeSet<BasisId>, ChatError> {
        self.live()?;
        self.require_admission(admission)?;
        crate::chat::action::validate_spec(spec)?;
        let labels = admission.core.body.ordinary_labels(&spec.realm)?;
        if admission.core.body.spec().subject != *signer
            || labels.grant != spec.grant
            || labels.basis != spec.basis
        {
            return Err(ChatError::Authority);
        }
        let required = crate::chat::action::dependencies_spec(spec);
        if required.len() > MAX_PARENTS + 1 || dependencies.len() != required.len() {
            return Err(ChatError::Dependency);
        }
        let mut covered = BTreeSet::new();
        let mut target = None;
        for (id, dependency) in required.iter().zip(dependencies) {
            if *id != dependency.action.id() || dependency.action.spec().realm != spec.realm {
                return Err(ChatError::Dependency);
            }
            self.require_admission(dependency.admission)?;
            dependency
                .admission
                .core
                .body
                .matches_action_delegation(dependency.action)?;
            covered.insert(dependency.action.spec().basis);
            if spec.body.create() == Some(*id) {
                if !matches!(dependency.action.spec().body, Body::Create { .. }) {
                    return Err(ChatError::Dependency);
                }
                target = Some(dependency.action);
            }
            if matches!(spec.body, Body::Create { reply_to: Some(reply), .. } if reply == *id)
                && !matches!(dependency.action.spec().body, Body::Create { .. })
            {
                return Err(ChatError::Dependency);
            }
        }
        if matches!(spec.body, Body::Edit { .. } | Body::Delete { .. })
            && target.is_none_or(|original| original.signer() != signer)
        {
            return Err(ChatError::Permission);
        }
        self.live()?;
        Ok(covered)
    }

    /// Owner-offline ordinary permission. Actual selected Admission provenance, exact
    /// action/dependencies and the SAME original OPEN/source/group/channel are all mandatory.
    /// A fresh public ProjectionOpenAdmission with equal labels never enters this path.
    /// Creates, replies, own Edit/Delete and Join remain Provisional until separate inclusion.
    pub fn provisional_action(
        &self,
        action: &SignedAction,
        admission: &V2AuthenticatedAdmission,
        dependencies: &[V2OrdinaryDependency<'_>],
        view: &ProjectionView,
    ) -> Result<ProjectionAdmissionToken, ChatError> {
        self.provisional_evidence(action, admission, dependencies, view)
            .map(|(_, token)| token)
    }

    /// Import one already opened ordinary action into the actual receiver
    /// ledger. Exact dependencies must be provided in canonical order. A
    /// pending action has no current effect and can be retried under the same
    /// original view after bounded semantic work or dependency recovery.
    pub fn project_provisional_action(
        &self,
        ledger: &mut ChatLedger,
        view: &ProjectionView,
        action: &SignedAction,
        admission: &V2AuthenticatedAdmission,
        dependencies: &[V2OrdinaryDependency<'_>],
        work: usize,
    ) -> Result<ActionState, ChatError> {
        self.require_receiver_ledger(ledger, view, action)?;
        let (verified, token) = self.provisional_evidence(action, admission, dependencies, view)?;
        Self::apply_provisional(ledger, action, work, verified, token)
    }

    fn apply_provisional(
        ledger: &mut ChatLedger,
        action: &SignedAction,
        work: usize,
        verified: VerifiedAuthorityBasis,
        token: ProjectionAdmissionToken,
    ) -> Result<ActionState, ChatError> {
        let state = Self::apply_verified(ledger, action, work, verified)?;
        if state == ActionState::Valid {
            ledger.admit_projection(&token, work)?;
        }
        Ok(state)
    }

    fn apply_verified(
        ledger: &mut ChatLedger,
        action: &SignedAction,
        work: usize,
        verified: VerifiedAuthorityBasis,
    ) -> Result<ActionState, ChatError> {
        let id = action.id();
        match ledger.action(id) {
            Some(original) if original == action => {}
            Some(_) => return Err(ChatError::AuthorityConflict),
            None => {
                ledger.import(action.clone())?;
            }
        }
        ledger.authorize(id, verified)?;
        ledger.advance(work);
        ledger.state(id).ok_or(ChatError::Unknown)
    }

    /// Resolve only the action's exact canonical dependencies from this
    /// ledger's Valid signed originals and their verified governing Admission.
    /// Missing evidence does not import a partially authorized child.
    pub fn project_provisional_from_ledger(
        &self,
        ledger: &mut ChatLedger,
        view: &ProjectionView,
        action: &SignedAction,
        admission: &V2AuthenticatedAdmission,
        work: usize,
    ) -> Result<V2OrdinaryReceiveStatus, ChatError> {
        self.require_receiver_ledger(ledger, view, action)?;
        let LedgerDependencies { missing, exact } = self.ledger_dependencies(ledger, action)?;
        if !missing.is_empty() {
            self.require_admission(admission)?;
            admission.core.body.matches_action_delegation(action)?;
            self.observe_live_member(admission.core.body.admission_id())?;
            return Ok(V2OrdinaryReceiveStatus::WaitingForDependencies(missing));
        }
        let dependencies = exact
            .iter()
            .map(|(original, owner)| V2OrdinaryDependency::new(original, owner))
            .collect::<Vec<_>>();
        let (verified, token) =
            self.provisional_evidence(action, admission, &dependencies, view)?;
        Self::apply_provisional(ledger, action, work, verified, token)
            .map(V2OrdinaryReceiveStatus::Retained)
    }

    /// Rebuild semantic history from exact stored originals and the verified
    /// governing prefix. Replay restores dependency knowledge without issuing
    /// an OPEN observation, projection token or current transport authority.
    pub fn replay_ordinary_history_original(
        &self,
        ledger: &mut ChatLedger,
        view: &ProjectionView,
        action: &SignedAction,
        admission: &V2AuthenticatedAdmission,
        work: usize,
    ) -> Result<V2OrdinaryReceiveStatus, ChatError> {
        self.require_receiver_ledger(ledger, view, action)?;
        let LedgerDependencies { missing, exact } = self.ledger_dependencies(ledger, action)?;
        if !missing.is_empty() {
            self.require_admission(admission)?;
            admission.core.body.matches_action_delegation(action)?;
            self.live()?;
            return Ok(V2OrdinaryReceiveStatus::WaitingForDependencies(missing));
        }
        let dependencies = exact
            .iter()
            .map(|(original, owner)| V2OrdinaryDependency::new(original, owner))
            .collect::<Vec<_>>();
        let verified = self.verify_action_semantics(action, admission, &dependencies)?;
        Self::apply_verified(ledger, action, work, verified).map(V2OrdinaryReceiveStatus::Retained)
    }

    fn ledger_dependencies<'a>(
        &self,
        ledger: &'a ChatLedger,
        action: &SignedAction,
    ) -> Result<LedgerDependencies<'a>, ChatError> {
        let profile = self.math.profile();
        let required = action.dependencies();
        let mut missing = Vec::new();
        let mut owners = Vec::with_capacity(required.len());
        for id in required {
            let Some(entry) = ledger.entries.get(&id) else {
                missing.push(id);
                continue;
            };
            match entry.state {
                ActionState::Pending(_) => {
                    missing.push(id);
                    continue;
                }
                ActionState::Invalid(_) => return Err(ChatError::Dependency),
                ActionState::Valid => {}
            }
            let Some(verified) = &entry.authority else {
                return Err(ChatError::Authority);
            };
            let (origin, owner, tenure) = match &verified.constraint {
                SemanticConstraint::V2OrdinaryAction {
                    profile,
                    admission,
                    tenure,
                    ..
                }
                | SemanticConstraint::V2FounderCreate {
                    profile,
                    admission,
                    tenure,
                    ..
                } => (profile, admission, tenure),
                SemanticConstraint::LegacyReusable => return Err(ChatError::Authority),
            };
            if *origin != profile.id() || *tenure != profile.tenure() {
                return Err(ChatError::Scope);
            }
            owners.push((&entry.action, self.admission(*owner)?));
        }
        Ok(LedgerDependencies {
            missing,
            exact: owners,
        })
    }

    fn require_receiver_ledger(
        &self,
        ledger: &ChatLedger,
        view: &ProjectionView,
        action: &SignedAction,
    ) -> Result<(), ChatError> {
        if !ledger
            .projection
            .current
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &view.life))
        {
            return Err(ChatError::Scope);
        }
        self.require_view(action, view)
    }

    fn provisional_evidence(
        &self,
        action: &SignedAction,
        admission: &V2AuthenticatedAdmission,
        dependencies: &[V2OrdinaryDependency<'_>],
        view: &ProjectionView,
    ) -> Result<(VerifiedAuthorityBasis, ProjectionAdmissionToken), ChatError> {
        let verified = self.verify_action_semantics(action, admission, dependencies)?;
        self.require_view(action, view)?;
        if !self
            .admissions
            .iter()
            .any(|a| Arc::ptr_eq(&a.core, &admission.core))
            || !admission.core.open_origin.live()
        {
            return Err(ChatError::Scope);
        }
        self.live()?;
        let token = ProjectionAdmissionToken {
            binding: ActionBinding::of(action.id(), action),
            context: EvidenceContext::V2Ordinary {
                profile: self.math.profile().id(),
                admission: admission.core.body.admission_id(),
                tenure: self.math.profile().tenure(),
            },
            view: view.life.clone(),
            open: None,
            support: ProjectionSupport::GeneralOpen {
                origin: admission.core.open_origin.clone(),
            },
        };
        Ok((verified, token))
    }

    /// Exact LOG inclusion is independent of OPEN. A frozen original removal cut may prove
    /// an old action accepted, but a late unproven action, timestamp or new transport seal
    /// cannot. This never appends to a cut or grants REVIEWED moderation. Proven Deletes
    /// deliberately refuse until the separate exact V2 negative-Delete capsule producer is
    /// implemented; issuing one without persistent proof-loss custody could resurrect text.
    #[allow(clippy::too_many_arguments)]
    pub fn accepted_action(
        &self,
        action: &SignedAction,
        source: V2GeneralAcceptanceSource<'_>,
        dependencies: &[V2OrdinaryDependency<'_>],
        record: &AcceptanceRecord,
        index: u64,
        siblings: &[TreeHash],
        view: &ProjectionView,
    ) -> Result<ProjectionAdmissionToken, ChatError> {
        let (admission, tip) = match source {
            V2GeneralAcceptanceSource::Current(admission) => {
                self.require_admission(admission)?;
                if !self
                    .admissions
                    .iter()
                    .any(|a| Arc::ptr_eq(&a.core, &admission.core))
                {
                    return Err(ChatError::Scope);
                }
                let slot = self
                    .math
                    .slot(admission.core.body.id())
                    .ok_or(ChatError::Dependency)?;
                (
                    admission,
                    TreeTip {
                        context: TreeContext {
                            profile: self.math.profile().id(),
                            admission: admission.core.body.admission_id(),
                        },
                        count: slot.image.accepted_count,
                        root: slot.image.accepted_root,
                    },
                )
            }
            V2GeneralAcceptanceSource::Frozen(frozen) => (&frozen.admission, frozen.cut.tip),
        };
        self.verify_action_semantics(action, admission, dependencies)?;
        self.require_view(action, view)?;
        if matches!(action.spec().body, Body::Delete { .. }) {
            return Err(ChatError::Dependency);
        }
        if record.spec().context != tip.context {
            return Err(ChatError::Scope);
        }
        record.matches_action(action)?;
        verify_inclusion(tip, record.leaf(), index, siblings).map_err(|_| ChatError::Authority)?;
        self.live()?;
        Ok(ProjectionAdmissionToken {
            binding: ActionBinding::of(action.id(), action),
            context: EvidenceContext::V2Ordinary {
                profile: self.math.profile().id(),
                admission: admission.core.body.admission_id(),
                tenure: self.math.profile().tenure(),
            },
            view: view.life.clone(),
            open: None,
            support: ProjectionSupport::GeneralAccepted {
                record: record.id(),
                leaf_index: index,
                origin: self.origin.clone(),
            },
        })
    }
}
