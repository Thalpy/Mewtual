//! Native connected-file service ownership. Source30 and helper29 require an explicit current
//! native attachment, authenticated P2P policy and a bound immediate member. Explicit native
//! reads use the bounded requester after direct candidates; codecs alone grant no authority.
pub mod admission;
pub mod helper;
pub mod read;
pub(crate) mod requester;
mod routes;
pub use requester::bridge_reference_variant;
pub use requester::{BridgeCatalog, BridgeChoice, BridgeError, BridgeStep, PreparedBridgePage};
pub(crate) mod service;
pub use service::NativeFileService;
pub(crate) mod scope;
pub mod wire;
