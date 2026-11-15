//! Paid original-runtime selection for native text operations. Numeric row
//! IDs and recovered index DATA cannot construct this association. It owns no
//! Entry, issuer, runtime body, physical lease or positive key permission.
use super::*;
use std::sync::Weak;

/// Association issued only by the actual retained, durably activated runtime.
/// Clone copies a paid metadata handle; native must separately bound/fund its
/// invocation wrappers. Selection alone authorizes no SDK, read, sign or save.
pub struct NativeTextRuntimeSelection<T: MeshTransport, R: CryptoRngCore> {
    pub(super) original: Arc<Selection<T, R>>,
}

pub(super) struct Selection<T: MeshTransport, R: CryptoRngCore> {
    pub(super) scope: Arc<Scope>,
    pub(super) runtime: Weak<RuntimeCell<T, R>>,
    pub(super) status: crate::store::NativeCreationStatusData,
    pub(super) invocation: [u8; 32],
    pub(super) request: [u8; 32],
    // LAST: actual weak RuntimeCell allocation and all copied metadata must
    // remain paid even after the whole canonical Sources/SDK has been dropped.
    _metadata: Arc<RuntimeMetadata>,
}

impl<T: MeshTransport, R: CryptoRngCore> Clone for NativeTextRuntimeSelection<T, R> {
    fn clone(&self) -> Self {
        Self {
            original: self.original.clone(),
        }
    }
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for NativeTextRuntimeSelection<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeTextRuntimeSelection { .. }")
    }
}
impl<T: MeshTransport, R: CryptoRngCore> NativeTextRuntimeSelection<T, R> {
    pub(super) fn from_stored(
        job: &NativeTextCreationJob<T, R>,
        status: crate::store::NativeCreationStatusData,
    ) -> Self {
        // The caller has just checked the actual covered row, retained whole
        // driver and current original UI/source. No decoded constructor exists.
        let body = job.loan.body.as_ref().expect("original stored runtime");
        let request = body.request.as_ref().expect("original canonical request");
        Self {
            original: Arc::new(Selection {
                scope: job.command.scope.clone(),
                runtime: Arc::downgrade(&job.loan.cell),
                status,
                invocation: request.invocation,
                request: request.digest,
                _metadata: body._metadata.clone(),
            }),
        }
    }

    /// Local index DATA only, not an authenticated native server credential.
    pub fn id_data(&self) -> u64 {
        self.original.status.id
    }

    /// Exact original runtime allocation, scope and covered row comparison.
    /// Equality survives closure; it never establishes present-time authority.
    pub fn same_original(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.original.scope, &other.original.scope)
            && Weak::ptr_eq(&self.original.runtime, &other.original.runtime)
            && self.original.status == other.original.status
            && self.original.invocation == other.original.invocation
            && self.original.request == other.original.request
    }
}

pub(super) fn invoice<T: MeshTransport, R: CryptoRngCore>() -> usize {
    2 * std::mem::size_of::<NativeTextRuntimeSelection<T, R>>()
        + std::mem::size_of::<Selection<T, R>>()
        + 2 * std::mem::size_of::<usize>() // actual Arc allocation header
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests;
