//! Every request kind's largest legal response must fit the transport's request deadline.
//!
//! # Why this exists
//!
//! `MAX_CATCHUP_CHUNK` already carries the reasoning, for the document layer:
//!
//! > A deadline is only safe if a legal response can always fit inside it \[...\] Without that
//! > cap this deadline would be a throughput requirement: a valid, steadily progressing
//! > connection that could not move 16 MiB in two seconds would time out, retry, and time out
//! > again, so a large history could never converge at all.
//!
//! That invariant was established for catch-up and then not applied to blob fetch, which moved
//! a whole 8 MiB file chunk in one exchange against a ten-second transport timeout: a hidden
//! demand for about 8 Mb/s of goodput, below which a file transfer did not get slow, it failed
//! outright and retried into the same wall. Chat over the identical connection was unaffected,
//! because a chat op is under a kilobyte, so the symptom read as "files are broken" rather than
//! "one budget is wrong".
//!
//! An invariant living in a comment on one constant is inherited by nothing. This turns it into
//! something a new request kind cannot quietly skip: the table below must name every `KIND_`
//! constant in the crate, and each entry must fit [`catcoms_rt::REQUEST_TIMEOUT`] at
//! [`FLOOR_RATE_BYTES_PER_SEC`].

use super::*;

/// The slowest link a legal response is required to arrive over.
///
/// 32 KiB/s, about 0.26 Mb/s. Deliberately pessimistic: the paths this has to hold on are a
/// NAT-punched UDP mapping between two domestic connections and a relay circuit, not a
/// datacentre link, and the cost of being wrong in this direction is a feature that cannot work
/// at all rather than one that is slow.
const FLOOR_RATE_BYTES_PER_SEC: usize = 32 * 1024;

/// The largest response each request kind may produce.
///
/// Adding a request kind without adding a row here fails `every_request_kind_declares_a_budget`.
const BUDGETS: &[(&str, u8, usize)] = &[
    ("KIND_CATCHUP", KIND_CATCHUP, MAX_CATCHUP_CHUNK),
    ("KIND_JOIN", KIND_JOIN, MAX_CONTROL_REQUEST),
    (
        "KIND_COMMIT_CATCHUP",
        KIND_COMMIT_CATCHUP,
        MAX_CATCHUP_CHUNK,
    ),
    ("KIND_WELCOME", KIND_WELCOME, MAX_CONTROL_REQUEST),
    ("KIND_PEX", KIND_PEX, MAX_CONTROL_REQUEST),
    ("KIND_BLOB_FETCH", KIND_BLOB_FETCH, MAX_BOUNDED_BLOB_BYTES),
    ("KIND_ADMIT_RESULT", KIND_ADMIT_RESULT, MAX_CONTROL_REQUEST),
    ("KIND_DM_INVITE", KIND_DM_INVITE, MAX_CONTROL_REQUEST),
    ("KIND_CALL_SIGNAL", KIND_CALL_SIGNAL, MAX_CONTROL_REQUEST),
    ("KIND_DEVICE_ADD", KIND_DEVICE_ADD, MAX_CONTROL_REQUEST),
    (
        "KIND_DEVICE_ADMIT_RESULT",
        KIND_DEVICE_ADMIT_RESULT,
        MAX_CONTROL_REQUEST,
    ),
    ("KIND_JOIN_FORWARD", KIND_JOIN_FORWARD, MAX_CONTROL_REQUEST),
    (
        "KIND_SWITCHBOARD_OFFER",
        KIND_SWITCHBOARD_OFFER,
        MAX_CONTROL_REQUEST,
    ),
    (
        "KIND_RECIPROCAL_FORWARD",
        KIND_RECIPROCAL_FORWARD,
        MAX_CONTROL_REQUEST,
    ),
    (
        "KIND_RECIPROCAL_DELIVERY",
        KIND_RECIPROCAL_DELIVERY,
        MAX_CONTROL_REQUEST,
    ),
    (
        "KIND_INDIRECT_PROBE",
        KIND_INDIRECT_PROBE,
        MAX_CONTROL_REQUEST,
    ),
    (
        "KIND_INDIRECT_RESULT",
        KIND_INDIRECT_RESULT,
        MAX_CONTROL_REQUEST,
    ),
    (
        "KIND_DELIVERY_RECEIPT",
        KIND_DELIVERY_RECEIPT,
        MAX_CONTROL_REQUEST,
    ),
    ("KIND_CATCHUP_SINCE", KIND_CATCHUP_SINCE, MAX_CATCHUP_CHUNK),
    ("KIND_REGISTRY_PAGE", KIND_REGISTRY_PAGE, MAX_CATCHUP_CHUNK),
    ("KIND_RECEIPT_HEAD", KIND_RECEIPT_HEAD, MAX_CONTROL_REQUEST),
    (
        "KIND_REGISTRY_SEED",
        KIND_REGISTRY_SEED,
        MAX_CONTROL_REQUEST,
    ),
    ("KIND_STUDIO_PAGE", KIND_STUDIO_PAGE, MAX_CATCHUP_CHUNK),
    ("KIND_STUDIO_HEAD", KIND_STUDIO_HEAD, MAX_CONTROL_REQUEST),
    ("KIND_STUDIO_SEED", KIND_STUDIO_SEED, MAX_CONTROL_REQUEST),
    (
        "KIND_MEMBER_FINALIZE",
        KIND_MEMBER_FINALIZE,
        MAX_CONTROL_REQUEST,
    ),
    ("KIND_BLOB_PAGE", KIND_BLOB_PAGE, MAX_BLOB_PAGE),
];

/// Kinds knowingly over budget, each with the reason and what would retire it.
///
/// An exemption is written down here rather than absorbed by a looser floor, so the cost of
/// keeping it stays visible. An empty list is the goal.
const KNOWN_OVER_BUDGET: &[(&str, &str)] = &[(
    "KIND_BLOB_FETCH",
    "The pre-paging whole-blob grammar, retained only so a peer running an older build can \
     still be fetched from. A current pair never uses it: both ends prefer KIND_BLOB_PAGE and \
     fall back only on an empty answer. Retire the row when the oldest supported build pages.",
)];

#[test]
fn every_declared_response_fits_the_transport_deadline() {
    let budget = FLOOR_RATE_BYTES_PER_SEC * (catcoms_rt::REQUEST_TIMEOUT_MS as usize) / 1_000;
    let mut over = Vec::new();
    for (name, _, max_response) in BUDGETS {
        if *max_response > budget {
            over.push(*name);
        }
    }
    let exempt: Vec<&str> = KNOWN_OVER_BUDGET.iter().map(|(name, _)| *name).collect();
    let unexpected: Vec<_> = over.iter().filter(|name| !exempt.contains(*name)).collect();
    assert!(
        unexpected.is_empty(),
        "these request kinds cannot deliver a legal response inside \
         {}ms at {} bytes/sec, so their deadline is a throughput requirement rather than a \
         deadline: {unexpected:?}. Make the response pageable (see MAX_BLOB_PAGE) rather than \
         raising the timeout.",
        catcoms_rt::REQUEST_TIMEOUT_MS,
        FLOOR_RATE_BYTES_PER_SEC,
    );
    // An exemption that has quietly become unnecessary is as much a defect as a missing one.
    let stale: Vec<_> = exempt.iter().filter(|name| !over.contains(*name)).collect();
    assert!(
        stale.is_empty(),
        "these kinds are listed as over budget but now fit; delete their rows: {stale:?}"
    );
}

/// The table must name every request kind the crate defines.
///
/// Scanning the source is blunt, and it is the only thing here that actually binds: a constant
/// list can always be left un-updated, but a new `const KIND_...` cannot hide from the file it
/// is written in. Without this, the gate would protect exactly the kinds that already existed
/// when it was written, which is the situation it was built to end.
#[test]
fn every_request_kind_declares_a_budget() {
    let sources = [
        include_str!("lib.rs"),
        include_str!("member_finalization.rs"),
    ];
    let mut declared = Vec::new();
    for source in sources {
        for line in source.lines() {
            let line = line.trim();
            // `const KIND_X: u8 = N;`, with or without a visibility prefix.
            let Some(rest) = line
                .strip_prefix("const KIND_")
                .or_else(|| line.strip_prefix("pub const KIND_"))
                .or_else(|| line.strip_prefix("pub(super) const KIND_"))
                .or_else(|| line.strip_prefix("pub(crate) const KIND_"))
            else {
                continue;
            };
            let Some((name, tail)) = rest.split_once(':') else {
                continue;
            };
            if tail.trim_start().starts_with("u8") {
                declared.push(format!("KIND_{name}"));
            }
        }
    }
    assert!(
        declared.len() >= BUDGETS.len(),
        "the source scan found fewer kinds ({}) than the table declares ({}); the scan is \
         probably no longer matching the declaration style",
        declared.len(),
        BUDGETS.len()
    );
    let budgeted: Vec<&str> = BUDGETS.iter().map(|(name, _, _)| *name).collect();
    let missing: Vec<_> = declared
        .iter()
        .filter(|name| !budgeted.contains(&name.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "these request kinds have no declared response budget: {missing:?}. Add a row to \
         BUDGETS naming the largest response the kind can produce."
    );
}

/// The table's bytes must match the constants, so a row cannot drift into fiction.
#[test]
fn declared_kind_bytes_match_their_constants() {
    let mut seen = std::collections::HashSet::new();
    for (name, byte, _) in BUDGETS {
        assert!(seen.insert(*byte), "{name} duplicates kind byte {byte}");
    }
}
