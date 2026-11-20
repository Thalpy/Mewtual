//! Only the actual merged one-shot private Add Cell can produce this witness.
//! Restoring a candidate or decoding matching labels supplies no constructor.
use super::*;
use crate::group::text_current_add::observation::MergedPrivateTextAddSource;

impl LocalMembershipChange {
    pub(in crate::group) fn from_private_text_add(
        source: MergedPrivateTextAddSource<'_>,
    ) -> Result<Self, MlsError> {
        let group = source.group();
        let device = source.device();
        if source.before_epoch().checked_add(1) != Some(group.epoch())
            || group.group_id_ref() != source.original_group()
            || device.public_key_ref() != source.author()
            || !group.is_active()
        {
            return Err(MlsError::Internal(
                "private Add observation original mismatch",
            ));
        }
        let value = Self {
            kind: LocalMembershipChangeKind::Add,
            group: source.original_group().to_vec(),
            author: source.author(),
            subject: source.subject(),
            before_epoch: source.before_epoch(),
            after_epoch: group.epoch(),
            state: group.epoch_authenticator_id(),
            commit: source.commit()?.to_vec(),
            key_package: Some(source.reference()),
            credential: Some(source.credential()),
        };
        if !value.matches_current(group, device) {
            return Err(MlsError::Internal(
                "private Add observation did not match merged group",
            ));
        }
        Ok(value)
    }
}
