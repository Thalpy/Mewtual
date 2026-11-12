//! The v1 owner checkpoint install and the V2 joined-member source are mutually exclusive on
//! one node incarnation, whichever claims it first. The install replaces group/device/tenure
//! outside `with_observed_mls_transition`, so a joined source's fence could never observe it.
//! Real-key/actual-Add fixture from the parent module; the trusted-host binding is the same
//! explicit TEST contract, now also supplying the saved operation binding the install checks.
use super::*;
use crate::owner_evidence::ManagedOwnerOperationBinding;
use crate::owner_v2::{JoinFactsSlot, JoinedFenceNode, V2JoinFacts, V2JoinedFenceTerminal};

/// The parent fixture's binding plus the saved predecessor/request the install compares.
struct InstallBinding<'a> {
    base: &'a Binding,
    predecessor: OwnerOperationPredecessor,
    request: [u8; 32],
}
impl ManagedChatScopeReservation for InstallBinding<'_> {
    fn group(&self) -> &[u8] {
        self.base.group()
    }
    fn signer(&self) -> &[u8; 32] {
        self.base.signer()
    }
    fn domain(&self) -> [u8; 32] {
        self.base.domain()
    }
    fn nonce(&self) -> [u8; 32] {
        self.base.nonce()
    }
    fn owner_identity(&self) -> Arc<()> {
        self.base.owner_identity()
    }
    fn reservation_observer(&self) -> RequestCancellation {
        self.base.reservation_observer()
    }
    fn revocation(&self) -> Arc<dyn ManagedChatScopeRevocation> {
        self.base.revocation()
    }
}
impl ManagedChatScopeBinding for InstallBinding<'_> {
    fn publication_authority(&self) -> RequestCancellation {
        self.base.publication_authority()
    }
}
impl ManagedOwnerEvidenceBinding for InstallBinding<'_> {
    fn profile_id(&self) -> OwnerFactId {
        self.base.profile_id()
    }
    fn owner_founder(&self) -> &[u8; 32] {
        self.base.owner_founder()
    }
}
impl ManagedOwnerOperationBinding for InstallBinding<'_> {
    fn predecessor(&self) -> OwnerOperationPredecessor {
        self.predecessor
    }
    fn saved_request_binding(&self) -> [u8; 32] {
        self.request
    }
}

/// Fixture node after the App's covering snapshot, with the install's binding and candidate.
fn frozen_fixture() -> (Node, Binding, Option<PreparedOwnerMembership>) {
    let (mut node, binding, candidate) = fixture();
    freeze(&mut node, &candidate).unwrap();
    (node, binding, Some(candidate))
}

fn install_binding<'a>(
    binding: &'a Binding,
    prepared: &Option<PreparedOwnerMembership>,
) -> InstallBinding<'a> {
    let operation = &prepared.as_ref().unwrap().operation;
    InstallBinding {
        base: binding,
        predecessor: operation.predecessor,
        request: operation.binding,
    }
}

#[test]
fn v1_install_refuses_after_the_joined_source_claimed_the_node_and_keeps_the_candidate() {
    let (mut node, binding, mut prepared) = frozen_fixture();
    let install = install_binding(&binding, &prepared);
    node.v2_join_facts = JoinFactsSlot::TakenByJoinedSource;
    assert!(node
        .install_owner_direct_add_checkpoint(&mut prepared, &install)
        .is_err());
    // Refused before take or replacement: the whole candidate and the node are untouched.
    assert!(prepared.is_some());
    assert_eq!(node.epoch(), 0);
    assert_eq!(node.member_count(), 1);
    assert!(node.owner_operation.is_none());
    assert!(node.initial_owner_evidence.is_none());
    // The refused install did not displace the joined source's claim.
    assert!(matches!(
        node.v2_join_facts,
        JoinFactsSlot::TakenByJoinedSource
    ));

    // Control on the same inputs: with no joined claim the identical install succeeds, so the
    // refusal above was the exclusion and not another check. (Releasing the claim is a test
    // control only; production never returns a taken claim.)
    node.v2_join_facts = JoinFactsSlot::Absent;
    node.install_owner_direct_add_checkpoint(&mut prepared, &install)
        .unwrap();
    assert!(prepared.is_none());
    assert_eq!(node.epoch(), 1);
}

#[test]
fn join_facts_can_never_be_taken_after_the_v1_install_bound_the_node() {
    let (mut node, binding, mut prepared) = frozen_fixture();
    let install = install_binding(&binding, &prepared);
    // Untaken facts captured from this node's own real group, with the live fence an invite
    // join leaves beside them.
    let facts = V2JoinFacts::capture(&node.group, &node.device, [0xd1; 32]).unwrap();
    node.v2_join_facts = JoinFactsSlot::from_handoff(Some(facts));
    node.v2_fence = JoinedFenceNode::for_join(&node.group);
    let fence = node.v2_fence.handle().unwrap();
    node.install_owner_direct_add_checkpoint(&mut prepared, &install)
        .unwrap();
    assert!(matches!(node.v2_join_facts, JoinFactsSlot::BoundToV1Owner));
    let instance = node.registry_instance();
    assert!(node.take_v2_joined_join_facts(&instance).is_err());
    // That install bypassed the transition observer, so the fence closed for good.
    assert!(fence.is_closed());
    assert_eq!(
        fence.observe_data().unwrap().terminal,
        Some(V2JoinedFenceTerminal::V1Owner)
    );
    // And a later v1 install path stays open for the v1 owner (the exclusion is one-sided
    // per claim, never a lockout of the owner that won).
    assert!(node.require_no_v2_joined_source().is_ok());
}
