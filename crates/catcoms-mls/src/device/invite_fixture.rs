//! Explicit-time packages for cross-crate real-MLS tests. This helper is never
//! available in an ordinary build and does not bypass a closed provider.
use super::*;

impl MlsDevice {
    /// Make a real signed invitation package with a fixed test lifetime. Unlike
    /// the legacy convenience builder, this never reads the wall clock. The
    /// normal SDK-operation admission and credential binding still apply.
    pub fn key_package_for_invite_fixture(
        &self,
        group_id: &[u8],
        invite_nonce: [u8; 16],
        not_before: u64,
        not_after: u64,
    ) -> Result<KeyPackage, MlsError> {
        self.require_sdk_operation_admission()?;
        let membership = MembershipCredential {
            device_id: self.device_id,
            group_id: group_id.to_vec(),
            invite_nonce,
        };
        let credential = CredentialWithKey {
            credential: BasicCredential::new(membership.encode()).into(),
            signature_key: self.signer.public().into(),
        };
        let bundle = KeyPackage::builder()
            .leaf_node_capabilities(capabilities())
            .key_package_lifetime(Lifetime::init(not_before, not_after))
            .build(CIPHERSUITE, &self.provider, &self.signer, credential)
            .map_err(proto)?;
        Ok(bundle.key_package().clone())
    }
}
