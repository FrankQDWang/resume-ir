use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use import_pipeline::ImportTaskOwnerLock;
use import_pipeline::{
    SearchProjectionRemoval, SearchProjectionRemovalReason, SearchPublicationVectorization,
};
use meta_store::{
    ImportProcessingContract, OwnedMetaStore, SourceRootDeletion, SourceRootDeletionPhase,
    SourceRootId, SourceWatcherState,
};

use crate::command_failure::CommandFailure;
use crate::import_command::{RootControlAction, RootControlCommand};

pub(crate) struct DeletionRequest {
    pub(crate) receipt: SourceRootDeletion,
}

static ACTIVE_WORKERS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

struct WorkerRegistration {
    root_id: String,
}

impl WorkerRegistration {
    fn acquire(root_id: &SourceRootId) -> Result<Option<Self>, CommandFailure> {
        let root_id = root_id.as_str().to_string();
        let mut active = ACTIVE_WORKERS
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
            .map_err(|_| CommandFailure::Internal)?;
        if !active.insert(root_id.clone()) {
            return Ok(None);
        }
        Ok(Some(Self { root_id }))
    }
}

impl Drop for WorkerRegistration {
    fn drop(&mut self) {
        if let Ok(mut active) = ACTIVE_WORKERS
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
        {
            active.remove(&self.root_id);
        }
    }
}

pub(crate) fn request(
    store: &OwnedMetaStore,
    root_id: &SourceRootId,
) -> Result<DeletionRequest, CommandFailure> {
    if let Some(receipt) = store
        .source_root_deletion(root_id)
        .map_err(|_| CommandFailure::Internal)?
        .filter(|receipt| {
            !matches!(
                receipt.phase,
                SourceRootDeletionPhase::Complete | SourceRootDeletionPhase::Failed
            )
        })
    {
        return Ok(DeletionRequest { receipt });
    }
    let now = crate::current_timestamp().map_err(|_| CommandFailure::Internal)?;
    let receipt = store
        .begin_source_root_deletion(root_id, now)
        .map_err(|error| match error.class() {
            meta_store::MetaStoreErrorClass::NotFound => {
                CommandFailure::NotFound("source root was not found")
            }
            _ => CommandFailure::Internal,
        })?;
    Ok(DeletionRequest { receipt })
}

pub(crate) fn spawn_worker(
    data_dir: PathBuf,
    store: OwnedMetaStore,
    processing_contract: ImportProcessingContract,
    root_id: SourceRootId,
) -> Result<(), CommandFailure> {
    let Some(registration) = WorkerRegistration::acquire(&root_id)? else {
        return Ok(());
    };
    thread::Builder::new()
        .name("source-root-deletion".to_string())
        .spawn(move || {
            let _registration = registration;
            let mut retry_index = 0_usize;
            loop {
                match resume(&data_dir, &store, &processing_contract, &root_id) {
                    Ok(_) => return,
                    Err(_) => {
                        const RETRY_DELAYS: [Duration; 5] = [
                            Duration::from_millis(250),
                            Duration::from_secs(1),
                            Duration::from_secs(4),
                            Duration::from_secs(15),
                            Duration::from_secs(30),
                        ];
                        thread::sleep(RETRY_DELAYS[retry_index.min(RETRY_DELAYS.len() - 1)]);
                        retry_index = retry_index.saturating_add(1);
                    }
                }
            }
        })
        .map(|_| ())
        .map_err(|_| CommandFailure::Internal)
}

pub(crate) fn spawn_pending(
    data_dir: &Path,
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
) -> Result<usize, CommandFailure> {
    let pending = store
        .incomplete_source_root_deletions()
        .map_err(|_| CommandFailure::Internal)?;
    for receipt in &pending {
        let sibling = store.open_sibling().map_err(|_| CommandFailure::Internal)?;
        spawn_worker(
            data_dir.to_path_buf(),
            sibling,
            processing_contract.clone(),
            receipt.root_id.clone(),
        )?;
    }
    Ok(pending.len())
}

pub(crate) fn resume_pending(
    data_dir: &Path,
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
) -> Result<usize, CommandFailure> {
    let pending = store
        .incomplete_source_root_deletions()
        .map_err(|_| CommandFailure::Internal)?;
    for receipt in &pending {
        resume(data_dir, store, processing_contract, &receipt.root_id)?;
    }
    Ok(pending.len())
}

fn resume(
    data_dir: &Path,
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    root_id: &SourceRootId,
) -> Result<SourceRootDeletion, CommandFailure> {
    let receipt = store
        .source_root_deletion(root_id)
        .map_err(|_| CommandFailure::Internal)?
        .ok_or(CommandFailure::NotFound(
            "source root deletion was not found",
        ))?;
    if matches!(
        receipt.phase,
        SourceRootDeletionPhase::Complete | SourceRootDeletionPhase::Failed
    ) {
        return Ok(receipt);
    }
    let now = crate::current_timestamp().map_err(|_| CommandFailure::Internal)?;
    let mut phase = receipt.phase;
    if phase == SourceRootDeletionPhase::Requested {
        store
            .set_source_root_deletion_phase(root_id, SourceRootDeletionPhase::Quiescing, now)
            .map_err(|_| CommandFailure::Internal)?;
        phase = SourceRootDeletionPhase::Quiescing;
    }
    let task_owners = if phase != SourceRootDeletionPhase::Verifying {
        store
            .source_root(root_id)
            .map_err(|_| CommandFailure::Internal)?
    } else {
        None
    }
    .map(|root| {
        let _ = crate::import_command::control_root(
            store,
            processing_contract,
            RootControlCommand {
                root_path: root.canonical_path.clone(),
                action: RootControlAction::Pause,
            },
        );
        store
            .set_source_root_state(root_id, root.state, SourceWatcherState::Paused, now)
            .map_err(|_| CommandFailure::Internal)?;
        acquire_root_task_quiescence(data_dir, store, &root.canonical_path)
    })
    .transpose()?
    .unwrap_or_default();

    if phase == SourceRootDeletionPhase::Quiescing {
        let documents = store
            .source_root_deletion_document_ids(root_id)
            .map_err(|_| CommandFailure::Internal)?;
        if !crate::ocr_worker::wait_for_documents_to_quiesce(&documents, Duration::from_secs(5)) {
            return Err(CommandFailure::ServiceUnavailable(
                "source root OCR is quiescing",
            ));
        }
        store
            .set_source_root_deletion_phase(root_id, SourceRootDeletionPhase::Publishing, now)
            .map_err(|_| CommandFailure::Internal)?;
        phase = SourceRootDeletionPhase::Publishing;
    }
    if phase == SourceRootDeletionPhase::Publishing {
        publish_root_removal(store, root_id, now)?;
        store
            .set_source_root_deletion_phase(root_id, SourceRootDeletionPhase::Purging, now)
            .map_err(|_| CommandFailure::Internal)?;
        phase = SourceRootDeletionPhase::Purging;
    }
    if phase == SourceRootDeletionPhase::Purging {
        purge_root_data(store, root_id, now)?;
        phase = SourceRootDeletionPhase::Verifying;
    }
    if phase != SourceRootDeletionPhase::Verifying {
        return Err(CommandFailure::Internal);
    }
    finish_root_data_cleanup(store, root_id)?;
    let receipt = store
        .complete_source_root_deletion(root_id, now)
        .map_err(|_| CommandFailure::Internal)?;
    drop(task_owners);
    Ok(receipt)
}

fn publish_root_removal(
    store: &OwnedMetaStore,
    root_id: &SourceRootId,
    now: meta_store::UnixTimestamp,
) -> Result<(), CommandFailure> {
    let documents = store
        .source_root_deletion_document_ids(root_id)
        .map_err(|_| CommandFailure::Internal)?;
    let removals = documents
        .iter()
        .cloned()
        .map(|document_id| SearchProjectionRemoval {
            document_id,
            reason: SearchProjectionRemovalReason::PrivacyRevocation,
        })
        .collect::<Vec<_>>();
    if !removals.is_empty() {
        import_pipeline::publish_search_projection_removals(
            store,
            &removals,
            now,
            &SearchPublicationVectorization::default(),
        )
        .map_err(|_| CommandFailure::Internal)?;
    }
    store
        .purge_import_tasks_for_deleted_documents(&documents)
        .map_err(|_| CommandFailure::Internal)?;
    store
        .purge_ingest_jobs_for_documents(&documents)
        .map(|_| ())
        .map_err(|_| CommandFailure::Internal)
}

fn purge_root_data(
    store: &OwnedMetaStore,
    root_id: &SourceRootId,
    now: meta_store::UnixTimestamp,
) -> Result<(), CommandFailure> {
    if store
        .source_root(root_id)
        .map_err(|_| CommandFailure::Internal)?
        .is_some()
    {
        store
            .purge_source_root_data(root_id, now)
            .map_err(|_| CommandFailure::Internal)?;
    }
    Ok(())
}

fn finish_root_data_cleanup(
    store: &OwnedMetaStore,
    root_id: &SourceRootId,
) -> Result<(), CommandFailure> {
    let unreferenced_content_hashes = store
        .source_root_unreferenced_content_hashes(root_id)
        .map_err(|_| CommandFailure::Internal)?;
    store
        .purge_ocr_page_cache_by_content_hashes(&unreferenced_content_hashes)
        .map_err(|_| CommandFailure::Internal)?;
    loop {
        let report = store
            .purge_source_root_deleted_documents(root_id)
            .map_err(|_| CommandFailure::Internal)?;
        if report.remaining_tombstones == 0 {
            break;
        }
    }
    store
        .destroy_retained_migration_predecessor()
        .map_err(|_| CommandFailure::Internal)?;
    Ok(())
}

fn acquire_root_task_quiescence(
    data_dir: &Path,
    store: &OwnedMetaStore,
    canonical_root_path: &str,
) -> Result<Vec<ImportTaskOwnerLock>, CommandFailure> {
    const QUIESCE_TIMEOUT: Duration = Duration::from_secs(5);
    const POLL_INTERVAL: Duration = Duration::from_millis(25);

    let tasks = store
        .active_import_tasks_for_root_quiescence(canonical_root_path)
        .map_err(|_| CommandFailure::Internal)?;
    let mut owners = Vec::with_capacity(tasks.len());
    let deadline = Instant::now() + QUIESCE_TIMEOUT;
    for task in tasks {
        loop {
            match ImportTaskOwnerLock::try_acquire(data_dir, &task.id) {
                Ok(Some(owner)) => {
                    owners.push(owner);
                    break;
                }
                Ok(None) if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
                Ok(None) => {
                    return Err(CommandFailure::ServiceUnavailable(
                        "source root deletion is quiescing",
                    ));
                }
                Err(_) => return Err(CommandFailure::Internal),
            }
        }
    }
    Ok(owners)
}
