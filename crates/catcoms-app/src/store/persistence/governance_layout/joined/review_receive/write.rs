//! Cover the exact immutable outputs before the manifest CAS. Recovering
//! a visible new manifest never recreates a missing selected output.
use super::{
    approval,
    index::{Indexes, MAX_PAGES},
    lane_head::LaneHead,
    manifest::{ReviewScope, SelectedLane},
    objects::{
        approval_name, fork_name, head_name, header_name, lane_directory_name, page_name,
        HeldObject,
    },
    wal::{Effect, WriteAhead},
    *,
};
use crate::store::persistence::v2_member_outbox::fs::{
    bounded_entries, create_namespace, finish_exact_write, observe, observe_directory,
    open_exact_write, remove_listed, Presence, WriteMode,
};

pub(super) const WAL_NAME: &str = "selection.pending";
pub(super) const MANIFEST_STAGE: &str = "manifest.stage";

const MAX_OUTPUTS: usize = 2 + 2 * MAX_PAGES;

struct Output<'a> {
    name: String,
    bytes: &'a [u8],
}

/// The caller already authenticated the write-ahead bytes, rederived the
/// exact effect and compared the predecessor against the same live row.
/// It continues to hold that authority through the final selector check.
pub(super) fn finish_covered_effect(
    root: &HeldDirectory,
    (scope, selected, head, first): (ReviewScope, SelectedLane, LaneHead, bool),
    current: &mut HeldManifest,
    mut wal: HeldObject,
    work: &WriteAhead<'_>,
    mut require: impl FnMut() -> Result<(), AppError>,
) -> Result<(), AppError> {
    require()?;
    root.require()?;
    wal.require()?;
    current.require_current(root)?;
    let committed = current.bytes.as_slice() == work.manifests[1];
    if !committed && current.bytes.as_slice() != work.manifests[0] {
        return Err(denied());
    }
    let lane_path = root.child(&lane_directory_name(scope));
    if observe_directory(&lane_path)? == Presence::Absent {
        if committed || !first {
            return Err(denied());
        }
        require()?;
        current.require_current(root)?;
        wal.require()?;
        create_namespace(&lane_path)?;
        root.sync()?;
    }
    let directory = HeldDirectory::open(lane_path)?;
    let mut expected = Vec::with_capacity(MAX_OUTPUTS);
    match &work.effect {
        Effect::Header { frame, paths } => {
            for path in paths {
                for (page, frame) in &path.exact {
                    expected.push(Output {
                        name: page_name(page.page),
                        bytes: frame,
                    });
                }
            }
            expected.push(Output {
                name: header_name(&Indexes::frame_digest(frame)),
                bytes: frame,
            });
        }
        Effect::Fork(frame) => {
            expected.push(Output {
                name: fork_name(&head.fork),
                bytes: frame,
            });
        }
        Effect::Approval { frame, path } => {
            for (page, frame) in &path.exact {
                expected.push(Output {
                    name: page_name(page.page),
                    bytes: frame,
                });
            }
            expected.push(Output {
                name: approval_name(&approval::digest(frame)),
                bytes: frame,
            });
        }
    }
    expected.push(Output {
        name: head_name(&selected.head),
        bytes: work.head,
    });
    if first
        && bounded_entries(&directory, expected.len())?
            .iter()
            .any(|name| {
                !expected
                    .iter()
                    .any(|output| name == std::ffi::OsStr::new(&output.name))
            })
    {
        return Err(denied());
    }
    let mut outputs = Vec::with_capacity(expected.len());
    for output in &expected {
        require()?;
        current.require_current(root)?;
        wal.require()?;
        let mut file = cover_file(&directory, &output.name, output.bytes, !committed)?;
        file.require(&directory, &output.name, output.bytes)?;
        outputs.push(file);
    }
    directory.sync()?;
    root.sync()?;

    let mut require_outputs = || {
        require()?;
        wal.require()?;
        directory.require()?;
        for (file, expected) in outputs.iter_mut().zip(&expected) {
            file.require(&directory, &expected.name, expected.bytes)?;
        }
        root.require()
    };
    let stage = root.child(MANIFEST_STAGE);
    let mut installed = if committed {
        absent(&stage)?;
        CoveredFile::existing(root, MANIFEST_NAME, work.manifests[1])?
    } else {
        let mut prepared = cover_file(root, MANIFEST_STAGE, work.manifests[1], true)?;
        root.sync()?;
        require_outputs()?;
        current.require_current(root)?;
        prepared.require(root, MANIFEST_STAGE, work.manifests[1])?;
        install_catalog_stage(&stage, &root.child(MANIFEST_NAME)).map_err(io)?;
        prepared
    };
    root.sync()?;
    require_outputs()?;
    installed.require(root, MANIFEST_NAME, work.manifests[1])?;
    #[cfg(test)]
    super::fault::cut(ReceiverFault::ManifestSelected)?;
    absent(&stage)?;
    root.require()?;
    wal.require()?;
    remove_listed(&root.child(WAL_NAME))?;
    drop(wal);
    root.sync()?;
    absent(&root.child(WAL_NAME))?;
    installed.require(root, MANIFEST_NAME, work.manifests[1])
}

pub(super) struct CoveredFile((File, SessionId));

impl CoveredFile {
    fn existing(root: &HeldDirectory, name: &str, expected: &[u8]) -> Result<Self, AppError> {
        let (file, id, bytes) = read_bounded_held(&root.child(name), expected.len(), false)?;
        if bytes.as_slice() != expected {
            return Err(denied());
        }
        Ok(Self((file, id)))
    }

    pub(super) fn require(
        &mut self,
        root: &HeldDirectory,
        name: &str,
        expected: &[u8],
    ) -> Result<(), AppError> {
        root.require()?;
        require_exact_held(&root.child(name), &mut self.0, expected)?;
        root.require()
    }

    pub(super) fn identity(&self) -> SessionId {
        self.0 .1
    }
}

pub(super) fn cover_file(
    root: &HeldDirectory,
    name: &str,
    expected: &[u8],
    allow_write: bool,
) -> Result<CoveredFile, AppError> {
    root.require()?;
    if !allow_write {
        return CoveredFile::existing(root, name, expected);
    }
    let path = root.child(name);
    let mode = if observe(&path)? == Presence::Absent {
        WriteMode::Create
    } else {
        WriteMode::Existing
    };
    let mut file = open_exact_write(&path, mode)?;
    finish_exact_write(&path, &mut file, expected)?;
    let mut result = CoveredFile(file);
    result.require(root, name, expected)?;
    Ok(result)
}
