//! Explicit counterpart of the existing replacement statement. The legacy
//! helper keeps its original SystemTime::now call and is not routed through an
//! earlier clock read in a public wrapper or error path.
use super::*;

impl MessageSecretsStore {
    pub(in crate::group::mls_group) fn replace_current_message_secrets_with_time(
        &mut self,
        message_secrets: MessageSecrets,
        now: SystemTime,
    ) -> MessageSecrets {
        let mut message_secrets = message_secrets.with_timestamp(now);
        std::mem::swap(&mut self.message_secrets, &mut message_secrets);
        message_secrets
    }
}
