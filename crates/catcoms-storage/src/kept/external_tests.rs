//! Platform primitives exercised on the real filesystem. These run on the Windows development
//! host; the Unix branches compile only in Linux CI.
use super::*;
use std::io::Write;

/// Make `link` a link to the directory `target`. Unprivileged Windows sessions without Developer
/// Mode cannot create symbolic links, but can always create a junction, which is the same
/// name-surrogate reparse point class for the purpose of "never follow at our name".
pub(in crate::kept) fn link_directory(target: &Path, link: &Path) -> bool {
    #[cfg(windows)]
    {
        if std::os::windows::fs::symlink_dir(target, link).is_ok() {
            return true;
        }
        std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|output| output.status.success())
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (target, link);
        false
    }
}

#[test]
fn identity_follows_the_object_not_the_name_and_counts_hard_links() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.bin");
    let second = dir.path().join("second.bin");
    let mut file = create_leaf(&first).unwrap();
    file.write_all(b"ours").unwrap();
    fs::write(&second, b"ours").unwrap();
    let ours = observe(&file).unwrap();
    assert!(ours.regular && !ours.directory);
    assert_eq!(ours.links, 1);
    assert_eq!(ours.identity.platform(), CURRENT_PLATFORM);
    let Leaf::Opened(again, reopened) = open_leaf(&first, false).unwrap() else {
        panic!("our own regular file opens");
    };
    assert_eq!(
        reopened.identity, ours.identity,
        "same object, second handle"
    );
    drop(again);
    let Leaf::Opened(_, other) = open_leaf(&second, false).unwrap() else {
        panic!("another regular file opens");
    };
    assert_ne!(
        other.identity, ours.identity,
        "equal bytes never make another file ours"
    );
    let linked = dir.path().join("linked.bin");
    fs::hard_link(&first, &linked).unwrap();
    let Leaf::Opened(_, through_link) = open_leaf(&linked, false).unwrap() else {
        panic!("hard link opens");
    };
    assert_eq!(through_link.identity, ours.identity);
    assert_eq!(observe(&file).unwrap().links, 2);
    // Replacing the name with a different object changes the identity seen at that name.
    drop(file);
    fs::remove_file(&first).unwrap();
    fs::write(&first, b"ours").unwrap();
    let Leaf::Opened(_, replaced) = open_leaf(&first, false).unwrap() else {
        panic!("replacement opens");
    };
    assert_ne!(replaced.identity, ours.identity);
}

#[test]
fn exclusive_creation_never_adopts_an_existing_entry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("taken.part");
    fs::write(&path, b"user data").unwrap();
    let error = create_leaf(&path).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&path).unwrap(), b"user data");
    let folder = dir.path().join("folder.part");
    fs::create_dir(&folder).unwrap();
    assert!(create_leaf(&folder).is_err());
    assert!(occupied(&path).unwrap() && occupied(&folder).unwrap());
    assert!(!occupied(&dir.path().join("absent")).unwrap());
}

#[test]
fn leaf_open_refuses_directories_and_links_without_following_them() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        open_leaf(&dir.path().join("absent"), true).unwrap(),
        Leaf::Missing
    ));
    let folder = dir.path().join("folder");
    fs::create_dir(&folder).unwrap();
    assert!(matches!(open_leaf(&folder, true).unwrap(), Leaf::Foreign));
    let link = dir.path().join("link.part");
    if link_directory(&folder, &link) {
        assert!(
            matches!(open_leaf(&link, true).unwrap(), Leaf::Foreign),
            "a link or junction at our name is never followed"
        );
        assert!(occupied(&link).unwrap());
    } else {
        eprintln!("skipping link refusal: this host cannot create a link or junction");
    }
}

/// Give `path` a WOF (non-surrogate) reparse point with `compact /exe`, as Windows does for
/// compressed system files. Returns false where the volume cannot (FAT, ReFS, old Windows).
#[cfg(windows)]
pub(in crate::kept) fn compact_wof(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    let compacted = std::process::Command::new("compact.exe")
        .args(["/c", "/exe:xpress4k"])
        .arg(path)
        .output()
        .is_ok_and(|output| output.status.success());
    compacted
        && fs::symlink_metadata(path).is_ok_and(|metadata| {
            u64::from(metadata.file_attributes()) & FILE_ATTRIBUTE_REPARSE_POINT != 0
        })
}

#[cfg(windows)]
#[test]
fn a_pinned_leaf_cannot_be_swapped_while_our_own_hard_link_still_works() {
    // Review M3: publication holds this handle across the path-based hard link.
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Downloads");
    fs::create_dir(&folder).unwrap();
    let staging = folder.join("notes.txt.mewtual-part");
    drop(create_leaf(&staging).unwrap());
    let Leaf::Opened(pinned, observed) = open_leaf_pinned(&staging).unwrap() else {
        panic!();
    };
    assert!(
        fs::rename(&staging, folder.join("swapped")).is_err(),
        "renaming the pinned file is refused"
    );
    assert!(fs::remove_file(&staging).is_err(), "deleting it is refused");
    assert!(
        fs::rename(&folder, dir.path().join("Elsewhere")).is_err(),
        "renaming its folder is refused"
    );
    fs::hard_link(&staging, folder.join("notes.txt")).expect("our own link still works");
    let Leaf::Opened(_, linked) = open_leaf(&folder.join("notes.txt"), false).unwrap() else {
        panic!();
    };
    assert_eq!(linked.identity, observed.identity);
    drop(pinned);
    fs::rename(&staging, folder.join("swapped")).expect("free again once the pin drops");
}

/// Create a FIFO without `unsafe` (the workspace denies it): the `mkfifo` utility.
#[cfg(unix)]
pub(in crate::kept) fn mkfifo(path: &Path) -> bool {
    std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .is_ok_and(|status| status.success())
}

/// Run `work` on its own thread and fail (rather than hang the suite) if it does not return
/// promptly. A regression that blocks on a FIFO leaks that one thread and fails here.
#[cfg(unix)]
pub(in crate::kept) fn bounded<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    let (done, wait) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(work());
    });
    wait.recv_timeout(std::time::Duration::from_secs(10))
        .expect("an open blocked on a FIFO")
}

#[cfg(unix)]
#[test]
fn a_fifo_at_a_leaf_or_folder_path_never_blocks_an_open() {
    // Review L11 and U-M6. Unix-only and compile-unverified on the Windows development host.
    let dir = tempfile::tempdir().unwrap();
    let leaf = dir.path().join("notes.txt.mewtual-part");
    let folder = dir.path().join("Downloads");
    if !mkfifo(&leaf) || !mkfifo(&folder) {
        eprintln!("skipping: mkfifo is unavailable");
        return;
    }
    let path = leaf.clone();
    assert!(matches!(
        bounded(move || open_leaf(&path, false)),
        Ok(Leaf::Foreign)
    ));
    let path = leaf.clone();
    assert!(matches!(
        bounded(move || open_leaf(&path, true)),
        Ok(Leaf::Foreign)
    ));
    let path = folder.clone();
    let refused = bounded(move || folder_identity(&path));
    assert!(refused.is_err(), "a FIFO is not a folder");
    let path = folder.clone();
    assert!(
        bounded(move || super::super::persistence::directory_barrier(&path)).is_err(),
        "the destination flush fails promptly too"
    );
}

#[test]
fn only_name_surrogate_reparse_points_make_a_file_foreign() {
    const DIRECTORY: u64 = 0x10;
    const REPARSE: u64 = 0x400;
    const ARCHIVE: u64 = 0x20;
    // A plain file, and one tagged by a cloud placeholder, WOF compression or dedup.
    assert!(regular_from(ARCHIVE, false));
    assert!(regular_from(ARCHIVE | REPARSE, false));
    // Symbolic links, junctions and mount points redirect elsewhere.
    assert!(!regular_from(ARCHIVE | REPARSE, true));
    assert!(!regular_from(DIRECTORY, false));
    assert!(!regular_from(DIRECTORY | REPARSE, true));
    // The name-surrogate flag alone, without a reparse point, is meaningless.
    assert!(regular_from(ARCHIVE, true));
}

#[cfg(windows)]
#[test]
fn a_wof_compressed_partial_is_still_ours_to_resume_and_remove() {
    // Review M2: OneDrive Files On-Demand, `compact /exe` and dedup add non-surrogate reparse
    // points to ordinary files. Our partial must not look "changed" and Remove must delete it.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt.mewtual-part");
    let mut file = create_leaf(&path).unwrap();
    file.write_all(&vec![b'a'; 256 * 1024]).unwrap();
    let identity = observe(&file).unwrap().identity;
    drop(file);
    if !compact_wof(&path) {
        eprintln!("skipping: compact /exe did not add a reparse point on this volume");
        return;
    }
    let Leaf::Opened(mut reopened, observed) = open_leaf(&path, true).unwrap() else {
        panic!("a WOF-compressed partial is still a plain file");
    };
    assert_eq!(observed.identity, identity);
    assert!(observed.regular);
    let mut head = [0u8; 4];
    std::io::Read::read_exact(&mut reopened, &mut head).unwrap();
    assert_eq!(
        &head, b"aaaa",
        "data is read through the filter, not the raw stream"
    );
    drop(reopened);
}

#[test]
fn folder_identity_follows_a_linked_folder_and_refuses_files_and_missing_folders() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = dir.path().join("Downloads");
    fs::create_dir(&downloads).unwrap();
    let identity = folder_identity(&downloads).unwrap();
    assert_eq!(folder_identity(&downloads).unwrap(), identity);
    let alias = dir.path().join("Redirected");
    if link_directory(&downloads, &alias) {
        assert_eq!(
            folder_identity(&alias).unwrap(),
            identity,
            "a redirected Downloads folder is the user's configured location"
        );
    }
    let other = dir.path().join("Other");
    fs::create_dir(&other).unwrap();
    assert_ne!(folder_identity(&other).unwrap(), identity);
    let file = dir.path().join("file.txt");
    fs::write(&file, b"x").unwrap();
    assert!(folder_identity(&file).is_err());
    let missing = folder_identity(&dir.path().join("gone")).unwrap_err();
    assert!(unreachable(&missing));
}

#[test]
fn identity_and_path_codecs_round_trip_and_refuse_foreign_shapes() {
    let identity = FileIdentity::for_test(CURRENT_PLATFORM.max(1), 0x0102_0304_0506_0708, 9);
    assert_eq!(FileIdentity::decode(&identity.encode()), Some(identity));
    let mut bad = identity.encode();
    bad[0] = 7;
    assert!(FileIdentity::decode(&bad).is_none());
    assert!(FileIdentity::decode(&identity.encode()[..24]).is_none());
    let path = OsString::from("C:\\Users\\Ünïcødé\\Downloads");
    let encoded = encode_os(&path);
    if CURRENT_PLATFORM != 0 {
        assert_eq!(decode_os(CURRENT_PLATFORM, &encoded), Some(path));
    }
    let foreign = if CURRENT_PLATFORM == PLATFORM_WINDOWS {
        PLATFORM_UNIX
    } else {
        PLATFORM_WINDOWS
    };
    assert!(
        decode_os(foreign, &encoded).is_none(),
        "another platform's path bytes are never reinterpreted"
    );
    assert!(decode_os(CURRENT_PLATFORM, &[]).is_none());
    #[cfg(windows)]
    assert!(decode_os(PLATFORM_WINDOWS, &[0x41]).is_none());
}

#[test]
fn leaf_names_are_single_safe_components_and_numbering_matches_native() {
    for good in [
        "notes.txt",
        "notes (1).txt.part",
        "mewtual-download",
        "Ünïcødé.png",
    ] {
        assert!(valid_leaf(good), "{good}");
    }
    for bad in [
        "",
        ".",
        "..",
        "a/b",
        "a\\b",
        "a:b",
        "C:",
        "trailing.",
        "trailing ",
        "tab\tname",
        "star*",
        &"x".repeat(MAX_LEAF_BYTES + 1),
    ] {
        assert!(!valid_leaf(bad), "{bad:?}");
    }
    assert_eq!(numbered("notes.txt", 0), "notes.txt");
    assert_eq!(numbered("notes.txt", 2), "notes (2).txt");
    assert_eq!(numbered("archive", 1), "archive (1)");
    assert_eq!(staging_for("notes (2).txt"), "notes (2).txt.mewtual-part");
    assert!(valid_leaf(&staging_for("notes (2).txt")));
}

#[test]
fn identity_requires_a_birth_time_and_nonzero_numbers_so_inode_reuse_cannot_match() {
    // Windows SystemTime has 100 ns resolution (NTFS FILETIME), so differ by a whole microsecond.
    let born = std::time::UNIX_EPOCH + std::time::Duration::from_micros(1_700_000_000_123);
    let ours = identity_of(PLATFORM_UNIX, 7, 42, Ok(born)).unwrap();
    let reused = identity_of(
        PLATFORM_UNIX,
        7,
        42,
        Ok(born + std::time::Duration::from_micros(1)),
    )
    .unwrap();
    assert_ne!(
        ours, reused,
        "a later file that inherits (dev, ino) differs by birth time"
    );
    for refused in [
        identity_of(PLATFORM_UNIX, 0, 42, Ok(born)),
        identity_of(PLATFORM_UNIX, 7, 0, Ok(born)),
        identity_of(PLATFORM_UNIX, 7, 42, Ok(std::time::UNIX_EPOCH)),
        identity_of(
            PLATFORM_UNIX,
            7,
            42,
            Err(io::Error::from(io::ErrorKind::Unsupported)),
        ),
    ] {
        assert_eq!(refused.unwrap_err().kind(), io::ErrorKind::Unsupported);
    }
}
