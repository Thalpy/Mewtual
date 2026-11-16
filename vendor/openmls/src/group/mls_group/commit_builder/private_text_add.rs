//! Narrow SERIAL computation for one PRIVATE ordinary text Add. This is a
//! negative SDK shape/runtime choice, never a fee, original Source, governance,
//! Current, membership observation, save/install or publication authority.
use super::*;
use crate::group::{
    mls_group::text_add_execution::TextAddExecution, validation_time::AddedKeyPackageValidationTime,
};
use openmls_traits::types::Ciphersuite;
use std::time::SystemTime;

fn unsupported() -> CreateCommitError {
    CreateCommitError::UnsupportedPrivateTextAdd
}

/// No allocation, crypto, key access, clock read or caller callback. Reject
/// malformed unmerged metadata BEFORE resolution() can assert on a blank leaf
/// or repeat references across unrelated subtrees. Local/remote authority is a
/// separate caller obligation; these facts merely close the supported bound.
fn require_tree(group: &MlsGroup) -> Result<(), CreateCommitError> {
    group.is_operational().map_err(|_| unsupported())?;
    if group.ciphersuite() != Ciphersuite::MLS_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519
        || group.pending_proposals().next().is_some()
    {
        return Err(unsupported());
    }
    let tree = group.treesync();
    let (leaf_slots, leaf_upper) = tree.tree().leaves().size_hint();
    let (parent_slots, parent_upper) = tree.tree().parents().size_hint();
    if leaf_upper != Some(leaf_slots)
        || parent_upper != Some(parent_slots)
        || !(1..=64).contains(&leaf_slots)
        || parent_slots != leaf_slots - 1
    {
        return Err(unsupported());
    }
    let mut present = [false; 64];
    let mut occupied = 0usize;
    for (index, _) in tree.full_leaves() {
        let i = index.u32() as usize;
        if i >= 64 || occupied == 64 || present[i] {
            return Err(unsupported());
        }
        present[i] = true;
        occupied += 1;
    }
    let mut blanks = 0usize;
    for index in tree.blank_leaves() {
        let i = index.u32() as usize;
        if i >= 64 || blanks == 64 || present[i] {
            return Err(unsupported());
        }
        blanks += 1;
    }
    let slots = occupied.checked_add(blanks).ok_or_else(unsupported)?;
    let own = group.own_leaf_index().u32() as usize;
    if slots != leaf_slots || own >= slots || !present[own] || (slots == 64 && blanks == 0) {
        return Err(unsupported());
    }
    let mut parents = 0usize;
    for (index, parent) in tree.full_parents() {
        if parents == 63 || index.u32() as usize >= slots - 1 {
            return Err(unsupported());
        }
        parents += 1;
        let node = index
            .u32()
            .checked_mul(2)
            .and_then(|n| n.checked_add(1))
            .ok_or_else(unsupported)?;
        let height = node.trailing_ones();
        let span = 1u32
            .checked_shl(height)
            .and_then(|n| n.checked_sub(1))
            .ok_or_else(unsupported)?;
        let first = node.checked_sub(span).ok_or_else(unsupported)? / 2;
        let last = node.checked_add(span).ok_or_else(unsupported)? / 2;
        let unmerged = parent.unmerged_leaves();
        if unmerged.len() > 64 {
            return Err(unsupported());
        }
        let mut previous = None;
        for leaf in unmerged {
            let i = leaf.u32();
            if i < first
                || i > last
                || i as usize >= slots
                || !present[i as usize]
                || previous.is_some_and(|old| old >= i)
            {
                return Err(unsupported());
            }
            previous = Some(i);
        }
    }
    for index in tree.blank_parents() {
        if parents == 63 || index.u32() as usize >= slots - 1 {
            return Err(unsupported());
        }
        parents += 1;
    }
    if parents != slots - 1 {
        return Err(unsupported());
    }
    group
        .epoch()
        .as_u64()
        .checked_add(1)
        .ok_or_else(unsupported)?;
    Ok(())
}

impl<'a, G: BorrowMut<MlsGroup>> CommitBuilder<'a, LoadedPsks, G> {
    /// Build exactly one forced-path Add with explicit added-KP lifetime time
    /// and SERIAL path derivation/encryption. It shares all ordinary validation,
    /// crypto, signing, error and message algorithms; only the three TreeKEM
    /// parallel iterator choices differ. No Rayon pool/job/thread initialization
    /// occurs on this route. Other SDK entrypoints retain their usual behavior.
    ///
    /// The original immutable-parent/candidate/payment and publication rules are
    /// NOT proved by this SDK method. The caller must pay and bound input before
    /// `load_psks`: this later guard cannot retroactively pay that earlier work.
    /// It requires actual Operational state, one Add, zero queued proposals/PSKs,
    /// no external commit, unchanged signer/default leaf parameters, suite3,
    /// <=64 actual leaf slots with room/reuse, and valid unmerged subtree targets.
    pub fn build_private_text_add_with_time<S: Signer>(
        self,
        rand: &impl OpenMlsRand,
        crypto: &impl OpenMlsCrypto,
        signer: &S,
        now: SystemTime,
    ) -> Result<CommitBuilder<'a, Complete, G>, CreateCommitError> {
        // SDK diagnostic features can format private content or capture opaque
        // stack graphs on error paths. This first paid private profile does not
        // admit those features; a nonalloc unit refusal precedes all SDK/crypto work here.
        // Default/legacy builders and their diagnostics remain unchanged.
        if !MlsGroup::private_text_add_profile_supported_data()
            || self.stage.own_proposals.len() != 1
            || !matches!(self.stage.own_proposals.first(), Some(Proposal::Add(_)))
            || !self.stage.force_self_update
            || !self.stage.consume_proposal_store
            || self.stage.external_commit_info.is_some()
            || !self.stage.leaf_node_parameters.is_empty()
            || !self.stage.psks.is_empty()
        {
            return Err(unsupported());
        }
        require_tree(self.group.borrow())?;
        self.build_internal(
            rand,
            crypto,
            signer,
            None::<NewSignerBundle<'_, S>>,
            AddedKeyPackageValidationTime::Explicit(now),
            TextAddExecution::Serial,
            |_| true,
        )
    }

    /// Exact new inline strategy/time/shape scratch DATA. Dynamic SDK graph,
    /// collection growth, crypto and serialization requests remain additional.
    pub const fn private_text_add_execution_layout_bytes() -> usize {
        std::mem::size_of::<TextAddExecution>()
            + std::mem::size_of::<AddedKeyPackageValidationTime>()
            + std::mem::size_of::<[bool; 64]>()
    }
}

impl MlsGroup {
    /// Compile-time support DATA for the independently paid private profile.
    /// This never changes settings or enables a mode/operation. A false result
    /// lets the host refuse BEFORE input crypto, copying or typed SDK loading;
    /// true grants no resource, Source, Current, governance or publication permit.
    /// The actual private builder repeats this hard feature guard before work.
    pub const fn private_text_add_profile_supported_data() -> bool {
        !cfg!(any(
            feature = "backtrace",
            feature = "crypto-debug",
            feature = "content-debug"
        ))
    }
}
