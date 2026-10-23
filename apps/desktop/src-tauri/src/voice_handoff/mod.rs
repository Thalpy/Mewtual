//! Native/browser handoff. Application bootstrap exposes fixed public metadata and an
//! authenticated Status/setup and existing-history loopback service. Action input/results and live call dispatch
//! remain disabled until original authority, funding and actual drainage are integrated.
mod admission;
pub(crate) mod application;
pub(crate) mod bootstrap;
mod channel;
pub(crate) mod commands;
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
mod replies;
mod requests;
mod service;
mod settlement;
