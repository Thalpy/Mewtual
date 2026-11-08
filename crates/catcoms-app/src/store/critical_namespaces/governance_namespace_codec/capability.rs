//! Numeric/immutable capability DATA consumed only by the private Current4 inspector.
//! No Source, Scope, actor, saved request or SDK authority is represented here.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct GovernanceCapabilityData {
    inner: GovernanceLayoutData,
}
impl std::fmt::Debug for GovernanceCapabilityData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GovernanceCapabilityData")
    }
}
impl GovernanceCapabilityData {
    pub(in crate::store) fn current_layout_data(self) -> current::CurrentLayoutData {
        current::CurrentLayoutData {
            vault: self.inner.vault,
            root: self.inner.root,
        }
    }
    pub(in crate::store) fn encode_data(self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.inner.encode_data()
    }
    pub(in crate::store) fn from_original_catalog_data(
        original: &[u8],
        domain: &[u8; 32],
    ) -> Result<Self, AppError> {
        Ok(Self {
            inner: original_data(original, domain)?.1,
        })
    }
    pub(in crate::store) fn decode_data(
        payload: &[u8],
        layout: current::CurrentLayoutData,
        original_current_descriptor: &[u8],
    ) -> Result<Self, AppError> {
        current::CurrentLayoutData::decode_data(original_current_descriptor, layout)?;
        let expected = GovernanceLayoutData {
            vault: layout.vault,
            root: layout.root,
            current_descriptor: current_descriptor_digest(original_current_descriptor),
        };
        Ok(Self {
            inner: GovernanceLayoutData::decode_data(payload, expected)?,
        })
    }
}
