mod bridge_admission;
mod daemon_client;
mod daemon_connection;
mod daemon_exchange;
mod daemon_lifecycle;
mod daemon_request;
mod daemon_response;
mod native_import;
mod runtime_state;
mod source_reveal;

use bridge_admission::{lane_for_operation, BridgeAdmissionState, BridgeLane};
use daemon_client::{DesktopError, DesktopRequest, DesktopResponse};
use daemon_exchange::SearchSelection;
use daemon_lifecycle::{DaemonLifecycleSnapshot, DaemonLifecycleState};
use native_import::{DiagnosticsExportReceipt, LegacyManagedRootsMigration};
use runtime_state::DesktopRuntimeState;
use tauri::path::BaseDirectory;
use tauri::Manager;

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct ManagedRootHandleRequest {
    root_id: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RevealSourceRequest {
    selection: SearchSelection,
}

#[tauri::command]
async fn daemon_request(
    request: DesktopRequest,
    app: tauri::AppHandle,
    admission: tauri::State<'_, BridgeAdmissionState>,
    runtime: tauri::State<'_, DesktopRuntimeState>,
) -> Result<DesktopResponse, DesktopError> {
    let _permit = admission.try_acquire(lane_for_operation(request.operation()))?;
    let root_control = request
        .root_control()?
        .map(|(id, action)| (id.to_owned(), action));
    let data_dir = runtime.data_dir().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        if let Some((root_handle, action)) = root_control {
            let lifecycle = app.state::<DaemonLifecycleState>();
            daemon_client::execute_source_root_control(&data_dir, &*lifecycle, &root_handle, action)
        } else {
            let lifecycle = app.state::<DaemonLifecycleState>();
            daemon_client::execute_from(&data_dir, &*lifecycle, request)
        }
    })
    .await
    .map_err(|_| DesktopError::internal())?
}

#[tauri::command]
async fn get_daemon_lifecycle(
    admission: tauri::State<'_, BridgeAdmissionState>,
    lifecycle: tauri::State<'_, DaemonLifecycleState>,
) -> Result<DaemonLifecycleSnapshot, DesktopError> {
    let _permit = admission.try_acquire(BridgeLane::Lifecycle)?;
    lifecycle.snapshot()
}

#[tauri::command]
async fn retry_daemon(
    app: tauri::AppHandle,
    admission: tauri::State<'_, BridgeAdmissionState>,
) -> Result<DaemonLifecycleSnapshot, DesktopError> {
    let _permit = admission.try_acquire(BridgeLane::Lifecycle)?;
    tauri::async_runtime::spawn_blocking(move || app.state::<DaemonLifecycleState>().retry())
        .await
        .map_err(|_| DesktopError::internal())?
}

#[tauri::command]
async fn select_import_root(
    app: tauri::AppHandle,
    admission: tauri::State<'_, BridgeAdmissionState>,
    runtime: tauri::State<'_, DesktopRuntimeState>,
) -> Result<Option<DesktopResponse>, DesktopError> {
    let _permit = admission.try_acquire(BridgeLane::NativeDialog)?;
    let Some(path) = native_import::pick_import_root().await else {
        return Ok(None);
    };
    let data_dir = runtime.data_dir().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        let prepared = native_import::prepare_import_root(&path)?;
        let lifecycle = app.state::<DaemonLifecycleState>();
        daemon_client::execute_source_root_register(
            &data_dir,
            &*lifecycle,
            prepared.path(),
            prepared.display_label(),
        )
    })
    .await
    .map_err(|_| DesktopError::internal())?
    .map(Some)
}

#[tauri::command]
async fn list_managed_roots(
    app: tauri::AppHandle,
    admission: tauri::State<'_, BridgeAdmissionState>,
    runtime: tauri::State<'_, DesktopRuntimeState>,
) -> Result<DesktopResponse, DesktopError> {
    let _permit = admission.try_acquire(BridgeLane::Import)?;
    let data_dir = runtime.data_dir().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        migrate_legacy_roots(&app, &data_dir)?;
        let lifecycle = app.state::<DaemonLifecycleState>();
        daemon_client::execute_source_roots_list(&data_dir, &*lifecycle)
    })
    .await
    .map_err(|_| DesktopError::internal())?
}

#[tauri::command]
async fn import_selected_root(
    request: ManagedRootHandleRequest,
    app: tauri::AppHandle,
    admission: tauri::State<'_, BridgeAdmissionState>,
    runtime: tauri::State<'_, DesktopRuntimeState>,
) -> Result<DesktopResponse, DesktopError> {
    let _permit = admission.try_acquire(BridgeLane::Import)?;
    let root_id = request.root_id;
    let data_dir = runtime.data_dir().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        let lifecycle = app.state::<DaemonLifecycleState>();
        daemon_client::execute_source_root_scan(&data_dir, &*lifecycle, &root_id)
    })
    .await
    .map_err(|_| DesktopError::internal())?
}

#[tauri::command]
async fn delete_source_root(
    request: ManagedRootHandleRequest,
    app: tauri::AppHandle,
    admission: tauri::State<'_, BridgeAdmissionState>,
    runtime: tauri::State<'_, DesktopRuntimeState>,
) -> Result<DesktopResponse, DesktopError> {
    let _permit = admission.try_acquire(BridgeLane::Import)?;
    let data_dir = runtime.data_dir().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        let lifecycle = app.state::<DaemonLifecycleState>();
        daemon_client::execute_source_root_delete(&data_dir, &*lifecycle, &request.root_id)
    })
    .await
    .map_err(|_| DesktopError::internal())?
}

#[tauri::command]
async fn reveal_source_file(
    request: RevealSourceRequest,
    app: tauri::AppHandle,
    admission: tauri::State<'_, BridgeAdmissionState>,
    runtime: tauri::State<'_, DesktopRuntimeState>,
) -> Result<source_reveal::RevealReceipt, DesktopError> {
    let _permit = admission.try_acquire(BridgeLane::Interactive)?;
    let data_dir = runtime.data_dir().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        let lifecycle = app.state::<DaemonLifecycleState>();
        source_reveal::reveal(&data_dir, &*lifecycle, &request.selection)
    })
    .await
    .map_err(|_| DesktopError::internal())?
}

#[tauri::command]
async fn export_diagnostics(
    app: tauri::AppHandle,
    admission: tauri::State<'_, BridgeAdmissionState>,
    runtime: tauri::State<'_, DesktopRuntimeState>,
) -> Result<Option<DiagnosticsExportReceipt>, DesktopError> {
    let _permit = admission.try_acquire(BridgeLane::Diagnostics)?;
    let data_dir = runtime.data_dir().to_path_buf();
    let diagnostics = tauri::async_runtime::spawn_blocking(move || {
        let lifecycle = app.state::<DaemonLifecycleState>();
        match daemon_client::execute_diagnostics_from(&data_dir, &*lifecycle) {
            Ok(response) if response.http_status == 200 => {
                lifecycle.diagnostics(response.diagnostics())
            }
            Ok(_) | Err(_) => lifecycle.diagnostics(None),
        }
    })
    .await
    .map_err(|_| DesktopError::internal())??;
    let _dialog_permit = admission.try_acquire(BridgeLane::NativeDialog)?;
    let Some(path) = native_import::pick_diagnostics_export_path().await else {
        return Ok(None);
    };
    tauri::async_runtime::spawn_blocking(move || {
        native_import::write_diagnostics_export(&path, &diagnostics)
    })
    .await
    .map_err(|_| DesktopError::internal())?
    .map(Some)
}

fn main() {
    let app = tauri::Builder::default()
        .manage(BridgeAdmissionState::default())
        .setup(|app| {
            let app_local_data_dir = app.path().app_local_data_dir()?;
            let runtime = DesktopRuntimeState::initialize(
                app_local_data_dir,
                runtime_state::configured_debug_data_dir(),
            )?;
            let legacy_roots = LegacyManagedRootsMigration::initialize(runtime.data_dir())?;
            let data_dir = runtime.data_dir().to_path_buf();
            app.manage(runtime);
            app.manage(legacy_roots);
            let current_exe = std::env::current_exe()?;
            let embedding_resource_dir = app
                .path()
                .resolve("embedding/runtime-pack", BaseDirectory::Resource)?;
            let ocr_resource_dir = app
                .path()
                .resolve("ocr/runtime-pack", BaseDirectory::Resource)?;
            let classifier_resource_dir = app
                .path()
                .resolve("classifier/runtime-pack", BaseDirectory::Resource)?;
            let pdfium_resource_dir = app
                .path()
                .resolve("pdfium/runtime-pack", BaseDirectory::Resource)?;
            app.manage(DaemonLifecycleState::initialize(
                &data_dir,
                &current_exe,
                &embedding_resource_dir,
                &ocr_resource_dir,
                &classifier_resource_dir,
                &pdfium_resource_dir,
            )?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_daemon_lifecycle,
            retry_daemon,
            daemon_request,
            select_import_root,
            list_managed_roots,
            import_selected_root,
            delete_source_root,
            reveal_source_file,
            export_diagnostics
        ])
        .build(tauri::generate_context!())
        .expect("resume-ir desktop runtime failed");
    app.run(|app_handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            app_handle.state::<DaemonLifecycleState>().shutdown();
        }
    });
}

fn migrate_legacy_roots(
    app: &tauri::AppHandle,
    data_dir: &std::path::Path,
) -> Result<(), DesktopError> {
    let legacy = app.state::<LegacyManagedRootsMigration>();
    let roots = legacy.pending_roots()?;
    if roots.is_empty() {
        return legacy.retire();
    }
    let lifecycle = app.state::<DaemonLifecycleState>();
    let registrations = roots
        .iter()
        .map(|root| (root.path(), root.display_label()))
        .collect::<Vec<_>>();
    daemon_client::execute_legacy_source_root_migration(data_dir, &*lifecycle, &registrations)?;
    legacy.retire()
}
