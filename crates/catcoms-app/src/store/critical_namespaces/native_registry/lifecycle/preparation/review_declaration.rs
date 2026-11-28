//! Read the real founder, network and saved source while the independent
//! declaration proof owns any Catalog Stage. No listener or key is entered.
use super::*;
use crate::store::{
    critical_namespaces::{
        archive_bootstrap::{MountedRoot, ROOT_METADATA_FEE},
        archive_current_namespace_codec::CurrentLayoutData,
        review_hold_namespace::declaration::ReviewDeclarationKind,
    },
    persistence::native_registry::lifecycle::review_declaration::ReviewDeclarationInventory,
    RetainedBootstrapSourceData, ServerStore,
};

pub(in crate::store) struct VerifiedFounderReviewParent {
    native: ReviewDeclarationInventory,
    layout: CurrentLayoutData,
    founder: super::super::network::ColdFounderSelection,
    source: RetainedBootstrapSourceData,
    root: MountedRoot,
    governance: crate::store::critical_namespaces::governance_namespace_codec::capability::GovernanceCapabilityData,
    outbox: crate::store::critical_namespaces::member_outbox_namespace_codec::MemberOutboxCapabilityData,
}

impl VerifiedFounderReviewParent {
    /// This proves unchanged founder leaves only. The declaration separately
    /// authenticates the exact Current/Stage/claim relationship before any IO.
    pub(in crate::store) fn capture(
        store: &ServerStore,
        kind: ReviewDeclarationKind,
    ) -> Result<Self, AppError> {
        let policy = store
            .governance_layout_policy
            .as_ref()
            .ok_or_else(refused)?;
        policy.require_persistent_capacity()?;
        let pool = policy.native_pool.as_ref().ok_or_else(refused)?;
        let native = ReviewDeclarationInventory::capture(store, pool)?;
        let layout = native.layout(store)?;
        let original = NativeCapabilityData::for_current_data(layout, &layout.encode_data()?)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let plain = Zeroizing::new(unseal(&key, &unframe(native.catalog().0)?)?);
        let parsed = match kind {
            ReviewDeclarationKind::Hold => {
                grammar::parse_review_hold_family(&plain, &layout.vault, true)
            }
            ReviewDeclarationKind::Decision => {
                grammar::parse_review_decision_family(&plain, &layout.vault, true)
                    .or_else(|_| grammar::parse_review_hold_family(&plain, &layout.vault, true))
            }
        }?;
        if parsed.current_layout != Some(layout) || parsed.count != 0 {
            return Err(refused());
        }
        if kind == ReviewDeclarationKind::Decision && parsed.review_hold_payload.is_none() {
            return Err(refused());
        }
        let index = authenticate_fixed_index(
            &key,
            Capability { original },
            parsed.native_registry_payload.ok_or_else(refused)?,
            native.index_and_marker(),
        )?;
        let founder = super::super::network::capture_declaring_founder(store, &index, pool)?;
        let root = store.capture_declared_archive_root(
            parsed.archive.as_ref().ok_or_else(refused)?,
            policy
                .current
                .archive
                .pending
                .pool
                .try_reserve(ROOT_METADATA_FEE)?,
        )?;
        let source = store.read_saved_bootstrap_source_under_root(
            &root,
            &founder.binding.group,
            founder.binding.signer,
        )?;
        founder.require_saved_source_data(&source)?;
        let value = Self {
            native,
            layout,
            founder,
            source,
            root,
            governance: parsed.governance.ok_or_else(refused)?,
            outbox: parsed.member_outbox.ok_or_else(refused)?,
        };
        value.require_original(store)?;
        Ok(value)
    }

    pub(in crate::store) fn layout(&self) -> CurrentLayoutData {
        self.layout
    }

    pub(in crate::store) fn original_catalog(&self) -> &[u8] {
        self.native.catalog().0
    }

    pub(in crate::store) fn catalog_identity(&self) -> catcoms_storage::kept::SessionId {
        self.native.catalog().1
    }

    pub(in crate::store) fn require_declared_storage_markers(
        &self,
        key: &[u8; 32],
        (governance, outbox): (&[u8], &[u8]),
    ) -> Result<(), AppError> {
        use crate::store::critical_namespaces::{governance_layout, member_outbox_layout};
        governance_layout::codec::require_marker(key, self.governance, governance)?;
        member_outbox_layout::codec::require_marker(key, self.outbox, outbox)
    }

    pub(in crate::store) fn index_digest(&self) -> [u8; 32] {
        *blake3::hash(self.native.index_and_marker().0).as_bytes()
    }

    pub(in crate::store) fn founder_digest(&self) -> [u8; 32] {
        let mut hash =
            blake3::Hasher::new_derive_key("catcoms/private-chat-v2-founder-review-parent/v1");
        hash.update(&self.founder.invocation);
        hash.update(&self.founder.request);
        hash.update(&self.founder.binding.source);
        hash.update(&self.founder.binding.pin);
        hash.update(&self.founder.network.frame_digest());
        *hash.finalize().as_bytes()
    }

    pub(in crate::store) fn require_original(&self, store: &ServerStore) -> Result<(), AppError> {
        self.native.require_original(store)?;
        self.require_preserved_leaves(store)?;
        self.native.require_original(store)
    }

    pub(in crate::store) fn require_preserved_leaves(
        &self,
        store: &ServerStore,
    ) -> Result<(), AppError> {
        self.native.require_preserved(store)?;
        self.founder.network.require(store)?;
        self.source.validate_under_archive_root(store, &self.root)?;
        self.native.require_preserved(store)
    }
}
