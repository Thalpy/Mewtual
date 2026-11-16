//! Readonly existing-generation proof in canonical's OWN kernel family. No Weak upgrade,
//! mutable slot/map, last-Life drop, registration, renewal or owner replacement exists here.
use super::*;
use crate::store::owner_evidence::canonical::current_vacancy::settlement::{
    GenesisSeedSettlementOwnerNamespace, SettledGenesisCurrentOwnerNamespace,
};

macro_rules! require_existing_settlement_writer {
    ($ty:ident) => {
        impl $ty<'_, '_> {
            pub(in crate::store) fn require_settlement_writer_observation(
                &self,
                original: &V2WriterObservation,
            ) -> Result<(), AppError> {
                self.validate_source_only()?;
                let (runtime, key) = self.settlement_registration_parts();
                runtime.require_installed_observation(key, original)?;
                self.validate_source_only()
            }
        }
    };
}
require_existing_settlement_writer!(GenesisSeedSettlementOwnerNamespace);
require_existing_settlement_writer!(SettledGenesisCurrentOwnerNamespace);
