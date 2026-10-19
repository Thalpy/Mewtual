//! Native connected-file service ownership. Source30 and helper29 require an explicit current
//! native attachment, authenticated P2P policy and a bound immediate member. Requester/Keep
//! integration is still separate; signed codecs and capacity alone grant no serving authority.
pub mod admission;
pub mod helper;
pub mod read;
mod routes;
pub(crate) mod service;
pub use service::NativeFileService;
pub(crate) mod scope;
pub mod wire;
