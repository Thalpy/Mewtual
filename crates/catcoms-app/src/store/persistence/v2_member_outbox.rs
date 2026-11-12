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
//! * [`production`]: the Store-scoped mount (registry, production parent, custody cells).
//!
//! Authority boundary. Nothing here decides membership, Admission, signing or publication
//! rights. A [`MemberSignedOriginal`] is custody DATA; a save additionally needs a
//! [`MemberOutboxSaveAdmission`], issued only by the operational host's member-save final edge
//! (`operational::host::member_save`) after the member lane's own save-source end, the live
//! mount and the native session authority. The parent comes only from the namespace5 mount
//! ([`MemberOutboxParentDirectory::from_mount`]).
//!
//! The dead-code allowance below remains for the publication half only: debt batches,
//! submission marks and checkpoints are consumed by transport, which is not wired yet. Remove
//! it when transport consumes them.
#![allow(dead_code)]

mod data;
mod engine;
mod frames;
mod fs;
mod production;
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
pub(in crate::store) use production::{
    member_outbox_key, MemberContextCell, MemberContextState, MemberOriginalCustody,
    MemberOutboxMount, MEMBER_OUTBOX_MOUNT_FEE,
};
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::store) use transaction::{
    member_outbox_call_at_for_test, member_outbox_crash_at_for_test, member_outbox_disarm_for_test,
    MemberOutboxTestCut,
};
#[allow(unused_imports)]
pub(in crate::store) use transaction::{
    AdmittedMemberAppend, AdmittedMemberCommit, MemberAppendAdmissionFacts, MemberAppendOutcome,
    MemberAppendRun, MemberCheckpointRun, MemberCheckpointSaved, MemberCommitRun,
    MemberOutboxSaveAdmission, PreparedMemberAppend, PreparedMemberCheckpoint,
    RefusedMemberAppendAdmission, ReturnedMemberDebtBatch, ReturnedMemberOriginal,
};
