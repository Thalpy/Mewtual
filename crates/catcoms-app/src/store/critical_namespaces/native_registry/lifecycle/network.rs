//! Persisted native-kind checks. Old DATA founder1 remains separate from
//! native founder4; no cached/decoded binding grants SDK or source authority.
use super::{
    data::{Backend, BirthKind, Index, Row},
    founder, *,
};
use crate::store::persistence::native_registry::network::{NetworkDependency, NetworkExpected};
use crate::store::ServerStore;
use std::sync::Arc;

pub(in crate::store::critical_namespaces::native_registry) struct ColdFounderSelection {
    pub(in crate::store::critical_namespaces::native_registry) status:
        crate::store::NativeCreationStatusData,
    pub(in crate::store::critical_namespaces::native_registry) name: String,
    pub(in crate::store::critical_namespaces::native_registry) invocation: [u8; 32],
    pub(in crate::store::critical_namespaces::native_registry) request: [u8; 32],
    pub(in crate::store::critical_namespaces::native_registry) binding:
        super::super::data::FounderBindingData,
    pub(in crate::store::critical_namespaces::native_registry) network: Arc<NetworkDependency>,
}

pub(in crate::store::critical_namespaces::native_registry) fn selected_cold_founder(
    store: &ServerStore,
    inventory: &crate::store::persistence::native_registry::lifecycle::upgrade::UpgradeInventory,
    networks: &[Arc<NetworkDependency>],
) -> Result<Option<ColdFounderSelection>, AppError> {
    let (index, _) = super::preparation::decode_selected_data(store, inventory)?;
    let Some(row) = checked_founder_row(&index)? else {
        return Ok(None);
    };
    let [network] = networks else {
        return Err(refused());
    };
    validate_founder_network(store, &index, row, network).map(Some)
}

fn checked_founder_row(index: &Index) -> Result<Option<&Row>, AppError> {
    let [row] = index.rows.as_slice() else {
        return Err(refused());
    };
    let Backend::Founder { birth, binding } = &row.backend else {
        return Ok(None);
    };
    if birth.kind != BirthKind::NativeFounder {
        return Ok(None);
    }
    if row.is_dm
        || binding.vault != index.capability.original.vault
        || binding.layout != index.capability.original.root
        || binding.signer != birth.signer
    {
        return Err(refused());
    }
    Ok(Some(row))
}

fn validate_founder_network(
    store: &ServerStore,
    index: &Index,
    row: &Row,
    network: &Arc<NetworkDependency>,
) -> Result<ColdFounderSelection, AppError> {
    let Backend::Founder { birth, binding } = &row.backend else {
        return Err(refused());
    };
    let expected = expected(index, row)?;
    if !network.matches(expected, None) {
        return Err(refused());
    }
    network.require(store)?;
    Ok(ColdFounderSelection {
        status: founder::status(row)?,
        name: row.name.clone(),
        invocation: birth.invocation,
        request: birth.request,
        binding: binding.clone(),
        network: network.clone(),
    })
}

/// Read the actual founder network file with a separately paid owner.
/// Unlike generic native startup this has no reason to inspect Catalog Stage.
pub(super) fn capture_declaring_founder(
    store: &ServerStore,
    index: &Index,
    pool: &crate::store::CriticalCatalogReadPool,
) -> Result<ColdFounderSelection, AppError> {
    let row = checked_founder_row(index)?.ok_or_else(refused)?;
    let expected = expected(index, row)?;
    let fee = Arc::new(pool.try_reserve_classified(
        crate::store::persistence::native_registry::network::COLD_FEE,
        || AppError::ChatV2RegistryBusy,
    )?);
    let network = NetworkDependency::capture_cold(store, expected, None, fee)?;
    validate_founder_network(store, index, row, &network)
}

impl ColdFounderSelection {
    pub(in crate::store::critical_namespaces::native_registry) fn require_saved_source_data(
        &self,
        source: &crate::store::RetainedBootstrapSourceData,
    ) -> Result<(), AppError> {
        let data = source.data();
        let context = data.view_data().context.spec_data();
        if context.vault != self.binding.vault
            || context.group != self.binding.group
            || context.signer != self.binding.signer
            || context.profile.as_bytes() != &self.binding.profile
            || data.source_id_data().as_bytes() != &self.binding.source
        {
            return Err(refused());
        }
        Ok(())
    }
}

fn expected(index: &Index, row: &Row) -> Result<NetworkExpected, AppError> {
    let birth = row.backend.birth().ok_or_else(refused)?;
    if birth.kind != BirthKind::NativeFounder {
        return Err(refused());
    }
    Ok(NetworkExpected {
        row: row.id,
        vault: index.capability.original.vault,
        root: index.capability.original.root,
        identity: row.identity,
        invocation: birth.invocation,
        request: birth.request,
        signer: birth.signer,
        network: birth.network,
    })
}

/// A native Pending frame is recoverable before any network file exists.
/// Operational Founder/Retired rows require actual continuously held outputs.
/// The pre-cleaner observation retains these proofs through the common opener.
pub(super) fn capture_selected(
    store: &ServerStore,
    index: &Index,
    handler: &super::admission::Handler,
) -> Result<Vec<Arc<NetworkDependency>>, AppError> {
    let mut proofs = Vec::new();
    for row in &index.rows {
        if row
            .backend
            .birth()
            .is_none_or(|birth| birth.kind != BirthKind::NativeFounder)
        {
            continue;
        }
        let expected = expected(index, row)?;
        match &row.backend {
            Backend::Pending { network_frame, .. } => {
                crate::store::persistence::native_registry::network::validate_frame(
                    store,
                    expected,
                    network_frame,
                )?;
            }
            Backend::Founder { .. } | Backend::RetiredFounder { .. } => {
                proofs.push(NetworkDependency::capture_cold(
                    store,
                    expected,
                    None,
                    Arc::new(handler.payment_tail()),
                )?)
            }
            Backend::AbandonedFounder { .. } => {}
            _ => return Err(refused()),
        }
    }
    Ok(proofs)
}

pub(super) fn require_activation(
    store: &ServerStore,
    selected: &Index,
    request: &founder::Activate,
    proof: &NetworkDependency,
) -> Result<(), AppError> {
    let row = selected
        .rows
        .iter()
        .find(|row| row.id == request.id)
        .ok_or_else(refused)?;
    if row.identity != request.identity || row.backend.birth() != Some(&request.birth) {
        return Err(refused());
    }
    let frame = match &row.backend {
        Backend::Pending { network_frame, .. } => Some(network_frame.as_slice()),
        Backend::Founder { binding, .. } if binding == &request.binding => None,
        _ => return Err(refused()),
    };
    if !proof.matches(expected(selected, row)?, frame) {
        return Err(refused());
    }
    proof.require(store)
}

pub(super) fn capture_activation(
    store: &ServerStore,
    selected: &Index,
    candidate: &Index,
    handler: &super::admission::Handler,
) -> Result<Option<Arc<NetworkDependency>>, AppError> {
    for (old, new) in selected.rows.iter().zip(&candidate.rows) {
        if old == new {
            continue;
        }
        if let (
            Backend::Pending {
                birth,
                network_frame,
            },
            Backend::Founder { .. },
        ) = (&old.backend, &new.backend)
        {
            if birth.kind == BirthKind::NativeFounder {
                return NetworkDependency::capture_cold(
                    store,
                    expected(selected, old)?,
                    Some(network_frame),
                    Arc::new(handler.payment_tail()),
                )
                .map(Some);
            }
        }
    }
    Ok(None)
}

/// At most one new native Pending can be born by this successor. Its frame is
/// authenticated under the original fixed invoice; no network file exists yet.
pub(super) fn require_birth_candidate(
    store: &ServerStore,
    selected: &Index,
    candidate: &Index,
) -> Result<(), AppError> {
    if candidate.rows.len() == selected.rows.len() + 1 {
        let row = candidate.rows.last().ok_or_else(refused)?;
        if let Backend::Pending {
            birth,
            network_frame,
        } = &row.backend
        {
            if birth.kind == BirthKind::NativeFounder {
                crate::store::persistence::native_registry::network::validate_frame(
                    store,
                    expected(candidate, row)?,
                    network_frame,
                )?;
            }
        }
    }
    Ok(())
}
