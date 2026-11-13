//! Paid pre-cleaner version3 selection. Never pass an old tuple to a version3
//! owner merely because an attributable first claim could be removed.
use super::{admission::Handler, preparation, *};
use crate::store::persistence::native_registry::lifecycle::upgrade::{
    FirstTornIo, UpgradeInventory, UpgradeIo,
};
use crate::store::ServerStore;
use zeroize::Zeroizing;

/// Continuously held across the COMMON Current/JOIN opener, not a cached ready
/// flag. The finite lease preserves OS exclusion if opening later fails and
/// drops its Store before the evidence's last handler clone retires.
pub(super) struct Observation {
    evidence: Evidence,
    _session: catcoms_storage::VaultPhysicalWorkLease,
}
enum Evidence {
    Selected {
        inventory: Box<UpgradeInventory>,
        catalog: preparation::VerifiedColdCatalog,
    },
    Repaired {
        io: Box<UpgradeIo>,
        output: Box<preparation::Output>,
    },
}

impl Handler {
    pub(in crate::store) fn inspect_storage(&self, store: &ServerStore) -> Result<(), AppError> {
        self.require()?;
        let inventory = UpgradeInventory::capture(store, self.debit(), self.pool())?;
        let layout = inventory.authenticate_expected(store)?;
        let expected = super::super::capability::NativeCapabilityData::for_current_data(
            layout,
            &layout.encode_data()?,
        )?;
        if let Some(raw) = inventory.seed() {
            let key = Zeroizing::new(store.keys.db_key()?);
            match preparation::Output::decode_seed(&key, expected, raw) {
                Ok(output) => {
                    let mut io = UpgradeIo::recover(store, inventory, &output.bytes())?;
                    io.advance_upgrade(store, &output.bytes())?;
                    io.require_complete(store, &output.bytes())?;
                    return self.retain_observation(Observation {
                        evidence: Evidence::Repaired {
                            io: Box::new(io),
                            output: Box::new(output),
                        },
                        _session: store._session.retain_for_physical_work(),
                    });
                }
                Err(preparation::SeedReadError::Unauthenticated(_)) if raw.len() >= 40 => {}
                Err(_) if raw.len() < 40 => {}
                // Authenticated malformed/future syntax is not disposable.
                Err(_) => return Err(refused()),
            }
            let proof = preparation::first_torn::verify(store, &inventory)?;
            let mut cleanup = FirstTornIo::prepare(store, inventory, proof)?;
            cleanup.run(store)?;
            // Successful cleanup still selected the original version2. Refuse
            // this strict3 open; a later explicit2 opener can recover its JOIN.
            return Err(refused());
        }
        let catalog = require_selected(store, &inventory)?;
        self.retain_observation(Observation {
            evidence: Evidence::Selected {
                inventory: Box::new(inventory),
                catalog,
            },
            _session: store._session.retain_for_physical_work(),
        })
    }

    fn retain_observation(&self, observation: Observation) -> Result<(), AppError> {
        let mut slot = self.cold.try_lock().map_err(|_| refused())?;
        if slot.is_some() {
            return Err(refused());
        }
        *slot = Some(observation);
        drop(slot);
        #[cfg(test)]
        super::super::operational::upgrade_source::owning::observe_edge(
            super::super::operational::upgrade_source::owning::Edge::AfterInspection,
        );
        Ok(())
    }

    /// Consume the original inspection only after the common opener completes.
    /// Never replace it with a fresh capture that could adopt equal new files.
    pub(in crate::store::critical_namespaces::native_registry) fn finish_cold(
        &self,
        store: &ServerStore,
    ) -> Result<(), AppError> {
        let observation = {
            self.cold
                .try_lock()
                .map_err(|_| refused())?
                .take()
                .ok_or_else(refused)?
        };
        let result = match &observation.evidence {
            Evidence::Selected { inventory, catalog } => {
                inventory.require_after_join(store, catalog)
            }
            Evidence::Repaired { io, output } => io.require_complete(store, &output.bytes()),
        };
        drop(observation); // No cell/catalog/Current guard; files then session.
        result
    }
}

fn require_selected(
    store: &ServerStore,
    inventory: &UpgradeInventory,
) -> Result<preparation::VerifiedColdCatalog, AppError> {
    if inventory.seed().is_some()
        || inventory.has_upgrade_outputs()
        || inventory.index_stage().is_some()
    {
        // Normal version3 index WAL recovery belongs to its own producer.
        // Do not reuse the smaller version2 decoder/cleanup classifications.
        return Err(refused());
    }
    let (_, catalog) = preparation::authenticate_selected(store, inventory)?;
    inventory.require(store)?;
    Ok(catalog)
}
