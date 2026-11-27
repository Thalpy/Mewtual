//! Exact ordinary dependency/delegation verification and separate effect issuance.
use super::*;
use crate::chat::owner_facts::OwnerFactId;

// A detached host replay can retain ancestry from the entire bounded membership prefix.
const MAX_LOCAL_ARCHIVE_RETIREMENTS: usize = 64;

/// Caller-supplied DATA loan, not a witness. The issuer independently checks exact minimal
/// dependency membership, every real signature/delegation and the opaque selected Admission.
/// It does not recursively expand arbitrary caller coverage or accept unused extras.
pub struct V2OrdinaryDependency<'a> {
    action: &'a SignedAction,
    admission: &'a V2AuthenticatedAdmission,
    local_archive_ancestor: bool,
}
impl<'a> V2OrdinaryDependency<'a> {
    pub fn new(action: &'a SignedAction, admission: &'a V2AuthenticatedAdmission) -> Self {
        Self {
            action,
            admission,
            local_archive_ancestor: false,
        }
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

/// One exact original selected by the trusted host's local archive. This
/// can restore detached display or an exact local reply ancestor. It
/// cannot open an Admission or prove another peer selected the action.
pub struct V2LocalArchiveOriginal<'a> {
    action: &'a SignedAction,
    admission: OwnerFactId,
    digest: [u8; 32],
}

redacted_debug!(V2LocalArchiveOriginal<'_>);

impl<'a> V2LocalArchiveOriginal<'a> {
    /// The host must authenticate its own selected record, including the
    /// archive scope and Admission, before supplying these original bytes.
    pub fn from_trusted_local_selection(
        action: &'a SignedAction,
        admission: OwnerFactId,
        original: &[u8],
    ) -> Result<Self, ChatError> {
        if zeroize::Zeroizing::new(action.encode()).as_slice() != original {
            return Err(ChatError::AuthorityConflict);
        }
        Ok(Self {
            action,
            admission,
            digest: *blake3::hash(original).as_bytes(),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DependencyEvidence {
    Exact,
    ArchiveDisplay,
    LocallySelectedAncestors,
    Incoming,
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
        self.apply_open_original(
            ledger,
            view,
            action,
            admission,
            (&[], DependencyEvidence::Exact),
            (work, true),
        )
    }

    /// Retain the same verified causal original without changing the live
    /// projection. The archive can then select it before readers see it.
    pub fn stage_provisional_from_ledger(
        &self,
        ledger: &mut ChatLedger,
        view: &ProjectionView,
        action: &SignedAction,
        admission: &V2AuthenticatedAdmission,
        work: usize,
    ) -> Result<V2OrdinaryReceiveStatus, ChatError> {
        self.apply_open_original(
            ledger,
            view,
            action,
            admission,
            (&[], DependencyEvidence::Exact),
            (work, false),
        )
    }

    /// Apply a current author's original. A retired ancestor needs this
    /// ledger's exact local receipt marker or it is reported as missing.
    pub fn apply_with_locally_selected_ancestors(
        &self,
        ledger: &mut ChatLedger,
        view: &ProjectionView,
        action: &SignedAction,
        admission: &V2AuthenticatedAdmission,
        retired: &[&V2FrozenAdmission],
        (work, publish): (usize, bool),
    ) -> Result<V2OrdinaryReceiveStatus, ChatError> {
        if retired.len() > MAX_LOCAL_ARCHIVE_RETIREMENTS {
            return Err(ChatError::Bound);
        }
        self.apply_open_original(
            ledger,
            view,
            action,
            admission,
            (retired, DependencyEvidence::Incoming),
            (work, publish),
        )
    }

    fn apply_open_original(
        &self,
        ledger: &mut ChatLedger,
        view: &ProjectionView,
        action: &SignedAction,
        admission: &V2AuthenticatedAdmission,
        (retired, evidence): (&[&V2FrozenAdmission], DependencyEvidence),
        (work, publish): (usize, bool),
    ) -> Result<V2OrdinaryReceiveStatus, ChatError> {
        if view.life.local_archive {
            return Err(ChatError::Scope);
        }
        self.require_receiver_ledger(ledger, view, action)?;
        let LedgerDependencies { missing, exact } =
            self.ledger_dependencies(ledger, action, retired, evidence)?;
        if !missing.is_empty() {
            self.require_admission(admission)?;
            admission.core.body.matches_action_delegation(action)?;
            self.observe_live_member(admission.core.body.admission_id())?;
            return Ok(V2OrdinaryReceiveStatus::WaitingForDependencies(missing));
        }
        let dependencies = exact
            .iter()
            .map(|(original, owner)| V2OrdinaryDependency {
                action: original,
                admission: owner,
                // Incoming checked the exact selected marker and refused
                // a retired mutation target before borrowing its owner.
                local_archive_ancestor: evidence == DependencyEvidence::Incoming,
            })
            .collect::<Vec<_>>();
        let (verified, token) =
            self.provisional_evidence(action, admission, &dependencies, view)?;
        let state = if publish {
            Self::apply_provisional(ledger, action, work, verified, token)?
        } else {
            Self::apply_verified(ledger, action, work, verified)?
        };
        Ok(V2OrdinaryReceiveStatus::Retained(state))
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
        frozen_dependencies: &[&V2FrozenAdmission],
        work: usize,
    ) -> Result<V2OrdinaryReceiveStatus, ChatError> {
        self.require_receiver_ledger(ledger, view, action)?;
        let LedgerDependencies { missing, exact } = self.ledger_dependencies(
            ledger,
            action,
            frozen_dependencies,
            DependencyEvidence::Exact,
        )?;
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

    /// Borrow the exact locally selected Create as the target of a new
    /// reply. Its selected Remove supplies the original Admission, and the
    /// original bytes and signature must still match that Admission. This
    /// loan supplies only the parent and cannot issue a live author token.
    pub fn local_archive_reply_parent<'a>(
        &self,
        original: V2LocalArchiveOriginal<'a>,
        retired: &'a [V2FrozenAdmission],
    ) -> Result<V2OrdinaryDependency<'a>, ChatError> {
        if !matches!(original.action.spec().body, Body::Create { .. })
            || retired.len() > MAX_LOCAL_ARCHIVE_RETIREMENTS
        {
            return Err(ChatError::Dependency);
        }
        let frozen = retired
            .iter()
            .find(|entry| entry.admission.core.body.admission_id() == original.admission)
            .ok_or(ChatError::Dependency)?;
        self.require_admission(&frozen.admission)?;
        frozen
            .admission
            .core
            .body
            .matches_action_delegation(original.action)?;
        self.live()?;
        Ok(V2OrdinaryDependency {
            action: original.action,
            admission: &frozen.admission,
            local_archive_ancestor: true,
        })
    }

    /// The native signing plan can borrow a retired original only as the
    /// explicit reply target. Previously admitted semantic and LOG evidence
    /// may still make sense of older causal heads without granting a key.
    pub(super) fn require_live_ordinary_dependencies(
        &self,
        spec: &ActionSpec,
        dependencies: &[V2OrdinaryDependency<'_>],
    ) -> Result<(), ChatError> {
        for dependency in dependencies {
            let active = self
                .admissions
                .iter()
                .any(|admission| Arc::ptr_eq(&admission.core, &dependency.admission.core));
            let retired_target = spec.body.create() == Some(dependency.action.id());
            if !active && (!dependency.local_archive_ancestor || retired_target) {
                return Err(ChatError::Dependency);
            }
        }
        Ok(())
    }

    /// Replay a previously selected original for this device's archive
    /// display. The actual signature and exact minimal causal ancestry are
    /// still reverified against this verifier's live and retired admissions.
    pub fn project_local_archive_original(
        &self,
        ledger: &mut ChatLedger,
        view: &ProjectionView,
        original: V2LocalArchiveOriginal<'_>,
        retired: &[&V2FrozenAdmission],
        work: usize,
    ) -> Result<V2OrdinaryReceiveStatus, ChatError> {
        let V2LocalArchiveOriginal {
            action,
            admission,
            digest,
        } = original;
        self.apply_local_archive_original(
            ledger,
            view,
            (action, admission),
            retired,
            work,
            (DependencyEvidence::ArchiveDisplay, Some(digest)),
        )
    }

    /// A local outbox original may also be an exact ancestry input. It
    /// supplies only semantic state and so cannot make a message visible.
    pub fn replay_local_archive_dependency(
        &self,
        ledger: &mut ChatLedger,
        view: &ProjectionView,
        action: &SignedAction,
        admission: OwnerFactId,
        retired: &[&V2FrozenAdmission],
        work: usize,
    ) -> Result<V2OrdinaryReceiveStatus, ChatError> {
        self.apply_local_archive_original(
            ledger,
            view,
            (action, admission),
            retired,
            work,
            (DependencyEvidence::ArchiveDisplay, None),
        )
    }

    /// Verify an exact selected record as historical semantic ancestry in
    /// an ordinary ledger. This never makes the ancestor visible.
    pub fn replay_locally_selected_ancestor(
        &self,
        ledger: &mut ChatLedger,
        view: &ProjectionView,
        original: V2LocalArchiveOriginal<'_>,
        retired: &[&V2FrozenAdmission],
        work: usize,
    ) -> Result<V2OrdinaryReceiveStatus, ChatError> {
        let V2LocalArchiveOriginal {
            action, admission, ..
        } = original;
        let status = self.apply_local_archive_original(
            ledger,
            view,
            (action, admission),
            retired,
            work,
            (DependencyEvidence::LocallySelectedAncestors, None),
        )?;
        if status == V2OrdinaryReceiveStatus::Retained(ActionState::Valid) {
            if !ledger.matches_valid_ordinary_original(action, admission) {
                return Err(ChatError::AuthorityConflict);
            }
            let entry = ledger
                .entries
                .get_mut(&action.id())
                .ok_or(ChatError::Unknown)?;
            entry.local_ancestry = Some(V2LocalAncestry {
                verifier: Arc::downgrade(&self.identity),
                admission,
                signature: *action.signature(),
            });
        }
        Ok(status)
    }

    /// True for a current verified dependency or this same verifier's
    /// selected local receipt on its exact retired signature. A hot Valid
    /// entry alone cannot skip the archive read after removal.
    pub fn locally_usable_ordinary_dependency(
        &self,
        ledger: &ChatLedger,
        action: ActionId,
    ) -> bool {
        let Some(entry) = ledger
            .entries
            .get(&action)
            .filter(|entry| entry.state == ActionState::Valid)
        else {
            return false;
        };
        let Some(verified) = &entry.authority else {
            return false;
        };
        let (profile, admission, tenure) = match &verified.constraint {
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
            SemanticConstraint::LegacyReusable => return false,
        };
        let expected = self.math.profile();
        *profile == expected.id()
            && *tenure == expected.tenure()
            && (self.admission(*admission).is_ok()
                || self.matches_local_ancestor(entry, *admission))
    }

    /// Recover own outbox ancestry as verified semantics. Its author must
    /// still have the exact current Admission, and the saved bytes give no
    /// receipt marker for the author after removal.
    pub fn replay_current_outbox_ancestor(
        &self,
        ledger: &mut ChatLedger,
        view: &ProjectionView,
        (action, admission): (&SignedAction, OwnerFactId),
        retired: &[&V2FrozenAdmission],
        work: usize,
    ) -> Result<V2OrdinaryReceiveStatus, ChatError> {
        self.admission(admission)?;
        self.apply_local_archive_original(
            ledger,
            view,
            (action, admission),
            retired,
            work,
            (DependencyEvidence::LocallySelectedAncestors, None),
        )
    }

    fn matches_local_ancestor(
        &self,
        entry: &crate::chat::ledger::Entry,
        admission: OwnerFactId,
    ) -> bool {
        entry.local_ancestry.as_ref().is_some_and(|selected| {
            selected.verifier.as_ptr() == Arc::as_ptr(&self.identity)
                && selected.admission == admission
                && &selected.signature == entry.action.signature()
        })
    }

    fn apply_local_archive_original(
        &self,
        ledger: &mut ChatLedger,
        view: &ProjectionView,
        (action, admission): (&SignedAction, OwnerFactId),
        retired: &[&V2FrozenAdmission],
        work: usize,
        (evidence, selected_digest): (DependencyEvidence, Option<[u8; 32]>),
    ) -> Result<V2OrdinaryReceiveStatus, ChatError> {
        self.require_receiver_ledger(ledger, view, action)?;
        if view.life.local_archive != (evidence == DependencyEvidence::ArchiveDisplay)
            || retired.len() > MAX_LOCAL_ARCHIVE_RETIREMENTS
        {
            return Err(ChatError::Scope);
        }
        let author = match self.admission(admission) {
            Ok(author) => author,
            Err(ChatError::Dependency) => retired
                .iter()
                .find(|frozen| frozen.admission.core.body.admission_id() == admission)
                .ok_or(ChatError::Dependency)?
                .admission
                .clone(),
            Err(error) => return Err(error),
        };
        let LedgerDependencies { missing, exact } =
            self.ledger_dependencies(ledger, action, retired, evidence)?;
        if !missing.is_empty() {
            self.require_admission(&author)?;
            author.core.body.matches_action_delegation(action)?;
            self.live()?;
            return Ok(V2OrdinaryReceiveStatus::WaitingForDependencies(missing));
        }
        let dependencies = exact
            .iter()
            .map(|(original, owner)| V2OrdinaryDependency::new(original, owner))
            .collect::<Vec<_>>();
        let verified = self.verify_action_semantics(action, &author, &dependencies)?;
        let state = if let Some(digest) = selected_digest {
            let token = ProjectionAdmissionToken {
                binding: ActionBinding::of(action.id(), action),
                context: EvidenceContext::V2Ordinary {
                    profile: self.math.profile().id(),
                    admission,
                    tenure: self.math.profile().tenure(),
                },
                view: view.life.clone(),
                open: None,
                support: ProjectionSupport::LocalArchive {
                    original: digest,
                    origin: self.origin.clone(),
                },
            };
            Self::apply_provisional(ledger, action, work, verified, token)?
        } else {
            Self::apply_verified(ledger, action, work, verified)?
        };
        Ok(V2OrdinaryReceiveStatus::Retained(state))
    }

    fn ledger_dependencies<'a>(
        &self,
        ledger: &'a ChatLedger,
        action: &SignedAction,
        frozen_dependencies: &[&V2FrozenAdmission],
        evidence: DependencyEvidence,
    ) -> Result<LedgerDependencies<'a>, ChatError> {
        let profile = self.math.profile();
        let required = action.dependencies();
        if evidence == DependencyEvidence::Exact && frozen_dependencies.len() > required.len() {
            return Err(ChatError::Dependency);
        }
        let mut missing = Vec::new();
        let mut owners = Vec::with_capacity(required.len());
        let mut used_frozen = 0_u128;
        const {
            assert!(MAX_LOCAL_ARCHIVE_RETIREMENTS < u128::BITS as usize);
        }
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
            let admission = match self.admission(*owner) {
                Ok(current) => current,
                Err(ChatError::Dependency) => {
                    // A retired predecessor can supply causal knowledge.
                    // A mutation's target must still name the current
                    // incarnation, even when the same signer rejoined.
                    if evidence == DependencyEvidence::Incoming
                        && action.spec().body.create() == Some(id)
                    {
                        return Err(ChatError::Permission);
                    }
                    if matches!(
                        evidence,
                        DependencyEvidence::Incoming | DependencyEvidence::LocallySelectedAncestors
                    ) && !self.matches_local_ancestor(entry, *owner)
                    {
                        missing.push(id);
                        continue;
                    }
                    let (index, historical) = frozen_dependencies
                        .iter()
                        .enumerate()
                        .find(|(_, frozen)| frozen.admission.core.body.admission_id() == *owner)
                        .ok_or(ChatError::Dependency)?;
                    self.require_admission(&historical.admission)?;
                    used_frozen |= 1_u128 << index;
                    historical.admission.clone()
                }
                Err(error) => return Err(error),
            };
            owners.push((&entry.action, admission));
        }
        if evidence == DependencyEvidence::Exact
            && missing.is_empty()
            && used_frozen.count_ones() as usize != frozen_dependencies.len()
        {
            return Err(ChatError::Dependency);
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
        if view.life.local_archive {
            return Err(ChatError::Scope);
        }
        self.require_live_ordinary_dependencies(action.spec(), dependencies)?;
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
        if view.life.local_archive {
            return Err(ChatError::Scope);
        }
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
