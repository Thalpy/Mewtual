//! Exact, owning v2 DATA conversion. This is NOT an owning physical upgrade:
//! the future upgrader must authenticate/select the actual original frame,
//! settle old work/debt and fund BOTH schemas/frames before calling it.
use super::super::operational::data as v2;
use super::capability::Capability;
use super::data::*;
use super::AppError;

fn birth(old: v2::Birth) -> Birth {
    Birth {
        kind: BirthKind::Founder,
        invocation: old.invocation,
        request: old.request,
        signer: old.signer,
        network: old.network,
        join_context: None,
    }
}

impl Index {
    /// Consume every old row, receipt and exact encrypted network Vec without
    /// recomputing highwater, generating bytes or claiming physical provenance.
    /// There is no direct-to-Joined conversion or fabricated Legacy receipt.
    pub(super) fn from_v2_data(old: v2::Index, frame: [u8; 32]) -> Result<Self, AppError> {
        old.encoded_length()?;
        let capability = Capability {
            original: old.capability.original,
        };
        let origin = MigrationOrigin {
            generation: old.generation,
            frame,
        };
        let mut rows = Vec::with_capacity(old.rows.len());
        for row in old.rows {
            let backend = match row.backend {
                v2::Backend::Legacy { invite } => Backend::Legacy {
                    origin: LegacyOrigin::Imported,
                    invite,
                },
                v2::Backend::Pending {
                    birth: original,
                    network_frame,
                } => Backend::Pending {
                    birth: birth(original),
                    network_frame,
                },
                v2::Backend::Founder {
                    birth: original,
                    binding,
                } => Backend::Founder {
                    birth: birth(original),
                    binding,
                },
                v2::Backend::Retired {
                    birth: original,
                    binding,
                    scope_terminal,
                } => Backend::RetiredFounder {
                    birth: birth(original),
                    binding,
                    terminal: scope_terminal,
                },
                v2::Backend::Abandoned {
                    birth: original,
                    reason,
                    journal_terminal,
                } => Backend::AbandonedFounder {
                    birth: birth(original),
                    reason,
                    terminal: journal_terminal,
                },
            };
            rows.push(Row {
                id: row.id,
                identity: row.identity,
                is_dm: row.is_dm,
                name: row.name,
                metadata_revision: 0,
                backend,
            });
        }
        let value = Self {
            capability,
            origin,
            generation: 1,
            parent: None,
            last_issued_id: old.last_issued_id,
            rows,
        };
        // Includes mandatory parent/activation/terminal capacity, not merely
        // round-trip fit. A supported predecessor must remain completable.
        value.encoded_length()?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests;
