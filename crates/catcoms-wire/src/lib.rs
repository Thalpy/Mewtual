//! Canonical, injective, length-prefixed wire encoding for CatComs.
//!
//! Every integer is fixed-width big-endian and every variable-length field is
//! length-prefixed. That makes the encoding *prefix-free*, and therefore
//! **injective**: two distinct logical values can never serialise to the same
//! bytes, and any byte string decodes to at most one value.
//!
//! Injectivity is load-bearing for security. Per-channel keys are derived from
//! the MLS exporter secret using a `(doc_type, doc_id)` *context*; if two
//! distinct documents could produce the same context bytes, they would share a
//! key and the "channels are cryptographically separated" guarantee would
//! silently break. [`context::exporter_context`] uses a fixed-width encoding so
//! that cannot happen, and the property is asserted in tests.

pub mod codec;
pub mod context;

pub use codec::{Decoder, Encoder, WireError};
pub use context::{
    context_bytes, exporter_context, DocType, CHANNEL_EXPORTER_LABEL, METADATA_EXPORTER_LABEL,
};
