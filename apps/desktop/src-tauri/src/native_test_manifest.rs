//! Reuse the exact resources Tauri already generated, for the Windows libtest.
//! Cargo's integration-test linker flags do not reach the library test harness.
//! This link is absent from shipped library/application builds; their existing
//! Tauri manifest, icon and linker path remain authoritative.

// Despite the historical `dylib` spelling, MSVC consumes resource.lib as a .res
// input. This is the same mechanism embed-resource uses for library-only crates.
// `static` would ask rustc to bundle it as a COFF archive, which it is not.
// The generated literal names the original absolute output, not a generic
// resource.lib search. It is compiler input under Cargo's OUT_DIR only.
include!(concat!(env!("OUT_DIR"), "/native_test_manifest_link.rs"));

#[link(name = "comctl32")]
unsafe extern "system" {
    fn TaskDialogIndirect(
        config: *const std::ffi::c_void,
        button: *mut i32,
        radio: *mut i32,
        checked: *mut i32,
    ) -> i32;
}

#[test]
fn native_dialog_entrypoint_is_loadable_with_the_original_manifest() {
    // Do not open a dialog. Keeping the actual symbol import means Windows must
    // resolve TaskDialogIndirect BEFORE entering libtest; absent the embedded v6
    // dependency, the process fails with STATUS_ENTRYPOINT_NOT_FOUND. black_box
    // prevents this address check being folded away and dropping the regression.
    let entrypoint = std::hint::black_box(TaskDialogIndirect as *const ());
    assert!(!entrypoint.is_null());
}
