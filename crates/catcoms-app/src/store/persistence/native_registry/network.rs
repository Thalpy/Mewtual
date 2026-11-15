//! Immutable initial native network record. Pending index DATA already covers
//! its exact sealed bytes. This role can only create/complete one original
//! fixed-name object; it cannot rename, replace, reseal or publish a runtime.
use super::*;
use critical::pending_join::CreatedStage;

const MAX_FRAME: usize = 64 * 1024;
mod dependency;
pub(in crate::store) use dependency::{validate_frame, COLD_FEE};
pub(in crate::store) use dependency::{NetworkDependency, NetworkExpected};

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) enum Fault {
    PartialWrite,
    BeforeCover,
    AfterCover,
}
#[cfg(test)]
thread_local! { static FAULT: std::cell::Cell<Option<Fault>> = const { std::cell::Cell::new(None) }; }
#[cfg(test)]
pub(in crate::store) fn arm(fault: Fault) {
    FAULT.with(|slot| {
        assert!(slot.get().is_none());
        slot.set(Some(fault));
    });
}

pub(in crate::store) struct NetworkIo {
    // Original output and parents remain continuously owned across every
    // partial write and failed covering barrier. Never restart with new handles.
    output: Option<CreatedStage>,
    servers: HeldDirectory,
    vault: HeldDirectory,
    row: u64,
    digest: [u8; 32],
}
impl NetworkIo {
    /// Continuously inherit the SAME kernel objects, never reopen a path to
    /// adopt an equal replacement. Cache this one dependency in the runtime;
    /// it follows uncertain activation IO with its original fee-only payer.
    pub(in crate::store) fn dependency(
        &mut self,
        store: &ServerStore,
        expected: NetworkExpected,
        frame: &[u8],
        payment: std::sync::Arc<dyn Send + Sync>,
    ) -> Result<std::sync::Arc<NetworkDependency>, AppError> {
        self.cover(store, frame)?;
        if self.row != expected.row {
            return Err(denied());
        }
        let output = self.output.as_ref().ok_or_else(denied)?;
        let dependency = NetworkDependency::from_original(
            expected,
            HeldNativeInput {
                file: output.file.try_clone().map_err(io)?,
                identity: output.identity.ok_or_else(denied)?,
                raw: Zeroizing::new(frame.to_vec()),
            },
            HeldDirectory {
                file: self.servers.file.try_clone().map_err(io)?,
                identity: self.servers.identity,
            },
            HeldDirectory {
                file: self.vault.file.try_clone().map_err(io)?,
                identity: self.vault.identity,
            },
            payment,
        );
        dependency.require(store)?;
        Ok(std::sync::Arc::new(dependency))
    }
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        row: u64,
        frame: &[u8],
    ) -> Result<Self, AppError> {
        if row == 0 || !(40..=MAX_FRAME).contains(&frame.len()) {
            return Err(denied());
        }
        let value = Self {
            output: None,
            servers: HeldDirectory::capture(&path(store, false, "servers")?)?,
            vault: HeldDirectory::capture(&path(store, false, "")?)?,
            row,
            digest: *blake3::hash(frame).as_bytes(),
        };
        value.require_parents(store)?;
        // A warm first attempt cannot adopt another file, even equal bytes.
        // Cold Pending recovery needs its own authenticated receipt role.
        absent(&value.output_path(store)?)?;
        Ok(value)
    }
    fn output_path(&self, store: &ServerStore) -> Result<PathBuf, AppError> {
        path(store, false, &format!("servers/{}.native-net-v1", self.row))
    }
    fn require_parents(&self, store: &ServerStore) -> Result<(), AppError> {
        self.vault.require(&path(store, false, "")?)?;
        self.servers.require(&path(store, false, "servers")?)
    }
    pub(in crate::store) fn cover(
        &mut self,
        store: &ServerStore,
        frame: &[u8],
    ) -> Result<(), AppError> {
        if !(40..=MAX_FRAME).contains(&frame.len())
            || *blake3::hash(frame).as_bytes() != self.digest
        {
            return Err(denied());
        }
        self.require_parents(store)?;
        let output = self.output_path(store)?;
        if self.output.is_none() {
            absent(&output)?;
        }
        // The shared private engine retains create_new's real File BEFORE
        // identity queries/writes, checks its exact prefix and never truncates
        // a foreign object. This wrapper narrows its maximum to64KiB.
        #[cfg(test)]
        let fault = FAULT.with(|slot| slot.take());
        ensure_archive_output(
            &output,
            frame,
            &mut self.output,
            ArchiveOutputMode::NewOriginal,
            #[cfg(test)]
            if fault == Some(Fault::PartialWrite) {
                201
            } else {
                0
            },
            #[cfg(test)]
            201,
        )?;
        self.require_parents(store)?;
        #[cfg(test)]
        if fault == Some(Fault::BeforeCover) {
            return Err(io("injected before native network directory cover"));
        }
        self.servers.file.sync_all().map_err(io)?;
        self.vault.file.sync_all().map_err(io)?;
        #[cfg(test)]
        if fault == Some(Fault::AfterCover) {
            return Err(io("injected after native network directory cover"));
        }
        self.require_parents(store)?;
        let original = self.output.as_ref().ok_or_else(denied)?;
        verify_archive_held_name(&output, original)?;
        let final_input = HeldNativeInput::capture(&output, frame.len())?.ok_or_else(denied)?;
        if Some(final_input.identity()) != original.identity || final_input.raw() != frame {
            return Err(denied());
        }
        final_input.require(&output)?;
        self.require_parents(store)
    }
}
