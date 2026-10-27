fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "bootstrap",
            "inspect",
            "release_endpoint",
            "finish_probe",
        ]),
    ))
    .expect("probe ACL manifest");
}
