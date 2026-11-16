//! Conservative named request inventory for ONE ordinary SERIAL Add. These are
//! logical requested buffers/work, not RSS, allocator metadata or thread stacks.
//! Existing runtime/log services own their separately bounded retained state.
//! No Source/governance/SDK permission is issued by arithmetic or positive caps.
use super::*;
use crate::persist::text_current_add::TextCurrentAddLimits;
use openmls::group::{text_add_element_bounds::*, text_add_layout::text_add_layout_data};

const N: usize = 64;
const D: usize = 6;
const U: usize = 64;
const C: usize = 512;
const TREE_NODES: usize = 2 * N - 1;
// Input preflight proves sortedunique/nonblank SAME-parent-subtree U targets.
// Add touches only the new leaf's ancestors; generated parents clear U. For a
// subtree with L leaf slots, resolution<=2L-1 (leaf/fullparent/blank induction).
// Own copath subtrees partition leaves outside own, so R<2N, NEVER N-1. This
// bound is valid ONLY for actual private one-Add entry's enforced invariants.
const R: usize = 2 * N;
const E: usize = TEXT_ADD_ELEMENT_BYTES;
const REF: usize = TEXT_ADD_REFERENCE_BYTES;
const ALIGN: usize = TEXT_ADD_ELEMENT_ALIGN;
const GROUP_CONTEXT_TLS: usize = 96;
const LEAF_TLS: usize = 33 + 33 + 2 + 2 + C + 11 + 34 + 1 + 66;
const PARENT_TLS: usize = 33 + 33 + 2 + U * 4;
const TREE_TLS: usize = N * LEAF_TLS + (N - 1) * PARENT_TLS + TREE_NODES * 2 + 4;
const GROUP_INFO_TLS: usize = GROUP_CONTEXT_TLS + TREE_TLS + 256;
const PACKAGE_TLS: usize = LEAF_TLS + 4 + 33 + 1 + 66 + 4;
const COMMIT_TLS: usize = PACKAGE_TLS + LEAF_TLS + D * 40 + R * 82 + 512;
// Fixed-family outputs serialize provider rows up to the independently enforced
// cap. Preserve unknown paid source rows; before SDK require headroom for ALL
// known final group rows +current epoch keys rather than assuming tiny inputs.
const PROVIDER_ROW: usize = 512 * 1024;
const FINAL_PROVIDER_EXTRA: usize = 14 * (PROVIDER_ROW + 512 + 8);
const PROVIDER_WRITES: usize = 16;

/// Public DATA to the sibling fee owner, not a resource reservation or operation.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SdkInvoice {
    pub(crate) typed: usize,
    pub(crate) work: usize,
    pub(crate) output: usize,
    pub(crate) snapshot_extra: usize,
    pub(crate) provider_serialized: usize,
    pub(crate) provider_bytes: usize,
    pub(crate) provider_work: usize,
}
fn add(a: usize, b: usize) -> Result<usize, MlsError> {
    a.checked_add(b).ok_or_else(bad)
}
fn mul(a: usize, b: usize) -> Result<usize, MlsError> {
    a.checked_mul(b).ok_or_else(bad)
}

/// Actual pinned RawVec push/collect growth, cumulative requested backing. Every
/// live/grown request is counted even if earlier backing or the Vec is dropped.
fn vector(count: usize, element: usize) -> Result<usize, MlsError> {
    if count == 0 {
        return Ok(0);
    }
    let mut cap = if element == 1 {
        8
    } else if element <= 1024 {
        4
    } else {
        1
    };
    let mut total = 0;
    loop {
        total = add(total, mul(cap, element)?)?;
        if cap >= count {
            return Ok(total);
        }
        cap = mul(cap, 2)?;
    }
}
/// Pinned std/hashbrown open-addressed table: <=next_pow2(2n), min4 buckets,
/// one control byte/bucket +64 tail/alignment. Twice the largest request covers
/// the sum of all geometric growth requests; payload copies are separate.
fn table(count: usize, element: usize) -> Result<usize, MlsError> {
    if count == 0 {
        return Ok(0);
    }
    let buckets = mul(count, 2)?
        .max(4)
        .checked_next_power_of_two()
        .ok_or_else(bad)?;
    mul(2, add(add(mul(buckets, element)?, buckets)?, 2 * ALIGN)?)
}
/// Pinned Rust BTree node has11 keys/values and12 child links. The compiled
/// pair cap includes both native key/value widths; six alignment/header slots
/// overpay parent link,index,len and array padding. <=2n+1 node requests safely
/// covers split/root allocations over n inserts into each initially empty diff.
fn btree(count: usize) -> Result<usize, MlsError> {
    let node = add(add(11 * E, 12 * std::mem::size_of::<usize>())?, 6 * ALIGN)?;
    mul(add(mul(2, count)?, 1)?, node)
}
fn hpke(info: usize, plaintext: usize) -> Result<usize, MlsError> {
    // Pinned hpke-rs/rust-crypto0.7 successful base seal's34 named requests:
    // recipient32; KEMsuite5/nine32/labels51,16,51,91/context64;
    // scheduleSuite10/five32/nonce12/labels28,26+I,23,87,94,87/context65;
    // sequence nonce4->12/XOR12/cipherP+16. Sum1234+I+P.
    // Pay three optional deterministic-PRNG256 buffers too, even though ordinary
    // native RustCrypto uses inline state/OS seeds and requests none of them.
    add(add(1234 + 3 * 256, info)?, plaintext)
}
fn encrypt_context(context: usize, label: usize) -> Result<(usize, usize), MlsError> {
    let full_label = add(8, label)?;
    let info = add(add(full_label, context)?, 8)?; // conservative two4-byte VL headers.
    let requests = add(
        add(vector(full_label, 1)?, full_label)?,
        add(context, info)?,
    )?;
    Ok((requests, info))
}

pub(crate) fn required() -> Result<SdkInvoice, MlsError> {
    let l = text_add_layout_data();
    let wrappers = [
        l.create_commit_result_bytes,
        l.pending_commit_state_bytes,
        l.mls_group_state_bytes,
        l.commit_initial_bytes,
        l.commit_loaded_psks_bytes,
        l.commit_complete_bytes,
        l.commit_builder_initial_bytes,
        l.commit_builder_loaded_psks_bytes,
        l.commit_builder_complete_bytes,
        l.commit_message_bundle_bytes,
        l.staged_commit_bytes,
        l.staged_commit_state_bytes,
        l.member_staged_commit_state_bytes,
        l.tree_sync_bytes,
        l.tree_sync_diff_bytes,
        l.staged_tree_sync_diff_bytes,
        l.public_group_diff_bytes,
        l.staged_public_group_diff_bytes,
        l.message_secrets_store_bytes,
        l.message_secrets_bytes,
        l.secret_tree_bytes,
        l.secret_tree_node_bytes,
        l.proposal_queue_bytes,
        l.queued_proposal_bytes,
        l.group_epoch_secrets_bytes,
        l.resumption_psk_store_bytes,
        l.encryption_key_pair_bytes,
        l.parent_node_bytes,
        l.plain_update_path_node_bytes,
        l.update_path_bytes,
        l.update_path_node_bytes,
        l.mls_message_out_bytes,
        l.group_info_bytes,
        l.welcome_bytes,
        l.key_package_in_bytes,
        l.leaf_node_bytes,
        l.leaf_node_in_bytes,
        l.secret_bytes,
        l.hpke_ciphertext_bytes,
        l.key_package_tbs_bytes,
        l.key_package_tbs_in_bytes,
        l.leaf_node_payload_bytes,
    ];
    let mut typed = 0;
    // Sixteen fixed wrapper/move copies overpay all builder stages/results,
    // mutually exclusive errors, recursion/closure headers and common callframes.
    // Transitive allocations are charged independently below; nesting may double
    // charge wrappers but never discounts a live fee-bearing graph.
    for bytes in wrappers {
        typed = add(typed, mul(16, bytes)?)?;
    }
    typed = add(typed, 16 * (openmls::group::CommitBuilder::<'static, openmls::group::LoadedPsks>::private_text_add_execution_layout_bytes()
        + MlsGroup::text_add_merge_time_layout_bytes()))?;
    // One Add's proposal pool/final queue/commit/invitation/diff leaf: eight
    // complete package/leaf clones cover the six named copies and move/error
    // overlap. Payloads use actual fixed widths +C, including parent-hash32.
    let leaf_heap = 2 * 32 + C + 64 + 32 + 3 * 8;
    typed = add(typed, mul(8, add(leaf_heap + 96, E)?)?)?;
    typed = add(typed, add(mul(8, table(1, E)?)?, mul(8, vector(1, E)?)?)?)?;
    // Full roster materializes Member (credential+TWO keys) in uniqueness
    // validation, unconditional merge leaves.collect even max_epochs==0, AND
    // observation.matches_current's new-subject lookup. The last is an iterator
    // with no native outer Vec, but paying it too is conservative.
    typed = add(typed, mul(3, add(vector(N, E)?, mul(N, C + 64)?)?)?)?;
    typed = add(typed, 16 + 32)?; // matches_current group_id/public_key comparison copies.
    typed = add(typed, add(mul(2, table(N + 1, REF)?)?, table(1, REF)?)?)?;
    // At most2 changed leaves and union of two direct paths<=2D changed parents.
    // Parent clone len63 may insert to64 and retain capacity126; charge clone,
    // new backing and old/new overlap rather than cap64 live length.
    typed = add(typed, add(btree(2)?, btree(2 * D)?)?)?;
    typed = add(typed, mul(2 * D, 64 + vector(2 * U, 4)?)?)?;
    // Two complete ratchet-tree exports coexist (Welcome extensions.clone and
    // exported GroupInfo), each with up to127 boxed native nodes and payloads.
    let parent_heap = 64 + U * 4;
    typed = add(
        typed,
        mul(
            2,
            add(
                vector(TREE_NODES, E)?,
                add(
                    mul(TREE_NODES, E)?,
                    add(mul(N, leaf_heap)?, mul(N - 1, parent_heap)?)?,
                )?,
            )?,
        )?,
    )?;
    // Repeated standard resolution, filtered copies and debug parent verification.
    // A recursive result can be appended/grown at every one of D+1 levels; each
    // Vec's cumulative requests<=4*its final nonzero count*compiledREFcap.
    let ordinary_entries = 5 * R * (D + 1) + 5 * R;
    let debug_entries = 2 * N * D * (D + 1);
    typed = add(typed, mul(4 * REF, ordinary_entries + debug_entries)?)?;
    typed = add(typed, mul(5 + 2 * (N - 1), vector(D, REF)?)?)?;
    // Derived path collect/unzip/unzip, keypair and owned-key vectors, cloned
    // recipient keys, path/cipher headers, before any crypto payload requests.
    typed = add(typed, mul(8, vector(D + 1, E)?)?)?;
    typed = add(typed, add(mul(2, vector(R, REF)?)?, mul(R, 32)?)?)?;
    typed = add(typed, mul(4, vector(D, E)?)?)?;
    // New epoch four full N-slot inline Vecs, old own BOTH initialized ratchets,
    // D path secrets/children, nine newEpoch32 secrets plus scheduling overlap.
    typed = add(typed, mul(4, vector(N, E)?)?)?;
    typed = add(typed, mul(2, vector(N, E)?)?)?; // merge leaf/parent resize growth overlap.
    typed = add(typed, mul(4 * D + 64, 32)?)?;
    typed = add(typed, add(vector(32, E)?, 32 * 32)?)?; // resumption ring growth/copies.
                                                        // Tree hash visits: two full trees +D copaths +debug all full parents.
                                                        // Debug computes two child subtree hashes and two parent hashes per parent;
                                                        // independently overpay a full tree at each such recursive invocation.
    let hash_visits = (D + 2 + 2 * (N - 1)) * TREE_NODES + 2 * (N - 1);
    let hash_request = LEAF_TLS.max(PARENT_TLS + 80) + 32 + parent_heap + vector(U, 4)?;
    typed = add(typed, mul(hash_visits, hash_request)?)?;
    // Path HPKE and one Welcome HPKE, each with its own MLS EncryptContext copies.
    let (context, info) = encrypt_context(GROUP_CONTEXT_TLS, 14)?;
    typed = add(typed, mul(R, add(context, hpke(info, 32)?)?)?)?;
    let (context, info) = encrypt_context(GROUP_INFO_TLS + 16, 7)?;
    typed = add(typed, add(context, hpke(info, 128)?)?)?;
    typed = add(typed, (D + 2) * (264 + 256))?; // deriveHPKE32 +optional PRNG256.
                                                // <=128 labelled KDF steps cover path/node, key schedule, both sender
                                                // ratchets, message key/nonce, sender-data, Welcome key/nonce and exporter.
                                                // Every step pays label growth+label/context copies+TLS info+output32;
                                                // extract's unconditional zero32 and SHA/HMAC/extract32 are separate.
    typed = add(
        typed,
        mul(128, vector(72, 1)? + 72 + GROUP_CONTEXT_TLS + 180 + 32)?,
    )?;
    typed = add(typed, 128 * (32 + 32))?;
    // Four Ed25519 signatures: new leaf, Commit, Welcome GI, exported GI.
    // Signable allocates unsignedB +cloneB +labeledB, retaining GI unsignedB.
    typed = add(typed, mul(3, 2 * GROUP_INFO_TLS + COMMIT_TLS + LEAF_TLS)?)?;
    typed = add(typed, 4 * (vector(72, 1)? + 72 + 16 + 64))?;
    // Direct backend AEAD allocatesP+16 AND copies ciphertext. Plain commit's
    // encode allocates its buffer then to_vec. Welcome GI/sender-data copies too.
    typed = add(
        typed,
        4 * (COMMIT_TLS + 16) + 2 * (GROUP_INFO_TLS + 16) + 8 * 64,
    )?;
    // These plaintexts are separate from their signature buffers and AEAD/HPKE
    // outputs: CommitBuilder encodes GroupInfo before encrypting it, and
    // GroupSecrets::new_encoded builds the Welcome HPKE input before sealing.
    typed = add(typed, GROUP_INFO_TLS + 128)?;
    // Reachable failure chains are fixed strings/codes at pinned suite widths.
    // 32 growth envelopes of1024 overpay all backend+SDK conversion strings;
    // backtraces/secret-content diagnostic features are refused before preparation.
    typed = add(typed, 32 * 1024)?;
    // Actual SDK logging defaults filter dependency debug/trace before collection;
    // explicit verbose console may format bounded contexts. Pay per-emission
    // scratch; existing process log service owns its bounded retained ring/queue.
    typed = add(typed, mul(512, 4 * (GROUP_CONTEXT_TLS + 256))?)?;
    // private_message.rs logs the complete encrypted Commit with ordinary
    // trace!, even without diagnostic features. Explicit RUST_LOG console
    // opt-in may render it: <=4 hex/separator chars per byte +fixed framing,
    // then four times that length pays all String growth requests/overlap.
    // Ring/file default filters reject this event before their collectors.
    typed = add(typed, 4 * (4 * (COMMIT_TLS + 16) + 256))?;
    let output = PACKAGE_TLS + 2 * COMMIT_TLS + 2 * GROUP_INFO_TLS + 256;
    // Monotonic work, not a wall-clock deadline: all bounded hash/serde traversals,
    // fixed curve work per128HPKE+8derivations+4signatures and SDK comparisons.
    let work = add(
        mul(hash_visits, (LEAF_TLS + PARENT_TLS + U) * 64)?,
        (R + D + 6) * (1 << 22),
    )?;
    let provider_serialized = PROVIDER_WRITES * PROVIDER_ROW;
    Ok(SdkInvoice {
        typed,
        work,
        output,
        snapshot_extra: FINAL_PROVIDER_EXTRA,
        provider_serialized,
        provider_bytes: provider_serialized + 64 * 512,
        provider_work: PROVIDER_WRITES * (PROVIDER_ROW * 128 + 512 * 128) + (1 << 28),
    })
}

pub(crate) fn require(
    limits: TextCurrentAddLimits,
    source_bytes: usize,
) -> Result<SdkInvoice, MlsError> {
    let invoice = required()?;
    if limits.sdk_typed_bytes < invoice.typed
        || limits.sdk_work < invoice.work
        || limits.output_bytes < invoice.output
        || limits.max_snapshot_bytes < add(source_bytes, invoice.snapshot_extra)?
        || limits.operation.max_serialized_bytes < invoice.provider_serialized
        || limits.operation.max_provider_bytes
            < add(invoice.provider_bytes, limits.max_provider_backing_bytes)?
        || limits.operation.max_work < invoice.provider_work
    {
        return Err(bad());
    }
    Ok(invoice)
}
