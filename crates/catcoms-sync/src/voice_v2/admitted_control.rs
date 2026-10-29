//! Separately typed Control7 protocol observations and bounded transport custody.
//! Query/Support are observations only: no endpoint reservation, agreement, Ready,
//! native row, SDK operation or activation permission is created by this module.
pub(crate) use super::admitted_wire::protocol::{Metadata, Object, ProtocolOperation, Verified};
use super::helper::{BoundRoute, RoomAuthority};
use super::resources::{Credit, ResourceKind, Resources};
use super::runtime::{ActorOwner, EffectAuthority, EpochObservation, IngressTicket};
use super::*;
use catcoms_rt::{Clock, CryptoRngCore, PeerId, RequestAuthority, SharedRequestKeepalive};
use std::sync::{Arc, Mutex};

mod ledger;
mod paid;
mod record;
mod table;
pub(crate) mod wire;
pub(crate) use ledger::{ControlLedger, Job, Outcome};
pub(crate) use record::{Binding, Owner, Receipt};
pub(crate) use table::Queries;
#[cfg(test)]
pub(crate) fn inventories_for_test() -> (usize, usize, usize) {
    (
        Owner::inventory_bytes(),
        Queries::inventory_bytes(),
        ControlLedger::inventory_bytes(),
    )
}
pub use table::{
    DirectHelperSupportObservation, DirectHelperSupportProbe, EndpointSupportObservation,
    EndpointSupportQuery, SupportQueryIdentity, SupportQueryMetadata,
};
