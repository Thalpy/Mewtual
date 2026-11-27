//! Rejoin index DATA while the physical namespace and A2 writer are staged
//! separately. A syntactic successor cannot prove either membership event.
use super::*;

/// The production caller must obtain these observations from its original
/// selected A1 row and a separately replayed, physically selected Remove.
/// This type is DATA for checking a candidate, never removal authority.
struct RemovalData {
    row_id: u64,
    row_identity: [u8; 32],
    own_admission: [u8; 32],
    header: [u8; 32],
    epoch: u64,
}

fn require_rejoin_successor(
    selected: &Index,
    candidate: &Index,
    frame: [u8; 32],
    removal: &RemovalData,
) -> Result<(), AppError> {
    selected.rejoin_encoded_length()?;
    candidate.rejoin_encoded_length()?;
    if selected.capability != candidate.capability
        || selected.origin != candidate.origin
        || selected.generation.checked_add(1) != Some(candidate.generation)
        || candidate.parent != Some(frame)
        || selected.rows.len().checked_add(1) != Some(candidate.rows.len())
    {
        return Err(refused());
    }

    let (added, prefix) = candidate.rows.split_last().ok_or_else(refused)?;
    if selected.last_issued_id.checked_add(1) != Some(added.id)
        || candidate.last_issued_id != added.id
        || added.metadata_revision != 0
    {
        return Err(refused());
    }
    let Backend::Joined {
        birth: added_birth,
        binding: active,
    } = &added.backend
    else {
        return Err(refused());
    };
    if !added_birth.kind.is_native_joined() {
        return Err(refused());
    }

    let mut consumed = false;
    for (old, new) in selected.rows.iter().zip(prefix) {
        if old.id != removal.row_id {
            if old != new {
                return Err(refused());
            }
            continue;
        }
        let (
            Backend::Joined { birth, binding },
            Backend::RetiredJoined {
                birth: retained_birth,
                binding: retained_binding,
                terminal,
            },
        ) = (&old.backend, &new.backend)
        else {
            return Err(refused());
        };
        if !birth.kind.is_native_joined()
            || old.identity != removal.row_identity
            || old.identity != new.identity
            || old.id != new.id
            || old.is_dm != new.is_dm
            || old.name != new.name
            || old.metadata_revision != new.metadata_revision
            || birth != retained_birth
            || binding != retained_binding
            || binding.own_add != removal.own_admission
            || *terminal != removal.header
            || !(binding.joined_epoch < removal.epoch && removal.epoch < active.joined_epoch)
            || !binding.same_device_group(active)
            || binding.source == active.source
            || binding.join_context == active.join_context
            || binding.own_add == active.own_add
            || birth.request == added_birth.request
            || old.name != added.name
            || binding.local_display != active.local_display
        {
            return Err(refused());
        }
        consumed = true;
    }
    if !consumed {
        return Err(refused());
    }
    Ok(())
}

#[path = "tests/rejoin.rs"]
mod tests;
