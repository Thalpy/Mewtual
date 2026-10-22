//! Native/browser handoff. Application bootstrap exposes fixed public metadata and an
//! authenticated status-only loopback service. Action input/results and live call dispatch
//! remain disabled until original authority, funding and actual drainage are integrated.
pub(crate) mod application;
pub(crate) mod bootstrap;
mod channel;
pub(crate) mod commands;
mod context;
pub(crate) mod custody;
mod derivation_result;
pub(crate) mod factory;
mod http;
pub(crate) mod ledger;
mod pending_close;
mod requests;
mod service;
