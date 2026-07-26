use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, RecvTimeoutError},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};

use embedder::{ResidentEmbeddingOwner, ResidentEmbeddingStatus};
use import_pipeline::DataDirectoryOwnerLease;
use meta_store::{ImportProcessingContract, OwnedMetaStore, ReadMetaStore};

use crate::daemon_error::{DaemonError, Result};
use crate::run_options::RunOptions;
use crate::{import_processing, ipc};

struct PersistentRuntime {
    store: OwnedMetaStore,
    ipc_store: ReadMetaStore,
    ipc_owned_store: OwnedMetaStore,
    processing_contract: ImportProcessingContract,
    startup_orphaned_recovered: usize,
    _resident_embedding_owner: Option<ResidentEmbeddingOwner>,
}

pub(crate) fn run_persistent_ipc(
    data_dir: &Path,
    options: RunOptions,
    data_directory_owner: Arc<DataDirectoryOwnerLease>,
    parent_shutdown: Option<Arc<AtomicBool>>,
    daemon_owner: ipc::DaemonGenerationOwner,
) -> Result<()> {
    run_persistent_ipc_with_hooks(
        data_dir,
        options,
        data_directory_owner,
        parent_shutdown,
        daemon_owner,
        &BootstrapHooks::default(),
    )
}

#[derive(Default)]
struct BootstrapHooks {
    #[cfg(test)]
    before_store_open: Option<Arc<dyn Fn() + Send + Sync>>,
    #[cfg(test)]
    store_opened: Option<Arc<dyn Fn() + Send + Sync>>,
}

fn run_persistent_ipc_with_hooks(
    data_dir: &Path,
    mut options: RunOptions,
    data_directory_owner: Arc<DataDirectoryOwnerLease>,
    parent_shutdown: Option<Arc<AtomicBool>>,
    daemon_owner: ipc::DaemonGenerationOwner,
    _hooks: &BootstrapHooks,
) -> Result<()> {
    let ipc_addr = options
        .ipc_listen
        .expect("persistent IPC mode has a validated listener address");
    let mut bound_server = ipc::server::BoundServer::bind(ipc_addr, daemon_owner)?;
    let (control_state, mut control_publisher) = ipc::ControlPlaneState::initializing();
    let initializing_server = bound_server.start_initializing(
        control_state.clone(),
        parent_shutdown.as_ref().map(Arc::clone),
    )?;
    if bootstrap_should_stop(&initializing_server, parent_shutdown.as_ref())? {
        bound_server
            .finish_initializing(initializing_server)
            .map_err(DaemonError::from)?;
        return Ok(());
    }

    let (runtimes, resident_embedding_owner) =
        resolve_optional_runtimes(&mut options, parent_shutdown.as_ref());
    if bootstrap_should_stop(&initializing_server, parent_shutdown.as_ref())? {
        bound_server
            .finish_initializing(initializing_server)
            .map_err(DaemonError::from)?;
        return Ok(());
    }
    control_publisher
        .set_runtimes(runtimes)
        .map_err(DaemonError::from)?;
    let initialization: Result<PersistentRuntime> = (|| {
        #[cfg(test)]
        if let Some(before_store_open) = _hooks.before_store_open.as_ref() {
            before_store_open();
        }
        let store = crate::open_owned_store(&data_directory_owner)?;
        #[cfg(test)]
        if let Some(store_opened) = _hooks.store_opened.as_ref() {
            store_opened();
        }
        let processing_contract = import_processing::current_contract(&options)?;
        let startup_orphaned_recovered = if options.has_worker_loop() {
            let startup_now = crate::current_timestamp()?;
            let recovered =
                import_processing::normalize_orphaned_running_tasks(&store, startup_now)?;
            import_processing::activate_contract(&store, &processing_contract, startup_now)?;
            recovered
        } else {
            0
        };
        let ipc_store = crate::open_store(data_dir)?;
        let ipc_owned_store = store.open_sibling().map_err(DaemonError::store)?;
        Ok(PersistentRuntime {
            store,
            ipc_store,
            ipc_owned_store,
            processing_contract,
            startup_orphaned_recovered,
            _resident_embedding_owner: resident_embedding_owner,
        })
    })();
    if bootstrap_should_stop(&initializing_server, parent_shutdown.as_ref())? {
        bound_server
            .finish_initializing(initializing_server)
            .map_err(DaemonError::from)?;
        return Ok(());
    }

    let runtime = match initialization {
        Ok(runtime) => runtime,
        Err(error) => {
            control_publisher
                .mark_blocked(error.core_block_reason())
                .map_err(DaemonError::from)?;
            bound_server
                .finish_initializing(initializing_server)
                .map_err(DaemonError::from)?;
            bound_server
                .serve_control_only(
                    control_state,
                    parent_shutdown.as_ref(),
                    options.max_requests,
                )
                .map_err(DaemonError::from)?;
            return Ok(());
        }
    };

    control_publisher.prepare_from_store(&runtime.ipc_store);
    bound_server
        .finish_initializing(initializing_server)
        .map_err(DaemonError::from)?;

    if options.has_worker_loop() {
        crate::worker_ipc::run(crate::worker_ipc::Runtime {
            data_dir,
            owned_store: &runtime.store,
            options: &options,
            processing_contract: &runtime.processing_contract,
            startup_orphaned_recovered: runtime.startup_orphaned_recovered,
            parent_shutdown: parent_shutdown.as_ref(),
            bound_server,
            control_state,
            control_publisher,
        })?;
        return Ok(());
    }

    bound_server
        .serve(ipc::server::Context {
            data_dir,
            store: &runtime.ipc_store,
            owned_store: &runtime.ipc_owned_store,
            max_requests: options.max_requests,
            search_runtime_config: options.search_runtime_config(),
            processing_contract: &runtime.processing_contract,
            shutdown: parent_shutdown.as_ref(),
            worker_result_receiver: None,
            artifact_fault_reporter: None,
            control_state,
            control_publisher: Some(control_publisher),
            runtime_health_receiver: None,
        })
        .map_err(DaemonError::from)
}

fn bootstrap_should_stop(
    initializing_server: &ipc::server::InitializingServer,
    parent_shutdown: Option<&Arc<AtomicBool>>,
) -> Result<bool> {
    initializing_server
        .check_health()
        .map_err(DaemonError::from)?;
    Ok(shutdown_requested(parent_shutdown))
}

pub(crate) fn resolve_standalone_runtimes(
    options: &mut RunOptions,
) -> Result<(ipc::OptionalRuntimeMatrix, Option<ResidentEmbeddingOwner>)> {
    let requested_import = options.work_imports || options.work_imports_once;
    let requested_ocr = options.work_ocr || options.work_ocr_once;
    let requested_index = options.work_index || options.work_index_once;
    let resolved = resolve_optional_runtimes(options, None);
    if requested_import && !(options.work_imports || options.work_imports_once) {
        return Err(DaemonError::configuration_invalid(
            "import worker capability unavailable",
        ));
    }
    if requested_ocr && !(options.work_ocr || options.work_ocr_once) {
        return Err(DaemonError::configuration_invalid(
            "ocr worker capability unavailable",
        ));
    }
    if requested_index && !(options.work_index || options.work_index_once) {
        return Err(DaemonError::configuration_invalid(
            "index publication capability unavailable",
        ));
    }
    Ok(resolved)
}

fn resolve_optional_runtimes(
    options: &mut RunOptions,
    shutdown: Option<&Arc<AtomicBool>>,
) -> (ipc::OptionalRuntimeMatrix, Option<ResidentEmbeddingOwner>) {
    if shutdown_requested(shutdown) {
        return cancelled_optional_runtimes(options);
    }
    let is_cancelled = || shutdown_requested(shutdown);
    let classifier = match options.classifier_model_path.as_ref() {
        None => ipc::OptionalRuntimeHealth::unavailable(ipc::OptionalRuntimeReason::NotConfigured),
        Some(path) => {
            match crate::runtime_pack::validate_classifier_with_cancel(path, &is_cancelled) {
                Err(reason) => ipc::OptionalRuntimeHealth::unavailable(reason),
                Ok(model) => {
                    let (health, policy) = start_classifier(model, shutdown);
                    if let Some(policy) = policy {
                        options.linear_promotion = policy;
                    }
                    health
                }
            }
        }
    };
    if shutdown_requested(shutdown) {
        return cancelled_optional_runtimes(options);
    }
    let ocr_path = options
        .ocr_command
        .clone()
        .or_else(|| options.ocr_tesseract_command.clone());
    let renderer_path = options
        .ocr_render_command
        .clone()
        .or_else(|| options.ocr_pdftoppm_command.clone());
    let tessdata_dir = std::env::var_os("TESSDATA_PREFIX").map(PathBuf::from);
    let ocr = match ocr_path {
        None => ipc::OptionalRuntimeHealth::unavailable(ipc::OptionalRuntimeReason::NotConfigured),
        Some(path) => match crate::runtime_pack::validated_ocr_runtime_with_cancel(
            &path,
            renderer_path.as_deref(),
            &options.ocr_lang,
            tessdata_dir.as_deref(),
            &is_cancelled,
        ) {
            Ok(runtime) => {
                let (engine, renderer) = runtime.into_paths();
                if options.ocr_command.is_some() {
                    options.ocr_command = Some(engine.clone());
                } else {
                    options.ocr_tesseract_command = Some(engine.clone());
                }
                if options.ocr_render_command.is_some() {
                    options.ocr_render_command = Some(renderer);
                } else {
                    options.ocr_pdftoppm_command = Some(renderer);
                }
                match tessdata_dir.as_deref() {
                    Some(tessdata_dir) => {
                        match crate::runtime_probe::probe_ocr_with_cancel(
                            &engine,
                            &options.ocr_lang,
                            tessdata_dir,
                            &is_cancelled,
                        ) {
                            Ok(()) => ipc::OptionalRuntimeHealth::available(),
                            Err(reason) => ipc::OptionalRuntimeHealth::unavailable(reason),
                        }
                    }
                    None => {
                        ipc::OptionalRuntimeHealth::unavailable(ipc::OptionalRuntimeReason::Missing)
                    }
                }
            }
            Err(reason) => ipc::OptionalRuntimeHealth::unavailable(reason),
        },
    };
    if shutdown_requested(shutdown) {
        return cancelled_optional_runtimes(options);
    }
    let embedding_runtime_dir =
        std::env::var_os("RESUME_IR_EMBEDDING_RUNTIME_DIR").map(PathBuf::from);
    let (embedding, resident_embedding_owner) = match options.embedding_command.as_ref() {
        None => (
            ipc::OptionalRuntimeHealth::unavailable(ipc::OptionalRuntimeReason::NotConfigured),
            None,
        ),
        Some(_)
            if options.embedding_model_id.is_none() || options.embedding_dimension.is_none() =>
        {
            (
                ipc::OptionalRuntimeHealth::unavailable(ipc::OptionalRuntimeReason::Invalid),
                None,
            )
        }
        Some(path) => match crate::runtime_pack::validate_embedding_with_cancel(
            path,
            options.embedding_model_id.as_deref().unwrap(),
            options.embedding_dimension.unwrap(),
            embedding_runtime_dir.as_deref(),
            &is_cancelled,
        ) {
            Err(reason) => (ipc::OptionalRuntimeHealth::unavailable(reason), None),
            Ok(()) if is_cancelled() => (
                ipc::OptionalRuntimeHealth::unavailable(ipc::OptionalRuntimeReason::StartFailed),
                None,
            ),
            Ok(()) => {
                let timeout_ms = options.embedding_timeout_ms;
                classify_embedding_start(
                    crate::embedding_runtime::start(options),
                    timeout_ms,
                    shutdown,
                )
            }
        },
    };
    let runtimes = ipc::OptionalRuntimeMatrix {
        embedding,
        ocr,
        classifier,
    };
    apply_runtime_worker_gates(options, runtimes);

    (runtimes, resident_embedding_owner)
}

fn load_classifier_policy(
    model: crate::runtime_pack::ValidatedClassifierModel,
    shutdown: Option<&Arc<AtomicBool>>,
) -> Option<import_pipeline::LinearPromotionPolicy> {
    let bytes = model.into_bytes();
    let Some(shutdown) = shutdown else {
        return Some(import_pipeline::LinearPromotionPolicy::load_attested_bundled_bytes(&bytes));
    };
    if shutdown.load(Ordering::Acquire) {
        return None;
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    let worker = thread::Builder::new()
        .name("classifier-bootstrap".to_string())
        .spawn(move || {
            let policy =
                import_pipeline::LinearPromotionPolicy::load_attested_bundled_bytes(&bytes);
            let _ = sender.send(policy);
        })
        .ok()?;
    loop {
        if shutdown.load(Ordering::Acquire) {
            // Classifier loading is read-only and owns no child process. The
            // process can leave this CPU-only worker detached while the
            // parent-driven shutdown path revokes the daemon generation.
            drop(worker);
            return None;
        }
        match receiver.recv_timeout(Duration::from_millis(10)) {
            Ok(policy) => {
                return worker.join().ok().map(|()| policy);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                let _ = worker.join();
                return None;
            }
        }
    }
}

fn start_classifier(
    model: crate::runtime_pack::ValidatedClassifierModel,
    shutdown: Option<&Arc<AtomicBool>>,
) -> (
    ipc::OptionalRuntimeHealth,
    Option<import_pipeline::LinearPromotionPolicy>,
) {
    match load_classifier_policy(model, shutdown) {
        Some(policy) if policy.enabled() => (ipc::OptionalRuntimeHealth::available(), Some(policy)),
        Some(_) | None => (
            ipc::OptionalRuntimeHealth::unavailable(ipc::OptionalRuntimeReason::StartFailed),
            None,
        ),
    }
}

fn cancelled_optional_runtimes(
    options: &mut RunOptions,
) -> (ipc::OptionalRuntimeMatrix, Option<ResidentEmbeddingOwner>) {
    let unavailable =
        || ipc::OptionalRuntimeHealth::unavailable(ipc::OptionalRuntimeReason::StartFailed);
    let runtimes = ipc::OptionalRuntimeMatrix {
        embedding: unavailable(),
        ocr: unavailable(),
        classifier: unavailable(),
    };
    apply_runtime_worker_gates(options, runtimes);
    (runtimes, None)
}

fn shutdown_requested(shutdown: Option<&Arc<AtomicBool>>) -> bool {
    shutdown.is_some_and(|shutdown| shutdown.load(Ordering::Acquire))
}

fn classify_embedding_start(
    start: Result<Option<ResidentEmbeddingOwner>>,
    timeout_ms: u64,
    shutdown: Option<&Arc<AtomicBool>>,
) -> (ipc::OptionalRuntimeHealth, Option<ResidentEmbeddingOwner>) {
    match start {
        Ok(Some(owner)) if wait_for_embedding_ready(&owner, timeout_ms, shutdown) => {
            (ipc::OptionalRuntimeHealth::available(), Some(owner))
        }
        Ok(Some(_)) | Err(_) => (
            ipc::OptionalRuntimeHealth::unavailable(ipc::OptionalRuntimeReason::StartFailed),
            None,
        ),
        Ok(None) => (
            ipc::OptionalRuntimeHealth::unavailable(ipc::OptionalRuntimeReason::NotConfigured),
            None,
        ),
    }
}

fn apply_runtime_worker_gates(options: &mut RunOptions, runtimes: ipc::OptionalRuntimeMatrix) {
    if runtimes.classifier.state != ipc::OptionalRuntimeState::Available {
        options.work_imports = false;
        options.work_imports_once = false;
        options.work_ocr = false;
        options.work_ocr_once = false;
    }
    if runtimes.ocr.state != ipc::OptionalRuntimeState::Available {
        options.work_ocr = false;
        options.work_ocr_once = false;
    }
    if runtimes.embedding.state != ipc::OptionalRuntimeState::Available {
        options.resident_embedding = None;
        options.search_vectorization = Default::default();
        options.work_imports = false;
        options.work_imports_once = false;
        options.work_ocr = false;
        options.work_ocr_once = false;
        options.work_index = false;
        options.work_index_once = false;
    }
}

fn wait_for_embedding_ready(
    owner: &ResidentEmbeddingOwner,
    timeout_ms: u64,
    shutdown: Option<&Arc<AtomicBool>>,
) -> bool {
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(timeout_ms.clamp(100, 30_000)))
        .unwrap_or_else(Instant::now);
    loop {
        if shutdown.is_some_and(|shutdown| shutdown.load(Ordering::Acquire)) {
            return false;
        }
        match owner.client().status() {
            ResidentEmbeddingStatus::Ready => return true,
            ResidentEmbeddingStatus::Unavailable | ResidentEmbeddingStatus::Shutdown => {
                return false;
            }
            ResidentEmbeddingStatus::Starting | ResidentEmbeddingStatus::Restarting => {}
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc, Condvar, Mutex,
    };
    use std::thread;
    use std::time::{Duration, Instant};

    use import_pipeline::{
        finalize_migration_rebuild, prepare_migration_rebuild_artifacts,
        DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, PipelineRunControl,
    };
    use meta_store::UnixTimestamp;

    use super::{
        apply_runtime_worker_gates, classify_embedding_start, resolve_optional_runtimes,
        resolve_standalone_runtimes, run_persistent_ipc_with_hooks, start_classifier,
        BootstrapHooks,
    };
    use crate::run_options::RunOptions;
    use crate::{ipc, DaemonError};

    #[test]
    fn real_bootstrap_keeps_control_plane_live_then_hands_ready_to_full_routes() {
        let directory = tempfile::tempdir().unwrap();
        let options = RunOptions {
            ipc_listen: Some("127.0.0.1:0".parse::<SocketAddr>().unwrap()),
            ..RunOptions::default()
        };
        {
            let seed_owner = match DataDirectoryOwnerLease::try_acquire(directory.path()).unwrap() {
                DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
                DataDirectoryOwnerAcquisition::Contended => panic!("synthetic owner contended"),
            };
            let seed_store = seed_owner.open_store().unwrap();
            let contract = crate::import_processing::current_contract(&options).unwrap();
            let now = UnixTimestamp::from_unix_seconds(1_800_300_000);
            crate::import_processing::activate_contract(&seed_store, &contract, now).unwrap();
            prepare_migration_rebuild_artifacts(&seed_store, now, &PipelineRunControl::default())
                .unwrap();
            finalize_migration_rebuild(
                &seed_store,
                now,
                &contract,
                &options.search_vectorization,
                &PipelineRunControl::default(),
            )
            .unwrap();
        }
        let owner = Arc::new(
            match DataDirectoryOwnerLease::try_acquire(directory.path()).unwrap() {
                DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
                DataDirectoryOwnerAcquisition::Contended => panic!("synthetic owner contended"),
            },
        );
        let launch_id = "b".repeat(64);
        let generation = ipc::DaemonGenerationOwner::acquire(
            Arc::clone(&owner),
            ipc::OwnerMode::DesktopSupervised,
            launch_id.clone(),
        )
        .unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        let barrier = Arc::new((Mutex::new(false), Condvar::new()));
        let hook_barrier = Arc::clone(&barrier);
        let (barrier_reached_sender, barrier_reached_receiver) = mpsc::sync_channel(1);
        let store_open_count = Arc::new(AtomicUsize::new(0));
        let hook_store_open_count = Arc::clone(&store_open_count);
        let hooks = BootstrapHooks {
            before_store_open: Some(Arc::new(move || {
                barrier_reached_sender.send(()).unwrap();
                let (lock, condition) = &*hook_barrier;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = condition.wait(released).unwrap();
                }
            })),
            store_opened: Some(Arc::new(move || {
                hook_store_open_count.fetch_add(1, Ordering::SeqCst);
            })),
        };
        let bootstrap_started = Instant::now();
        let data_dir = directory.path().to_path_buf();
        let daemon_data_dir = data_dir.clone();
        thread::scope(|scope| {
            let cleanup = BootstrapCleanup {
                barrier: Arc::clone(&barrier),
                shutdown: Arc::clone(&shutdown),
            };
            let daemon_shutdown = Arc::clone(&shutdown);
            let daemon = scope.spawn(move || {
                run_persistent_ipc_with_hooks(
                    &daemon_data_dir,
                    options,
                    owner,
                    Some(daemon_shutdown),
                    generation,
                    &hooks,
                )
            });
            barrier_reached_receiver
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
            assert!(bootstrap_started.elapsed() < Duration::from_secs(10));
            assert_eq!(store_open_count.load(Ordering::SeqCst), 0);

            let manifest_path = data_dir.join("ipc.endpoints.json");
            let auth_path = data_dir.join("ipc.auth");
            let manifest_before = fs::read(&manifest_path).unwrap();
            let auth_before = fs::read(&auth_path).unwrap();
            let manifest: serde_json::Value = serde_json::from_slice(&manifest_before).unwrap();
            let auth: serde_json::Value = serde_json::from_slice(&auth_before).unwrap();
            let endpoint = manifest["status"].as_str().unwrap();
            let token = auth["token"].as_str().unwrap();
            assert_eq!(manifest["launch_id"], launch_id);
            assert_eq!(auth["launch_id"], launch_id);

            let initial = authenticated_status(endpoint, token);
            assert_eq!(initial["core"]["state"], "initializing");
            let initializing_business = authenticated_request(endpoint, token, "POST", "/search");
            assert!(
                initializing_business.starts_with("HTTP/1.1 503"),
                "{initializing_business}"
            );
            assert!(initializing_business.contains("SERVICE_INITIALIZING"));
            assert_eq!(store_open_count.load(Ordering::SeqCst), 0);

            thread::sleep(Duration::from_millis(10_050));
            let still_initializing = authenticated_status(endpoint, token);
            assert_eq!(still_initializing["core"]["state"], "initializing");
            assert_eq!(store_open_count.load(Ordering::SeqCst), 0);
            assert_eq!(fs::read(&manifest_path).unwrap(), manifest_before);
            assert_eq!(fs::read(&auth_path).unwrap(), auth_before);

            let (lock, condition) = &*barrier;
            *lock.lock().unwrap() = true;
            condition.notify_one();
            let ready_deadline = Instant::now() + Duration::from_secs(5);
            let ready_status = loop {
                let status = authenticated_status(endpoint, token);
                if status["core"]["state"] == "ready" {
                    break Ok(status);
                }
                if Instant::now() >= ready_deadline {
                    break Err(status);
                }
                thread::sleep(Duration::from_millis(10));
            };
            assert_eq!(store_open_count.load(Ordering::SeqCst), 1);
            assert_eq!(fs::read(&manifest_path).unwrap(), manifest_before);
            assert_eq!(fs::read(&auth_path).unwrap(), auth_before);

            let first_ready_business = ready_status
                .as_ref()
                .map(|_| authenticated_request(endpoint, token, "POST", "/search"));
            cleanup.stop();
            daemon.join().unwrap().unwrap();
            assert!(
                ready_status.is_ok(),
                "bootstrap did not become ready: {}",
                ready_status.unwrap_err()["core"]
            );
            let first_ready_business = first_ready_business.unwrap();
            assert!(
                !first_ready_business.starts_with("HTTP/1.1 404"),
                "{first_ready_business}"
            );
            assert!(first_ready_business.contains("resume-ir.error.v2"));
        });
    }

    fn authenticated_status(endpoint: &str, token: &str) -> serde_json::Value {
        let response = authenticated_request(endpoint, token, "GET", "/status");
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    }

    fn authenticated_request(endpoint: &str, token: &str, method: &str, path: &str) -> String {
        let address = endpoint
            .strip_prefix("http://")
            .unwrap()
            .strip_suffix("/status")
            .unwrap();
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let body = if method == "POST" { "{}" } else { "" };
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    struct BootstrapCleanup {
        barrier: Arc<(Mutex<bool>, Condvar)>,
        shutdown: Arc<AtomicBool>,
    }

    impl BootstrapCleanup {
        fn stop(&self) {
            self.shutdown.store(true, Ordering::Release);
            let (lock, condition) = &*self.barrier;
            *lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
            condition.notify_all();
        }
    }

    impl Drop for BootstrapCleanup {
        fn drop(&mut self) {
            self.stop();
        }
    }

    #[test]
    fn missing_optional_commands_leave_only_existing_read_paths_available() {
        let mut options = RunOptions {
            work_imports: true,
            work_ocr: true,
            work_index: true,
            ..RunOptions::default()
        };
        let (runtimes, owner) = resolve_optional_runtimes(&mut options, None);

        assert!(owner.is_none());
        assert_eq!(
            runtimes.embedding.reason,
            Some(ipc::OptionalRuntimeReason::NotConfigured)
        );
        assert_eq!(
            runtimes.ocr.reason,
            Some(ipc::OptionalRuntimeReason::NotConfigured)
        );
        assert_eq!(
            runtimes.classifier.reason,
            Some(ipc::OptionalRuntimeReason::NotConfigured)
        );
        let core = ipc::CoreHealth {
            state: ipc::CoreState::Ready,
            reason: None,
        };
        let capabilities = ipc::CapabilityMatrix::derive(core, runtimes);
        assert_eq!(
            capabilities.keyword_search.state,
            ipc::CapabilityState::Available
        );
        assert_eq!(capabilities.detail.state, ipc::CapabilityState::Available);
        assert_eq!(
            capabilities.text_import.state,
            ipc::CapabilityState::Unavailable
        );
        assert_eq!(
            capabilities.index_publication.state,
            ipc::CapabilityState::Unavailable
        );
        assert!(!options.work_imports);
        assert!(!options.work_ocr);
        assert!(!options.work_index);
        assert!(
            !options.has_worker_loop(),
            "runtime capability gate must run before any worker can claim or mutate a task"
        );
    }

    #[test]
    fn parent_shutdown_cancels_runtime_resolution_before_worker_enablement() {
        let shutdown = Arc::new(AtomicBool::new(true));
        let mut options = RunOptions {
            work_imports: true,
            work_ocr: true,
            work_index: true,
            ..RunOptions::default()
        };

        let (runtimes, owner) = resolve_optional_runtimes(&mut options, Some(&shutdown));

        assert!(owner.is_none());
        for runtime in [runtimes.embedding, runtimes.ocr, runtimes.classifier] {
            assert_eq!(runtime.state, ipc::OptionalRuntimeState::Unavailable);
            assert_eq!(
                runtime.reason,
                Some(ipc::OptionalRuntimeReason::StartFailed)
            );
        }
        assert!(!options.has_worker_loop());
    }

    #[test]
    fn all_eight_runtime_combinations_gate_capabilities_and_worker_claims() {
        let core = ipc::CoreHealth {
            state: ipc::CoreState::Ready,
            reason: None,
        };
        for bits in 0_u8..8 {
            let embedding = bits & 0b001 != 0;
            let ocr = bits & 0b010 != 0;
            let classifier = bits & 0b100 != 0;
            let health = |available| {
                if available {
                    ipc::OptionalRuntimeHealth::available()
                } else {
                    ipc::OptionalRuntimeHealth::unavailable(ipc::OptionalRuntimeReason::Invalid)
                }
            };
            let runtimes = ipc::OptionalRuntimeMatrix {
                embedding: health(embedding),
                ocr: health(ocr),
                classifier: health(classifier),
            };
            let capabilities = ipc::CapabilityMatrix::derive(core, runtimes);
            let mut options = all_workers();
            apply_runtime_worker_gates(&mut options, runtimes);
            let import_available = embedding && classifier;
            let index_available = embedding;
            let ocr_available = import_available && ocr;

            assert_eq!(
                capabilities.keyword_search.state,
                ipc::CapabilityState::Available,
                "runtime matrix row {bits:03b}"
            );
            assert_eq!(
                capabilities.detail.state,
                ipc::CapabilityState::Available,
                "runtime matrix row {bits:03b}"
            );
            assert_eq!(
                capabilities.semantic_search.state == ipc::CapabilityState::Available,
                embedding,
                "runtime matrix row {bits:03b}"
            );
            assert_eq!(
                capabilities.text_import.state == ipc::CapabilityState::Available,
                import_available,
                "runtime matrix row {bits:03b}"
            );
            assert_eq!(
                capabilities.ocr_import.state == ipc::CapabilityState::Available,
                ocr_available,
                "runtime matrix row {bits:03b}"
            );
            assert_eq!(
                capabilities.index_publication.state == ipc::CapabilityState::Available,
                index_available,
                "runtime matrix row {bits:03b}"
            );
            assert_eq!(options.work_imports, import_available);
            assert_eq!(options.work_imports_once, import_available);
            assert_eq!(options.work_index, index_available);
            assert_eq!(options.work_index_once, index_available);
            assert_eq!(options.work_ocr, ocr_available);
            assert_eq!(options.work_ocr_once, ocr_available);
        }
    }

    #[test]
    fn standalone_worker_requests_fail_before_store_open_when_runtime_is_missing() {
        let mut options = RunOptions {
            work_index_once: true,
            ..RunOptions::default()
        };

        assert!(resolve_standalone_runtimes(&mut options).is_err());
        assert!(!options.work_index_once);
    }

    #[test]
    fn runtime_start_errors_have_a_distinct_start_failed_reason() {
        let (health, owner) = classify_embedding_start(
            Err(DaemonError::configuration_invalid(
                "synthetic embedding startup failure",
            )),
            100,
            None,
        );

        assert!(owner.is_none());
        assert_eq!(health.state, ipc::OptionalRuntimeState::Unavailable);
        assert_eq!(health.reason, Some(ipc::OptionalRuntimeReason::StartFailed));
    }

    #[test]
    fn classifier_activation_failure_is_distinct_from_pack_validation() {
        let model = crate::runtime_pack::ValidatedClassifierModel::from_bytes_for_test(
            b"synthetic-invalid-attested-classifier-envelope".to_vec(),
        );
        let (health, policy) = start_classifier(model, None);

        assert!(policy.is_none());
        assert_eq!(health.state, ipc::OptionalRuntimeState::Unavailable);
        assert_eq!(health.reason, Some(ipc::OptionalRuntimeReason::StartFailed));
    }

    fn all_workers() -> RunOptions {
        RunOptions {
            work_imports: true,
            work_imports_once: true,
            work_ocr: true,
            work_ocr_once: true,
            work_index: true,
            work_index_once: true,
            ..RunOptions::default()
        }
    }
}
