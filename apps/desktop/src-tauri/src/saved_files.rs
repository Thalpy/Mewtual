//! The per-server saved-files list (PR #56, `docs/design-saved-files-list.md`): native records each
//! finished save of a shared file in the sealed `servers/<id>.saved`, the renderer lists, checks,
//! shows and forgets entries by their random ID, and the opt-in "Replace my older saved copy"
//! removes Mewtual's own earlier copy of a file saved again.
//!
//! Boundaries kept here:
//!
//! - **Paths stay native.** The renderer receives an entry ID, the CID, the saved file's own name,
//!   when and how large, whether it went to the Downloads folder and whether it can be checked.
//!   Folder paths and identities never cross IPC, and no command takes one.
//! - **Only native writes the list,** only after a real publication, and only for the server
//!   incarnation that saved: the write runs under the store lock and then the registry lock (the
//!   order `persist_captured` and the vault backup use) with the entry's `instance` checked, so a
//!   save racing a leave or an account switch writes nothing (leave removes the registry row before
//!   `remove_server`, and an account switch replaces every incarnation). The record names the group
//!   and this device's member id for it, so a stale record never applies to a later membership.
//! - **A record this build cannot read is never written over** (a newer build may have written
//!   it); only leaving the server removes it, and only a damaged one can be started anew by the
//!   user. Saves then simply are not recorded, and the result says so.
//! - **Bounded work.** Checks run read-only on the blocking pool with one of PR #51's two check
//!   slots and a bounded wait on the sanctioned clock; a check of the whole list is one worker that
//!   stops between entries once its wait is over. A replace holds one cleanup slot and stops at
//!   its deadline.
use super::*;
use catcoms_app::{DownloadReady, SavedFilesLoad, SavedLimitLoad};
use catcoms_storage::kept::{
    drop_first_name, replace_older_copy, saved_file_location, saved_file_presence, ExportJournal,
    FirstNameDrop, NoCandidate, ReplaceOutcome, ReplaceRefusal, ReplaceRequest, SavedFile,
    SavedFileId, SavedFiles, SavedLimit, SavedLocation, SavedPresence,
};
use private_downloads::{SavedCheck, SAVED_CHECK_WAIT};

/// How long a check of the whole list may take before the entries it did not reach are left out
/// (the renderer shows them as not checked yet).
pub(super) const LIST_CHECK_WAIT: Duration = Duration::from_secs(5);
/// Deadline of one replace, on the sanctioned clock (design review L9): hashing a large old copy on
/// a slow share must not hold the save, a cleanup slot and (on Windows) a delete-denying handle
/// without end.
pub(super) const REPLACE_DEADLINE: Duration = Duration::from_secs(120);

/// Shown with a successful save that could not be added to the list.
pub(super) const NOT_RECORDED: &str = "Mewtual could not add this file to its list of saved files.";
/// Refusal of an action while the record cannot be read.
pub(super) const LIST_UNREADABLE: &str =
    "Mewtual could not read its list of saved files on this device, so it changed nothing.";
/// An entry that is not in the list any more (cleared meanwhile, or a stale view).
pub(super) const ENTRY_GONE: &str = "This saved file is no longer listed. Open Transfers again.";
/// "Start a new list" found a record that is not damaged (any more).
pub(super) const RESET_REFUSED: &str =
    "The list of saved files is not damaged, so Mewtual left it as it is.";
/// A lower "Remember saved files" setting while the list cannot be read: lowering trims the list
/// first, and an unreadable list cannot be trimmed (nor may its read bound shrink under it).
pub(super) const LIMIT_LIST_UNREADABLE: &str =
    "Mewtual could not read its list of saved files right now, so it kept the current setting.";
/// The list changed between the preview the user confirmed and the change (a save meanwhile).
pub(super) const LIMIT_LIST_CHANGED: &str =
    "The list of saved files changed meanwhile, so Mewtual kept the current setting. Choose again.";
/// The shortened list could not be written (or not finished): the setting was not stored.
pub(super) const LIMIT_NOT_TRIMMED: &str =
    "Mewtual could not finish shortening the list of saved files, so it kept the current setting.";
/// The list was shortened, but the new setting could not be stored afterwards.
fn limit_not_stored(forgot: usize) -> String {
    let entries = if forgot == 1 {
        "the oldest saved file".to_owned()
    } else {
        format!("the {forgot} oldest saved files")
    };
    format!("Mewtual forgot {entries} from the list, but could not store the new setting.")
}

// "Replace my older saved copy" notes (design section 5). None claims a copy exists unless the
// sequence saw it.
pub(super) const REPLACED: &str = "It replaced the copy Mewtual saved before.";
pub(super) const NOT_REPLACED_ELSEWHERE: &str =
    "Mewtual did not replace the copy it saved before, because this one went to another folder.";
pub(super) const NOT_REPLACED_CHANGED: &str = "Mewtual did not replace the copy it saved before, because that copy was changed or is not the one Mewtual saved.";
pub(super) const NOT_REPLACED_NAMES: &str =
    "Mewtual did not replace the copy it saved before, because that file has another name too.";
pub(super) const NOT_REPLACED_READ_ONLY: &str =
    "Mewtual did not replace the copy it saved before, because that copy is read-only.";
pub(super) const NOT_REPLACED_SHARED: &str =
    "Mewtual did not replace the copy it saved before, because other accounts may change it there.";
pub(super) const NOT_REPLACED_UNCHECKED: &str =
    "Mewtual did not replace the copy it saved before, because it could not be checked right now.";
pub(super) const NOT_REPLACED_CANNOT_CHECK: &str = "Mewtual did not replace the copy it saved before, because it cannot check files in that folder.";
/// Said when a replace was asked for but none could be considered (a save that could not be
/// listed, a kept record, a recovered save). It claims no earlier copy exists (adversarial review
/// L3).
pub(super) const NOT_REPLACED_THIS_TIME: &str =
    "Mewtual did not replace any earlier copy this time.";
/// The new copy was not there (or not the same size) when the replace started, or had gone right
/// before the delete (adversarial review H1).
pub(super) const NOT_REPLACED_NEW_GONE: &str = "Mewtual did not replace the copy it saved before, because the new copy was moved, changed or deleted meanwhile.";
/// The old copy went, and the new copy was then seen gone from its own name too.
pub(super) const REMOVED_NEW_GONE: &str =
    "Mewtual removed the copy it saved before, but the new copy was moved or deleted meanwhile.";
/// The old copy went, the new copy could not take its name, and its own name could not be read
/// (re-review LOW 1): no name is claimed.
pub(super) const REMOVED_NOT_MOVED: &str =
    "Mewtual removed the copy it saved before, but could not give this copy its name.";
/// After the link, neither name could be shown to hold the copy (re-review MEDIUM): no name is
/// claimed.
pub(super) const NAME_UNCONFIRMED: &str =
    "Mewtual could not confirm which name this saved file has now.";
pub(super) const REPLACE_STRANDED: &str = "Mewtual could not finish replacing the copy it saved before. It may be in that folder under a name starting with mewtual-removing-.";

/// "Mewtual removed the copy it saved before, but could not give this copy its name, so it keeps
/// the name <name>."
fn kept_own_name(name: &str) -> String {
    format!(
        "Mewtual removed the copy it saved before, but could not give this copy its name, so it keeps the name {name}."
    )
}
/// "The saved file also kept the name <name>. ..."
fn second_name_left(name: &str) -> String {
    format!(
        "The saved file also kept the name {name}. Both names are the same file, so deleting either one keeps the other."
    )
}

fn refusal_note(refusal: ReplaceRefusal) -> &'static str {
    match refusal {
        ReplaceRefusal::OtherFolder => NOT_REPLACED_ELSEWHERE,
        ReplaceRefusal::Changed => NOT_REPLACED_CHANGED,
        // Adversarial review L2: a folder that moved says nothing about the copy itself.
        ReplaceRefusal::FolderChanged => NOT_REPLACED_UNCHECKED,
        ReplaceRefusal::NewGone => NOT_REPLACED_NEW_GONE,
        ReplaceRefusal::MoreNames => NOT_REPLACED_NAMES,
        ReplaceRefusal::ReadOnly => NOT_REPLACED_READ_ONLY,
        ReplaceRefusal::Shared => NOT_REPLACED_SHARED,
        ReplaceRefusal::NoIdentity | ReplaceRefusal::Reparse => NOT_REPLACED_CANNOT_CHECK,
        ReplaceRefusal::Unchecked | ReplaceRefusal::Stopped => NOT_REPLACED_UNCHECKED,
    }
}

/// One entry as the renderer sees it. Never a path, folder or identity.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct UiSavedFile {
    id: String,
    cid: String,
    /// The saved file's own name (the leaf, for example `report (1).pdf`).
    name: String,
    saved_at: u64,
    size: u64,
    /// `"downloads"` when the recorded folder is exactly the resolved Downloads folder, `"other"`
    /// when it is not, `None` when no Downloads folder resolves here. Never a folder name.
    saved_to: Option<&'static str>,
    /// The entry has identities from this platform, so native can check, show and replace it.
    checkable: bool,
}

/// The whole list for the renderer, newest first. `state` is `"ok"`, or why it cannot be read:
/// `"newer"` (a newer Mewtual wrote it), `"damaged"` (it can be started anew) or `"unavailable"`
/// (it could not be read right now). Files only with `"ok"`.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct UiSavedFiles {
    state: &'static str,
    files: Vec<UiSavedFile>,
}

/// The "Remember saved files" setting for the renderer (Server settings, Storage).
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct UiSavedFilesLimit {
    /// `"ok"`, or `"unreadable"` when the stored setting cannot be read: no save is listed until
    /// the user chooses one again.
    state: &'static str,
    /// The setting in force (the default when none was chosen); `None` while unreadable.
    limit: Option<u32>,
    /// Entries the list holds now; `None` while the list cannot be read.
    listed: Option<usize>,
    min: u32,
    max: u32,
    default: u32,
}

/// What choosing a setting would do (`applied: false`) or did.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct UiSavedLimitChange {
    /// The setting, clamped into the range.
    limit: u32,
    /// How many of the oldest list entries it forgets (or forgot). Files are never touched.
    forgets: usize,
    applied: bool,
}

/// The setting as stored, with the list's size. Read-only.
pub(super) fn limit_view(store: &ServerStore, server: u64, owner: &[u8]) -> UiSavedFilesLimit {
    let limit = store.load_saved_files_limit(server, owner).limit();
    UiSavedFilesLimit {
        state: if limit.is_some() { "ok" } else { "unreadable" },
        limit: limit.map(SavedLimit::get),
        listed: load_list(store, server, owner)
            .ok()
            .map(|list| list.entries().len()),
        min: SavedLimit::MIN,
        max: SavedLimit::MAX,
        default: SavedLimit::DEFAULT.get(),
    }
}

/// Whether `limit` may be stored while the list cannot be read (a newer build's record, or an I/O
/// error right now), so its length is unknown and nothing could be shown before confirming. Only
/// a choice that keeps every entry the current setting keeps: at least the setting in force (the
/// default when none was chosen), or the largest when the setting itself cannot be read. A lower
/// one would let the next save, once the list reads again, forget entries without warning, and
/// could narrow the read bound under a list that then reads as damaged (adversarial review of the
/// setting, MEDIUM-1).
pub(super) fn unknown_list_allows(current: SavedLimitLoad, limit: SavedLimit) -> bool {
    limit == SavedLimit::LARGEST || current.limit().is_some_and(|now| limit >= now)
}

/// Choose the setting: `requested` is clamped into the range. With `apply` `None` nothing changes
/// and the answer says how many of the oldest entries it would forget (the preview the page shows
/// before the user confirms). With `apply: Some(expected)` it changes, but only while it forgets
/// exactly `expected` entries: saves made while the user decided may have changed that, and then
/// nothing changes and the page asks again (review LOW-1).
///
/// Lowering forgets the oldest entries from the list (never a file): the trimmed list is written
/// first and the setting second, so the list always fits what the stored setting lets a reader
/// read, and a failure in between leaves the earlier, larger setting over a shorter list (the
/// error then says how many entries went, review LOW-2). While the list cannot be read only a
/// choice that keeps every entry is accepted ([`unknown_list_allows`]); a damaged list is left
/// alone. Raising keeps every entry and writes only the setting.
pub(super) fn change_limit(
    store: &ServerStore,
    server: u64,
    owner: &[u8],
    requested: u64,
    apply: Option<u64>,
) -> Result<UiSavedLimitChange, String> {
    let limit = SavedLimit::clamped(requested);
    let current = store.load_saved_files_limit(server, owner);
    let mut forgets = 0;
    match store.load_saved_files(server, owner) {
        SavedFilesLoad::Found(mut list) => {
            forgets = list.trimmed_by(limit);
            if let Some(expected) = apply {
                if forgets as u64 != expected {
                    return Err(LIMIT_LIST_CHANGED.into());
                }
                if forgets > 0 {
                    list.trim(limit);
                    store
                        .save_saved_files(server, owner, &list, &mut OsCryptoRng)
                        .map_err(|error| format!("{LIMIT_NOT_TRIMMED} {error}"))?;
                }
            }
        }
        // Nothing to trim: no list yet, or a damaged one (which only "Start a new list" or
        // leaving replaces; its size says nothing).
        SavedFilesLoad::Missing | SavedFilesLoad::Damaged => {}
        SavedFilesLoad::Newer | SavedFilesLoad::Unavailable => {
            if !unknown_list_allows(current, limit) {
                return Err(LIMIT_LIST_UNREADABLE.into());
            }
        }
    }
    if apply.is_some() && current != SavedLimitLoad::Found(limit) {
        store
            .save_saved_files_limit(server, owner, limit, &mut OsCryptoRng)
            .map_err(|error| match forgets {
                0 => error.to_string(),
                n => format!("{} {error}", limit_not_stored(n)),
            })?;
    }
    Ok(UiSavedLimitChange {
        limit: limit.get(),
        forgets,
        applied: apply.is_some(),
    })
}

/// One answer of a whole-list check, for the entry and name it was made for.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct UiSavedFileCheck {
    id: String,
    name: String,
    check: SavedCheck,
}

pub(super) fn project(list: &SavedFiles, downloads: Option<&Path>) -> UiSavedFiles {
    let mut files: Vec<UiSavedFile> = list
        .entries()
        .iter()
        .map(|entry| UiSavedFile {
            id: entry.id.to_hex(),
            cid: entry.cid.to_hex(),
            name: entry.location.leaf().to_owned(),
            saved_at: entry.saved_at_ms,
            size: entry.size,
            saved_to: downloads.map(|folder| {
                if entry.location.is_in_folder(folder) {
                    "downloads"
                } else {
                    "other"
                }
            }),
            checkable: entry.location.checkable(),
        })
        .collect();
    files.reverse();
    UiSavedFiles { state: "ok", files }
}

fn check_word(presence: SavedPresence) -> SavedCheck {
    match presence {
        SavedPresence::Ours => SavedCheck::Present,
        SavedPresence::Missing => SavedCheck::Missing,
        SavedPresence::Replaced => SavedCheck::Replaced,
        SavedPresence::Unknown => SavedCheck::Unknown,
    }
}

/// The list as loaded, for the closures that change it.
pub(super) enum Loaded<'a> {
    Readable(&'a mut SavedFiles),
    /// `"newer"`, `"damaged"` or `"unavailable"`: never written over.
    Unreadable(&'static str),
}

/// Run `act` with the store and server `server`'s owner binding under the store lock and then the
/// registry lock, for incarnation `instance` only. `None`: the server is not that incarnation any
/// more (or the vault is not mounted), so nothing was read or written. The session commit guard
/// is never taken in here.
async fn with_owner<T>(
    state: &AppState,
    server: u64,
    instance: u64,
    act: impl FnOnce(&ServerStore, &[u8]) -> T,
) -> Option<T> {
    let guard = state.store.lock().await;
    let store = guard.as_ref()?;
    let servers = state.servers.lock().await;
    let entry = servers
        .get(&server)
        .filter(|entry| entry.instance == instance)?;
    // The group, then this device's member id in it: a later membership (leave and join again)
    // has another member id, so a record that survived a leave never applies to it.
    let owner = [entry.group_id.as_slice(), entry.device_id.as_bytes()].concat();
    let value = act(store, &owner);
    drop(servers);
    Some(value)
}

/// The list as loaded, or why it cannot be written over.
fn load_list(store: &ServerStore, server: u64, owner: &[u8]) -> Result<SavedFiles, &'static str> {
    match store.load_saved_files(server, owner) {
        SavedFilesLoad::Found(list) => Ok(list),
        SavedFilesLoad::Missing => Ok(SavedFiles::new(owner)),
        SavedFilesLoad::Newer => Err("newer"),
        SavedFilesLoad::Damaged => Err("damaged"),
        SavedFilesLoad::Unavailable => Err("unavailable"),
    }
}

/// Run `change` on server `server`'s list (see [`with_owner`] for the locks and the incarnation),
/// with the "Remember saved files" setting in force (`None` when it cannot be read), and write
/// the result if `change` asks to and the list was readable. `Ok(None)`: nothing was read or
/// written.
async fn with_list_at<T>(
    state: &AppState,
    server: u64,
    instance: u64,
    change: impl FnOnce(Loaded<'_>, Option<SavedLimit>) -> (T, bool),
) -> Result<Option<T>, String> {
    with_owner(state, server, instance, |store, owner| {
        let limit = store.load_saved_files_limit(server, owner).limit();
        let mut list = load_list(store, server, owner);
        let (value, write) = match &mut list {
            Ok(list) => change(Loaded::Readable(list), limit),
            Err(why) => change(Loaded::Unreadable(why), limit),
        };
        if write {
            if let Ok(list) = &list {
                store
                    .save_saved_files(server, owner, list, &mut OsCryptoRng)
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(value)
    })
    .await
    .transpose()
}

/// [`with_list_at`] for changes that never grow the list (forget, rename, lookups), which need no
/// setting.
async fn with_list<T>(
    state: &AppState,
    server: u64,
    instance: u64,
    change: impl FnOnce(Loaded<'_>) -> (T, bool),
) -> Result<Option<T>, String> {
    with_list_at(state, server, instance, |list, _| change(list)).await
}

/// What a save published, for its entry.
pub(super) struct PublishedSave {
    pub(super) cid: catcoms_storage::Cid,
    pub(super) size: u64,
    pub(super) location: SavedLocation,
}

/// A save that asked for "Replace my older saved copy". `claim` is `None` when another save of the
/// same file is replacing right now (this one then makes a normal copy). `started_from` is native's
/// sanitized Downloads name for a fresh one-click save (`Exported::started_from`), `None` for a
/// name chosen in Save as and for a reopened or recovered journal.
pub(super) struct ReplaceAsk {
    pub(super) claim: Option<ReplaceClaim>,
    pub(super) started_from: Option<String>,
}

/// What a replace will act on, decided from the list under the same lock that recorded the save.
pub(super) struct ReplacePlan {
    pub(super) request: ReplaceRequest,
    /// This save's own entry (renamed when the new copy takes the old name).
    pub(super) entry: SavedFileId,
    _claim: ReplaceClaim,
}
impl std::fmt::Debug for ReplacePlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReplacePlan { .. }")
    }
}

/// What recording means for a replace.
#[derive(Debug)]
pub(super) enum ReplaceChoice {
    NotAsked,
    /// Asked, but it cannot run; the note says why (`None`: nothing to say).
    Skip(Option<&'static str>),
    Run(Box<ReplacePlan>),
}

/// What recording a save did.
#[derive(Debug)]
pub(super) enum Recorded {
    /// The entry is durably in the list. Only then may a replace run (design review M4).
    Yes { replace: ReplaceChoice },
    /// Nothing was written: the server was left or replaced meanwhile (say nothing; that server's
    /// records are gone or going), or the list could not be read or written (tell the user).
    No { tell: bool },
}
impl Recorded {
    /// The note for the save result, when the user should hear that the save is not listed.
    pub(super) fn note(&self) -> Option<&'static str> {
        matches!(self, Self::No { tell: true }).then_some(NOT_RECORDED)
    }
}

/// Record a published save (PR #56). Never fails the save: every refusal is a [`Recorded::No`].
/// With `replace`, the candidate for "Replace my older saved copy" is read from the same locked
/// list right before this save's entry is added, so it is exactly what was listed when it finished.
pub(super) async fn record_save(
    state: &AppState,
    server: u64,
    instance: u64,
    save: PublishedSave,
    replace: Option<ReplaceAsk>,
) -> Recorded {
    let entry = SavedFile {
        id: SavedFileId::fresh(&mut OsCryptoRng),
        cid: save.cid,
        size: save.size,
        saved_at_ms: SystemClock.now_ms(),
        location: save.location,
    };
    let written = with_list_at(state, server, instance, move |list, limit| {
        // An unreadable setting records nothing: recording trims to the setting, and guessing it
        // could forget entries the user chose to keep.
        let (Loaded::Readable(list), Some(limit)) = (list, limit) else {
            return (None, false);
        };
        let choice = match replace {
            None => ReplaceChoice::NotAsked,
            Some(ask) => match (list.replace_candidate(&entry), ask.claim) {
                (Err(NoCandidate::NoneSaved), _) => ReplaceChoice::Skip(None),
                (Err(NoCandidate::Elsewhere), _) => {
                    ReplaceChoice::Skip(Some(NOT_REPLACED_ELSEWHERE))
                }
                (Err(NoCandidate::Unprovable), _) => {
                    ReplaceChoice::Skip(Some(NOT_REPLACED_CANNOT_CHECK))
                }
                (Ok(_), None) => ReplaceChoice::Skip(Some(NOT_REPLACED_UNCHECKED)),
                (Ok(old), Some(claim)) => ReplaceChoice::Run(Box::new(ReplacePlan {
                    request: ReplaceRequest {
                        old,
                        new: entry.location.clone(),
                        started_from: ask.started_from,
                    },
                    entry: entry.id,
                    _claim: claim,
                })),
            },
        };
        if list.record(entry, limit) {
            (Some(choice), true)
        } else {
            (None, false)
        }
    })
    .await;
    match written {
        Ok(Some(Some(replace))) => Recorded::Yes { replace },
        Ok(Some(None)) => Recorded::No { tell: true },
        Ok(None) => Recorded::No { tell: false },
        Err(error) => {
            tracing::warn!(target: "catcoms_app", server, %error, "VAULT.SAVED_FILES.WRITE_FAILED");
            Recorded::No { tell: true }
        }
    }
}

/// Forget entry `id` for incarnation `instance`. Returns whether it was there and is gone now.
pub(super) async fn forget_entry(
    state: &AppState,
    server: u64,
    instance: u64,
    id: SavedFileId,
) -> bool {
    matches!(
        with_list(state, server, instance, |list| match list {
            Loaded::Readable(list) => {
                let changed = list.forget(id);
                (changed, changed)
            }
            Loaded::Unreadable(_) => (false, false),
        })
        .await,
        Ok(Some(true))
    )
}

/// After "Replace my older saved copy" gave the new copy its old name: change entry `id`'s name
/// from `from` to `to`, only while it still names `from`. Returns whether it changed durably.
pub(super) async fn rename_entry(
    state: &AppState,
    server: u64,
    instance: u64,
    id: SavedFileId,
    from: &str,
    to: &str,
) -> bool {
    matches!(
        with_list(state, server, instance, |list| match list {
            Loaded::Readable(list) => {
                let changed = list.rename(id, from, to);
                (changed, changed)
            }
            Loaded::Unreadable(_) => (false, false),
        })
        .await,
        Ok(Some(true))
    )
}

/// Clear of a Saved private row (PR #51) also forgets the list entry of exactly that saved file
/// (same folder and file identity), so the row goes away instead of a second row appearing. A
/// different copy of the same file stays listed. Called only after the record was released.
pub(super) async fn forget_journal(
    state: &AppState,
    server: u64,
    instance: u64,
    journal: &ExportJournal,
) {
    let _ = with_list(state, server, instance, |list| match list {
        Loaded::Readable(list) => {
            let changed = list.forget_journal(journal);
            ((), changed)
        }
        Loaded::Unreadable(_) => ((), false),
    })
    .await;
}

/// The list entry each Saved private row stands for (its published journal's identity), for the
/// renderer's merge: that entry is not drawn as a second row. Read-only.
pub(super) async fn entries_for_journals(
    state: &AppState,
    server: u64,
    instance: u64,
    journals: &[ExportJournal],
) -> Vec<Option<String>> {
    with_list(state, server, instance, |list| match list {
        Loaded::Readable(list) => (
            journals
                .iter()
                .map(|journal| list.for_journal(journal).map(|entry| entry.id.to_hex()))
                .collect(),
            false,
        ),
        Loaded::Unreadable(_) => (vec![None; journals.len()], false),
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| vec![None; journals.len()])
}

/// One "Replace my older saved copy" at a time per (server, file): a second save of the same file
/// while one replaces makes a normal copy instead of racing it.
pub(super) struct ReplaceClaim(u64, [u8; 32]);
/// Held (server, file CID) pairs.
type ReplaceClaims = std::sync::Mutex<std::collections::HashSet<(u64, [u8; 32])>>;
fn replace_claims() -> &'static ReplaceClaims {
    static CLAIMS: std::sync::OnceLock<ReplaceClaims> = std::sync::OnceLock::new();
    CLAIMS.get_or_init(Default::default)
}
impl ReplaceClaim {
    pub(super) fn try_acquire(server: u64, cid: [u8; 32]) -> Option<Self> {
        // The guard must be gone before a claim value exists: a claim's drop takes the same lock,
        // and a claim built for a busy key (`then_some` builds its argument eagerly) would, on
        // drop, both deadlock here and release the other claimant's key.
        let inserted = replace_claims()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((server, cid));
        if inserted {
            Some(Self(server, cid))
        } else {
            None
        }
    }
}
impl Drop for ReplaceClaim {
    fn drop(&mut self) {
        replace_claims()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&(self.0, self.1));
    }
}

/// The effects of one replace, behind a seam so the real sequence runs in tests.
#[async_trait::async_trait]
pub(super) trait ReplacePorts: Send + Sync {
    /// Steps 1 to 4 on the blocking pool with a cleanup slot and the invocation's `keep_going`.
    async fn replace(&self, request: ReplaceRequest) -> ReplaceOutcome;
    async fn forget(&self, id: SavedFileId) -> bool;
    async fn rename(&self, id: SavedFileId, from: String, to: String) -> bool;
    /// Step 6 on the blocking pool: what the first and old names hold afterwards.
    async fn drop_first(&self, new: SavedLocation, first: String, old: String) -> FirstNameDrop;
    /// The invocation and the server incarnation are still current.
    fn live(&self) -> bool;
}

/// What a replace leaves for the save result.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Replaced {
    /// The new copy's name now, when it took the old name.
    pub(super) renamed_to: Option<String>,
    pub(super) notes: Vec<String>,
}

/// "Replace my older saved copy" after the save was recorded and its private record released.
/// Each list change is made only after the effect it describes, and step 6 only after step 5
/// succeeded (design review M4); every stopping point is a row of the design's crash table.
pub(super) async fn replace_sequence(ports: &impl ReplacePorts, plan: ReplacePlan) -> Replaced {
    let old = plan.request.old.id;
    let first = plan.request.new.leaf().to_owned();
    let new = plan.request.new.clone();
    let mut replaced = Replaced::default();
    match ports.replace(plan.request).await {
        ReplaceOutcome::Missing | ReplaceOutcome::AlreadyNew => {
            // The old copy is not at its name (or the name holds the new copy): its entry is
            // stale. Nothing was replaced, so nothing is said.
            ports.forget(old).await;
        }
        ReplaceOutcome::Removed => {
            ports.forget(old).await;
            replaced.notes.push(REPLACED.into());
        }
        ReplaceOutcome::RemovedNotMoved { still_named } => {
            ports.forget(old).await;
            replaced.notes.push(match still_named {
                Some(true) => kept_own_name(&first),
                Some(false) => REMOVED_NEW_GONE.into(),
                None => REMOVED_NOT_MOVED.into(),
            });
        }
        ReplaceOutcome::Linked { old_leaf } => {
            ports.forget(old).await;
            replaced.notes.push(REPLACED.into());
            let renamed = ports.live()
                && ports
                    .rename(plan.entry, first.clone(), old_leaf.clone())
                    .await;
            if !renamed {
                // Step 4 just saw both names hold the copy, and nothing has been removed.
                replaced.notes.push(second_name_left(&first));
            } else if !ports.live() {
                // Step 6 never ran: both names are left as step 4 saw them.
                replaced.renamed_to = Some(old_leaf.clone());
                replaced.notes.push(second_name_left(&first));
            } else {
                // Re-review MEDIUM: say only what step 6 saw afterwards, so the result never sends
                // the user to a name that went (deleting the other one would lose the only copy).
                match ports.drop_first(new, first.clone(), old_leaf.clone()).await {
                    FirstNameDrop::Removed | FirstNameDrop::OnlyOld => {
                        replaced.renamed_to = Some(old_leaf);
                    }
                    FirstNameDrop::BothNames => {
                        replaced.renamed_to = Some(old_leaf);
                        replaced.notes.push(second_name_left(&first));
                    }
                    FirstNameDrop::OnlyFirst => {
                        // The copy is only at its first name: the list follows it back (a failed
                        // move back leaves an entry that reads "missing" or "replaced", never a
                        // wrong claim), and the result points at the first name.
                        if ports.live() {
                            ports.rename(plan.entry, old_leaf, first.clone()).await;
                        }
                        replaced.notes.push(kept_own_name(&first));
                    }
                    FirstNameDrop::Unknown => {
                        replaced.renamed_to = Some(old_leaf);
                        replaced.notes.push(NAME_UNCONFIRMED.into());
                    }
                }
            }
        }
        ReplaceOutcome::Refused(refusal) => replaced.notes.push(refusal_note(refusal).into()),
        ReplaceOutcome::Stranded => replaced.notes.push(REPLACE_STRANDED.into()),
    }
    replaced
}

/// Production replace effects for one save invocation.
pub(super) struct ActorReplace<'a> {
    pub(super) state: &'a AppState,
    pub(super) server: u64,
    pub(super) instance: u64,
    pub(super) origin: RequestCancellation,
    pub(super) ready: DownloadReady,
    /// The cleanup slot of the whole sequence. Each blocking step holds a clone, so a step that
    /// outlives a dropped invocation keeps the slot until it really ends (as PR #51's deleting
    /// workers do with their claim).
    pub(super) slot: Arc<tokio::sync::OwnedSemaphorePermit>,
    /// Monotonic deadline (sanctioned clock) of the whole sequence.
    pub(super) deadline: u64,
}
impl ActorReplace<'_> {
    fn keep_going(
        origin: &RequestCancellation,
        ready: &DownloadReady,
        deadline: u64,
    ) -> impl Fn() -> bool {
        let (origin, ready) = (origin.clone(), ready.clone());
        move || {
            !origin.is_cancelled()
                && ready.invocation_current()
                && SystemClock.monotonic_ms() < deadline
        }
    }
}
#[async_trait::async_trait]
impl ReplacePorts for ActorReplace<'_> {
    async fn replace(&self, request: ReplaceRequest) -> ReplaceOutcome {
        let keep_going = Self::keep_going(&self.origin, &self.ready, self.deadline);
        let slot = self.slot.clone();
        private_downloads::blocking(move || {
            let _held = slot;
            replace_older_copy(&request, &keep_going, &mut OsCryptoRng)
        })
        .await
        .unwrap_or(ReplaceOutcome::Refused(ReplaceRefusal::Unchecked))
    }
    async fn forget(&self, id: SavedFileId) -> bool {
        forget_entry(self.state, self.server, self.instance, id).await
    }
    async fn rename(&self, id: SavedFileId, from: String, to: String) -> bool {
        rename_entry(self.state, self.server, self.instance, id, &from, &to).await
    }
    async fn drop_first(&self, new: SavedLocation, first: String, old: String) -> FirstNameDrop {
        let slot = self.slot.clone();
        private_downloads::blocking(move || {
            let _held = slot;
            drop_first_name(&new, &first, &old, &mut OsCryptoRng)
        })
        .await
        .unwrap_or(FirstNameDrop::Unknown)
    }
    fn live(&self) -> bool {
        Self::keep_going(&self.origin, &self.ready, self.deadline)()
    }
}

/// Run a replace plan in production: take a cleanup slot (busy means "could not be checked right
/// now"), bound the whole sequence by [`REPLACE_DEADLINE`], and keep the slot until it ends.
pub(super) async fn run_replace(
    state: &AppState,
    server: u64,
    instance: u64,
    origin: &RequestCancellation,
    ready: &DownloadReady,
    plan: ReplacePlan,
) -> Replaced {
    let Ok(slot) = private_downloads::cleanup_slots().try_acquire_owned() else {
        return Replaced {
            renamed_to: None,
            notes: vec![NOT_REPLACED_UNCHECKED.into()],
        };
    };
    let ports = ActorReplace {
        state,
        server,
        instance,
        origin: origin.clone(),
        ready: ready.clone(),
        slot: Arc::new(slot),
        deadline: SystemClock
            .monotonic_ms()
            .saturating_add(REPLACE_DEADLINE.as_millis() as u64),
    };
    replace_sequence(&ports, plan).await
}

/// Whether the invocation's UI session and server entry are still the ones that asked.
async fn still_current(state: &AppState, server: u64, instance: u64, generation: u64) -> bool {
    let session = keep_session::invocation_generation(state)
        .await
        .is_ok_and(|now| now == generation);
    let entry = actor_instance_of(state, server)
        .await
        .is_ok_and(|(_, now)| now == instance);
    session && entry
}

/// Read the list for one server incarnation: `Err(why)` inside when it cannot be read.
async fn read_list(
    state: &AppState,
    server: u64,
    instance: u64,
) -> Result<Result<SavedFiles, &'static str>, String> {
    with_list(state, server, instance, |list| match list {
        Loaded::Readable(list) => (Ok(list.clone()), false),
        Loaded::Unreadable(why) => (Err(why), false),
    })
    .await?
    .ok_or_else(|| "server changed".to_string())
}

/// This server's saved files on this device, newest first (display data only).
#[tauri::command]
pub(super) async fn get_saved_files(
    app: AppHandle,
    state: State<'_, AppState>,
    server: u64,
) -> Result<UiSavedFiles, String> {
    let generation = keep_session::invocation_generation(&state).await?;
    let (_, instance) = actor_instance_of(&state, server).await?;
    let list = read_list(&state, server, instance).await?;
    let downloads = app.path().download_dir().ok();
    if !still_current(&state, server, instance, generation).await {
        return Err("the UI session changed while the operation was in progress".into());
    }
    Ok(match list {
        Ok(list) => project(&list, downloads.as_deref()),
        Err(why) => UiSavedFiles {
            state: why,
            files: Vec::new(),
        },
    })
}

/// The effects of a whole-list check and of Show in folder, behind a seam so the real sequences
/// run in tests.
#[async_trait::async_trait]
pub(super) trait SavedListPorts: Send + Sync {
    /// The list now, `None` when it cannot be read.
    async fn list(&self) -> Result<Option<SavedFiles>, String>;
    /// Read-only checks of `entries` in order, on one blocking worker, stopping between entries
    /// once `stop` is set; each answer is pushed into `found` as it is made.
    async fn check_all(
        &self,
        entries: Vec<SavedFile>,
        stop: Arc<std::sync::atomic::AtomicBool>,
        found: Arc<std::sync::Mutex<Vec<(SavedFileId, SavedPresence)>>>,
    );
    /// The saved file's path only while the read-only check proves it is the saved file.
    async fn locate(&self, entry: SavedFile) -> Result<PathBuf, SavedPresence>;
    /// Sanctioned-clock wait bounding the checks.
    async fn wait(&self, delay: Duration);
    /// The UI session and server entry are still the ones that asked.
    async fn current(&self) -> bool;
    /// Show `path` selected in the system file manager. Never opens or runs the file.
    fn reveal(&self, path: &Path) -> Result<(), String>;
}

/// Check every checkable entry read-only, newest first, for at most [`LIST_CHECK_WAIT`]. Answers
/// are returned only for entries that are unchanged (same ID, name and location) in a re-read of
/// the list afterwards, and only while the session is still the one that asked. Entries not reached
/// are left out (the renderer shows them as not checked yet, never as "could not check").
pub(super) async fn check_list_sequence(
    ports: &impl SavedListPorts,
) -> Result<Vec<UiSavedFileCheck>, String> {
    let Some(before) = ports.list().await? else {
        return Err(LIST_UNREADABLE.into());
    };
    let entries: Vec<SavedFile> = before
        .entries()
        .iter()
        .rev()
        .filter(|entry| entry.location.checkable())
        .cloned()
        .collect();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let found = Arc::new(std::sync::Mutex::new(Vec::new()));
    tokio::select! {
        biased;
        () = ports.check_all(entries, stop.clone(), found.clone()) => {}
        () = ports.wait(LIST_CHECK_WAIT) => {}
    }
    stop.store(true, std::sync::atomic::Ordering::Release);
    let answers = std::mem::take(&mut *found.lock().unwrap_or_else(|e| e.into_inner()));
    let Some(after) = ports.list().await? else {
        return Err(LIST_UNREADABLE.into());
    };
    if !ports.current().await {
        return Err("The download is no longer active.".into());
    }
    Ok(answers
        .into_iter()
        .filter_map(|(id, presence)| {
            let was = before.get(id)?;
            let now = after.get(id)?;
            (was == now).then(|| UiSavedFileCheck {
                id: id.to_hex(),
                name: now.location.leaf().to_owned(),
                check: check_word(presence),
            })
        })
        .collect())
}

/// The production checker's loop: read-only checks in order, stopping between entries once
/// `stop` is set (the bounded wait ended).
pub(super) fn check_entries(
    entries: Vec<SavedFile>,
    stop: &std::sync::atomic::AtomicBool,
    found: &std::sync::Mutex<Vec<(SavedFileId, SavedPresence)>>,
) {
    for entry in entries {
        if stop.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        let presence = saved_file_presence(&entry);
        found
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((entry.id, presence));
    }
}

/// "Show in folder" for an entry: PR #51's `reveal_saved_sequence` over the list. The same
/// bounded read-only identity check, the entry re-read and compared, the session rechecked, then
/// the file manager shows the file selected. Only success or a fixed refusal comes back.
pub(super) async fn reveal_entry_sequence(
    ports: &impl SavedListPorts,
    id: SavedFileId,
) -> Result<(), String> {
    let Some(list) = ports.list().await? else {
        return Err(LIST_UNREADABLE.into());
    };
    let Some(entry) = list.get(id).cloned() else {
        return Err(ENTRY_GONE.into());
    };
    let located = tokio::select! {
        biased;
        located = ports.locate(entry.clone()) => Some(located),
        () = ports.wait(SAVED_CHECK_WAIT) => None,
    };
    match ports.list().await? {
        Some(again) if again.get(id) == Some(&entry) => {}
        _ => return Err(ENTRY_GONE.into()),
    }
    if !ports.current().await {
        return Err("The download is no longer active.".into());
    }
    match located {
        Some(Ok(path)) => ports
            .reveal(&path)
            .map_err(|_| private_downloads::REVEAL_FAILED.into()),
        Some(Err(SavedPresence::Missing)) => Err(private_downloads::REVEAL_MISSING.into()),
        Some(Err(SavedPresence::Replaced)) => Err(private_downloads::REVEAL_REPLACED.into()),
        Some(Err(SavedPresence::Ours | SavedPresence::Unknown)) | None => {
            Err(private_downloads::REVEAL_UNCHECKED.into())
        }
    }
}

/// Production effects for one server incarnation and UI generation.
pub(super) struct ActorSavedList<'a> {
    pub(super) state: &'a AppState,
    pub(super) server: u64,
    pub(super) instance: u64,
    pub(super) generation: u64,
}
#[async_trait::async_trait]
impl SavedListPorts for ActorSavedList<'_> {
    async fn list(&self) -> Result<Option<SavedFiles>, String> {
        Ok(read_list(self.state, self.server, self.instance)
            .await?
            .ok())
    }
    async fn check_all(
        &self,
        entries: Vec<SavedFile>,
        stop: Arc<std::sync::atomic::AtomicBool>,
        found: Arc<std::sync::Mutex<Vec<(SavedFileId, SavedPresence)>>>,
    ) {
        // One worker for the whole list, holding one of the two check slots: a renderer asking
        // repeatedly can pin at most those two threads on a stalled share.
        let Ok(slot) = private_downloads::check_slots().acquire_owned().await else {
            return;
        };
        let _ = private_downloads::blocking(move || {
            let _held = slot;
            check_entries(entries, &stop, &found);
        })
        .await;
    }
    async fn locate(&self, entry: SavedFile) -> Result<PathBuf, SavedPresence> {
        let Ok(slot) = private_downloads::check_slots().acquire_owned().await else {
            return Err(SavedPresence::Unknown);
        };
        private_downloads::blocking(move || {
            let _held = slot;
            saved_file_location(&entry)
        })
        .await
        .unwrap_or(Err(SavedPresence::Unknown))
    }
    async fn wait(&self, delay: Duration) {
        SystemClock.sleep(delay).await;
    }
    async fn current(&self) -> bool {
        still_current(self.state, self.server, self.instance, self.generation).await
    }
    fn reveal(&self, path: &Path) -> Result<(), String> {
        reveal_path(path)
    }
}

/// Check every saved file of this server read-only (bounded). Entries not reached in time are
/// left out of the answer.
#[tauri::command]
pub(super) async fn check_saved_files(
    state: State<'_, AppState>,
    server: u64,
) -> Result<Vec<UiSavedFileCheck>, String> {
    let generation = keep_session::invocation_generation(&state).await?;
    let (_, instance) = actor_instance_of(&state, server).await?;
    check_list_sequence(&ActorSavedList {
        state: &state,
        server,
        instance,
        generation,
    })
    .await
}

fn entry_id(text: &str) -> Result<SavedFileId, String> {
    SavedFileId::from_hex(text).ok_or_else(|| "Invalid saved file ID.".to_string())
}

/// Show a saved file in the system file manager, selected, after the read-only identity check.
#[tauri::command]
pub(super) async fn reveal_saved_file(
    state: State<'_, AppState>,
    server: u64,
    entry: String,
) -> Result<(), String> {
    let generation = keep_session::invocation_generation(&state).await?;
    let id = entry_id(&entry)?;
    let (_, instance) = actor_instance_of(&state, server).await?;
    reveal_entry_sequence(
        &ActorSavedList {
            state: &state,
            server,
            instance,
            generation,
        },
        id,
    )
    .await
}

/// Forget one saved file (Clear). The file itself is never touched; nothing is written when the
/// entry is already gone.
#[tauri::command]
pub(super) async fn forget_saved_file(
    state: State<'_, AppState>,
    server: u64,
    entry: String,
) -> Result<(), String> {
    forget_saved_file_in_session(&state, server, entry).await
}

/// Keep the real command path usable by native concurrency regressions without a window.
async fn forget_saved_file_in_session(
    state: &AppState,
    server: u64,
    entry: String,
) -> Result<(), String> {
    let generation = keep_session::invocation_generation(state).await?;
    let id = entry_id(&entry)?;
    let (_, instance) = actor_instance_of(state, server).await?;
    // Commit -> Store -> Servers is the mutation order. Lock may register while those
    // locks are busy; its atomic fence is repeated inside the synchronous mutation.
    let _session = require_ui_session_generation(state, generation).await?;
    with_list(state, server, instance, |list| {
        if let Err(error) = state.ui_session_gate().check_generation(generation) {
            return (Err(error), false);
        }
        match list {
            Loaded::Readable(list) => {
                let changed = list.forget(id);
                (Ok(()), changed)
            }
            Loaded::Unreadable(_) => (Err(LIST_UNREADABLE.to_string()), false),
        }
    })
    .await?
    .ok_or_else(|| "server changed".to_string())?
}

/// "Start a new list": replace this server's saved-files record with an empty list, only while it
/// reads as damaged. A record a newer Mewtual wrote is never replaced.
#[tauri::command]
pub(super) async fn reset_saved_files(
    state: State<'_, AppState>,
    server: u64,
) -> Result<(), String> {
    reset_saved_files_in_session(&state, server).await
}

async fn reset_saved_files_in_session(state: &AppState, server: u64) -> Result<(), String> {
    let generation = keep_session::invocation_generation(state).await?;
    let (_, instance) = actor_instance_of(state, server).await?;
    let _session = require_ui_session_generation(state, generation).await?;
    let guard = state.store.lock().await;
    let store = guard
        .as_ref()
        .ok_or_else(|| "The vault is locked.".to_string())?;
    let servers = state.servers.lock().await;
    let entry = servers
        .get(&server)
        .filter(|entry| entry.instance == instance)
        .ok_or_else(|| "server changed".to_string())?;
    let owner = [entry.group_id.as_slice(), entry.device_id.as_bytes()].concat();
    state.ui_session_gate().check_generation(generation)?;
    if store
        .reset_saved_files(server, &owner, &mut OsCryptoRng)
        .map_err(|error| error.to_string())?
    {
        Ok(())
    } else {
        Err(RESET_REFUSED.into())
    }
}

/// This server's "Remember saved files" setting on this device, with how many files the list
/// holds now (Server settings, Storage).
#[tauri::command]
pub(super) async fn get_saved_files_limit(
    state: State<'_, AppState>,
    server: u64,
) -> Result<UiSavedFilesLimit, String> {
    let generation = keep_session::invocation_generation(&state).await?;
    let (_, instance) = actor_instance_of(&state, server).await?;
    let view = with_owner(&state, server, instance, |store, owner| {
        limit_view(store, server, owner)
    })
    .await
    .ok_or_else(|| "server changed".to_string())?;
    if !still_current(&state, server, instance, generation).await {
        return Err("the UI session changed while the operation was in progress".into());
    }
    Ok(view)
}

/// Choose this server's "Remember saved files" setting, clamped into 16 to 4096. With `apply`
/// false it only says how many of the oldest entries the choice would forget, so the renderer can
/// ask first; with `apply` it changes the setting, forgetting those entries (never files), but
/// only while that is still exactly `expected_forgets` (the count the user confirmed; required).
#[tauri::command]
pub(super) async fn set_saved_files_limit(
    state: State<'_, AppState>,
    server: u64,
    limit: u64,
    apply: bool,
    expected_forgets: Option<u64>,
) -> Result<UiSavedLimitChange, String> {
    set_saved_files_limit_in_session(&state, server, limit, apply, expected_forgets).await
}

async fn set_saved_files_limit_in_session(
    state: &AppState,
    server: u64,
    limit: u64,
    apply: bool,
    expected_forgets: Option<u64>,
) -> Result<UiSavedLimitChange, String> {
    let apply = match (apply, expected_forgets) {
        (false, _) => None,
        (true, Some(expected)) => Some(expected),
        (true, None) => return Err(LIMIT_LIST_CHANGED.into()),
    };
    let generation = keep_session::invocation_generation(state).await?;
    let (_, instance) = actor_instance_of(state, server).await?;
    let _session = require_ui_session_generation(state, generation).await?;
    with_owner(state, server, instance, |store, owner| {
        state.ui_session_gate().check_generation(generation)?;
        change_limit(store, server, owner, limit, apply)
    })
    .await
    .ok_or_else(|| "server changed".to_string())?
}

#[cfg(test)]
#[path = "saved_files/tests.rs"]
mod tests;
