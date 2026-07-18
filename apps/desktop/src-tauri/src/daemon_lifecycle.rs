use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::daemon_client::DesktopError;
use crate::daemon_connection::ConnectionGenerationSource;

#[path = "daemon_lifecycle/policy.rs"]
mod policy;
#[path = "daemon_lifecycle/process.rs"]
mod process;
#[path = "daemon_lifecycle/receipt.rs"]
mod receipt;
#[path = "daemon_lifecycle/supervisor.rs"]
mod supervisor;

use process::ProductionDaemonRuntime;
use receipt::LifecycleReceiptRecorder;
use supervisor::DaemonLifecycleKind;
pub(crate) use supervisor::{DaemonLifecycleSnapshot, DaemonLifecycleState};

const DESKTOP_IMPORT_RESCAN_MIN_AGE_SECONDS: i64 = 300;
#[cfg(any(not(debug_assertions), test))]
const PACKAGED_EMBEDDING_MODEL_ID: &str = "intfloat-multilingual-e5-small-qint8-r1";
#[cfg(any(not(debug_assertions), test))]
const PACKAGED_EMBEDDING_DIMENSION: usize = 384;
#[cfg(any(not(debug_assertions), test))]
const PACK_MANIFEST_MAX_BYTES: u64 = 64 * 1024;
#[cfg(any(debug_assertions, test))]
const EMBEDDING_MODEL_ID: &str = "intfloat/multilingual-e5-small";
#[cfg(any(debug_assertions, test))]
const EMBEDDING_DIMENSION: usize = 384;
#[cfg(any(debug_assertions, test))]
const EMBEDDING_MODEL_ID_MAX_BYTES: usize = 128;
const EMBEDDING_PATH_MAX_BYTES: usize = 4096;
const PACKAGED_OCR_LANG: &str = "eng+chi_sim";
const OCR_JOBS_PER_TICK: usize = 1;
const DAEMON_IPC_PROTOCOL: &str = "resume-ir.daemon-ipc.v2";

impl DaemonLifecycleState {
    pub(crate) fn initialize(
        data_dir: &Path,
        current_exe: &Path,
        embedding_resource_dir: &Path,
        ocr_resource_dir: &Path,
    ) -> Result<Self, DesktopError> {
        Self::launch(
            ProductionDaemonRuntime::initialize(
                data_dir,
                current_exe,
                embedding_resource_dir,
                ocr_resource_dir,
            ),
            LifecycleReceiptRecorder::initialize(data_dir),
        )
    }
}

fn configured_daemon_binary() -> Result<PathBuf, DesktopError> {
    let configured = configured_debug_daemon_binary();
    let current_exe = std::env::current_exe().map_err(|_| {
        DesktopError::new(
            "daemon_binary_unavailable",
            "无法定位本地 daemon 可执行文件",
        )
    })?;
    let debug_binary = debug_daemon_binary();
    select_daemon_binary(configured.as_deref(), &current_exe, debug_binary.as_deref()).ok_or_else(
        || {
            DesktopError::new(
                "daemon_binary_unavailable",
                "本地 daemon 可执行文件尚未准备好",
            )
        },
    )
}

impl ConnectionGenerationSource for DaemonLifecycleState {
    fn ready_generation(&self) -> Option<u64> {
        self.snapshot().ok().and_then(|snapshot| {
            (snapshot.state == DaemonLifecycleKind::Ready && snapshot.generation > 0)
                .then_some(snapshot.generation)
        })
    }
}

#[cfg(debug_assertions)]
fn debug_daemon_binary() -> Option<PathBuf> {
    Some(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../target/debug")
            .join(binary_name()),
    )
}

#[cfg(not(debug_assertions))]
fn debug_daemon_binary() -> Option<PathBuf> {
    None
}

#[cfg(debug_assertions)]
fn configured_debug_daemon_binary() -> Option<OsString> {
    std::env::var_os("RESUME_IR_DAEMON_BINARY").filter(|value| !value.is_empty())
}

#[cfg(not(debug_assertions))]
fn configured_debug_daemon_binary() -> Option<OsString> {
    None
}

fn select_daemon_binary(
    configured: Option<&OsStr>,
    current_exe: &Path,
    debug_binary: Option<&Path>,
) -> Option<PathBuf> {
    let sibling = current_exe
        .parent()
        .map(|parent| parent.join(binary_name()));
    configured
        .map(PathBuf::from)
        .into_iter()
        .chain(sibling)
        .chain(debug_binary.map(PathBuf::from))
        .find(|candidate| candidate.is_file())
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "resume-daemon.exe"
    } else {
        "resume-daemon"
    }
}

#[derive(Debug)]
struct EmbeddingRuntime {
    command: PathBuf,
    model_id: String,
    dimension: usize,
    path_prepend: Option<PathBuf>,
    model_dir: Option<PathBuf>,
    runtime_dir: Option<PathBuf>,
}

impl EmbeddingRuntime {
    fn configure_command(&self, command: &mut Command) -> Result<(), DesktopError> {
        command
            .env("TRANSFORMERS_OFFLINE", "1")
            .env("HF_HUB_OFFLINE", "1");
        if let Some(model_dir) = &self.model_dir {
            command.env("RESUME_IR_E5_MODEL_DIR", model_dir);
        }
        if let Some(runtime_dir) = &self.runtime_dir {
            command.env("RESUME_IR_EMBEDDING_RUNTIME_DIR", runtime_dir);
        }
        if let Some(path_prepend) = &self.path_prepend {
            let inherited_path = std::env::var_os("PATH").unwrap_or_default();
            let mut paths = vec![path_prepend.clone()];
            paths.extend(std::env::split_paths(&inherited_path));
            let joined = std::env::join_paths(paths).map_err(|_| embedding_runtime_invalid())?;
            command.env("PATH", joined);
        }
        Ok(())
    }
}

#[cfg(debug_assertions)]
fn configured_embedding_runtime(
    _current_exe: &Path,
    _resource_dir: &Path,
) -> Result<Option<EmbeddingRuntime>, DesktopError> {
    let configured_command = std::env::var_os("RESUME_IR_EMBEDDING_COMMAND");
    let configured_model_id = std::env::var("RESUME_IR_EMBEDDING_MODEL_ID").ok();
    let configured_dimension = std::env::var("RESUME_IR_EMBEDDING_DIMENSION").ok();
    resolve_embedding_runtime(
        configured_command,
        configured_model_id,
        configured_dimension,
        debug_embedding_runtime(),
    )
}

#[cfg(not(debug_assertions))]
fn configured_embedding_runtime(
    current_exe: &Path,
    resource_dir: &Path,
) -> Result<Option<EmbeddingRuntime>, DesktopError> {
    resolve_packaged_embedding_runtime(current_exe, resource_dir).map(Some)
}

#[cfg(any(not(debug_assertions), test))]
fn resolve_packaged_embedding_runtime(
    current_exe: &Path,
    resource_dir: &Path,
) -> Result<EmbeddingRuntime, DesktopError> {
    let command = current_exe
        .parent()
        .map(|parent| parent.join(embedding_binary_name()))
        .ok_or_else(embedding_runtime_invalid)?;
    let command = validate_embedding_path(command, false)?;
    let runtime_dir = validate_resource_pack_directory(resource_dir.to_path_buf())?;
    Ok(EmbeddingRuntime {
        command,
        model_id: PACKAGED_EMBEDDING_MODEL_ID.to_string(),
        dimension: PACKAGED_EMBEDDING_DIMENSION,
        path_prepend: None,
        model_dir: None,
        runtime_dir: Some(runtime_dir),
    })
}

#[cfg(any(not(debug_assertions), test))]
fn embedding_binary_name() -> &'static str {
    if cfg!(windows) {
        "resume-embedding-runtime.exe"
    } else {
        "resume-embedding-runtime"
    }
}

#[cfg(any(debug_assertions, test))]
fn resolve_embedding_runtime(
    command: Option<OsString>,
    model_id: Option<String>,
    dimension: Option<String>,
    fallback: Option<EmbeddingRuntime>,
) -> Result<Option<EmbeddingRuntime>, DesktopError> {
    if command.is_none() && model_id.is_none() && dimension.is_none() {
        return Ok(fallback);
    }
    let (Some(command), Some(model_id), Some(dimension)) = (command, model_id, dimension) else {
        return Err(embedding_runtime_invalid());
    };
    let command = validate_embedding_path(PathBuf::from(command), false)?;
    validate_embedding_model_id(&model_id)?;
    let dimension = dimension
        .parse::<usize>()
        .ok()
        .filter(|value| (1..=4096).contains(value))
        .ok_or_else(embedding_runtime_invalid)?;
    Ok(Some(EmbeddingRuntime {
        command,
        model_id,
        dimension,
        path_prepend: None,
        model_dir: None,
        runtime_dir: None,
    }))
}

#[cfg(debug_assertions)]
fn debug_embedding_runtime() -> Option<EmbeddingRuntime> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let command = validate_embedding_path(
        root.join("scripts/local/embedding-runtime-e5-onnx.py"),
        false,
    )
    .ok()?;
    let path_prepend =
        validate_embedding_path(root.join(".cache/resume-ir-e5-onnx-py312/bin"), true).ok()?;
    let snapshots =
        root.join(".cache/resume-ir-e5-hf/hub/models--intfloat--multilingual-e5-small/snapshots");
    let model_dir = unique_model_snapshot(&snapshots)?;
    Some(EmbeddingRuntime {
        command,
        model_id: EMBEDDING_MODEL_ID.to_string(),
        dimension: EMBEDDING_DIMENSION,
        path_prepend: Some(path_prepend),
        model_dir: Some(model_dir),
        runtime_dir: None,
    })
}

#[cfg(debug_assertions)]
fn unique_model_snapshot(snapshots: &Path) -> Option<PathBuf> {
    let mut candidates = snapshots
        .read_dir()
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate.is_dir()
                && (candidate.join("onnx/model.onnx").is_file()
                    || candidate.join("model.onnx").is_file())
                && candidate.join("tokenizer.json").is_file()
                && candidate.join("config.json").is_file()
        });
    let candidate = candidates.next()?;
    if candidates.next().is_some() {
        return None;
    }
    validate_embedding_path(candidate, true).ok()
}

fn validate_embedding_path(path: PathBuf, directory: bool) -> Result<PathBuf, DesktopError> {
    let text = path.to_str().ok_or_else(embedding_runtime_invalid)?;
    if !path.is_absolute() || text.len() > EMBEDDING_PATH_MAX_BYTES {
        return Err(embedding_runtime_invalid());
    }
    let direct_metadata = path
        .symlink_metadata()
        .map_err(|_| embedding_runtime_invalid())?;
    if direct_metadata.file_type().is_symlink() {
        return Err(embedding_runtime_invalid());
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| embedding_runtime_invalid())?;
    let expected_kind = if directory {
        canonical.is_dir()
    } else {
        canonical.is_file()
    };
    if !expected_kind {
        return Err(embedding_runtime_invalid());
    }
    #[cfg(unix)]
    if !directory
        && canonical
            .metadata()
            .map_err(|_| embedding_runtime_invalid())?
            .permissions()
            .mode()
            & 0o111
            == 0
    {
        return Err(embedding_runtime_invalid());
    }
    Ok(canonical)
}

#[cfg(any(not(debug_assertions), test))]
fn validate_resource_pack_directory(path: PathBuf) -> Result<PathBuf, DesktopError> {
    let directory = validate_embedding_path(path, true)?;
    let manifest = directory.join("runtime-pack.json");
    let metadata = manifest
        .symlink_metadata()
        .map_err(|_| embedding_runtime_invalid())?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > PACK_MANIFEST_MAX_BYTES
    {
        return Err(embedding_runtime_invalid());
    }
    Ok(directory)
}

#[cfg(any(debug_assertions, test))]
fn validate_embedding_model_id(model_id: &str) -> Result<(), DesktopError> {
    if model_id.is_empty()
        || model_id.len() > EMBEDDING_MODEL_ID_MAX_BYTES
        || !model_id.bytes().all(|value| {
            value.is_ascii_alphanumeric() || matches!(value, b'/' | b'-' | b'_' | b'.')
        })
    {
        return Err(embedding_runtime_invalid());
    }
    Ok(())
}

fn embedding_runtime_invalid() -> DesktopError {
    DesktopError::new(
        "embedding_runtime_invalid",
        "本地语义检索运行时配置无效或不完整",
    )
}

#[derive(Debug)]
struct OcrRuntime {
    engine_command: PathBuf,
    renderer_command: PathBuf,
    tessdata_dir: PathBuf,
}

impl OcrRuntime {
    fn configure_command(&self, command: &mut Command) {
        command
            .env("TESSDATA_PREFIX", &self.tessdata_dir)
            .env("OMP_THREAD_LIMIT", "1");
    }
}

#[cfg(debug_assertions)]
fn configured_ocr_runtime(
    _current_exe: &Path,
    _resource_dir: &Path,
) -> Result<Option<OcrRuntime>, DesktopError> {
    Ok(None)
}

#[cfg(not(debug_assertions))]
fn configured_ocr_runtime(
    current_exe: &Path,
    resource_dir: &Path,
) -> Result<Option<OcrRuntime>, DesktopError> {
    resolve_packaged_ocr_runtime(current_exe, resource_dir).map(Some)
}

#[cfg(any(not(debug_assertions), test))]
fn resolve_packaged_ocr_runtime(
    current_exe: &Path,
    resource_dir: &Path,
) -> Result<OcrRuntime, DesktopError> {
    let renderer_command = current_exe
        .parent()
        .map(|parent| parent.join(pdf_renderer_binary_name()))
        .ok_or_else(ocr_runtime_invalid)?;
    let renderer_command =
        validate_embedding_path(renderer_command, false).map_err(|_| ocr_runtime_invalid())?;
    let resource_dir = validate_ocr_pack_directory(resource_dir)?;
    let engine_command = validate_embedding_path(resource_dir.join("tesseract"), false)
        .map_err(|_| ocr_runtime_invalid())?;
    let tessdata_dir = validate_embedding_path(resource_dir.join("tessdata"), true)
        .map_err(|_| ocr_runtime_invalid())?;
    for file in ["eng.traineddata", "chi_sim.traineddata", "configs/tsv"] {
        validate_ocr_data_file(tessdata_dir.join(file))?;
    }
    Ok(OcrRuntime {
        engine_command,
        renderer_command,
        tessdata_dir,
    })
}

#[cfg(any(not(debug_assertions), test))]
fn validate_ocr_data_file(path: PathBuf) -> Result<PathBuf, DesktopError> {
    let text = path.to_str().ok_or_else(ocr_runtime_invalid)?;
    if !path.is_absolute() || text.len() > EMBEDDING_PATH_MAX_BYTES {
        return Err(ocr_runtime_invalid());
    }
    let metadata = path.symlink_metadata().map_err(|_| ocr_runtime_invalid())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() == 0 {
        return Err(ocr_runtime_invalid());
    }
    path.canonicalize().map_err(|_| ocr_runtime_invalid())
}

#[cfg(any(not(debug_assertions), test))]
fn validate_ocr_pack_directory(resource_dir: &Path) -> Result<PathBuf, DesktopError> {
    let directory = validate_embedding_path(resource_dir.to_path_buf(), true)
        .map_err(|_| ocr_runtime_invalid())?;
    for file in ["runtime-pack.json", "THIRD-PARTY-NOTICES.json"] {
        let metadata = directory
            .join(file)
            .symlink_metadata()
            .map_err(|_| ocr_runtime_invalid())?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() == 0
            || metadata.len() > PACK_MANIFEST_MAX_BYTES
        {
            return Err(ocr_runtime_invalid());
        }
    }
    Ok(directory)
}

#[cfg(any(not(debug_assertions), test))]
fn pdf_renderer_binary_name() -> &'static str {
    if cfg!(windows) {
        "resume-pdf-render-runtime.exe"
    } else {
        "resume-pdf-render-runtime"
    }
}

#[cfg(any(not(debug_assertions), test))]
fn ocr_runtime_invalid() -> DesktopError {
    DesktopError::new("ocr_runtime_invalid", "本地 OCR 运行时配置无效或不完整")
}

fn daemon_arguments(
    data_dir: &Path,
    embedding: Option<&EmbeddingRuntime>,
    ocr: Option<&OcrRuntime>,
) -> Vec<OsString> {
    let mut arguments: Vec<OsString> = [
        OsString::from("--data-dir"),
        data_dir.as_os_str().to_os_string(),
        OsString::from("run"),
        OsString::from("--foreground"),
        OsString::from("--parent-lifecycle-stdin"),
        OsString::from("--work-imports"),
        OsString::from("--work-index"),
        OsString::from("--rescan-completed-imports"),
        OsString::from("--watch-import-roots"),
        OsString::from("--import-rescan-min-age-seconds"),
        OsString::from(DESKTOP_IMPORT_RESCAN_MIN_AGE_SECONDS.to_string()),
        OsString::from("--expected-ipc-protocol"),
        OsString::from(DAEMON_IPC_PROTOCOL),
        OsString::from("--ipc-listen"),
        OsString::from("127.0.0.1:0"),
    ]
    .into_iter()
    .collect();
    if let Some(embedding) = embedding {
        arguments.extend([
            OsString::from("--embedding-command"),
            embedding.command.as_os_str().to_os_string(),
            OsString::from("--embedding-model-id"),
            OsString::from(&embedding.model_id),
            OsString::from("--embedding-dimension"),
            OsString::from(embedding.dimension.to_string()),
        ]);
    }
    if let Some(ocr) = ocr {
        arguments.extend([
            OsString::from("--work-ocr"),
            OsString::from("--ocr-tesseract-command"),
            ocr.engine_command.as_os_str().to_os_string(),
            OsString::from("--ocr-render-command"),
            ocr.renderer_command.as_os_str().to_os_string(),
            OsString::from("--ocr-lang"),
            OsString::from(PACKAGED_OCR_LANG),
            OsString::from("--ocr-jobs-per-tick"),
            OsString::from(OCR_JOBS_PER_TICK.to_string()),
        ]);
    }
    arguments
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn daemon_command_is_bounded_to_local_workers_and_loopback_ipc() {
        let arguments = daemon_arguments(Path::new("synthetic-data"), None, None);
        assert_eq!(
            arguments,
            [
                "--data-dir",
                "synthetic-data",
                "run",
                "--foreground",
                "--parent-lifecycle-stdin",
                "--work-imports",
                "--work-index",
                "--rescan-completed-imports",
                "--watch-import-roots",
                "--import-rescan-min-age-seconds",
                "300",
                "--expected-ipc-protocol",
                "resume-ir.daemon-ipc.v2",
                "--ipc-listen",
                "127.0.0.1:0",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn daemon_command_enables_atomic_vector_publication_only_with_complete_runtime() {
        let embedding = EmbeddingRuntime {
            command: PathBuf::from("/synthetic/embedding-runtime"),
            model_id: EMBEDDING_MODEL_ID.to_string(),
            dimension: EMBEDDING_DIMENSION,
            path_prepend: None,
            model_dir: None,
            runtime_dir: None,
        };
        let arguments = daemon_arguments(Path::new("synthetic-data"), Some(&embedding), None);
        assert_eq!(
            &arguments[15..],
            [
                "--embedding-command",
                "/synthetic/embedding-runtime",
                "--embedding-model-id",
                EMBEDDING_MODEL_ID,
                "--embedding-dimension",
                "384",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn partial_or_untrusted_embedding_configuration_fails_closed() {
        assert!(resolve_embedding_runtime(
            Some(OsString::from("/synthetic/runtime")),
            None,
            Some("384".to_string()),
            None,
        )
        .is_err());
        assert!(validate_embedding_model_id("model id with spaces").is_err());
        assert!(validate_embedding_model_id(EMBEDDING_MODEL_ID).is_ok());
    }

    #[test]
    fn packaged_embedding_runtime_is_sibling_scoped_and_resource_scoped() {
        let root = std::env::temp_dir().join(format!(
            "resume-ir-packaged-embedding-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let executable_dir = root.join("MacOS");
        let resource_dir = root.join("Resources/embedding/runtime-pack");
        fs::create_dir_all(&executable_dir).unwrap();
        fs::create_dir_all(&resource_dir).unwrap();
        let runtime = executable_dir.join(embedding_binary_name());
        fs::write(&runtime, "synthetic-runtime").unwrap();
        #[cfg(unix)]
        {
            let mut permissions = fs::metadata(&runtime).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&runtime, permissions).unwrap();
        }
        fs::write(resource_dir.join("runtime-pack.json"), "{}").unwrap();

        let embedding = resolve_packaged_embedding_runtime(
            &executable_dir.join("resume-desktop"),
            &resource_dir,
        )
        .unwrap();
        assert_eq!(embedding.command, runtime.canonicalize().unwrap());
        assert_eq!(embedding.model_id, PACKAGED_EMBEDDING_MODEL_ID);
        assert_eq!(embedding.dimension, PACKAGED_EMBEDDING_DIMENSION);
        let expected_resource_dir = resource_dir.canonicalize().unwrap();
        assert_eq!(
            embedding.runtime_dir.as_deref(),
            Some(expected_resource_dir.as_path())
        );
        assert!(embedding.path_prepend.is_none());
        assert!(embedding.model_dir.is_none());

        let mut command = Command::new("synthetic-daemon");
        embedding.configure_command(&mut command).unwrap();
        let runtime_environment = command
            .get_envs()
            .find(|(key, _)| *key == OsStr::new("RESUME_IR_EMBEDDING_RUNTIME_DIR"))
            .and_then(|(_, value)| value)
            .map(PathBuf::from);
        assert_eq!(runtime_environment, embedding.runtime_dir);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn packaged_ocr_runtime_is_sibling_scoped_resource_scoped_and_bounded() {
        let root = std::env::temp_dir().join(format!(
            "resume-ir-packaged-ocr-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let executable_dir = root.join("MacOS");
        let resource_dir = root.join("Resources/ocr/runtime-pack");
        let tessdata_dir = resource_dir.join("tessdata");
        fs::create_dir_all(tessdata_dir.join("configs")).unwrap();
        fs::create_dir_all(&executable_dir).unwrap();
        let renderer = executable_dir.join(pdf_renderer_binary_name());
        let engine = resource_dir.join("tesseract");
        for executable in [&renderer, &engine] {
            fs::write(executable, "synthetic-runtime").unwrap();
            #[cfg(unix)]
            {
                let mut permissions = fs::metadata(executable).unwrap().permissions();
                permissions.set_mode(0o755);
                fs::set_permissions(executable, permissions).unwrap();
            }
        }
        fs::write(resource_dir.join("runtime-pack.json"), "{}").unwrap();
        fs::write(resource_dir.join("THIRD-PARTY-NOTICES.json"), "{}").unwrap();
        fs::write(tessdata_dir.join("eng.traineddata"), "eng").unwrap();
        fs::write(tessdata_dir.join("chi_sim.traineddata"), "chi").unwrap();
        fs::write(tessdata_dir.join("configs/tsv"), "tsv").unwrap();

        let ocr =
            resolve_packaged_ocr_runtime(&executable_dir.join("resume-desktop"), &resource_dir)
                .unwrap();
        assert_eq!(ocr.engine_command, engine.canonicalize().unwrap());
        assert_eq!(ocr.renderer_command, renderer.canonicalize().unwrap());
        assert_eq!(ocr.tessdata_dir, tessdata_dir.canonicalize().unwrap());

        let mut command = Command::new("synthetic-daemon");
        ocr.configure_command(&mut command);
        let tessdata_environment = command
            .get_envs()
            .find(|(key, _)| *key == OsStr::new("TESSDATA_PREFIX"))
            .and_then(|(_, value)| value)
            .map(PathBuf::from);
        assert_eq!(
            tessdata_environment.as_deref(),
            Some(ocr.tessdata_dir.as_path())
        );
        let arguments = daemon_arguments(Path::new("synthetic-data"), None, Some(&ocr));
        assert_eq!(
            &arguments[15..],
            [
                OsString::from("--work-ocr"),
                OsString::from("--ocr-tesseract-command"),
                ocr.engine_command.as_os_str().to_os_string(),
                OsString::from("--ocr-render-command"),
                ocr.renderer_command.as_os_str().to_os_string(),
                OsString::from("--ocr-lang"),
                OsString::from("eng+chi_sim"),
                OsString::from("--ocr-jobs-per-tick"),
                OsString::from("1"),
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_daemon_binary_wins() {
        let root = std::env::temp_dir().join(format!(
            "resume-ir-desktop-lifecycle-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let configured = root.join("configured-daemon");
        let sibling = root.join(binary_name());
        fs::write(&configured, "synthetic").unwrap();
        fs::write(&sibling, "synthetic").unwrap();

        let selected =
            select_daemon_binary(Some(configured.as_os_str()), &root.join("desktop"), None);
        assert_eq!(selected.as_deref(), Some(configured.as_path()));

        fs::remove_dir_all(root).unwrap();
    }
}
