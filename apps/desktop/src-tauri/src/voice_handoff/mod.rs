//! Native/browser handoff. Application bootstrap exposes fixed public metadata and the
//! authenticated typed action/history loopback service. Native maintenance produces Derive
//! and confirmation work from original history or proven native actions. UI V2 activation and SDP/ICE/RTC
//! integration remain unavailable; a working crypto handoff does not establish live calls.
mod admission;
pub(crate) mod application;
pub(crate) mod bootstrap;
mod channel;
pub(crate) mod commands;
mod confirmation_admission;
mod confirmation_begin;
mod confirmation_history;
mod confirmation_input;
mod confirmation_result;
mod confirmation_settlement;
mod context;
pub(crate) mod custody;
mod derivation_result;
pub(crate) mod factory;
mod finalization;
mod history;
mod http;
mod input;
#[cfg(test)]
pub(crate) mod integration;
pub(crate) mod ledger;
mod pending_close;
mod plan_final;
mod replies;
mod requests;
mod service;
mod settlement;
