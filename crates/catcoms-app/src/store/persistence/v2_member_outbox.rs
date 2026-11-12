//! Chat V2 ordinary-member durable outbox: one member's signed originals survive crashes and
//! can be retried and published while the founder is offline.
//!
//! This is a separately versioned namespace, independent of the founder A/I/S/B/WC
//! certification journal (`v2_text`), its closed inventory, and the legacy Automerge
//! `durable_chat` outbox. Founder certification never gates anything here. Design and
//! recovery rules: `docs/communication-recovery/CHAT-V2-MEMBER-OUTBOX.md`.
//!
//! Layout of one namespace `mo-<context id hex>` under a separately provisioned parent:
//!
//! ```text
//! head.bin               selected local Head (generation, trie root, ordinals, counters)
//! head.stage.v1          selector-CAS stage (exists only between WAL MAIN and the CAS)
//! outbox.wal             WAL MAIN: the committed, authenticated transition being finished
//! outbox.wal.stage.v1    pre-commit WAL stage (promoted if complete, removed if torn)
//! a-<id>.bin             immutable original-action chunk pages (general cold-action codec)
//! e-<id>.bin             immutable entries (one per saved request)
//! n-<id>.bin             immutable copy-on-write trie nodes (receipts, debt, dispositions)
//! ```
//!
//! Module map:
//! * [`data`]: identities, stable context, request digests, signed-original custody.
//! * [`frames`]: exact file formats (control envelopes, head, WAL, entry, chunk wrapper).
//! * [`trie`]: the authenticated copy-on-write index and its seal/verify sinks.
//! * [`fs`]: the only raw filesystem mutation sites (listed in the raw-fs gate).
//! * [`engine`]: the exclusive owner, open/create, cold recovery and reads.
//! * [`transaction`]: presealed transitions, the final save-admission edge, workers.
//!
//! Authority boundary. Nothing here decides membership, Admission, signing or publication
//! rights. A [`MemberSignedOriginal`] is custody DATA; a save additionally needs a
//! [`MemberOutboxSaveAdmission`] from the primary's real member-source guard, which is NOT
//! implemented yet. Until that guard exists, production activation is explicitly incomplete:
//! only cfg(test) durability fixtures can construct the signed input, the parent directory and
//! the save admission. The dead-code allowance below exists for exactly that reason and must be
//! removed when the real admission and provisioning are wired.
#![allow(dead_code)]

mod data;
mod engine;
mod frames;
mod fs;
mod transaction;
mod trie;

#[cfg(test)]
mod tests;

// Consumed by the primary's member-source/native integration, which is not yet wired.
#[allow(unused_imports)]
pub(in crate::store) use data::{
    MemberHeadId, MemberOutboxContextData, MemberOutboxContextId, MemberOutboxContextSpec,
    MemberReceiptData, MemberRequestData, MemberRequestDigest, MemberRequestToken,
    MemberSignedOriginal,
};
#[allow(unused_imports)]
pub(in crate::store) use engine::{
    MemberDebtBatch, MemberDebtClass, MemberDebtItem, MemberEntryStatus, MemberLookup,
    MemberOutbox, MemberOutboxError, MemberOutboxOpen, MemberOutboxParentDirectory,
    MemberOutboxPressure, MemberOutboxQuota, MemberOutboxRefusal, MemberOutboxRegistry,
    MemberSavedOriginal, MEMBER_OUTBOX_APPEND_FEE, MEMBER_OUTBOX_CHECKPOINT_FEE,
    MEMBER_OUTBOX_DEBT_FEE, MEMBER_OUTBOX_DEFAULT_QUOTA, MEMBER_OUTBOX_LOOKUP_FEE,
    MEMBER_OUTBOX_OWNER_FEE, MEMBER_OUTBOX_RECOVERY_FEE,
};
#[allow(unused_imports)]
pub(in crate::store) use frames::DispositionCode;
#[allow(unused_imports)]
pub(in crate::store) use transaction::{
    AdmittedMemberAppend, MemberAppendAdmissionFacts, MemberAppendOutcome, MemberAppendRun,
    MemberCheckpointRun, MemberCheckpointSaved, MemberOutboxSaveAdmission, PreparedMemberAppend,
    PreparedMemberCheckpoint, RefusedMemberAppendAdmission, ReturnedMemberDebtBatch,
    ReturnedMemberOriginal,
};
