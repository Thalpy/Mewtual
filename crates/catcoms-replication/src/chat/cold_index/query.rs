//! Owned one-node query DATA. Root/key are fixed through child opens, but the root selection is
//! still the caller's untrusted DATA. No completed-source or new-acceptance capability is issued.
use super::*;
use catcoms_storage::private_chat_page::open_private_chat_page;
use catcoms_storage::StorageError;

#[derive(Debug, thiserror::Error)]
pub enum IndexDataError {
    #[error(transparent)]
    Codec(#[from] ChatError),
    #[error(transparent)]
    Sealed(#[from] StorageError),
}
#[derive(Clone, Copy)]
pub struct QueryRootData<'a> {
    pub context: IndexContextData<'a>,
    pub root: NodeRefData,
    pub key: [u8; 32],
    /// Actions requires a positive bound>=root unique keys. Directory must use None. It is
    /// DATA consistency only; a forged bound cannot supply actual selected-tip provenance.
    pub accepted_leaf_count: Option<u64>,
}
impl std::fmt::Debug for QueryRootData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryRootData")
            .field("summary", &self.root.summary)
            .finish_non_exhaustive()
    }
}
pub struct IndexQueryData {
    plain: Zeroizing<Vec<u8>>,
    current: NodeRefData,
    root_page: PrivateChatPageId,
    key: [u8; 32],
    accepted_leaf_count: Option<u64>,
}
impl std::fmt::Debug for IndexQueryData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexQueryData")
            .field("summary", &self.current.summary)
            .finish_non_exhaustive()
    }
}

pub fn open_query_data(
    key: &[u8; 32],
    spec: QueryRootData<'_>,
    frame: &[u8],
) -> Result<IndexQueryData, IndexDataError> {
    spec.context.validate()?;
    spec.root.summary.validate_data(NodePlacementData::Root)?;
    match (spec.context.scope, spec.accepted_leaf_count) {
        (IndexScopeData::Directory, None) => {}
        (IndexScopeData::Actions(_), Some(n)) if n >= spec.root.summary.keys => {}
        _ => return Err(ChatError::Scope.into()),
    }
    let plain = open_plain(key, spec.root.page, frame)?;
    {
        let node = NodeFrame::decode(&plain, NodePlacementData::Root)?;
        if node.context() != spec.context || node.summary() != spec.root.summary {
            return Err(ChatError::Scope.into());
        }
        check_claim_bounds(&node, spec.accepted_leaf_count)?;
    }
    Ok(IndexQueryData {
        plain,
        current: spec.root,
        root_page: spec.root.page,
        key: spec.key,
        accepted_leaf_count: spec.accepted_leaf_count,
    })
}
impl IndexQueryData {
    pub fn node(&self) -> NodeFrame<'_> {
        // The only constructors validated these immutable owned bytes; no mutable accessor.
        NodeFrame::decode(&self.plain, NodePlacementData::Root)
            .expect("immutable validated index DATA")
    }
    pub fn reference_data(&self) -> ChildRefData<'_> {
        ChildRefData {
            context: self.node().context(),
            node: self.current,
        }
    }
    pub fn root_page_data(&self) -> PrivateChatPageId {
        self.root_page
    }
    pub fn key_data(&self) -> [u8; 32] {
        self.key
    }
    /// Structural query choice only. Actual head/root/prefix/negative custody is not supplied.
    pub fn search_data(&self) -> Result<SearchData<'_>, ChatError> {
        let node = self.node();
        let context = node.context();
        if self.key < node.summary.min || self.key > node.summary.max {
            return Ok(miss(context.scope, self.root_page, self.key));
        }
        if node.kind == NodeKindData::Leaf {
            for index in 0..node.rows {
                let row = node.leaf_row(index)?;
                if row.key == self.key {
                    return Ok(match row.value {
                        ValueData::Directory(value) => SearchData::Directory(DirectoryHitData {
                            context,
                            key: row.key,
                            value,
                        }),
                        ValueData::Action(value) => SearchData::Action(value),
                    });
                }
                if row.key > self.key {
                    break;
                }
            }
            return Ok(miss(context.scope, self.root_page, self.key));
        }
        let mut chosen = 0;
        for index in 1..node.rows {
            if node.branch_row(index)?.0 <= self.key {
                chosen = index;
            } else {
                break;
            }
        }
        let (lower, page, count) = node.branch_row(chosen)?;
        let upper = if chosen + 1 < node.rows {
            Some(node.branch_row(chosen + 1)?.0)
        } else {
            None
        };
        Ok(SearchData::Child(ChildStepData {
            context,
            root_page: self.root_page,
            key: self.key,
            page,
            height: node.summary.height - 1,
            count,
            lower,
            upper,
            parent_max: node.summary.max,
            accepted_leaf_count: self.accepted_leaf_count,
        }))
    }
}
/// Fixed path relation, constructible only by querying this parsed node. It authenticates no
/// global source: arbitrary roots can still be supplied to the DATA entry point above.
pub struct ChildStepData<'a> {
    context: IndexContextData<'a>,
    root_page: PrivateChatPageId,
    key: [u8; 32],
    page: PrivateChatPageId,
    height: u8,
    count: u64,
    lower: [u8; 32],
    upper: Option<[u8; 32]>,
    parent_max: [u8; 32],
    accepted_leaf_count: Option<u64>,
}
impl std::fmt::Debug for ChildStepData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChildStepData")
            .field("height", &self.height)
            .field("keys", &self.count)
            .finish_non_exhaustive()
    }
}
impl ChildStepData<'_> {
    pub fn expected_page_data(&self) -> PrivateChatPageId {
        self.page
    }
    pub fn open_data(
        &self,
        key: &[u8; 32],
        frame: &[u8],
    ) -> Result<IndexQueryData, IndexDataError> {
        // context.group borrows the PARENT's plaintext. The caller must pay that retained
        // allocation alongside raw/cipher/new-plaintext while opening this child. The returned
        // query owns independent plaintext, so an iterative caller can then drop the parent.
        if self.key < self.lower || self.upper.is_some_and(|upper| self.key >= upper) {
            return Err(ChatError::Scope.into());
        }
        let plain = open_plain(key, self.page, frame)?;
        let summary;
        {
            let node = NodeFrame::decode(&plain, NodePlacementData::Child)?;
            summary = node.summary();
            if node.context() != self.context
                || summary.height != self.height
                || summary.keys != self.count
                || summary.min != self.lower
                || self.upper.is_some_and(|upper| summary.max >= upper)
                || (self.upper.is_none() && summary.max != self.parent_max)
            {
                return Err(ChatError::Scope.into());
            }
            check_claim_bounds(&node, self.accepted_leaf_count)?;
        }
        Ok(IndexQueryData {
            plain,
            current: NodeRefData {
                page: self.page,
                summary,
            },
            root_page: self.root_page,
            key: self.key,
            accepted_leaf_count: self.accepted_leaf_count,
        })
    }
}
#[derive(Clone, Copy)]
pub struct DirectoryHitData<'a> {
    context: IndexContextData<'a>,
    key: [u8; 32],
    value: DirectoryData,
}
impl std::fmt::Debug for DirectoryHitData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectoryHitData")
            .field("value", &self.value)
            .finish_non_exhaustive()
    }
}
impl<'a> DirectoryHitData<'a> {
    pub fn value_data(&self) -> DirectoryData {
        self.value
    }
    /// Uses its OWN admission key. Caller supplies only an action key, never replacement A.
    pub fn action_route_data(&self, action: [u8; 32]) -> DirectoryRouteData<'a> {
        match self.value.tree {
            DirectoryTreeData::Unknown => DirectoryRouteData::Unknown(self.value),
            DirectoryTreeData::Empty => DirectoryRouteData::Empty(self.value),
            DirectoryTreeData::Present(root) => DirectoryRouteData::Root(QueryRootData {
                context: IndexContextData {
                    scope: IndexScopeData::Actions(OwnerFactId::from_bytes(self.key)),
                    ..self.context
                },
                root,
                key: action,
                accepted_leaf_count: Some(self.value.accepted_leaf_count),
            }),
        }
    }
}
#[derive(Debug)]
pub enum DirectoryRouteData<'a> {
    Unknown(DirectoryData),
    Empty(DirectoryData),
    Root(QueryRootData<'a>),
}
/// NoKey is only structural consistency under a DATA root, never complete-source absence.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MissData {
    root: PrivateChatPageId,
    key: [u8; 32],
}
impl MissData {
    pub fn root_page_data(self) -> PrivateChatPageId {
        self.root
    }
    pub fn key_data(self) -> [u8; 32] {
        self.key
    }
}
impl std::fmt::Debug for MissData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MissData { .. }")
    }
}
#[derive(Debug)]
pub enum SearchData<'a> {
    Child(ChildStepData<'a>),
    Directory(DirectoryHitData<'a>),
    Action(ActionClaimData),
    NoDirectoryEntry(MissData),
    NoActionKey(MissData),
}
fn miss(scope: IndexScopeData, root: PrivateChatPageId, key: [u8; 32]) -> SearchData<'static> {
    let data = MissData { root, key };
    match scope {
        IndexScopeData::Directory => SearchData::NoDirectoryEntry(data),
        IndexScopeData::Actions(_) => SearchData::NoActionKey(data),
    }
}
fn open_plain(
    key: &[u8; 32],
    expected: PrivateChatPageId,
    frame: &[u8],
) -> Result<Zeroizing<Vec<u8>>, IndexDataError> {
    // Enforce the tighter INDEX bound before the generic64KiB private-page AEAD allocation.
    if frame.len() > MAX_INDEX_FRAME_BYTES {
        return Err(ChatError::Bound.into());
    }
    Ok(open_private_chat_page(key, expected, frame)?)
}
fn check_claim_bounds(node: &NodeFrame<'_>, leaves: Option<u64>) -> Result<(), ChatError> {
    if node.kind == NodeKindData::Leaf {
        for index in 0..node.rows {
            if let ValueData::Action(claim) = node.leaf_row(index)?.value {
                claim.check_leaf_bound(leaves.ok_or(ChatError::Scope)?)?;
            }
        }
    }
    Ok(())
}
