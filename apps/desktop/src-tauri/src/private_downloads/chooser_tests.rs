//! "Save as" through the [`DestinationChooser`] seam: no real dialog ever opens. One sequential
//! test, because the one-dialog turn is process-wide and parallel tests would contend for it.
use super::*;
use catcoms_storage::kept::ChoiceRefusal;

/// Answers with a scripted result, optionally waiting on a gate first (a dialog left open).
struct Scripted {
    answer: StdMutex<Option<Result<Option<PathBuf>, String>>>,
    asked: StdMutex<Vec<(PathBuf, String)>>,
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
        directory: PathBuf,
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
    let picked = choose_destination(&cancel, downloads.clone(), "a/b:c.txt")
        .await
        .unwrap();
    assert!(picked.is_none());
    assert_eq!(
        cancel.asked.lock().unwrap()[0],
        (downloads.clone(), "b_c.txt".to_string())
    );
    assert_eq!(std::fs::read_dir(&chosen).unwrap().count(), 0);

    // An existing name is refused, even though the OS dialog already asked to replace it, and the
    // file is left byte-identical.
    let existing = chosen.join("report.bin");
    std::fs::write(&existing, b"the user's own report").unwrap();
    let replace = Scripted::answering(Ok(Some(existing.clone())));
    let refused = choose_destination(&replace, downloads.clone(), "report.bin")
        .await
        .unwrap_err();
    assert_eq!(refused, ChoiceRefusal::Exists.to_string());
    assert_eq!(std::fs::read(&existing).unwrap(), b"the user's own report");

    // A usable choice comes back validated, with the exact name the user typed.
    let fresh = Scripted::answering(Ok(Some(chosen.join("Renamed.bin"))));
    let choice = choose_destination(&fresh, downloads.clone(), "report.bin")
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
        choose_destination(&relative, downloads.clone(), "x")
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
        let downloads = downloads.clone();
        async move { choose_destination(&*open, downloads, "one.bin").await }
    });
    while open.asked.lock().unwrap().is_empty() {
        tokio::task::yield_now().await;
    }
    let second = Scripted::answering(Ok(None));
    assert_eq!(
        choose_destination(&second, downloads.clone(), "two.bin")
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
    assert!(choose_destination(&again, downloads.clone(), "three.bin")
        .await
        .unwrap()
        .is_none());

    // A chooser error releases the turn too.
    let broken = Scripted::answering(Err("The Save as window could not be opened.".into()));
    assert!(choose_destination(&broken, downloads.clone(), "x")
        .await
        .is_err());
    let after = Scripted::answering(Ok(None));
    assert!(choose_destination(&after, downloads, "x").await.is_ok());
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
