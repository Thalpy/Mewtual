//! Exact bounded file views for one manifest-selected lane. A missing
//! selected object always refuses; only an authenticated manifest miss can
//! supply the separate first-lane path.
use super::{
    approval::MAX_APPROVAL_FRAME,
    header::{ReceiverKey, MAX_HEADER_BYTES},
    lane_head::{LaneHead, MAX_HEAD_BYTES},
    manifest::{head_digest, ReviewScope, SelectedLane},
    *,
};
use catcoms_replication::chat::cold_index::{MAX_INDEX_FRAME_BYTES, MAX_INDEX_HEIGHT};
use catcoms_storage::private_chat_page::PrivateChatPageId;

pub(super) const MAX_VIEW_FILES: usize = 7 * (MAX_INDEX_HEIGHT as usize + 1) + 6;

const HEAD_SUFFIX: &str = "head";
const HEADER_SUFFIX: &str = "header";
const PAGE_SUFFIX: &str = "page";
const FORK_SUFFIX: &str = "fork";
const APPROVAL_SUFFIX: &str = "receipt";

pub(super) fn lane_directory_name(scope: ReviewScope) -> String {
    format!("g-{}", hex::encode(scope.as_bytes()))
}

pub(super) fn head_name(digest: &[u8; 32]) -> String {
    object_name(digest, HEAD_SUFFIX)
}

pub(super) fn header_name(digest: &[u8; 32]) -> String {
    object_name(digest, HEADER_SUFFIX)
}

pub(super) fn page_name(id: PrivateChatPageId) -> String {
    object_name(id.as_bytes(), PAGE_SUFFIX)
}

pub(super) fn fork_name(digest: &[u8; 32]) -> String {
    object_name(digest, FORK_SUFFIX)
}

pub(super) fn approval_name(digest: &[u8; 32]) -> String {
    object_name(digest, APPROVAL_SUFFIX)
}

fn object_name(digest: &[u8; 32], suffix: &str) -> String {
    format!("{}.{}", hex::encode(digest), suffix)
}

pub(super) struct HeldLane {
    pub(super) directory: HeldDirectory,
    pub(super) head: LaneHead,
    pub(super) sealed_head: HeldObject,
    seen: RefCell<Vec<HeldObject>>,
}

pub(super) struct HeldObject {
    path: PathBuf,
    file: (File, SessionId),
    pub(super) bytes: Zeroizing<Vec<u8>>,
}

use std::cell::RefCell;

impl HeldObject {
    pub(super) fn identity(&self) -> SessionId {
        self.file.1
    }

    pub(super) fn read(path: PathBuf, maximum: usize) -> Result<Self, AppError> {
        let (file, id, bytes) = read_bounded_held(&path, maximum, false)?;
        Ok(Self {
            path,
            file: (file, id),
            bytes,
        })
    }

    pub(super) fn require(&mut self) -> Result<(), AppError> {
        require_exact_held(&self.path, &mut self.file, &self.bytes)
    }
}

impl HeldLane {
    pub(super) fn read(
        root: &HeldDirectory,
        (scope, selected): (ReviewScope, SelectedLane),
        key: &ReceiverKey,
    ) -> Result<Self, AppError> {
        root.require()?;
        let directory = HeldDirectory::open(root.child(&lane_directory_name(scope)))?;
        let sealed_head =
            HeldObject::read(directory.child(&head_name(&selected.head)), MAX_HEAD_BYTES)?;
        let head = LaneHead::open(key, &sealed_head.bytes)?;
        if head_digest(&sealed_head.bytes) != selected.head || head.is_frozen() != selected.frozen {
            return Err(denied());
        }
        let mut lane = Self {
            directory,
            head,
            sealed_head,
            seen: RefCell::new(Vec::with_capacity(MAX_VIEW_FILES)),
        };
        lane.require(root)?;
        Ok(lane)
    }

    pub(super) fn page(&self, id: PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.keep(self.directory.child(&page_name(id)), MAX_INDEX_FRAME_BYTES)
    }

    pub(super) fn header(&self, digest: [u8; 32]) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.keep(
            self.directory.child(&header_name(&digest)),
            MAX_HEADER_BYTES,
        )
    }

    pub(super) fn approval(&self, digest: &[u8; 32]) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.keep(
            self.directory.child(&approval_name(digest)),
            MAX_APPROVAL_FRAME,
        )
    }

    fn keep(&self, path: PathBuf, maximum: usize) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.directory.require()?;
        let mut seen = self.seen.borrow_mut();
        if let Some(existing) = seen.iter_mut().find(|object| object.path == path) {
            existing.require()?;
            return Ok(existing.bytes.clone());
        }
        if seen.len() == MAX_VIEW_FILES {
            return Err(denied());
        }
        let mut object = HeldObject::read(path, maximum)?;
        object.require()?;
        self.directory.require()?;
        let bytes = object.bytes.clone();
        seen.push(object);
        Ok(bytes)
    }

    pub(super) fn require(&mut self, root: &HeldDirectory) -> Result<(), AppError> {
        root.require()?;
        self.directory.require()?;
        self.sealed_head.require()?;
        for file in self.seen.get_mut() {
            file.require()?;
        }
        self.directory.require()?;
        root.require()
    }
}
