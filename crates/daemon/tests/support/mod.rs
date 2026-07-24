#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};

#[cfg(unix)]
use nix::fcntl::{Flock, FlockArg};

use import_pipeline::{
    current_import_processing_contract, finalize_migration_rebuild,
    prepare_migration_rebuild_artifacts, ImportOptions, SearchPublicationVectorization,
};
use meta_store::{
    ImportProcessingContract, ImportRootKind, ImportScanProfile, ImportScanScope, ImportTask,
    ImportTaskStatus, OwnedMetaStore, SearchProjectionServiceState, UnixTimestamp,
};

const EMBEDDING_MODEL_ID: &str = "intfloat-multilingual-e5-small-qint8-r1";
const EMBEDDING_DIMENSION: &str = "384";

/// Holds the sole host-local test slot for an attested resident embedding
/// runtime. The thread guard protects sibling libtest cases and the file lock
/// protects separately spawned integration-test binaries.
pub struct ImportRuntimeCapacityLease {
    _thread_guard: MutexGuard<'static, ()>,
    #[cfg(unix)]
    _process_lock: Flock<fs::File>,
}

/// Acquires the host-wide capacity required to start an attested resident
/// embedding runtime. Callers must retain this lease until every daemon and
/// its runtime child have exited.
pub fn import_runtime_capacity_lease() -> ImportRuntimeCapacityLease {
    static IMPORT_RUNTIME_CAPACITY: OnceLock<Mutex<()>> = OnceLock::new();
    let thread_guard = IMPORT_RUNTIME_CAPACITY
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    #[cfg(unix)]
    let process_lock = {
        let root = attested_test_runtime_root();
        fs::create_dir_all(&root).expect("create attested runtime capacity root");
        let lock_file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("import-runtime-capacity.lock"))
            .expect("open attested runtime capacity lock");
        Flock::lock(lock_file, FlockArg::LockExclusive)
            .unwrap_or_else(|(_, error)| panic!("lock attested runtime capacity: {error}"))
    };

    ImportRuntimeCapacityLease {
        _thread_guard: thread_guard,
        #[cfg(unix)]
        _process_lock: process_lock,
    }
}

/// Builds a daemon command whose mutating import/index capabilities are backed
/// by the same reviewed, digest-pinned packs used by the desktop bundle. The
/// daemon and native runtime processes are rebuilt under a strict test
/// attestation; no debug-only executable bypass is used.
pub fn import_capable_daemon_command(_capacity: &ImportRuntimeCapacityLease) -> Command {
    #[cfg(unix)]
    let mut command = {
        let mut command = Command::new("/bin/sh");
        command.arg(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/import-capable-daemon.sh"),
        );
        command
    };
    #[cfg(windows)]
    let mut command = Command::new(env!("CARGO_BIN_EXE_resume-daemon"));

    let pack_root = reviewed_pack_root();
    let embedding_pack = pack_root.join("embedding-runtime-pack");
    let classifier_model = reviewed_classifier_model();
    assert_reviewed_pack_file(&embedding_pack.join("runtime-pack.json"));
    assert_reviewed_pack_file(&classifier_model);
    let runtime = attested_test_runtime();

    command
        .env("RESUME_IR_TEST_DAEMON_BIN", &runtime.daemon)
        .env("RESUME_IR_TEST_EMBEDDING_COMMAND", &runtime.embedding)
        .env("RESUME_IR_TEST_CLASSIFIER_MODEL", &classifier_model)
        .env("RESUME_IR_TEST_EMBEDDING_MODEL_ID", EMBEDDING_MODEL_ID)
        .env("RESUME_IR_TEST_EMBEDDING_DIMENSION", EMBEDDING_DIMENSION)
        .env("RESUME_IR_EMBEDDING_RUNTIME_DIR", embedding_pack);
    command
}

pub fn fully_capable_daemon_command(capacity: &ImportRuntimeCapacityLease) -> Command {
    let mut command = import_capable_daemon_command(capacity);
    let ocr_pack = reviewed_pack_root().join("ocr-runtime-pack");
    let ocr_command = reviewed_ocr_command();
    let tessdata = reviewed_ocr_tessdata();
    assert_reviewed_pack_file(&ocr_pack.join("runtime-pack.json"));
    assert_reviewed_pack_file(&ocr_command);
    assert!(
        tessdata.is_dir(),
        "reviewed OCR tessdata directory is missing"
    );
    command
        .env("RESUME_IR_TEST_OCR_COMMAND", ocr_command)
        .env(
            "RESUME_IR_TEST_OCR_RENDER_COMMAND",
            &attested_test_runtime().pdf_renderer,
        )
        .env("TESSDATA_PREFIX", tessdata);
    command
}

pub fn reviewed_ocr_command() -> PathBuf {
    let command = reviewed_pack_root()
        .join("ocr-runtime-pack")
        .join(format!("tesseract{}", std::env::consts::EXE_SUFFIX));
    assert_reviewed_pack_file(&command);
    command
}

pub fn reviewed_ocr_tessdata() -> PathBuf {
    let tessdata = reviewed_pack_root().join("ocr-runtime-pack/tessdata");
    assert!(
        tessdata.is_dir(),
        "reviewed OCR tessdata directory is missing"
    );
    tessdata
}

pub fn reviewed_classifier_model() -> PathBuf {
    let model = reviewed_pack_root()
        .join("classifier-model-pack")
        .join("linear-promotion-model.json");
    assert_reviewed_pack_file(&model);
    model
}

pub fn copy_attested_pdf_renderer(destination_root: &Path) -> PathBuf {
    fs::create_dir_all(destination_root).expect("create PDF renderer test directory");
    let destination_root = destination_root
        .canonicalize()
        .expect("canonical PDF renderer test directory");
    let destination = destination_root.join("resume-pdf-render-runtime");
    copy_executable(&attested_test_runtime().pdf_renderer, &destination);
    destination
}

pub fn attested_pdf_renderer() -> PathBuf {
    attested_test_runtime().pdf_renderer.clone()
}

fn reviewed_pack_root() -> PathBuf {
    if let Some(root) = std::env::var_os("RESUME_IR_TEST_RUNTIME_PACK_ROOT") {
        let root = PathBuf::from(root);
        assert!(
            root.is_absolute(),
            "test runtime pack root must be absolute"
        );
        return root;
    }
    let daemon = Path::new(env!("CARGO_BIN_EXE_resume-daemon"));
    daemon
        .parent()
        .and_then(Path::parent)
        .expect("daemon binary must live below the Cargo target directory")
        .join("tauri-resources")
}

fn assert_reviewed_pack_file(path: &Path) {
    assert!(
        path.is_file(),
        "reviewed test runtime pack is missing at {}; run the desktop sidecar preparation gate or set RESUME_IR_TEST_RUNTIME_PACK_ROOT",
        path.display()
    );
}

struct AttestedTestRuntime {
    daemon: PathBuf,
    embedding: PathBuf,
    pdf_renderer: PathBuf,
}

fn attested_test_runtime() -> &'static AttestedTestRuntime {
    static RUNTIME: OnceLock<AttestedTestRuntime> = OnceLock::new();
    RUNTIME.get_or_init(build_attested_test_runtime)
}

fn build_attested_test_runtime() -> AttestedTestRuntime {
    assert_eq!(
        std::env::consts::OS,
        "macos",
        "attested runtime execution tests are native macOS-only"
    );
    assert_eq!(
        std::env::consts::ARCH,
        "aarch64",
        "attested runtime execution tests require Apple Silicon"
    );
    let repo_root = repository_root();
    let root = attested_test_runtime_root();
    fs::create_dir_all(&root).expect("create attested daemon test root");
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("build.lock"))
        .expect("open attested daemon build lock");
    #[cfg(unix)]
    let _lock = Flock::lock(lock_file, FlockArg::LockExclusive)
        .unwrap_or_else(|(_, error)| panic!("lock attested daemon build: {error}"));
    #[cfg(not(unix))]
    let _lock = lock_file;

    let build_target = root.join("build");
    let target_triple = "aarch64-apple-darwin";
    run_checked(
        Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
            .current_dir(&repo_root)
            .args([
                "build",
                "-p",
                "resume-embedding-runtime",
                "--bin",
                "resume-embedding-runtime",
                "--locked",
                "--target",
                target_triple,
                "--target-dir",
            ])
            .arg(&build_target)
            .env("CARGO_PROFILE_DEV_DEBUG", "0")
            .env("CARGO_PROFILE_DEV_SPLIT_DEBUGINFO", "off"),
        "build attested embedding test runtime",
    );

    let attestation_root = root.join("attestation");
    fs::create_dir_all(&attestation_root).expect("create runtime attestation root");
    let embedding_build =
        attestation_root.join(format!("resume-embedding-runtime-{target_triple}"));
    copy_executable(
        &build_target.join(format!("{target_triple}/debug/resume-embedding-runtime")),
        &embedding_build,
    );
    let renderer_build =
        attestation_root.join(format!("resume-pdf-render-runtime-{target_triple}"));
    run_checked(
        Command::new("xcrun")
            .current_dir(&repo_root)
            .args([
                "clang",
                "-O0",
                "-fobjc-arc",
                "-arch",
                "arm64",
                "-mmacosx-version-min=13.0",
                "-framework",
                "Foundation",
                "-framework",
                "CoreGraphics",
            ])
            .arg(repo_root.join("apps/desktop/native/macos/pdf_render_runtime.m"))
            .arg("-o")
            .arg(&renderer_build),
        "build attested PDF renderer test runtime",
    );

    let attestation = attestation_root.join("runtime-executable-attestation.json");
    let attestation_module =
        repo_root.join("apps/desktop/scripts/runtime-executable-attestation.mjs");
    run_checked(
        Command::new("node")
            .current_dir(&repo_root)
            .args([
                "--input-type=module",
                "--eval",
                "import { pathToFileURL } from 'node:url'; const module = await import(pathToFileURL(process.env.RESUME_IR_ATTESTATION_MODULE).href); await module.stageRuntimeExecutableAttestation({ destination: process.env.RESUME_IR_ATTESTATION_PATH, profile: 'debug', targetTriple: 'aarch64-apple-darwin' }, [{ binaryName: 'resume-embedding-runtime', destination: process.env.RESUME_IR_EMBEDDING_BUILD }, { binaryName: 'resume-pdf-render-runtime', destination: process.env.RESUME_IR_RENDERER_BUILD }]);",
            ])
            .env("RESUME_IR_ATTESTATION_MODULE", &attestation_module)
            .env("RESUME_IR_ATTESTATION_PATH", &attestation)
            .env("RESUME_IR_EMBEDDING_BUILD", &embedding_build)
            .env("RESUME_IR_RENDERER_BUILD", &renderer_build),
        "stage strict test runtime attestation",
    );

    run_checked(
        Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
            .current_dir(&repo_root)
            .args([
                "build",
                "-p",
                "resume-daemon",
                "--bin",
                "resume-daemon",
                "--locked",
                "--target",
                target_triple,
                "--target-dir",
            ])
            .arg(&build_target)
            .env("CARGO_PROFILE_DEV_DEBUG", "0")
            .env("CARGO_PROFILE_DEV_SPLIT_DEBUGINFO", "off")
            .env("RESUME_IR_RUNTIME_EXECUTABLE_ATTESTATION", &attestation),
        "build strict attested test daemon",
    );

    // The build root is serialized, but the resulting files must remain
    // immutable after the lock is released: another integration-test process
    // may rebuild the same attestation with a different Mach-O UUID. Publish a
    // per-process snapshot before returning any executable path.
    let snapshots_root = root.join("snapshots");
    fs::create_dir_all(&snapshots_root).expect("create attested runtime snapshots root");
    let snapshot_root = snapshots_root.join(format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock supports attested runtime snapshot")
            .as_nanos()
    ));
    fs::create_dir(&snapshot_root).expect("create immutable attested runtime snapshot");
    let embedding = snapshot_root.join("resume-embedding-runtime");
    let pdf_renderer = snapshot_root.join("resume-pdf-render-runtime");
    let daemon = snapshot_root.join("resume-daemon");
    copy_executable(&embedding_build, &embedding);
    copy_executable(&renderer_build, &pdf_renderer);
    copy_executable(
        &build_target.join(format!("{target_triple}/debug/resume-daemon")),
        &daemon,
    );
    AttestedTestRuntime {
        daemon,
        embedding,
        pdf_renderer,
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonical repository root")
}

fn attested_test_runtime_root() -> PathBuf {
    repository_root().join("target/daemon-test-attested")
}

fn run_checked(command: &mut Command, operation: &str) {
    let output = command.output().unwrap_or_else(|error| {
        panic!("{operation} could not start: {error}");
    });
    assert!(
        output.status.success(),
        "{operation} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn copy_executable(source: &Path, destination: &Path) {
    assert_reviewed_pack_file(source);
    let temporary = destination.with_extension(format!("tmp-{}", std::process::id()));
    fs::copy(source, &temporary).expect("copy attested test runtime");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o755))
            .expect("secure attested test runtime permissions");
    }
    fs::rename(&temporary, destination).expect("publish attested test runtime");
}

pub fn default_processing_contract() -> ImportProcessingContract {
    current_import_processing_contract(&ImportOptions::default()).unwrap()
}

pub fn reviewed_import_options() -> ImportOptions {
    let bytes = fs::read(reviewed_classifier_model()).expect("read reviewed classifier model");
    let linear_promotion =
        import_pipeline::LinearPromotionPolicy::load_attested_bundled_bytes(&bytes);
    assert!(
        linear_promotion.enabled(),
        "reviewed classifier model must activate"
    );
    ImportOptions {
        linear_promotion,
        ..ImportOptions::default()
    }
}

pub fn reviewed_processing_contract() -> ImportProcessingContract {
    current_import_processing_contract(&reviewed_import_options()).unwrap()
}

pub fn activate_reviewed_processing_contract(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
) -> ImportProcessingContract {
    let contract = reviewed_processing_contract();
    store
        .activate_migration_rebuild_contract(&contract, now)
        .unwrap();
    contract
}

pub fn activate_default_processing_contract(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
) -> ImportProcessingContract {
    let contract = default_processing_contract();
    store
        .activate_migration_rebuild_contract(&contract, now)
        .unwrap();
    contract
}

pub fn empty_import_scan_scope(task: &ImportTask) -> ImportScanScope {
    ImportScanScope {
        import_task_id: task.id.clone(),
        root_kind: ImportRootKind::Explicit,
        root_preset: None,
        scan_profile: ImportScanProfile::Explicit,
        requested_root_path: task.root_path.clone(),
        canonical_root_path: task.root_path.clone(),
        files_discovered: 0,
        ignored_entries: 0,
        scan_errors: 0,
        searchable_documents: 0,
        ocr_required_documents: 0,
        ocr_jobs_queued: 0,
        failed_documents: 0,
        deleted_documents: 0,
        scan_budget_kind: None,
        scan_budget_limit: None,
        scan_budget_observed: None,
        scan_budget_exhausted: false,
        updated_at: task.updated_at,
    }
}

pub fn insert_import_task(store: &OwnedMetaStore, task: &ImportTask) -> ImportProcessingContract {
    insert_import_task_with_scope(store, task, &empty_import_scan_scope(task))
}

pub fn insert_import_task_with_scope(
    store: &OwnedMetaStore,
    task: &ImportTask,
    scope: &ImportScanScope,
) -> ImportProcessingContract {
    let contract = activate_default_processing_contract(store, task.queued_at);
    insert_import_task_with_scope_for_contract(store, task, scope, &contract);
    contract
}

pub fn insert_import_task_with_reviewed_contract(
    store: &OwnedMetaStore,
    task: &ImportTask,
) -> ImportProcessingContract {
    let contract = activate_reviewed_processing_contract(store, task.queued_at);
    insert_import_task_with_scope_for_contract(
        store,
        task,
        &empty_import_scan_scope(task),
        &contract,
    );
    contract
}

fn insert_import_task_with_scope_for_contract(
    store: &OwnedMetaStore,
    task: &ImportTask,
    scope: &ImportScanScope,
    contract: &ImportProcessingContract,
) {
    assert_ne!(task.status, ImportTaskStatus::Completed);
    prepare_migration_rebuild_artifacts(
        store,
        task.queued_at,
        &import_pipeline::PipelineRunControl::default(),
    )
    .unwrap();
    finalize_migration_rebuild(
        store,
        task.queued_at,
        contract,
        &SearchPublicationVectorization::default(),
        &import_pipeline::PipelineRunControl::default(),
    )
    .unwrap();
    assert_eq!(
        store.search_projection_state().unwrap().service_state,
        SearchProjectionServiceState::Ready
    );
    let queued = ImportTask {
        id: task.id.clone(),
        root_path: task.root_path.clone(),
        status: ImportTaskStatus::Queued,
        queued_at: task.queued_at,
        started_at: None,
        finished_at: None,
        updated_at: task.queued_at,
    };
    let mut initial_scope = scope.clone();
    initial_scope.updated_at = task.queued_at;
    store
        .insert_import_task_with_scan_scope(&queued, &initial_scope, contract)
        .unwrap();
    if task.status != ImportTaskStatus::Queued {
        let running_at = task.started_at.unwrap_or(task.updated_at);
        let claimed = store
            .claim_observed_import_task_for_worker(&queued, running_at)
            .unwrap()
            .unwrap();
        if task.status == ImportTaskStatus::Running && task.updated_at != claimed.updated_at {
            assert!(store
                .heartbeat_running_import_task(&task.id, task.updated_at)
                .unwrap());
        }
    }
    if matches!(
        task.status,
        ImportTaskStatus::FailedRetryable | ImportTaskStatus::FailedPermanent
    ) {
        store
            .update_import_task_status(&task.id, task.status, task.updated_at)
            .unwrap();
    }
    let mut persisted_scope = scope.clone();
    persisted_scope.updated_at = task.updated_at;
    store.upsert_import_scan_scope(&persisted_scope).unwrap();
}
