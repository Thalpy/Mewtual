//! "Save as": a destination the user picked in the native dialog. Uses the parent test module's
//! store and plan helpers; the dialog itself is never involved at this layer.
use super::*;

fn chosen_folder(fx: &Fixture) -> PathBuf {
    let folder = fx.downloads.with_file_name("Chosen place");
    fs::create_dir_all(&folder).unwrap();
    folder
}
fn begin_chosen(
    store: &mut KeptBlobStore<ChaCha20Rng>,
    lease: &DownloadExportLease,
    choice: &ChosenDestination,
) -> ExportFile {
    let file = ExportFile::create_chosen(choice, export_plan()).unwrap();
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
    assert!(ChosenDestination::validate(&folder.join("COM10.txt")).is_ok());
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
    file.finish();
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
        ExportFile::create_chosen(&choice, export_plan()),
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
        ExportFile::create_chosen(&choice, export_plan()),
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
        with_identity_unsupported(1, || ExportFile::create_chosen(&choice, export_plan())),
        Err(ExportError::Refused(ChoiceRefusal::FolderChanged))
    ));
    assert!(matches!(
        with_identity_unsupported(1, || choice.create_unjournaled()),
        Err(ExportError::Refused(ChoiceRefusal::FolderChanged))
    ));
    assert_eq!(fs::read_dir(&folder).unwrap().count(), 0);
}

#[test]
fn a_chosen_folder_without_links_is_refused_before_any_plaintext_exists() {
    let fx = fixture();
    let folder = chosen_folder(&fx);
    let choice = ChosenDestination::validate(&folder.join("report.bin")).unwrap();
    assert!(matches!(
        injected(Effect::Link, io::ErrorKind::Unsupported, || {
            ExportFile::create_chosen(&choice, export_plan())
        }),
        Err(ExportError::NoLinks)
    ));
    assert_eq!(fs::read_dir(&folder).unwrap().count(), 0);
    // The same refusal on an identity-less folder, through the non-resumable reservation.
    let choice = identityless(&folder, "report.bin");
    assert!(matches!(
        injected(Effect::Link, io::ErrorKind::PermissionDenied, || {
            choice.create_unjournaled()
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
        ExportFile::create_chosen(&choice, export_plan()),
        Err(ExportError::Unsupported)
    ));
    assert_eq!(
        fs::read_dir(&folder).unwrap().count(),
        0,
        "nothing left behind"
    );
    // Exact name, beside it, no numbering.
    let (file, staging, final_path) = choice.create_unjournaled().unwrap();
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
        choice.create_unjournaled(),
        Err(ExportError::Refused(ChoiceRefusal::PartialExists))
    ));
    fs::remove_file(&staging).unwrap();
    // The chosen name taken after selection is refused and left byte-identical.
    fs::write(&final_path, b"someone else's").unwrap();
    assert!(matches!(
        choice.create_unjournaled(),
        Err(ExportError::Refused(ChoiceRefusal::Exists))
    ));
    assert_eq!(fs::read(&final_path).unwrap(), b"someone else's");
    assert!(!staging.exists());
    // A removed folder is refused, never recreated.
    fs::remove_file(&final_path).unwrap();
    fs::remove_dir(&folder).unwrap();
    assert!(matches!(
        choice.create_unjournaled(),
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
    let downloads = ExportFile::create(&fx.downloads, "notes.txt", export_plan()).unwrap();
    assert!(store
        .begin_download_export(&lease, downloads.journal(), downloads.presence())
        .is_err());
    downloads.discard();
    // Nor can a journal for a different choice in the same folder.
    let other = ChosenDestination::validate(&folder.join("other.bin")).unwrap();
    let wrong = ExportFile::create_chosen(&other, export_plan()).unwrap();
    assert!(store
        .begin_download_export(&lease, wrong.journal(), wrong.presence())
        .is_err());
    wrong.discard();
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
        ExportFile::create_chosen(&choice, export_plan())
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
