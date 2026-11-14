//! Whole-close observation of the original metadata/UI work slots. Source,
//! SDK/TCP and completed-debt custody must be composed separately by native.
use super::upgrade_source::owning::WorkDrain;
use super::*;

/// Atomic-only owning capture, including Version2/foreign-credit refusal.
/// Never destroy it under native guards. Native must retain the SAME facade in
/// canonical Closing custody through capture loss, verification and drainage.
pub struct NativeTextWorkRetirementPreparation {
    original: Option<WorkDrain>,
    expected: NativeUiInvocationBudget,
    credit: NativeUiInvocationMetadata,
}
/// Paid original metadata/UI observation only. It neither holds Physical nor
/// proves that Sources, SDK/TCP, completed debt or the affine facade was dropped.
/// Missing/version2 evidence has no successful constructor.
pub struct NativeTextWorkRetirement {
    original: WorkDrain,
    _credit: NativeUiInvocationMetadata,
}

impl NativeTextVault {
    /// Whole-vault permanent closure. Native first finishes continuity while
    /// Host is live, then drains original Sources. Only then capture this under
    /// a short SAME-original Store borrow. No pool lock, notification, IO,
    /// callback or whole-owner destruction occurs in this capture.
    pub fn capture_work_retirement(
        &self,
        credit: NativeUiInvocationMetadata,
    ) -> NativeTextWorkRetirementPreparation {
        self.close_admission();
        let original = match &self.index {
            facade::IndexOwner::Version3(index) => Some(index.observe_closed_work()),
            facade::IndexOwner::Version2(_) => None,
        };
        NativeTextWorkRetirementPreparation {
            original,
            expected: self.ui_invocation_budget(),
            credit,
        }
    }
}
impl NativeTextWorkRetirementPreparation {
    /// Validate the actual original caller debit only OUTSIDE native/Root
    /// guards. Refusal returns the whole capture and its original payment.
    pub fn finish(self) -> Result<NativeTextWorkRetirement, Box<Self>> {
        if self.original.is_none() || self.expected.require_credit(&self.credit).is_err() {
            return Err(Box::new(self));
        }
        Ok(NativeTextWorkRetirement {
            original: self.original.expect("original version3 work controls"),
            _credit: self.credit,
        })
    }
}
impl NativeTextWorkRetirement {
    /// Both original slots CLOSED+zero; a fresh registry or idle OPEN slot can
    /// never satisfy this observation. This is NOT universal physical release.
    pub fn is_drained_cached(&self) -> bool {
        self.original.is_drained()
    }
    /// Await actual capsule acknowledgments without native Store/UI/lane guards
    /// and without joining an IPC/unlock task (which may itself await close).
    pub async fn wait(&self) {
        self.original.wait().await;
    }
}
impl std::fmt::Debug for NativeTextWorkRetirementPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeTextWorkRetirementPreparation { .. }")
    }
}
impl std::fmt::Debug for NativeTextWorkRetirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeTextWorkRetirement { .. }")
    }
}

// Two public envelopes and overlap are paid independently before native capture;
// original slot controls and their narrow debit tail have the Physical invoice.
const _: () = assert!(
    2 * (std::mem::size_of::<NativeTextWorkRetirementPreparation>()
        + std::mem::size_of::<NativeTextWorkRetirement>())
        + 1024
        <= super::native_ui::INVOCATION_BYTES
);

#[cfg(test)]
mod tests;
