//! Finite one-shot PRIVATE SDK work. Actual Source/governance/key admission is
//! a separate Root boundary. The immutable parent is never borrowed mutably.
//! Once Add or merge was entered, an uncertain SDK error is sticky: retrying an
//! SDK call cannot reconstruct outputs lost inside that failed call.
use super::*;
use openmls_traits::OpenMlsProvider;
use std::time::{Duration, SystemTime};

impl PreparedTextCurrentAdd {
    /// Execute only this independently paid private candidate. The host must
    /// durably reserve the SAME governance request and establish its actual
    /// Source/key cut before invoking this method. This library primitive does
    /// not implement that host admission, save, install or publication edge.
    ///
    /// All owning SDK outputs are cached before codec/merge/postchecks. A late
    /// callback error therefore preserves the exact original outputs. Real
    /// Add/merge failures remain sticky, including a consumed Pending state;
    /// neither an Operational no-op nor another invocation proves success.
    pub fn execute_private(&mut self) -> Result<(), MlsError> {
        if matches!(
            self.phase,
            Phase::Failed | Phase::AddEntered | Phase::MergeEntered
        ) {
            return Err(bad());
        }
        if self.phase == Phase::Complete {
            return Ok(());
        }
        if self.phase == Phase::Prepared {
            self.enter_and_stage()?;
        }
        if self.phase == Phase::OutputsCached {
            self.serialize_outputs()?;
        }
        if self.phase == Phase::Serialized {
            self.enter_and_merge()?;
        }
        if self.phase == Phase::Merged {
            self.finish_merged()?;
        }
        if self.phase == Phase::Complete {
            Ok(())
        } else {
            Err(bad())
        }
    }

    fn fail(&mut self) {
        self.phase = Phase::Failed;
        self.graph.policy.fail();
    }

    fn time(&self) -> Result<SystemTime, MlsError> {
        SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_millis(self.now_ms))
            .ok_or_else(bad)
    }

    fn enter_and_stage(&mut self) -> Result<(), MlsError> {
        let now = self.time()?;
        // Full deterministic inventory was checked before constructor copies.
        // Repeat only its cached numeric comparison before irreversible work.
        invoice::require(self.admission.limits(), 0)?;
        self.graph.policy.enter_add().map_err(|_| bad())?;
        self.phase = Phase::AddEntered;
        // The unique validated package is consumed exactly once. Its exact
        // original frame/metadata remains in the Cell on every SDK error.
        let package = self.package.take().ok_or_else(bad)?.into_package();
        #[cfg(test)]
        self.after_cut_for_test(TestCut::AddEntered)?;
        #[cfg(test)]
        {
            self.faults.sdk_dispatches += 1;
        }
        let device = &self.graph.device;
        let result = self
            .graph
            .group
            .group
            .commit_builder()
            .consume_proposal_store(true)
            .force_self_update(true)
            .propose_adds([package])
            .load_psks(device.provider().storage())
            .and_then(|builder| {
                builder.build_private_text_add_with_time(
                    device.provider().rand(),
                    device.provider().crypto(),
                    device.signer(),
                    now,
                )
            })
            .map_err(proto)
            .and_then(|builder| builder.stage_commit(device.provider()).map_err(proto));
        let bundle = match result {
            Ok(bundle) => bundle,
            Err(error) => {
                self.fail();
                return Err(error);
            }
        };
        // Move ALL actual returned owning objects first. No welcome clone,
        // serializer, merge, allocation or fallible postcheck intervenes.
        let (commit, welcome, group_info) = bundle.into_messages();
        self.outputs = Some(SdkOutputs {
            commit,
            welcome,
            group_info,
        });
        self.phase = Phase::OutputsCached;
        if self.graph.policy.outputs_cached().is_err() {
            self.fail();
            return Err(bad());
        }
        #[cfg(test)]
        self.after_cut_for_test(TestCut::OutputsCached)?;
        Ok(())
    }

    fn serialize_outputs(&mut self) -> Result<(), MlsError> {
        if self.commit.is_none() {
            let result = self
                .outputs
                .as_ref()
                .ok_or_else(bad)
                .and_then(|outputs| encode(&outputs.commit, &mut self.output_remaining));
            self.commit = Some(match result {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.fail();
                    return Err(error);
                }
            });
            #[cfg(test)]
            self.after_cut_for_test(TestCut::CommitSerialized)?;
        }
        if self.welcome.is_none() {
            let result = self
                .outputs
                .as_ref()
                .and_then(|outputs| outputs.welcome.as_ref())
                .ok_or_else(bad)
                .and_then(|message| encode(message, &mut self.output_remaining));
            self.welcome = Some(match result {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.fail();
                    return Err(error);
                }
            });
            #[cfg(test)]
            self.after_cut_for_test(TestCut::WelcomeSerialized)?;
        }
        if self.group_info.is_none() {
            let result = self
                .outputs
                .as_ref()
                .and_then(|outputs| outputs.group_info.as_ref())
                .ok_or_else(bad)
                .and_then(|message| encode(message, &mut self.output_remaining));
            self.group_info = Some(match result {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.fail();
                    return Err(error);
                }
            });
            #[cfg(test)]
            self.after_cut_for_test(TestCut::GroupInfoSerialized)?;
        }
        self.phase = Phase::Serialized;
        #[cfg(test)]
        self.after_cut_for_test(TestCut::Serialized)?;
        Ok(())
    }

    fn enter_and_merge(&mut self) -> Result<(), MlsError> {
        let now = self.time()?;
        self.graph.policy.enter_merge().map_err(|_| bad())?;
        self.phase = Phase::MergeEntered;
        #[cfg(test)]
        self.after_cut_for_test(TestCut::MergeEntered)?;
        #[cfg(test)]
        {
            self.faults.merge_dispatches += 1;
        }
        if let Err(error) = self
            .graph
            .group
            .group
            .merge_pending_commit_with_time(self.graph.device.provider(), now)
        {
            self.fail();
            return Err(proto(error));
        }
        self.phase = Phase::Merged;
        #[cfg(test)]
        self.after_cut_for_test(TestCut::Merged)?;
        Ok(())
    }

    fn finish_merged(&mut self) -> Result<(), MlsError> {
        if !self.provider_closed {
            if self
                .graph
                .policy
                .close_merged(self.graph.device.provider().storage())
                .is_err()
            {
                self.fail();
                return Err(bad());
            }
            self.provider_closed = true;
            #[cfg(test)]
            self.after_cut_for_test(TestCut::ProviderClosed)?;
        }
        if self.observation.is_none() {
            // The only new observation issuer consumes a receiver-only borrowed
            // witness from THIS merged, closed, original one-Add Cell. Raw
            // labels/epochs or a restored graph cannot mint this observation.
            let bytes = self
                .commit
                .as_ref()
                .ok_or_else(bad)?
                .len()
                .checked_add(16)
                .ok_or_else(bad)?;
            self.output_remaining = self.output_remaining.checked_sub(bytes).ok_or_else(bad)?;
            self.observation = Some(
                match LocalMembershipChange::from_private_text_add(self.observation_source()?) {
                    Ok(value) => value,
                    Err(error) => {
                        self.fail();
                        return Err(error);
                    }
                },
            );
            #[cfg(test)]
            self.after_cut_for_test(TestCut::ObservationCached)?;
        }
        if self.snapshot.is_none() {
            let limits = self.admission.limits();
            self.snapshot = Some(
                match crate::persist::snapshot_server_bounded(
                    &self.graph.device,
                    &self.graph.group,
                    limits.max_snapshot_bytes,
                    limits.max_provider_backing_bytes,
                ) {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        self.fail();
                        return Err(error);
                    }
                },
            );
            #[cfg(test)]
            self.after_cut_for_test(TestCut::SnapshotCached)?;
        }
        self.phase = Phase::Complete;
        #[cfg(test)]
        self.after_cut_for_test(TestCut::Complete)?;
        Ok(())
    }
}

fn encode(message: &MlsMessageOut, remaining: &mut usize) -> Result<Vec<u8>, MlsError> {
    use tls_codec::Size as _;
    // The pinned codec writes one exact-length Vec. Debit before entering it;
    // a true codec error is terminal, so it cannot repeat paid allocation work.
    let length = message.tls_serialized_len();
    *remaining = remaining.checked_sub(length).ok_or_else(bad)?;
    let encoded = message.tls_serialize_detached().map_err(proto)?;
    if encoded.len() != length || encoded.capacity() > length {
        return Err(bad());
    }
    Ok(encoded)
}
