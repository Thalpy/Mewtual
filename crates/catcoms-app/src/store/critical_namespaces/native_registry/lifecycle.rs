//! Namespace7/version3 production lifecycle contract, under construction.
//!
//! This module is not yet registered by any opener. Version1/2 remain closed;
//! no decoded descriptor/row here grants SDK, join, source or native authority.
//! See CHAT-V2-NATIVE-INDEX3-DESIGN.md for the reviewed upgrade boundary.
mod capability;
mod codec;
mod data;
mod migration;

use super::super::refused;
use crate::AppError;
