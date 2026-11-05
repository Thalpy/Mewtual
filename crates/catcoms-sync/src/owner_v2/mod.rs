//! Actual private V2 source producers, distinct from public replay and mounted Saved authority.
//! No production dispatcher, publisher, root acquisition, archive saver or positive issuer is
//! enabled here. Public codec frames remain DATA until the actual host covers their originals.
mod founder;
pub use founder::{
    FounderDataView, FounderPrivateSnapshotView, FounderSnapshotDigest, FounderSnapshotLimits,
    FrozenFounder, PreparedFounder, FOUNDER_PRIVATE_INVOICE,
};
