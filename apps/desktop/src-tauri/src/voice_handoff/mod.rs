//! Disabled native encrypted-action handoff pilot. No listener, command or call dispatch is
//! enabled by this module. Admission must compose native owner/executor/factory registration,
//! paid control/data workers and actual browser/result drainage before the live driver uses it.
mod channel;
pub(crate) mod custody;
pub(crate) mod factory;
mod http;
pub(crate) mod ledger;
