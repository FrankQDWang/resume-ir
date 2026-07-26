use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread;

use meta_store::{ImportProcessingContract, OwnedMetaStore};

use crate::daemon_error::{DaemonError, Result};
use crate::run_options::RunOptions;
use crate::worker_runtime::{run_worker_loop, WorkerLoopRuntime, WorkerSummaryOutput};
use crate::{ipc, open_store};

pub(crate) struct Runtime<'a> {
    pub(crate) data_dir: &'a Path,
    pub(crate) owned_store: &'a OwnedMetaStore,
    pub(crate) options: &'a RunOptions,
    pub(crate) processing_contract: &'a ImportProcessingContract,
    pub(crate) startup_orphaned_recovered: usize,
    pub(crate) parent_shutdown: Option<&'a Arc<AtomicBool>>,
    pub(crate) bound_server: ipc::server::BoundServer,
    pub(crate) control_state: ipc::ControlPlaneState,
    pub(crate) control_publisher: ipc::ControlPlanePublisher,
}

pub(crate) fn run(runtime: Runtime<'_>) -> Result<()> {
    let Runtime {
        data_dir,
        owned_store,
        options,
        processing_contract,
        startup_orphaned_recovered,
        parent_shutdown,
        bound_server,
        control_state,
        control_publisher,
    } = runtime;
    let ipc_store = open_store(data_dir)?;
    let ipc_owned_store = owned_store.open_sibling().map_err(DaemonError::store)?;
    let worker_store = owned_store.open_sibling().map_err(DaemonError::store)?;
    let stop_worker = parent_shutdown
        .cloned()
        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
    let worker_stop = Arc::clone(&stop_worker);
    let worker_data_dir = data_dir.to_path_buf();
    let worker_options = options.clone();
    let worker_capability_state = control_state.clone();
    let (runtime_health_reporter, runtime_health_receiver) = ipc::runtime_health_channel();
    let worker_processing_contract = processing_contract.clone();
    let (artifact_fault_reporter, artifact_fault_receiver) = if options.work_index {
        let (reporter, receiver) = ipc::search_service::artifact_fault_latch();
        (Some(reporter), Some(receiver))
    } else {
        (None, None)
    };
    let (worker_result_sender, worker_result_receiver) =
        mpsc::channel::<std::result::Result<(), ipc::DaemonFatalError>>();
    let worker_handle = thread::spawn(move || {
        let result = run_worker_loop(
            &worker_data_dir,
            &worker_store,
            &worker_options,
            &worker_processing_contract,
            WorkerLoopRuntime {
                startup_orphaned_recovered,
                stop_signal: Some(worker_stop),
                artifact_fault_receiver,
                summary_output: WorkerSummaryOutput::Suppressed,
                capability_state: Some(worker_capability_state),
                runtime_health_reporter: Some(runtime_health_reporter),
            },
        );
        let _ = worker_result_sender.send(result.map(|_| ()));
    });

    let ipc_result = bound_server.serve(ipc::server::Context {
        data_dir,
        store: &ipc_store,
        owned_store: &ipc_owned_store,
        max_requests: options.max_requests,
        search_runtime_config: options.search_runtime_config(),
        processing_contract,
        shutdown: Some(&stop_worker),
        worker_result_receiver: Some(&worker_result_receiver),
        artifact_fault_reporter,
        control_state,
        control_publisher: Some(control_publisher),
        runtime_health_receiver: Some(runtime_health_receiver),
    });
    stop_worker.store(true, Ordering::Release);
    if let Err(fatal) = ipc_result {
        abort_worker_for_process_exit(worker_handle);
        return Err(DaemonError::from(fatal));
    }
    worker_handle
        .join()
        .map_err(|_| DaemonError::control_plane("worker thread panicked"))?;
    Ok(())
}

fn abort_worker_for_process_exit(worker_handle: thread::JoinHandle<()>) {
    // The stop signal is already raised. The process supervisor containment
    // deadline owns final tree termination if a data-plane call cannot join.
    drop(worker_handle);
}
