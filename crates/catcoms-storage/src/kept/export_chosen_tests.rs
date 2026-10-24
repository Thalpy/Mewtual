//! "Save as": a destination the user picked in the native dialog. Uses the parent test module's
//! store and plan helpers; the dialog itself is never involved at this layer.
use super::*;

fn chosen_folder(fx: &Fixture) -> PathBuf {
    let folder = fx.downloads.with_file_name("Chosen place");
    create_folder(&folder).unwrap(); // 0755 on Unix whatever the umask (review M-6)
    folder
}
fn begin_chosen(
    store: &mut KeptBlobStore<ChaCha20Rng>,
    lease: &DownloadExportLease,
    choice: &ChosenDestination,
) -> ExportFile {
    let file = ExportFile::create_chosen(choice, export_plan(), &mut rng()).unwrap();
    store
        .begin_download_export(lease, file.journal(), file.presence())
        .unwrap();
    file
}
/// Start acquisition and remember `choice` in the same step, as the actor does.
fn planned_run(
    store: &mut KeptBlobStore<ChaCha20Rng>,
    choice: &ChosenDestination,
) -> (DownloadId, DownloadRun) {
    let run = store.begin_download(vec![plan()], 0, [9; 32]).unwrap();
    store.plan_download_destination(run, choice).unwrap();
    (run.id(), run)
}
fn remembered(store: &KeptBlobStore<ChaCha20Rng>, id: DownloadId) -> ChosenDestination {
    store
        .download_planned_destination(id)
        .unwrap()
        .expect("a choice is remembered")
        .expect("decodable on this platform")
}
fn with_identity_unsupported<T>(count: u32, run: impl FnOnce() -> T) -> T {
    INJECT.with(|slot| slot.set(Some((Effect::Identity, io::ErrorKind::Unsupported, count))));
    let result = run();
    INJECT.with(|slot| slot.set(None));
    result
}
/// A destination on a filesystem without stable identities, as `validate` would build it there.
fn identityless(folder: &Path, name: &str) -> ChosenDestination {
    let choice =
        with_identity_unsupported(1, || ChosenDestination::validate(&folder.join(name))).unwrap();
    assert!(choice.folder_id.is_none());
    choice
}

#[test]
fn selection_checks_refuse_unusable_paths_and_never_touch_an_existing_file() {
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let existing = folder.join("report.bin");
    fs::write(&existing, b"the user's own report").unwrap();
    assert_eq!(
        ChosenDestination::validate(&existing).unwrap_err(),
        ChoiceRefusal::Exists,
        "an existing name is refused even after the dialog asked to replace it"
    );
    assert_eq!(fs::read(&existing).unwrap(), b"the user's own report");
    fs::write(folder.join("other.bin.mewtual-part"), b"old partial").unwrap();
    assert_eq!(
        ChosenDestination::validate(&folder.join("other.bin")).unwrap_err(),
        ChoiceRefusal::PartialExists
    );
    for (path, refusal) in [
        (PathBuf::from("relative.bin"), ChoiceRefusal::InvalidName),
        (folder.join("bad:name.bin"), ChoiceRefusal::InvalidName),
        (folder.join("trailing."), ChoiceRefusal::InvalidName),
        // Windows device names address a device, not a file, whatever the extension.
        (folder.join("NUL.txt"), ChoiceRefusal::InvalidName),
        (folder.join("com1"), ChoiceRefusal::InvalidName),
        (folder.join("Lpt9 .log"), ChoiceRefusal::InvalidName),
        // Picker review residual: superscript and zero ports and the console names, too.
        (folder.join("COM\u{00B9}.txt"), ChoiceRefusal::InvalidName),
        (folder.join("lpt\u{00B3}"), ChoiceRefusal::InvalidName),
        (folder.join("com0.log"), ChoiceRefusal::InvalidName),
        (folder.join("CONIN$"), ChoiceRefusal::InvalidName),
        (folder.join("conout$.txt"), ChoiceRefusal::InvalidName),
        (
            folder.join("missing").join("x.bin"),
            ChoiceRefusal::FolderUnavailable,
        ),
        (existing.join("x.bin"), ChoiceRefusal::FolderUnavailable),
    ] {
        assert_eq!(
            ChosenDestination::validate(&path).unwrap_err(),
            refusal,
            "{path:?}"
        );
    }
    for fine in ["COM10.txt", "COM\u{2074}.txt", "CONSOLE.txt", "LPT.txt"] {
        assert!(
            ChosenDestination::validate(&folder.join(fine)).is_ok(),
            "{fine}"
        );
    }
    // A chosen name is never shortened: one without room for the suffixes is refused.
    let long = folder.join(format!("{}.txt", "\u{1F600}".repeat(120)));
    assert_eq!(
        ChosenDestination::validate(&long).unwrap_err(),
        ChoiceRefusal::NameTooLong
    );
    let fine = ChosenDestination::validate(&folder.join("fresh.bin")).unwrap();
    assert_eq!(fine.folder, folder);
    assert_eq!(fine.name(), "fresh.bin");
    assert!(fine.folder_id.is_some());
    let debug = format!("{fine:?} {:?}", fine.planned());
    assert!(
        !debug.contains("Chosen place") && !debug.contains("fresh"),
        "{debug}"
    );
    assert_eq!(
        fs::read_dir(&folder).unwrap().count(),
        2,
        "validation creates nothing"
    );
}

#[test]
fn a_chosen_destination_publishes_at_the_exact_name_and_resumes_there_after_restart() {
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let choice = ChosenDestination::validate(&folder.join("report.bin")).unwrap();
    let mut file = begin_chosen(&mut store, &lease, &choice);
    assert_eq!(file.journal().final_leaf, "report.bin");
    assert_eq!(file.journal().staging, "report.bin.mewtual-part");
    assert!(folder.join("report.bin.mewtual-part").exists());
    assert!(
        fs::read_dir(&fx.downloads).unwrap().next().is_none(),
        "nothing goes to Downloads"
    );
    file.accept(0, CHUNKS[0]).unwrap();
    drop(file);
    drop(store); // crash or restart mid-save
    let mut store = open(&fx.vault);
    let ExportState::Writing(journal) = state(&store, id) else {
        panic!("still writing");
    };
    // Resume uses only the sealed chosen folder; no Downloads path is involved.
    assert!(matches!(ExportFile::probe(&journal), ExportProbe::Ready));
    let lease = reverified(&mut store, id);
    let ExportReopen::Ready(mut file) = ExportFile::reopen(&journal, export_plan()).unwrap() else {
        panic!("our partial reopens at the chosen folder");
    };
    store
        .resume_download_export(&lease, &journal, file.presence())
        .unwrap();
    write_all(&mut file);
    let path = publish(&mut store, &lease, &mut file);
    file.finish(&mut rng()).unwrap();
    assert_eq!(path, folder.join("report.bin"));
    assert_eq!(fs::read(path).unwrap(), CHUNKS.concat());
}

#[test]
fn a_collision_after_resume_is_numbered_from_the_chosen_name_not_the_listed_one() {
    // The user chose "Chosen.bin" for a file listed as something else; the chosen name is taken
    // while the save is paused. Publication numbers from the sealed choice.
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let choice = ChosenDestination::validate(&folder.join("Chosen.bin")).unwrap();
    let mut file = begin_chosen(&mut store, &lease, &choice);
    file.accept(0, CHUNKS[0]).unwrap();
    drop(file);
    fs::write(folder.join("Chosen.bin"), b"arrived while paused").unwrap();
    drop(store);
    let mut store = open(&fx.vault);
    let ExportState::Writing(journal) = state(&store, id) else {
        panic!();
    };
    let lease = reverified(&mut store, id);
    let ExportReopen::Ready(mut file) = ExportFile::reopen(&journal, export_plan()).unwrap() else {
        panic!();
    };
    store
        .resume_download_export(&lease, &journal, file.presence())
        .unwrap();
    write_all(&mut file);
    let path = publish(&mut store, &lease, &mut file);
    assert_eq!(path, folder.join("Chosen (1).bin"));
    assert_eq!(
        fs::read(folder.join("Chosen.bin")).unwrap(),
        b"arrived while paused"
    );
}

#[test]
fn a_chosen_folder_renamed_or_replaced_while_paused_halts_without_touching_anything() {
    for replaced in [false, true] {
        let fx = fixture();
        let folder = chosen_folder(&fx);
        let mut store = open(&fx.vault);
        let (id, lease) = completed(&mut store);
        let choice = ChosenDestination::validate(&folder.join("report.bin")).unwrap();
        let mut file = begin_chosen(&mut store, &lease, &choice);
        file.accept(0, CHUNKS[0]).unwrap();
        let journal = file.journal().clone();
        drop(file);
        let moved = folder.with_file_name("Renamed place");
        fs::rename(&folder, &moved).unwrap();
        let expected = if replaced {
            fs::create_dir(&folder).unwrap();
            ExportHalt::Changed
        } else {
            ExportHalt::Unavailable
        };
        assert!(
            matches!(ExportFile::probe(&journal), ExportProbe::Halt(halt) if halt == expected),
            "replaced={replaced}"
        );
        halt(&mut store, id, &journal, expected).unwrap();
        assert!(
            moved.join("report.bin.mewtual-part").exists(),
            "the moved partial is left alone"
        );
        if replaced {
            assert_eq!(fs::read_dir(&folder).unwrap().count(), 0);
        }
    }
}

#[test]
fn a_parent_swapped_or_name_taken_after_selection_is_refused_before_any_write() {
    // Folder swapped for a different directory (or a junction to one) at the same path.
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let choice = ChosenDestination::validate(&folder.join("report.bin")).unwrap();
    let elsewhere = folder.with_file_name("Elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    fs::rename(&folder, folder.with_file_name("Original")).unwrap();
    if !external::tests::link_directory(&elsewhere, &folder) {
        fs::create_dir(&folder).unwrap();
    }
    assert!(matches!(
        ExportFile::create_chosen(&choice, export_plan(), &mut rng()),
        Err(ExportError::Refused(ChoiceRefusal::FolderChanged))
    ));
    assert_eq!(
        fs::read_dir(&elsewhere).unwrap().count(),
        0,
        "nothing is written through a swapped parent"
    );
    // The chosen name created by someone else while the file was being acquired.
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let choice = ChosenDestination::validate(&folder.join("report.bin")).unwrap();
    fs::write(folder.join("report.bin"), b"arrived meanwhile").unwrap();
    assert!(matches!(
        ExportFile::create_chosen(&choice, export_plan(), &mut rng()),
        Err(ExportError::Refused(ChoiceRefusal::Exists))
    ));
    assert_eq!(
        fs::read(folder.join("report.bin")).unwrap(),
        b"arrived meanwhile"
    );
    assert!(!folder.join("report.bin.mewtual-part").exists());
    // A folder that had an identity and now reports none is not provably the chosen one; it
    // must not silently drop to the identity-less writer.
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let choice = ChosenDestination::validate(&folder.join("report.bin")).unwrap();
    assert!(matches!(
        with_identity_unsupported(1, || ExportFile::create_chosen(
            &choice,
            export_plan(),
            &mut rng()
        )),
        Err(ExportError::Refused(ChoiceRefusal::FolderChanged))
    ));
    assert!(matches!(
        with_identity_unsupported(1, || choice.create_unjournaled(&mut rng())),
        Err(ExportError::Refused(ChoiceRefusal::FolderChanged))
    ));
    assert_eq!(fs::read_dir(&folder).unwrap().count(), 0);
}

/// Swap `folder` for a fresh empty directory at the same path the first time any exclusive
/// creation runs inside `run`, modelling another program acting between the folder identity
/// check and the path-based create. Returns where the original folder went.
fn with_swap_before_create<T>(folder: &Path, run: impl FnOnce() -> T) -> (T, PathBuf) {
    let moved = folder.with_file_name("Swapped away");
    let (path, target) = (folder.to_owned(), moved.clone());
    let mut armed = true;
    external::BEFORE_CREATE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_| {
            if std::mem::take(&mut armed) {
                fs::rename(&path, &target).unwrap();
                fs::create_dir(&path).unwrap();
            }
        }))
    });
    let result = run();
    external::BEFORE_CREATE.with(|hook| *hook.borrow_mut() = None);
    (result, moved)
}

#[test]
fn a_folder_swapped_between_the_identity_check_and_creation_is_refused() {
    // Picker review L4: the partial must not stay (and later fill with plaintext) in a folder
    // swapped in after the identity check. Each creation re-checks, removes its own partial and
    // refuses, leaving both folders empty.
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let choice = ChosenDestination::validate(&folder.join("report.bin")).unwrap();
    let (created, moved) = with_swap_before_create(&folder, || {
        ExportFile::create_chosen(&choice, export_plan(), &mut rng())
    });
    assert!(matches!(
        created,
        Err(ExportError::Refused(ChoiceRefusal::FolderChanged))
    ));
    assert_eq!(
        fs::read_dir(&folder).unwrap().count(),
        0,
        "swapped-in folder"
    );
    assert_eq!(fs::read_dir(&moved).unwrap().count(), 0, "original folder");

    // The non-resumable reservation at an identified chosen place.
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let choice = ChosenDestination::validate(&folder.join("report.bin")).unwrap();
    let (created, moved) =
        with_swap_before_create(&folder, || choice.create_unjournaled(&mut rng()));
    assert!(matches!(
        created,
        Err(ExportError::Refused(ChoiceRefusal::FolderChanged))
    ));
    assert_eq!(fs::read_dir(&folder).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&moved).unwrap().count(), 0);

    // The one-click Downloads save: refused as a local failure, nothing left behind.
    let fx = fixture();
    fs::create_dir_all(&fx.downloads).unwrap();
    let (created, moved) = with_swap_before_create(&fx.downloads, || {
        ExportFile::create(&fx.downloads, "report.bin", export_plan(), &mut rng())
    });
    assert!(matches!(created, Err(ExportError::Local { .. })));
    assert_eq!(fs::read_dir(&fx.downloads).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&moved).unwrap().count(), 0);
}

#[test]
fn a_downloads_reservation_without_links_is_refused_before_any_plaintext() {
    // Re-review 2 LOW 4: native's one-click `.part` in a Downloads folder that cannot carry a
    // journal now probes links on its fresh file, as the chosen-place reservation does.
    let fx = fixture();
    let staging = fx.downloads.join("notes.txt.part");
    let file = external::create_leaf(&staging).unwrap();
    assert!(matches!(
        without_links(io::ErrorKind::PermissionDenied, || {
            confirm_staging_links(file, &fx.downloads, "notes.txt.part", &mut rng())
        }),
        Err(ExportError::NoLinks)
    ));
    assert_eq!(
        fs::read_dir(&fx.downloads).unwrap().count(),
        0,
        "the empty reservation is removed"
    );
    // A folder that links keeps the reservation and nothing else.
    let file = external::create_leaf(&staging).unwrap();
    let file = confirm_staging_links(file, &fx.downloads, "notes.txt.part", &mut rng()).unwrap();
    drop(file);
    assert_eq!(fs::read_dir(&fx.downloads).unwrap().count(), 1);
    assert!(staging.exists());
}

#[test]
fn a_remembered_choice_is_preflighted_without_creating_anything() {
    // Re-review 2 LOW 3: an unplugged, swapped or occupied chosen place is found before Resume
    // spends allowance, and the check itself creates nothing.
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let choice = ChosenDestination::validate(&folder.join("report.bin")).unwrap();
    assert_eq!(choice.preflight(), Ok(()));
    assert_eq!(fs::read_dir(&folder).unwrap().count(), 0, "nothing created");
    fs::write(folder.join("report.bin"), b"arrived meanwhile").unwrap();
    assert_eq!(choice.preflight(), Err(ChoiceRefusal::Exists));
    fs::remove_file(folder.join("report.bin")).unwrap();
    fs::write(folder.join("report.bin.mewtual-part"), b"").unwrap();
    assert_eq!(choice.preflight(), Err(ChoiceRefusal::PartialExists));
    fs::remove_file(folder.join("report.bin.mewtual-part")).unwrap();
    // Swapped for another directory at the same path.
    fs::rename(&folder, folder.with_file_name("Moved away")).unwrap();
    fs::create_dir(&folder).unwrap();
    assert_eq!(choice.preflight(), Err(ChoiceRefusal::FolderChanged));
    // Gone entirely (an unplugged drive).
    fs::remove_dir(&folder).unwrap();
    assert_eq!(choice.preflight(), Err(ChoiceRefusal::FolderUnavailable));
    assert!(!folder.exists(), "never recreated");
    // Without a captured identity: still a directory, and the `.part` name counts too.
    fs::create_dir(&folder).unwrap();
    let loose = identityless(&folder, "loose.bin");
    assert_eq!(loose.preflight(), Ok(()));
    fs::write(folder.join("loose.bin.part"), b"").unwrap();
    assert_eq!(loose.preflight(), Err(ChoiceRefusal::PartialExists));
    fs::remove_file(folder.join("loose.bin.part")).unwrap();
    fs::remove_dir(&folder).unwrap();
    assert_eq!(loose.preflight(), Err(ChoiceRefusal::FolderUnavailable));
}

#[test]
fn a_swap_and_swap_back_around_creation_with_a_planted_file_writes_nothing() {
    // Picker re-review LOW 2: the folder is swapped before the create (our partial lands in the
    // other folder), then restored with a file planted at our name before the recheck. The
    // identity recheck passes; creation must still refuse, because the name is not our handle's
    // object (the held-folder name check in `confirm_folder`, on both platforms since the
    // coordinator's review L1), and the planted file must be left alone.
    // The swapped-in folder is a link (junction on Windows) to a decoy directory: Windows refuses
    // to rename a directory while our new file inside it is open, but removing a link is allowed.
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let choice = ChosenDestination::validate(&folder.join("report.bin")).unwrap();
    let original = folder.with_file_name("Original");
    let decoy = folder.with_file_name("Decoy");
    fs::create_dir(&decoy).unwrap();
    let probe = folder.with_file_name("Link probe");
    if !external::tests::link_directory(&decoy, &probe) {
        eprintln!("skipped: this host cannot create a directory link");
        return;
    }
    fs::remove_dir(&probe)
        .or_else(|_| fs::remove_file(&probe))
        .unwrap();
    let (path, held, target) = (folder.clone(), original.clone(), decoy.clone());
    external::BEFORE_CREATE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_| {
            fs::rename(&path, &held).unwrap();
            assert!(external::tests::link_directory(&target, &path));
        }))
    });
    let (path, held) = (folder.clone(), original.clone());
    external::AFTER_CREATE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_| {
            fs::remove_dir(&path)
                .or_else(|_| fs::remove_file(&path))
                .unwrap();
            fs::rename(&held, &path).unwrap();
            fs::write(path.join("report.bin.part"), b"planted").unwrap();
        }))
    });
    let created = choice.create_unjournaled(&mut rng());
    external::BEFORE_CREATE.with(|hook| *hook.borrow_mut() = None);
    external::AFTER_CREATE.with(|hook| *hook.borrow_mut() = None);
    assert!(
        matches!(
            created,
            Err(ExportError::Refused(ChoiceRefusal::FolderChanged))
        ),
        "the planted file is never adopted"
    );
    assert_eq!(
        fs::read(folder.join("report.bin.part")).unwrap(),
        b"planted",
        "the planted file is left alone"
    );
    assert_eq!(
        fs::metadata(decoy.join("report.bin.part")).unwrap().len(),
        0,
        "our own partial, in the other folder, holds no plaintext"
    );
}

/// Where [`with_swaps_around_creation`] swaps the folder for the decoy a third time.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ThirdSwap {
    /// No third swap.
    Never,
    /// Just before the held-folder name check, after the path-based identity recheck passed.
    BeforeNameCheck,
    /// Windows: just before the path-based link probe (the coordinator's review L1).
    #[cfg(not(unix))]
    BeforeProbe,
}

/// Picker re-review LOW 2: run `create` while the chosen `folder` is swapped for a directory link
/// to a fresh decoy just before the exclusive create (so our partial lands in the decoy) and
/// swapped back right after it, planting a file named `plant` in the restored folder when given.
/// With `third`, the folder is swapped for the decoy once more at that point. Returns the result
/// and the decoy, or `None` when this host cannot create directory links. The swapped-in folder
/// is a link (a junction on Windows) because Windows refuses to rename a directory while our new
/// file inside it is open, but removing a link is allowed; a third swap renames the restored real
/// folder, which holds nothing open, and points a link at the decoy again.
fn with_swaps_around_creation<T>(
    folder: &Path,
    plant: Option<&str>,
    third: ThirdSwap,
    create: impl FnOnce() -> T,
) -> Option<(T, PathBuf)> {
    let original = folder.with_file_name("Original");
    let decoy = folder.with_file_name("Decoy");
    fs::create_dir(&decoy).unwrap();
    let probe = folder.with_file_name("Link probe");
    if !external::tests::link_directory(&decoy, &probe) {
        eprintln!("skipped: this host cannot create a directory link");
        return None;
    }
    fs::remove_dir(&probe)
        .or_else(|_| fs::remove_file(&probe))
        .unwrap();
    let swap_in = |path: PathBuf, held: PathBuf, target: PathBuf| -> Box<dyn FnMut(&Path)> {
        Box::new(move |_: &Path| {
            fs::rename(&path, &held).unwrap();
            assert!(external::tests::link_directory(&target, &path));
        })
    };
    let first = swap_in(folder.to_owned(), original.clone(), decoy.clone());
    external::BEFORE_CREATE.with(|hook| *hook.borrow_mut() = Some(first));
    let (path, held, planted) = (
        folder.to_owned(),
        original.clone(),
        plant.map(str::to_owned),
    );
    external::AFTER_CREATE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_| {
            fs::remove_dir(&path)
                .or_else(|_| fs::remove_file(&path))
                .unwrap();
            fs::rename(&held, &path).unwrap();
            if let Some(planted) = &planted {
                fs::write(path.join(planted), b"planted").unwrap();
            }
        }))
    });
    let third_hook = swap_in(folder.to_owned(), original.clone(), decoy.clone());
    match third {
        ThirdSwap::Never => {}
        ThirdSwap::BeforeNameCheck => {
            external::BEFORE_NAME_CHECK.with(|hook| *hook.borrow_mut() = Some(third_hook));
        }
        #[cfg(not(unix))]
        ThirdSwap::BeforeProbe => {
            external::BEFORE_PROBE.with(|hook| *hook.borrow_mut() = Some(third_hook));
        }
    }
    let result = create();
    external::BEFORE_CREATE.with(|hook| *hook.borrow_mut() = None);
    external::AFTER_CREATE.with(|hook| *hook.borrow_mut() = None);
    external::BEFORE_NAME_CHECK.with(|hook| *hook.borrow_mut() = None);
    #[cfg(not(unix))]
    external::BEFORE_PROBE.with(|hook| *hook.borrow_mut() = None);
    Some((result, decoy))
}

/// No file in `folder` holds a single byte (our abandoned partial is empty or was removed).
fn holds_no_plaintext(folder: &Path) -> bool {
    fs::read_dir(folder)
        .unwrap()
        .all(|entry| entry.unwrap().metadata().unwrap().len() == 0)
}

#[test]
fn a_swap_and_swap_back_around_a_journaled_creation_starts_no_journal() {
    // The same attack against the journaled writer (Linux CI regression: on Unix PR51 had dropped
    // the staging-name check with the link probe, so the journal would have started with its
    // handle on a partial in the swapped-in folder and written the plaintext there).
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let choice = ChosenDestination::validate(&folder.join("report.bin")).unwrap();
    let staging_name = external::staging_for("report.bin");
    let Some((created, decoy)) =
        with_swaps_around_creation(&folder, Some(&staging_name), ThirdSwap::Never, || {
            ExportFile::create_chosen(&choice, export_plan(), &mut rng())
        })
    else {
        return;
    };
    // A detected swap is a refusal, never `Unsupported`, which would quietly hand the save to the
    // non-resumable writer (review of the Linux CI fix, MEDIUM 1).
    assert!(
        matches!(
            created,
            Err(ExportError::Refused(ChoiceRefusal::FolderChanged))
        ),
        "no journal starts on a planted name"
    );
    assert_eq!(
        fs::read(folder.join(&staging_name)).unwrap(),
        b"planted",
        "the planted file is left alone"
    );
    assert_eq!(
        fs::metadata(decoy.join(&staging_name)).unwrap().len(),
        0,
        "our own partial, in the other folder, holds no plaintext"
    );
}

#[test]
fn a_swap_and_swap_back_around_creation_with_nothing_planted_writes_nothing() {
    // Review of the Linux CI fix, LOW 3: a name missing from the restored folder is never ours
    // either, for both writers.
    for journaled in [true, false] {
        let fx = fixture();
        let folder = chosen_folder(&fx);
        let choice = ChosenDestination::validate(&folder.join("report.bin")).unwrap();
        let Some((created, decoy)) =
            with_swaps_around_creation(&folder, None, ThirdSwap::Never, || {
                if journaled {
                    ExportFile::create_chosen(&choice, export_plan(), &mut rng()).map(drop)
                } else {
                    choice.create_unjournaled(&mut rng()).map(drop)
                }
            })
        else {
            return;
        };
        // Before the coordinator's review L1, Windows read this as a folder without links (the
        // probe found nothing to link); the held-folder name check now says what happened.
        assert!(
            matches!(
                created,
                Err(ExportError::Refused(ChoiceRefusal::FolderChanged))
            ),
            "journaled: {journaled}"
        );
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 0, "the real folder");
        assert!(holds_no_plaintext(&decoy), "journaled: {journaled}");
    }
}

/// Run both writers' creation at a fresh chosen place with a third swap at `third`, and require
/// the changed-folder refusal with no plaintext anywhere: the real folder (renamed aside by the
/// third swap) is empty and nothing in the decoy holds a byte.
fn a_third_swap_is_refused(third: ThirdSwap) {
    for journaled in [true, false] {
        let fx = fixture();
        let folder = chosen_folder(&fx);
        let choice = ChosenDestination::validate(&folder.join("report.bin")).unwrap();
        let Some((created, decoy)) = with_swaps_around_creation(&folder, None, third, || {
            if journaled {
                ExportFile::create_chosen(&choice, export_plan(), &mut rng()).map(drop)
            } else {
                choice.create_unjournaled(&mut rng()).map(drop)
            }
        }) else {
            return;
        };
        assert!(
            matches!(
                created,
                Err(ExportError::Refused(ChoiceRefusal::FolderChanged))
            ),
            "journaled: {journaled}, got {created:?}"
        );
        // The real folder sits at `folder` again when the third swap never ran (refused before
        // it), or under "Original" when it did.
        let original = folder.with_file_name("Original");
        let real = if original.exists() {
            &original
        } else {
            &folder
        };
        assert!(
            fs::symlink_metadata(real).unwrap().is_dir(),
            "the real folder itself"
        );
        assert_eq!(
            fs::read_dir(real).unwrap().count(),
            0,
            "nothing in the real folder"
        );
        assert!(holds_no_plaintext(&decoy), "journaled: {journaled}");
    }
}

#[test]
fn a_third_swap_before_the_name_check_is_refused() {
    // Review of the Linux CI fix, MEDIUM 2 (Unix) and the coordinator's review L1 (Windows):
    // swapped before the create, swapped back before the path-based identity recheck (which
    // passes), then swapped for the decoy again before the name check. Two path-based reads would
    // each have been answered by a different folder; the held-folder check reads the identity and
    // the name through one handle and refuses.
    a_third_swap_is_refused(ThirdSwap::BeforeNameCheck);
}

#[cfg(not(unix))]
#[test]
fn a_junction_retargeted_before_the_link_probe_is_refused() {
    // The coordinator's review L1: on Windows the probe used to be the only name check, and it
    // resolves the folder by path. Retargeting the path (a junction) to the decoy just before the
    // probe made it link our own partial there, so creation succeeded with the journal bound to the
    // real folder and every plaintext byte written into the decoy, which Remove could not clean.
    // The held-folder name check in `confirm_folder` now refuses before the probe runs (the probe
    // also checks its own name through a held handle).
    a_third_swap_is_refused(ThirdSwap::BeforeProbe);
}

#[test]
fn a_swap_and_swap_back_around_an_identityless_reservation_writes_nothing() {
    // Without a captured folder identity nothing re-checks the folder, so the only defence is that
    // the staging name must hold our new file (Unix: the path-based check in
    // `confirm_staging_links`; Windows: the link probe). Native's one-click Downloads reservation
    // takes the same path.
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let choice = identityless(&folder, "report.bin");
    let Some((created, decoy)) =
        with_swaps_around_creation(&folder, Some("report.bin.part"), ThirdSwap::Never, || {
            choice.create_unjournaled(&mut rng()).map(drop)
        })
    else {
        return;
    };
    assert!(
        matches!(
            created,
            Err(ExportError::Local {
                retryable: false,
                ..
            })
        ),
        "the planted file is never adopted"
    );
    assert_eq!(
        fs::read(folder.join("report.bin.part")).unwrap(),
        b"planted",
        "the planted file is left alone"
    );
    assert!(holds_no_plaintext(&decoy));
}

#[test]
fn removing_a_just_created_partial_needs_its_identity_to_still_match() {
    // Picker re-review LOW 3: the proven-removal branch of `remove_created`.
    let fx = fixture();
    let folder = chosen_folder(&fx);
    // The name was swapped for someone else's file: left alone.
    let swapped = folder.join("swapped.mewtual-part");
    let file = external::create_leaf(&swapped).unwrap();
    fs::rename(&swapped, folder.join("ours elsewhere")).unwrap();
    fs::write(&swapped, b"someone else's").unwrap();
    remove_created(file, &swapped, &mut rng());
    assert_eq!(fs::read(&swapped).unwrap(), b"someone else's");
    // Still our own object: removed.
    let kept = folder.join("ours.mewtual-part");
    let file = external::create_leaf(&kept).unwrap();
    remove_created(file, &kept, &mut rng());
    assert!(!kept.exists());
    // No number to prove it with: left in place (PR51 H1 changed this contract on purpose: it
    // used to be removed by path; an empty file left behind discloses nothing, while deleting
    // without proof could remove someone else's file).
    let unproven = folder.join("unproven.mewtual-part");
    let file = external::create_leaf(&unproven).unwrap();
    with_identity_unsupported(1, || remove_created(file, &unproven, &mut rng()));
    assert!(unproven.exists());
}

#[test]
fn an_orphan_partial_from_a_crash_before_begin_is_refused_and_left_alone() {
    // Picker review L3: a crash between `create_chosen` and Begin leaves an empty
    // `<name>.mewtual-part` at the chosen place. Nothing proves it is ours, so a later save to the
    // remembered choice refuses it (never adopts or deletes it) and says how to recover.
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let mut store = open(&fx.vault);
    let choice = ChosenDestination::validate(&folder.join("report.bin")).unwrap();
    let (id, _run) = planned_run(&mut store, &choice);
    let orphan =
        ExportFile::create_chosen(&remembered(&store, id), export_plan(), &mut rng()).unwrap();
    drop(orphan); // the crash: no Begin, no discard
    let staging = folder.join("report.bin.mewtual-part");
    assert!(staging.exists());
    let refused =
        ExportFile::create_chosen(&remembered(&store, id), export_plan(), &mut rng()).unwrap_err();
    assert!(matches!(
        refused,
        ExportError::Refused(ChoiceRefusal::PartialExists)
    ));
    let message = refused.to_string().to_lowercase();
    assert!(
        message.contains("earlier interrupted save")
            && message.contains("delete it from the folder")
            && !message.contains("remove"),
        "says where it came from and how to recover: {message}"
    );
    assert_eq!(fs::read(&staging).unwrap(), b"", "the orphan is untouched");
    assert_eq!(fs::read_dir(&folder).unwrap().count(), 1);
}

#[test]
fn a_chosen_folder_without_links_is_refused_before_any_plaintext_exists() {
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let choice = ChosenDestination::validate(&folder.join("report.bin")).unwrap();
    assert!(matches!(
        without_links(io::ErrorKind::Unsupported, || {
            ExportFile::create_chosen(&choice, export_plan(), &mut rng())
        }),
        Err(ExportError::NoLinks)
    ));
    assert_eq!(fs::read_dir(&folder).unwrap().count(), 0);
    // The same refusal on an identity-less folder, through the non-resumable reservation.
    let choice = identityless(&folder, "report.bin");
    assert!(matches!(
        without_links(io::ErrorKind::PermissionDenied, || {
            choice.create_unjournaled(&mut rng())
        }),
        Err(ExportError::NoLinks)
    ));
    assert_eq!(fs::read_dir(&folder).unwrap().count(), 0);
}

#[test]
fn an_identityless_chosen_folder_falls_back_with_the_chosen_location_rules() {
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let choice = identityless(&folder, "report.bin");
    assert!(matches!(
        ExportFile::create_chosen(&choice, export_plan(), &mut rng()),
        Err(ExportError::Unsupported)
    ));
    assert_eq!(
        fs::read_dir(&folder).unwrap().count(),
        0,
        "nothing left behind"
    );
    // Exact name, beside it, no numbering.
    let (file, staging, final_path) = choice.create_unjournaled(&mut rng()).unwrap();
    assert_eq!(staging, folder.join("report.bin.part"));
    assert_eq!(final_path, folder.join("report.bin"));
    assert_eq!(
        fs::read_dir(&folder).unwrap().count(),
        1,
        "the probe name is gone"
    );
    drop(file);
    // An existing .part is refused, never adopted or numbered around.
    assert!(matches!(
        choice.create_unjournaled(&mut rng()),
        Err(ExportError::Refused(ChoiceRefusal::PartialExists))
    ));
    fs::remove_file(&staging).unwrap();
    // The chosen name taken after selection is refused and left byte-identical.
    fs::write(&final_path, b"someone else's").unwrap();
    assert!(matches!(
        choice.create_unjournaled(&mut rng()),
        Err(ExportError::Refused(ChoiceRefusal::Exists))
    ));
    assert_eq!(fs::read(&final_path).unwrap(), b"someone else's");
    assert!(!staging.exists());
    // A removed folder is refused, never recreated.
    fs::remove_file(&final_path).unwrap();
    fs::remove_dir(&folder).unwrap();
    assert!(matches!(
        choice.create_unjournaled(&mut rng()),
        Err(ExportError::Refused(ChoiceRefusal::FolderUnavailable))
    ));
    assert!(!folder.exists());
}

#[test]
fn a_remembered_choice_survives_an_interrupted_acquisition_and_restart() {
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let mut store = open(&fx.vault);
    let choice = ChosenDestination::validate(&folder.join("report.bin")).unwrap();
    let (id, run) = planned_run(&mut store, &choice);
    // The first chunk arrives, then the acquisition is interrupted by a restart.
    store.put_download(run, b"first").unwrap();
    assert!(store.downloads().unwrap()[0].planned);
    let index = store.download_index(id).unwrap();
    assert_eq!(
        store.transfers[index].record.encoding,
        record::RecordEncoding::DownloadV2,
        "remembering a choice upgrades the record"
    );
    drop(store);
    let mut store = open(&fx.vault);
    assert!(store.downloads().unwrap()[0].planned);
    let again = remembered(&store, id);
    assert_eq!(again, choice);
    // Resume finishes the encrypted copy; the export goes to the remembered place.
    let lease = reverified(&mut store, id);
    let file = begin_chosen(&mut store, &lease, &again);
    assert_eq!(file.journal().final_leaf, "report.bin");
    assert!(
        store.download_planned_destination(id).unwrap().is_none(),
        "Begin consumed the plan; the journal now names the destination"
    );
    assert!(!store.downloads().unwrap()[0].planned);
    drop(file);
    drop(store);
    let store = open(&fx.vault);
    assert!(matches!(state(&store, id), ExportState::Writing(_)));
    assert!(store.download_planned_destination(id).unwrap().is_none());
}

#[test]
fn begin_refuses_a_journal_that_is_not_the_remembered_choice() {
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let mut store = open(&fx.vault);
    let choice = ChosenDestination::validate(&folder.join("report.bin")).unwrap();
    let (id, run) = planned_run(&mut store, &choice);
    let lease = verify_private(&mut store, run);
    // A one-click Downloads save that raced the choice cannot start.
    let downloads =
        ExportFile::create(&fx.downloads, "notes.txt", export_plan(), &mut rng()).unwrap();
    assert!(store
        .begin_download_export(&lease, downloads.journal(), downloads.presence())
        .is_err());
    downloads.discard(&mut rng());
    // Nor can a journal for a different choice in the same folder.
    let other = ChosenDestination::validate(&folder.join("other.bin")).unwrap();
    let wrong = ExportFile::create_chosen(&other, export_plan(), &mut rng()).unwrap();
    assert!(store
        .begin_download_export(&lease, wrong.journal(), wrong.presence())
        .is_err());
    wrong.discard(&mut rng());
    assert_eq!(remembered(&store, id), choice, "the plan is untouched");
    assert!(store.download_export_state(id).unwrap().is_none());
    // The matching journal starts and consumes it.
    let file = begin_chosen(&mut store, &lease, &choice);
    assert!(store.download_planned_destination(id).unwrap().is_none());
    drop(file);
}

#[test]
fn a_choice_can_be_replaced_but_never_redirects_an_existing_journal() {
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let mut store = open(&fx.vault);
    let first = ChosenDestination::validate(&folder.join("first.bin")).unwrap();
    let second = ChosenDestination::validate(&folder.join("second.bin")).unwrap();
    let (id, run) = planned_run(&mut store, &first);
    store.plan_download_destination(run, &first).unwrap();
    store.plan_download_destination(run, &second).unwrap();
    assert_eq!(remembered(&store, id), second, "Save as again replaces it");
    let lease = verify_private(&mut store, run);
    let file = begin_chosen(&mut store, &lease, &second);
    drop(file);
    // With a journal, a new choice is refused by the run fence or the journal check.
    let run = store.resume_download(id).unwrap();
    assert!(store.plan_download_destination(run, &first).is_err());
    assert!(store.download_planned_destination(id).unwrap().is_none());
    // A run that is no longer current cannot remember anything.
    store.pause_download(run).unwrap();
    assert!(store.plan_download_destination(run, &first).is_err());
}

#[test]
fn planned_destination_encoding_and_successor_rules() {
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let mut store = open(&fx.vault);
    let choice = ChosenDestination::validate(&folder.join("report.bin")).unwrap();
    let run = store.begin_download(vec![plan()], 0, [9; 32]).unwrap();
    let index = store.download_index(run.id()).unwrap();
    let v1 = store.transfers[index].record.clone();
    store.plan_download_destination(run, &choice).unwrap();
    let upgraded = store.transfers[index].record.clone();
    assert_eq!(upgraded.encoding, record::RecordEncoding::DownloadV2);
    assert!(v1.validate_successor(&upgraded).is_ok());
    // Without a plan the same upgrade is refused: V2 needs a journal or a plan.
    let mut bare = upgraded.clone();
    bare.planned = None;
    assert!(v1.validate_successor(&bare).is_err());
    verify_private(&mut store, run);
    let planned = store.transfers[index].record.clone();
    assert_eq!(planned.phase, KeepPhase::Completed);
    // Round trip, including an identity-less choice.
    assert_eq!(record::Record::decode(&planned.encode()).unwrap(), planned);
    let mut identityless_record = planned.clone();
    identityless_record.planned = Some(identityless(&folder, "x.bin").planned());
    assert_eq!(
        record::Record::decode(&identityless_record.encode()).unwrap(),
        identityless_record
    );
    // A V1 record cannot carry a plan, and a plan cannot sit beside a journal.
    let mut v1_with_plan = v1.clone();
    v1_with_plan.planned = planned.planned.clone();
    assert!(v1_with_plan.validate().is_err());
    let mut beside = planned.clone();
    beside.export = Export::Writing(
        ExportFile::create_chosen(&choice, export_plan(), &mut rng())
            .unwrap()
            .journal()
            .clone(),
    );
    assert!(beside.validate().is_err());
    beside.planned = None;
    assert!(beside.validate().is_ok(), "the plan alone made it invalid");
    // Begin's successor must drop exactly the matching plan.
    let mut begun = planned.successor().unwrap();
    begun.export = beside.export.clone();
    begun.planned = None;
    assert!(planned.validate_successor(&begun).is_ok());
    let mut other = begun.clone();
    let Export::Writing(journal) = &mut other.export else {
        unreachable!()
    };
    journal.final_leaf = "other.bin".into();
    journal.staging = "other.bin.mewtual-part".into();
    assert!(planned.validate_successor(&other).is_err());
    // Malformed planned bodies are refused.
    for corrupt in [
        PlannedDestination {
            platform: 9,
            ..choice.planned()
        },
        PlannedDestination {
            folder: Vec::new(),
            ..choice.planned()
        },
        PlannedDestination {
            name: "NUL".into(),
            ..choice.planned()
        },
        PlannedDestination {
            folder_id: Some(FileIdentity::for_test(
                if external::CURRENT_PLATFORM == external::PLATFORM_WINDOWS {
                    external::PLATFORM_UNIX
                } else {
                    external::PLATFORM_WINDOWS
                },
                1,
                1,
            )),
            ..choice.planned()
        },
    ] {
        let mut bad = planned.clone();
        bad.planned = Some(corrupt);
        assert!(record::Record::decode(&bad.encode()).is_err());
    }
    let mut trailing = planned.encode();
    trailing.push(0);
    assert!(record::Record::decode(&trailing).is_err());
    // Retiring a plan without Begin, or moving it on Deleting or a Retry grant, is refused.
    let mut cleared = planned.successor().unwrap();
    cleared.planned = None;
    assert!(planned.validate_successor(&cleared).is_err());
    let mut deleting = planned.successor().unwrap();
    deleting.phase = KeepPhase::Deleting;
    assert!(planned.validate_successor(&deleting).is_ok());
    deleting.planned = Some(identityless(&folder, "y.bin").planned());
    assert!(planned.validate_successor(&deleting).is_err());
    let mut exhausted = planned.successor().unwrap();
    exhausted.exhausted = true;
    exhausted.remaining.slices = 0;
    assert!(planned.validate_successor(&exhausted).is_ok());
    let (mut granted, _) = exhausted
        .retry(KeepRetry {
            transfer: exhausted.transfer,
            expected_grant: exhausted.grant,
            request: [5; 16],
        })
        .unwrap();
    assert!(exhausted.validate_successor(&granted).is_ok());
    granted.planned = None;
    assert!(exhausted.validate_successor(&granted).is_err());
}

#[cfg(unix)]
#[test]
fn a_choice_on_a_drive_without_links_is_refused_before_anything_starts() {
    // Second review R2-M3: Save as on a FAT or exFAT drive is refused at validation (read-only,
    // nothing created), so Save new copy never abandons an old partial for a save that cannot
    // finish. The FAT drive is modelled through the filesystem-class seam.
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let refused = injected(Effect::FsClass, io::ErrorKind::Unsupported, || {
        ChosenDestination::validate(&folder.join("report.bin"))
    });
    assert!(matches!(refused, Err(ChoiceRefusal::NoLinks)));
    assert!(ChoiceRefusal::NoLinks.to_string().contains("FAT"));
    assert_eq!(fs::read_dir(&folder).unwrap().count(), 0, "nothing created");
    assert!(ChosenDestination::validate(&folder.join("report.bin")).is_ok());
}
