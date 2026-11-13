//! Native tar's actual xattr dependency must stay compatible with patched rustix.
//! A regular ext4 inode often cannot hold a >4KiB xattr, so this Linux regression
//! explicitly requires the runner's /dev/shm tmpfs rather than silently skipping
//! the fallback it is intended to test. Windows has no POSIX xattr path.
#![cfg(target_os = "linux")]

use std::collections::BTreeSet;
use std::fs::File;
use xattr::FileExt;

#[test]
fn native_large_xattr_value_and_list_survive_the_rustix_query_api_change() {
    let folder =
        tempfile::tempdir_in("/dev/shm").expect("Linux xattr regression needs /dev/shm tmpfs");
    let file = File::create(folder.path().join("only-owned-test-file")).unwrap();
    let value: Vec<u8> = (0..5109).map(|i| (i % 251) as u8).collect();
    let large = "user.mewtual.native-query-compat.large";
    file.set_xattr(large, &value).unwrap();
    assert_eq!(
        file.get_xattr(large).unwrap().as_deref(),
        Some(value.as_slice())
    );

    // Exercise the separate >4KiB name-list fallback as well as the value path.
    let mut expected = BTreeSet::from([large.to_string()]);
    for index in 0..80 {
        let name = format!(
            "user.mewtual.native-query-compat.{index:03}.{}",
            "n".repeat(64)
        );
        file.set_xattr(&name, b"small").unwrap();
        expected.insert(name);
    }
    assert!(expected.iter().map(|name| name.len() + 1).sum::<usize>() > 4096);
    let actual: BTreeSet<String> = file
        .list_xattr()
        .unwrap()
        .filter_map(|name| name.into_string().ok())
        .filter(|name| name.starts_with("user.mewtual.native-query-compat."))
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(
        file.get_xattr(large).unwrap().as_deref(),
        Some(value.as_slice())
    );
}
