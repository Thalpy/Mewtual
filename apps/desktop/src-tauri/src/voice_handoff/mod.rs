//! Disabled native encrypted-action handoff pilot. No listener, command or call dispatch is
//! enabled by this module. Admission must compose native owner/executor/factory registration,
//! paid control/data workers and actual browser/result drainage before the live driver uses it.
pub(crate) mod custody;
mod http;
pub(crate) mod ledger;
