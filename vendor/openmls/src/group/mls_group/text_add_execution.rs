//! Private computation strategy, never an SDK/Source/governance permission.
//! Only the separately named, structurally checked native Add builder selects
//! Serial. All existing/default wrappers select Legacy and retain their Rayon
//! execution. No thread-local/global switch or runtime pool is installed.
#[derive(Clone, Copy)]
pub(crate) enum TextAddExecution {
    Legacy,
    #[cfg(all(
        not(target_arch = "wasm32"),
        not(any(
            feature = "extensions-draft",
            feature = "virtual-clients-draft",
            feature = "targeted-messages-draft",
            feature = "migration-import",
            feature = "fork-resolution",
            feature = "draft-ietf-mls-pq-ciphersuites",
        ))
    ))]
    Serial,
}
impl TextAddExecution {
    pub(crate) fn is_serial(self) -> bool {
        match self {
            Self::Legacy => false,
            #[cfg(all(
                not(target_arch = "wasm32"),
                not(any(
                    feature = "extensions-draft",
                    feature = "virtual-clients-draft",
                    feature = "targeted-messages-draft",
                    feature = "migration-import",
                    feature = "fork-resolution",
                    feature = "draft-ietf-mls-pq-ciphersuites",
                ))
            ))]
            Self::Serial => true,
        }
    }
}
