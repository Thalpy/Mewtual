//! Encrypted CRDT replication engine (Phase 4); the data plane.
//!
//! Every channel, wiki page, status feed and calendar is an [`doc::EncryptedDoc`]:
//! an automerge document plus an append-only log of inner-signed operations. The
//! design-review fixes that live here:
//!
//! - **Inner per-op signatures** ([`op::SignedOp`]); authorship is verified
//!   independently of transport encryption, so re-sealed history cannot be forged.
//! - **Snapshot/log catch-up re-sealed under the current epoch**; a latecomer
//!   converges without ever holding old epoch keys, preserving forward secrecy.
//! - **Per-document keys from the MLS epoch exporter**; confidentiality and
//!   access control come from group membership; no separate ACL is trusted.
//!
//! Out-of-order delivery, de-duplication and concurrent merge are handled by
//! automerge's change DAG; this layer adds authentication, encryption and the
//! signed-op log.
//!
//! Deferred to the mesh phase (where partitions can be simulated): the
//! proposal/commit linearization of MLS membership changes and the anti-entropy
//! sync protocol over the network.

pub mod doc;
pub mod op;

use thiserror::Error;

pub use doc::{EncryptedDoc, MAX_DELIVERY_TARGETS};
pub use op::{SealedOp, SignedOp};

/// Errors from the replication engine.
#[derive(Debug, Error)]
pub enum ReplError {
    /// An error from the MLS layer (e.g. deriving the channel key).
    #[error(transparent)]
    Mls(#[from] catcoms_mls::MlsError),
    /// An error sealing/opening (wrong key or tampered ciphertext).
    #[error(transparent)]
    Keystore(#[from] catcoms_crypto::KeystoreError),
    /// An op's inner signature did not verify.
    #[error("op signature is invalid")]
    BadSignature,
    /// An op was for a different document than the one ingesting it.
    #[error("op is for a different document")]
    WrongDocument,
    /// An op was sealed under an epoch whose key is not available (needs catch-up).
    #[error("op sealed under unavailable epoch {0}")]
    EpochUnavailable(u64),
    /// The automerge document rejected a change.
    #[error("automerge error: {0}")]
    Automerge(String),
    /// A local edit produced no change to sign.
    #[error("local edit produced no change")]
    NoChange,
    /// An op or sealed op was malformed.
    #[error("malformed op")]
    Malformed,
}
