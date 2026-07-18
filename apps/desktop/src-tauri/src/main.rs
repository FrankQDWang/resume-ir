mod bridge_admission;
mod daemon_client;
mod daemon_connection;
mod daemon_exchange;
mod daemon_lifecycle;
mod daemon_request;
mod daemon_response;
mod native_import;
mod runtime_state;

use bridge_admission::{lane_for_operation, BridgeAdmissionState, BridgeLane};
use daemon_client::{DesktopError, DesktopRequest, DesktopResponse};
use daemon_lifecycle::{DaemonLifecycleSnapshot, DaemonLifecycleState};
use native_import::{
    DiagnosticsExportReceipt, ManagedRoots, NativeImportState, SelectedImportRoot,
};
use runtime_state::DesktopRuntimeState;
use tauri::path::BaseDirectory;
use tauri::Manager;

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct ManagedRootHandleRequest {
    root_handle: String,
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
        .map(|(handle, action)| (handle.to_owned(), action));
    let data_dir = runtime.data_dir().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        if let Some((root_handle, action)) = root_control {
            let root = app.state::<NativeImportState>().resolve(&root_handle)?;
            let lifecycle = app.state::<DaemonLifecycleState>();
            daemon_client::execute_root_control_from(&data_dir, &*lifecycle, &root, action)
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
) -> Result<Option<SelectedImportRoot>, DesktopError> {
    let _permit = admission.try_acquire(BridgeLane::NativeDialog)?;
    let Some(path) = native_import::pick_import_root().await else {
        return Ok(None);
    };
    tauri::async_runtime::spawn_blocking(move || {
        let prepared = native_import::prepare_import_root(&path)?;
        app.state::<NativeImportState>().register(prepared)
    })
    .await
    .map_err(|_| DesktopError::internal())?
    .map(Some)
}

#[tauri::command]
async fn list_managed_roots(
    app: tauri::AppHandle,
    admission: tauri::State<'_, BridgeAdmissionState>,
) -> Result<ManagedRoots, DesktopError> {
    let _permit = admission.try_acquire(BridgeLane::NativeDialog)?;
    tauri::async_runtime::spawn_blocking(move || app.state::<NativeImportState>().managed_roots())
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
    let root_handle = request.root_handle;
    let data_dir = runtime.data_dir().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        let root = app
            .state::<NativeImportState>()
            .resolve_for_import(&root_handle)?;
        let lifecycle = app.state::<DaemonLifecycleState>();
        daemon_client::execute_import_from(&data_dir, &*lifecycle, &root)
    })
    .await
    .map_err(|_| DesktopError::internal())?
}

#[tauri::command]
async fn reauthorize_managed_root(
    request: ManagedRootHandleRequest,
    app: tauri::AppHandle,
    admission: tauri::State<'_, BridgeAdmissionState>,
) -> Result<Option<SelectedImportRoot>, DesktopError> {
    let _permit = admission.try_acquire(BridgeLane::NativeDialog)?;
    let root_handle = request.root_handle;
    app.state::<NativeImportState>().resolve(&root_handle)?;
    let Some(path) = native_import::pick_reauthorization_root().await else {
        return Ok(None);
    };
    tauri::async_runtime::spawn_blocking(move || {
        let prepared = native_import::prepare_import_root(&path)?;
        app.state::<NativeImportState>()
            .reauthorize(&root_handle, prepared)
    })
    .await
    .map_err(|_| DesktopError::internal())?
    .map(Some)
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
            let native_import = NativeImportState::initialize(runtime.data_dir())?;
            let data_dir = runtime.data_dir().to_path_buf();
            app.manage(runtime);
            app.manage(native_import);
            let current_exe = std::env::current_exe()?;
            let embedding_resource_dir = app
                .path()
                .resolve("embedding/runtime-pack", BaseDirectory::Resource)?;
            let ocr_resource_dir = app
                .path()
                .resolve("ocr/runtime-pack", BaseDirectory::Resource)?;
            app.manage(DaemonLifecycleState::initialize(
                &data_dir,
                &current_exe,
                &embedding_resource_dir,
                &ocr_resource_dir,
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
            reauthorize_managed_root,
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
