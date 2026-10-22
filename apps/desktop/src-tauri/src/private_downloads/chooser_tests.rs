//! "Save as" through the [`DestinationChooser`] seam: no real dialog ever opens. One sequential
//! test, because the one-dialog turn is process-wide and parallel tests would contend for it.
use super::*;
use catcoms_storage::kept::ChoiceRefusal;

/// Answers with a scripted result, optionally waiting on a gate first (a dialog left open).
struct Scripted {
    answer: StdMutex<Option<Result<Option<PathBuf>, String>>>,
    asked: StdMutex<Vec<(Option<PathBuf>, String)>>,
    gate: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}
impl Scripted {
    fn answering(answer: Result<Option<PathBuf>, String>) -> Self {
        Self {
            answer: StdMutex::new(Some(answer)),
            asked: StdMutex::new(Vec::new()),
            gate: tokio::sync::Mutex::new(None),
        }
    }
}
#[async_trait::async_trait]
impl DestinationChooser for Scripted {
    async fn choose(
        &self,
        directory: Option<PathBuf>,
        suggested_name: String,
    ) -> Result<Option<PathBuf>, String> {
        self.asked.lock().unwrap().push((directory, suggested_name));
        if let Some(gate) = self.gate.lock().await.take() {
            let _ = gate.await;
        }
        self.answer.lock().unwrap().take().expect("asked once")
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_as_selection_through_the_chooser_seam() {
    let root = tempfile::tempdir().unwrap();
    let downloads = root.path().join("Downloads");
    let chosen = root.path().join("Chosen place");
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::create_dir_all(&chosen).unwrap();

    // Defaults: the dialog starts in Downloads with the sanitized peer name. Cancel changes
    // nothing and creates nothing.
    let cancel = Scripted::answering(Ok(None));
    let picked = choose_destination(&cancel, Some(downloads.clone()), "a/b:c.txt")
        .await
        .unwrap();
    assert!(picked.is_none());
    assert_eq!(
        cancel.asked.lock().unwrap()[0],
        (Some(downloads.clone()), "b_c.txt".to_string())
    );
    assert_eq!(std::fs::read_dir(&chosen).unwrap().count(), 0);

    // An existing name is refused, even though the OS dialog already asked to replace it, and the
    // file is left byte-identical.
    let existing = chosen.join("report.bin");
    std::fs::write(&existing, b"the user's own report").unwrap();
    let replace = Scripted::answering(Ok(Some(existing.clone())));
    let refused = choose_destination(&replace, Some(downloads.clone()), "report.bin")
        .await
        .unwrap_err();
    assert_eq!(refused, ChoiceRefusal::Exists.to_string());
    assert_eq!(std::fs::read(&existing).unwrap(), b"the user's own report");

    // A usable choice comes back validated, with the exact name the user typed.
    let fresh = Scripted::answering(Ok(Some(chosen.join("Renamed.bin"))));
    let choice = choose_destination(&fresh, Some(downloads.clone()), "report.bin")
        .await
        .unwrap()
        .expect("a valid choice");
    assert_eq!(choice.name(), "Renamed.bin");
    assert_eq!(
        std::fs::read_dir(&chosen).unwrap().count(),
        1,
        "nothing created"
    );

    // A relative or otherwise unusable result is refused.
    let relative = Scripted::answering(Ok(Some(PathBuf::from("relative.bin"))));
    assert_eq!(
        choose_destination(&relative, Some(downloads.clone()), "x")
            .await
            .unwrap_err(),
        ChoiceRefusal::InvalidName.to_string()
    );

    // One dialog at a time (picker review M4): while one is open, another is refused; after it
    // closes, a new one may open.
    let (release, gate) = tokio::sync::oneshot::channel();
    let open = Arc::new(Scripted::answering(Ok(None)));
    *open.gate.lock().await = Some(gate);
    let first = tokio::spawn({
        let open = open.clone();
        let downloads = Some(downloads.clone());
        async move { choose_destination(&*open, downloads, "one.bin").await }
    });
    while open.asked.lock().unwrap().is_empty() {
        tokio::task::yield_now().await;
    }
    let second = Scripted::answering(Ok(None));
    assert_eq!(
        choose_destination(&second, Some(downloads.clone()), "two.bin")
            .await
            .unwrap_err(),
        DIALOG_BUSY
    );
    assert!(
        second.asked.lock().unwrap().is_empty(),
        "no second dialog opened"
    );
    release.send(()).unwrap();
    assert!(first.await.unwrap().unwrap().is_none());
    let again = Scripted::answering(Ok(None));
    assert!(
        choose_destination(&again, Some(downloads.clone()), "three.bin")
            .await
            .unwrap()
            .is_none()
    );

    // A chooser error releases the turn too.
    let broken = Scripted::answering(Err("The Save as window could not be opened.".into()));
    assert!(choose_destination(&broken, Some(downloads.clone()), "x")
        .await
        .is_err());
    let after = Scripted::answering(Ok(None));
    assert!(choose_destination(&after, Some(downloads), "x")
        .await
        .is_ok());

    // Picker review L6: with no resolvable Downloads folder the dialog still opens, at the OS
    // default, and its choice is validated the same way.
    let nowhere = Scripted::answering(Ok(Some(chosen.join("Without downloads.bin"))));
    let choice = choose_destination(&nowhere, None, "report.bin")
        .await
        .unwrap()
        .expect("Save as works without a Downloads folder");
    assert_eq!(choice.name(), "Without downloads.bin");
    assert_eq!(
        nowhere.asked.lock().unwrap()[0],
        (None, "report.bin".to_string())
    );

    // A peer-chosen name never suggests `%VAR%`, which the Windows dialog would expand into a
    // path the user did not see.
    let expand = Scripted::answering(Ok(None));
    choose_destination(&expand, None, "%APPDATA%_x.txt")
        .await
        .unwrap();
    assert_eq!(expand.asked.lock().unwrap()[0].1, "_APPDATA__x.txt");
}

#[test]
fn the_record_not_the_invocation_selects_where_a_fresh_save_goes() {
    // Picker review coverage: the selection that used to be inline in save_group_file.
    let root = tempfile::tempdir().unwrap();
    let downloads = root.path().join("Downloads");
    let choice = Arc::new(ChosenDestination::validate(&root.path().join("chosen.bin")).unwrap());
    let unused = || -> Result<PathBuf, String> { panic!("Downloads is not the destination") };

    // A remembered choice wins, whether this invocation chose it or an earlier one did.
    for chose_now in [true, false] {
        match select_destination(Some(Ok(choice.clone())), chose_now, unused) {
            Ok(Destination::Chosen(picked)) => assert_eq!(*picked, *choice),
            _ => panic!("the remembered choice is the destination"),
        }
    }
    // A choice this platform cannot decode is refused, never redirected to Downloads.
    assert_eq!(
        select_destination(Some(Err(ChoiceRefusal::FolderUnavailable)), false, unused).err(),
        Some(ChoiceRefusal::FolderUnavailable.to_string())
    );
    // This Save as found its choice already consumed by another save of the same file.
    assert_eq!(
        select_destination(None, true, unused).err().as_deref(),
        Some(PLACE_CHANGED)
    );
    // The one-click default resolves Downloads only now, and reports its failure.
    match select_destination(None, false, || Ok(downloads.clone())) {
        Ok(Destination::Downloads(folder)) => assert_eq!(folder, downloads),
        _ => panic!("the one-click default is Downloads"),
    }
    assert_eq!(
        select_destination(None, false, || Err("no Downloads folder".into()))
            .err()
            .as_deref(),
        Some("no Downloads folder")
    );
}

#[test]
fn a_save_counts_as_in_downloads_only_directly_inside_the_downloads_folder() {
    // Picker re-review LOW 1b and its re-check: the result's inDownloads flag fails closed.
    let root = tempfile::tempdir().unwrap();
    let downloads = root.path().join("Downloads");
    assert!(saved_in_downloads(
        Some(&downloads),
        &downloads.join("c (1).bin")
    ));
    // A resumed or recovered journal kept its own (earlier chosen) folder.
    assert!(!saved_in_downloads(
        Some(&downloads),
        &root.path().join("Chosen place").join("c.bin")
    ));
    assert!(!saved_in_downloads(
        Some(&downloads),
        &downloads.join("nested").join("c.bin")
    ));
    // No resolvable Downloads folder asserts nothing.
    assert!(!saved_in_downloads(None, &downloads.join("c.bin")));
}

#[test]
fn the_projection_says_a_choice_is_remembered_without_naming_it() {
    let status = DownloadStatus {
        id: DownloadId::from_bytes([5; 32]),
        state: catcoms_storage::kept::KeptTransfer {
            plan: catcoms_storage::kept::KeepPlan {
                cid: catcoms_storage::Cid::of(b"plaintext"),
                version: [1; 32],
                manifest: Vec::new(),
                chunks: vec![(catcoms_storage::Cid::of(b"a"), 5)],
            },
            listing: [0; 32],
            transfer: [5; 32],
            phase: KeepPhase::Resumable,
            verified_chunks: 0,
            running: false,
            user_paused: false,
            blocked: false,
            intent: 2,
            exhausted: false,
            grant: 1,
            remaining: catcoms_storage::kept::KeepAllowance {
                requests: 1,
                pages: 1,
                bytes: 1,
                slices: 1,
            },
        },
        checked: false,
        export: ExportView::None,
        planned: true,
    };
    let json = serde_json::to_string(&project(status)).unwrap();
    assert!(json.contains("\"chosenPlace\":true"), "{json}");
    assert!(!json.contains("path") && !json.contains("folder"), "{json}");
}
