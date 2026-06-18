//! Core runtime **seams** that every CatComs layer is written against.
//!
//! Two dependency-injection points are mandatory so the whole stack is
//! deterministically testable on one machine:
//!
//! - [`Clock`] — all time flows through this. No layer may read the OS clock
//!   directly; only [`SystemClock`] does, and CI enforces it
//!   (`scripts/check-no-ambient.sh`). Tests use [`ManualClock`].
//! - [`MeshTransport`] — pub/sub fan-out plus addressed request/response. The
//!   same node logic runs over the in-memory [`MemNetwork`] in tests and over
//!   rust-libp2p in production, unchanged.
//!
//! Keeping these abstractions small and correct now avoids a costly refactor of
//! every consumer later.

pub mod clock;
pub mod mem;
pub mod rng;
pub mod transport;

pub use clock::{Clock, ManualClock, SystemClock};
pub use mem::{Hub, MemNetwork};
pub use rng::{CryptoRng, CryptoRngCore, OsCryptoRng, RngCore};
pub use transport::{
    MeshTransport, PeerId, ProtocolId, Responder, ResponderRx, Topic, TransportError,
    TransportEvent,
};
