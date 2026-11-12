//! Cached actual-operation state DATA only. No SDK/hash/codec/current check,
//! raw state adopter or operation/save/publication permission is provided.
use super::super::LocalMembershipChange;

impl LocalMembershipChange {
    pub(in crate::group) fn private_text_add_after_authenticator_data(&self) -> &[u8; 32] {
        &self.state
    }
}
