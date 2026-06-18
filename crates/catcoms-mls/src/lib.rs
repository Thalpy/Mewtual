//! MLS group core for CatComs (Phase 2): **one MLS group == one server/connection**.
//!
//! Built on openmls (RFC 9420). A [`device::MlsDevice`] is one device's leaf
//! identity (its MLS signature key, whose public bytes content-address its
//! [`catcoms_crypto::DeviceId`]). A [`group::ServerGroup`] wraps an `MlsGroup`:
//! create it, add/remove devices, exchange encrypted application messages, and
//! derive **independent per-document keys** from the group's epoch exporter
//! secret via the canonical injective context from `catcoms-wire`
//! ([`group::ServerGroup::channel_secret`]).
//!
//! Hardening that is fixed here: the single pinned [`CIPHERSUITE`], a
//! `PrivateMessage`-only wire format (relays never see handshake metadata in the
//! clear), and capabilities locked to exactly that one ciphersuite (a downgrade
//! floor). Group state is held by each device's own openmls provider; the
//! persistent, sealed storage provider is wired up later with the storage layer.

pub mod channel;
pub mod config;
pub mod device;
pub mod group;
pub mod invite;

use thiserror::Error;

pub use config::CIPHERSUITE;
pub use device::{serialize_key_package, MlsDevice};
pub use group::{Incoming, ServerGroup};
pub use invite::{InviteError, InviteLedger, InviteToken, MembershipCredential};

/// Errors from MLS operations.
#[derive(Debug, Error)]
pub enum MlsError {
    /// An error surfaced by the underlying openmls / codec machinery.
    #[error("mls protocol error: {0}")]
    Protocol(String),
    /// A message was not of the expected type (e.g. expected a Welcome).
    #[error("message was not of the expected type")]
    WrongMessageType,
    /// No member with the requested device id is in the group.
    #[error("no such member in the group")]
    MemberNotFound,
    /// An invite admission was rejected.
    #[error(transparent)]
    Invite(#[from] InviteError),
    /// An internal invariant was violated.
    #[error("internal invariant violated: {0}")]
    Internal(&'static str),
}

/// Wrap any `Display` error (openmls, tls_codec, …) as [`MlsError::Protocol`].
pub(crate) fn proto(e: impl core::fmt::Display) -> MlsError {
    MlsError::Protocol(e.to_string())
}
