fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "get_daemon_lifecycle",
            "retry_daemon",
            "daemon_request",
            "select_import_root",
            "list_managed_roots",
            "import_selected_root",
            "reauthorize_managed_root",
            "export_diagnostics",
        ]),
    ))
    .expect("failed to build the resume-ir desktop manifest")
}
