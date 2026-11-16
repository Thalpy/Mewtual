//! Scalar negative oracle for the real TCP helper fixture, with no capability
//! extraction and no production layout/authority change.
use super::Coordinator;
impl Coordinator {
    pub(crate) fn has_no_call_or_key_for_test(&self) -> bool {
        self.controller.is_none()
            && self.owner.is_none()
            && self.activations.is_empty()
            && self.join_generations.is_empty()
    }
}
