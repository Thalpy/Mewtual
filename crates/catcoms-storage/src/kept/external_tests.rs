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
fn long_multibyte_names_are_shortened_to_leave_room_for_every_suffix() {
    // Review U-L3: the staging, probe and numbering suffixes must fit the component limit in
    // this platform's units (UTF-16 on Windows, bytes elsewhere), keeping the extension.
    assert_eq!(fit_leaf("notes.txt"), "notes.txt");
    let emoji = |count: usize| format!("{}.txt", "\u{1F600}".repeat(count));
    for name in [
        emoji(62),
        emoji(120),
        format!("{}.pdf", "a".repeat(300)),
        format!("{}. .txt", "\u{4E2D}".repeat(200)),
    ] {
        let fitted = fit_leaf(&name);
        assert!(leaf_fits(&fitted), "{} units", component_units(&fitted));
        for derived in [
            numbered(&fitted, 999),
            staging_for(&fitted),
            format!("{}-probe3", staging_for(&fitted)),
        ] {
            assert!(component_units(&derived) <= 255, "{derived}");
        }
        assert!(valid_leaf(&fitted) && valid_leaf(&staging_for(&fitted)));
        assert_eq!(
            Path::new(&fitted).extension(),
            Path::new(&name).extension(),
            "the extension is kept"
        );
        if leaf_fits(&name) {
            assert_eq!(fitted, name, "a name that already fits is unchanged");
        }
    }
    // 120 non-BMP characters (240 UTF-16 units, 480 bytes) never fit as they are.
    assert!(!leaf_fits(&emoji(120)));
    assert_eq!(
        fit_leaf("..."),
        "...",
        "fits already; validity is checked separately"
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

const ME: u32 = 1000;
const MY_GROUP: u32 = 1000;

/// [`shared_folder`] for a folder with this owner, group and mode, where the private-group proof
/// answers `private` (and records whether it was asked at all).
fn shared(owner: u32, group: u32, mode: u32, private: bool) -> (bool, bool) {
    let asked = std::cell::Cell::new(false);
    let result = shared_folder(FolderOwnership { owner, group, mode }, ME, MY_GROUP, |_| {
        asked.set(true);
        private
    });
    (result, asked.get())
}

#[test]
fn other_accounts_rule_follows_owner_and_mode_bits() {
    // PR51 H1 (Unix): automatic deletion is refused in a folder other accounts can modify.
    let is_shared = |owner, group, mode, private| shared(owner, group, mode, private).0;
    assert!(
        !is_shared(ME, MY_GROUP, 0o755, false),
        "a private home folder"
    );
    assert!(!is_shared(ME, MY_GROUP, 0o700, false));
    assert!(
        !is_shared(0, 0, 0o755, false),
        "root-owned, not writable by others"
    );
    assert!(
        !is_shared(0, 0, 0o1777, false),
        "sticky /tmp: others cannot move our entries"
    );
    assert!(
        is_shared(0, 0, 0o777, true),
        "world-writable without sticky"
    );
    assert!(
        is_shared(ME, MY_GROUP, 0o757, true),
        "other-writable is shared even with a private group"
    );
    assert!(is_shared(ME, MY_GROUP, 0o777, true));
    assert!(
        !is_shared(ME, MY_GROUP, 0o1777, true),
        "world-writable but sticky"
    );
    assert!(
        is_shared(2000, MY_GROUP, 0o755, true),
        "owned by another account"
    );
    assert!(
        is_shared(2000, 2000, 0o1777, true),
        "another account owns even a sticky folder"
    );
}

#[test]
fn a_group_writable_folder_is_private_only_for_a_proven_user_private_group() {
    // User decision, 23 October: 0775 counts as private only when the folder is ours, its group
    // is our primary group, and that group provably has no other member.
    assert_eq!(
        shared(ME, MY_GROUP, 0o775, true),
        (false, true),
        "a user-private group"
    );
    assert_eq!(
        shared(ME, MY_GROUP, 0o775, false),
        (true, true),
        "membership not proven: shared"
    );
    assert!(
        shared(ME, 2000, 0o775, true).0,
        "another group, whatever its members"
    );
    assert!(
        shared(0, MY_GROUP, 0o775, true).0,
        "owned by root, not by us"
    );
    // The proof is consulted only when everything else already holds.
    assert!(!shared(ME, MY_GROUP, 0o755, false).1);
    assert!(!shared(ME, 2000, 0o775, true).1);
}

/// Account files for a host where `alice` (uid and gid 1000) has a user-private group.
fn accounts(passwd: &str, group: &str, nsswitch: NsSwitch) -> AccountFiles {
    AccountFiles {
        passwd: Some(passwd.to_owned()),
        group: Some(group.to_owned()),
        nsswitch,
        userdb_records: Some(false),
        host_namespace: true,
    }
}
const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\n\
    alice:x:1000:1000:Alice:/home/alice:/bin/bash\n\
    bob:x:1001:1001:Bob:/home/bob:/bin/bash\n";
const GROUP: &str = "root:x:0:\nsudo:x:27:alice\nalice:x:1000:\nbob:x:1001:\n";
const NSSWITCH: &str = "# comment\npasswd:         files systemd\ngroup:          files [SUCCESS=merge] systemd\nhosts: files dns\n";

#[test]
fn a_user_private_group_is_proven_only_from_the_local_files() {
    let text = || NsSwitch::Text(NSSWITCH.into());
    assert!(group_is_private(
        &accounts(PASSWD, GROUP, text()),
        MY_GROUP,
        ME
    ));
    assert!(
        group_is_private(&accounts(PASSWD, GROUP, NsSwitch::Absent), MY_GROUP, ME),
        "no nsswitch.conf: the files are the only source"
    );
    // Listing ourselves as a supplementary member is still a group of one.
    let self_member = GROUP.replace("alice:x:1000:", "alice:x:1000:alice");
    assert!(group_is_private(
        &accounts(PASSWD, &self_member, text()),
        MY_GROUP,
        ME
    ));
    // A differently named group of one is private too: the name is only a supporting signal.
    let renamed = GROUP.replace("alice:x:1000:", "staff-alice:x:1000:");
    assert!(group_is_private(
        &accounts(PASSWD, &renamed, text()),
        MY_GROUP,
        ME
    ));

    // Another supplementary member.
    let shared_member = GROUP.replace("alice:x:1000:", "alice:x:1000:bob");
    assert!(!group_is_private(
        &accounts(PASSWD, &shared_member, text()),
        MY_GROUP,
        ME
    ));
    // Another account's primary group.
    let other_primary = PASSWD.replace("bob:x:1001:1001", "bob:x:1001:1000");
    assert!(!group_is_private(
        &accounts(&other_primary, GROUP, text()),
        MY_GROUP,
        ME
    ));
    // macOS: everyone's primary group is the shared `staff` (gid 20).
    let staff = "root:*:0:0:System Administrator:/var/root:/bin/sh\n\
        alice:*:1000:20:Alice:/Users/alice:/bin/zsh\n\
        bob:*:1001:20:Bob:/Users/bob:/bin/zsh\n";
    let staff_group = "staff:*:20:root\n";
    assert!(!group_is_private(
        &accounts(staff, staff_group, NsSwitch::Absent),
        20,
        ME
    ));
}

#[test]
fn unprovable_membership_fails_closed() {
    let text = |value: &str| NsSwitch::Text(value.into());
    let private = |files: AccountFiles| group_is_private(&files, MY_GROUP, ME);
    // Directory services could add members Mewtual cannot see.
    for nss in [
        "passwd: files sss\ngroup: files sss\n",
        "passwd: files\ngroup: files ldap\n",
        "passwd: compat\ngroup: nis files\n",
        "passwd: files\n",
        "group: files\n",
        "passwd:\ngroup: files\n",
    ] {
        assert!(!private(accounts(PASSWD, GROUP, text(nss))), "{nss:?}");
    }
    assert!(!private(accounts(PASSWD, GROUP, NsSwitch::Unreadable)));
    // The group resolves only through NSS (no local entry), or twice.
    let without = GROUP.replace("alice:x:1000:\n", "");
    assert!(!private(accounts(PASSWD, &without, NsSwitch::Absent)));
    let twice = format!("{GROUP}alice2:x:1000:\n");
    assert!(!private(accounts(PASSWD, &twice, NsSwitch::Absent)));
    // Our own account is not in the local files.
    let no_me = PASSWD.replace("alice:x:1000:1000:Alice:/home/alice:/bin/bash\n", "");
    assert!(!private(accounts(&no_me, GROUP, NsSwitch::Absent)));
    // NIS compat lines and malformed lines.
    assert!(!private(accounts(
        &format!("{PASSWD}+::::::\n"),
        GROUP,
        NsSwitch::Absent
    )));
    assert!(!private(accounts(
        PASSWD,
        &format!("{GROUP}+:::\n"),
        NsSwitch::Absent
    )));
    assert!(!private(accounts(
        &format!("{PASSWD}broken:line\n"),
        GROUP,
        NsSwitch::Absent
    )));
    assert!(!private(accounts(
        PASSWD,
        &format!("{GROUP}x:x:notanumber:\n"),
        NsSwitch::Absent
    )));
    // Unreadable files.
    for (passwd, group) in [(None, Some(GROUP)), (Some(PASSWD), None)] {
        let files = AccountFiles {
            passwd: passwd.map(str::to_owned),
            group: group.map(str::to_owned),
            ..accounts(PASSWD, GROUP, NsSwitch::Absent)
        };
        assert!(!private(files));
    }
}

#[test]
fn directory_services_cannot_hide_behind_nsswitch_syntax() {
    // Decisions review M2: glibc ends a service name at `[` and starts one right after `]`, and
    // `initgroups` supplies supplementary groups at login.
    let text = |value: &str| NsSwitch::Text(value.into());
    let private = |files: AccountFiles| group_is_private(&files, MY_GROUP, ME);
    for nss in [
        "passwd: files\ngroup: files ldap[NOTFOUND=return]\n",
        "passwd: files\ngroup: files [SUCCESS=merge]sss\n",
        "passwd: files\ngroup: files\ninitgroups: files sss\n",
        "passwd: files\ngroup: files [NOTFOUND=return\n",
        "passwd: files\ngroup: files ]\n",
        "passwd: files\ngroup: files\ninitgroups:\n",
    ] {
        assert!(!private(accounts(PASSWD, GROUP, text(nss))), "{nss:?}");
    }
    assert!(private(accounts(
        PASSWD,
        GROUP,
        text("passwd: files\ngroup: files [NOTFOUND=return]\ninitgroups: files\n")
    )));
    assert_eq!(
        nss_services("files ldap[NOTFOUND=return] [SUCCESS=merge]sss  systemd"),
        Some(vec!["files", "ldap", "sss", "systemd"])
    );
    assert_eq!(nss_services("files [x"), None);
}

#[test]
fn systemd_accounts_and_containers_fail_closed() {
    // Decisions review M3: nss-systemd also serves userdb records and systemd-homed, which can add
    // members or primary groups that /etc never shows. M4: inside a user namespace or a container,
    // /etc does not describe the accounts that can reach a bind-mounted folder.
    let systemd = || NsSwitch::Text(NSSWITCH.into());
    let with = |userdb_records, host_namespace| AccountFiles {
        userdb_records,
        host_namespace,
        ..accounts(PASSWD, GROUP, systemd())
    };
    assert!(group_is_private(&with(Some(false), true), MY_GROUP, ME));
    assert!(!group_is_private(&with(Some(true), true), MY_GROUP, ME));
    assert!(!group_is_private(&with(None, true), MY_GROUP, ME));
    assert!(!group_is_private(&with(Some(false), false), MY_GROUP, ME));
    // Without `systemd` listed, its records are not consulted.
    let files_only = AccountFiles {
        userdb_records: None,
        ..accounts(
            PASSWD,
            GROUP,
            NsSwitch::Text("passwd: files\ngroup: files\n".into()),
        )
    };
    assert!(group_is_private(&files_only, MY_GROUP, ME));
    // The identity map of the initial namespace, and nothing else.
    assert!(identity_map("         0          0 4294967295\n"));
    assert!(!identity_map("0 100000 65536\n"));
    assert!(!identity_map("0 0 4294967295\n1 1 1\n"));
    assert!(!identity_map(""));
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn an_extended_acl_is_never_ruled_out_without_proof() {
    // Decisions review HIGH: with an extended POSIX ACL the group bits are the ACL mask, which can
    // carry another account's write access.
    assert!(acl_present_from(Ok(0)));
    assert!(acl_present_from(Ok(44)));
    assert!(!acl_present_from(Err(rustix::io::Errno::NODATA)));
    assert!(!acl_present_from(Err(rustix::io::Errno::OPNOTSUPP)));
    assert!(acl_present_from(Err(rustix::io::Errno::ACCESS)));
}

#[cfg(unix)]
#[test]
fn the_account_files_are_read_bounded_from_the_root() {
    // Decisions review L5: the production loader, pointed at a prepared root.
    let root = tempfile::tempdir().unwrap();
    let put = |path: &str, text: &str| {
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    put("etc/passwd", PASSWD);
    put("etc/group", GROUP);
    put("proc/self/uid_map", "0 0 4294967295\n");
    put("proc/self/gid_map", "0 0 4294967295\n");
    let files = load_account_files(root.path());
    assert!(matches!(files.nsswitch, NsSwitch::Absent));
    assert_eq!(files.userdb_records, Some(false));
    assert!(files.host_namespace);
    assert!(group_is_private(&files, MY_GROUP, ME));
    // A userdb record, a container marker, a non-identity map, an oversized file.
    put("usr/lib/userdb/bob.user", "{}");
    put("etc/nsswitch.conf", NSSWITCH);
    let files = load_account_files(root.path());
    assert_eq!(files.userdb_records, Some(true));
    assert!(!group_is_private(&files, MY_GROUP, ME));
    for marker in [
        "run/.containerenv",
        ".dockerenv",
        "run/systemd/container",
        "run/host/container-manager",
    ] {
        put(marker, "");
        assert!(!load_account_files(root.path()).host_namespace, "{marker}");
        fs::remove_file(root.path().join(marker)).unwrap();
    }
    assert!(load_account_files(root.path()).host_namespace);
    put("proc/self/uid_map", "0 100000 65536\n");
    assert!(!load_account_files(root.path()).host_namespace);
    put("etc/passwd", &"x".repeat(1024 * 1024 + 1));
    assert!(load_account_files(root.path()).passwd.is_none());
}

#[test]
fn filesystem_classes_fail_closed() {
    // PR51 M3: only filesystems with stable identities and hard links get a journal.
    assert_eq!(classify_linux_magic(0xEF53), FsClass::Journal, "ext4");
    assert_eq!(classify_linux_magic(0x5846_5342), FsClass::Journal, "xfs");
    assert_eq!(classify_linux_magic(0x9123_683E), FsClass::Journal, "btrfs");
    assert_eq!(classify_linux_magic(0x0102_1994), FsClass::Journal, "tmpfs");
    assert_eq!(classify_linux_magic(0x4D44), FsClass::NoLinks, "msdos/vfat");
    assert_eq!(classify_linux_magic(0x2011_BAB0), FsClass::NoLinks, "exfat");
    assert_eq!(classify_linux_magic(0x6969), FsClass::Unknown, "nfs");
    assert_eq!(classify_linux_magic(0xFF53_4D42), FsClass::Unknown, "cifs");
    assert_eq!(classify_linux_magic(0x6573_5546), FsClass::Unknown, "fuse");
    assert_eq!(classify_apple("apfs", true), FsClass::Journal);
    assert_eq!(classify_apple("apfs", false), FsClass::Unknown, "not local");
    assert_eq!(classify_apple("smbfs", false), FsClass::Unknown);
    assert_eq!(classify_apple("msdos", true), FsClass::NoLinks);
    assert_eq!(classify_apple("exfat", true), FsClass::NoLinks);
    assert_eq!(
        classify_windows_canonical(r"\\?\C:\Users\me\Downloads"),
        FsClass::Journal
    );
    assert_eq!(
        classify_windows_canonical(r"\\?\UNC\server\share\Downloads"),
        FsClass::Unknown,
        "a mapped network drive canonicalizes to UNC"
    );
    assert_eq!(
        classify_windows_canonical(r"\\server\share"),
        FsClass::Unknown
    );
    assert_eq!(
        classify_windows_canonical(r"\\?\unc\wsl$\Ubuntu"),
        FsClass::Unknown
    );
    // The real local test folder classifies as journal-capable on the hosts CI uses.
    let dir = tempfile::tempdir().unwrap();
    assert_ne!(fs_class(dir.path()).unwrap(), FsClass::NoLinks);
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
