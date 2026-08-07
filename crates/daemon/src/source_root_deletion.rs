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
    ImportProcessingContract, OwnedMetaStore, SearchProjectionServiceState, SearchRepairReason,
    SourceRootDeletion, SourceRootDeletionErrorCode, SourceRootDeletionPhase, SourceRootId,
    SourceWatcherState,
};

use crate::command_failure::CommandFailure;
use crate::import_command::{RootControlAction, RootControlCommand};

pub(crate) struct DeletionRequest {
    pub(crate) receipt: SourceRootDeletion,
}

struct DeletionAttemptFailure {
    code: SourceRootDeletionErrorCode,
    transport: CommandFailure,
}

impl DeletionAttemptFailure {
    fn internal(code: SourceRootDeletionErrorCode) -> Self {
        Self {
            code,
            transport: CommandFailure::Internal,
        }
    }

    fn unavailable(code: SourceRootDeletionErrorCode, message: &'static str) -> Self {
        Self {
            code,
            transport: CommandFailure::ServiceUnavailable(message),
        }
    }
}

const RETRY_DELAYS: [Duration; 5] = [
    Duration::from_millis(250),
    Duration::from_secs(1),
    Duration::from_secs(4),
    Duration::from_secs(15),
    Duration::from_secs(30),
];

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
            drive_worker_until_complete(
                || attempt_resume(&data_dir, &store, &processing_contract, &root_id).map(|_| ()),
                thread::sleep,
            );
        })
        .map(|_| ())
        .map_err(|_| CommandFailure::Internal)
}

fn drive_worker_until_complete(
    mut attempt: impl FnMut() -> Result<(), CommandFailure>,
    mut wait: impl FnMut(Duration),
) {
    let mut retry_index = 0_usize;
    loop {
        if let Ok(()) = attempt() {
            return;
        }
        let delay = RETRY_DELAYS[retry_index.min(RETRY_DELAYS.len() - 1)];
        retry_index = retry_index.saturating_add(1);
        wait(delay);
    }
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
        let Some(_registration) = WorkerRegistration::acquire(&receipt.root_id)? else {
            continue;
        };
        attempt_resume(data_dir, store, processing_contract, &receipt.root_id)?;
    }
    Ok(pending.len())
}

fn attempt_resume(
    data_dir: &Path,
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    root_id: &SourceRootId,
) -> Result<SourceRootDeletion, CommandFailure> {
    let now = crate::current_timestamp().map_err(|_| CommandFailure::Internal)?;
    store
        .begin_source_root_deletion_attempt(root_id, now)
        .map_err(|_| CommandFailure::Internal)?;
    match resume(data_dir, store, processing_contract, root_id) {
        Ok(receipt) => Ok(receipt),
        Err(failure) => {
            let phase = store
                .source_root_deletion(root_id)
                .map_err(|_| CommandFailure::Internal)?
                .ok_or(CommandFailure::Internal)?
                .phase;
            let failed_at = crate::current_timestamp().map_err(|_| CommandFailure::Internal)?;
            store
                .record_source_root_deletion_attempt_failure(
                    root_id,
                    phase,
                    failure.code,
                    failed_at,
                )
                .map_err(|_| CommandFailure::Internal)?;
            Err(failure.transport)
        }
    }
}

fn resume(
    data_dir: &Path,
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    root_id: &SourceRootId,
) -> Result<SourceRootDeletion, DeletionAttemptFailure> {
    let receipt = store
        .source_root_deletion(root_id)
        .map_err(|_| DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::Internal))?
        .ok_or_else(|| DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::Internal))?;
    if matches!(
        receipt.phase,
        SourceRootDeletionPhase::Complete | SourceRootDeletionPhase::Failed
    ) {
        return Ok(receipt);
    }
    let now = crate::current_timestamp()
        .map_err(|_| DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::Internal))?;
    let mut phase = receipt.phase;
    if phase == SourceRootDeletionPhase::Requested {
        store
            .set_source_root_deletion_phase(root_id, SourceRootDeletionPhase::Quiescing, now)
            .map_err(|_| DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::Internal))?;
        phase = SourceRootDeletionPhase::Quiescing;
    }
    let task_owners = if phase != SourceRootDeletionPhase::Verifying {
        store
            .source_root(root_id)
            .map_err(|_| DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::Internal))?
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
            .map_err(|_| DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::Internal))?;
        acquire_root_task_quiescence(data_dir, store, &root.canonical_path, now)
    })
    .transpose()?
    .unwrap_or_default();

    if phase == SourceRootDeletionPhase::Quiescing {
        let documents = store
            .source_root_deletion_document_ids(root_id)
            .map_err(|_| DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::Internal))?;
        if !crate::ocr_worker::cancel_and_wait_for_documents_to_quiesce(
            &documents,
            Duration::from_secs(5),
        ) {
            return Err(DeletionAttemptFailure::unavailable(
                SourceRootDeletionErrorCode::OcrQuiescenceTimeout,
                "source root OCR is quiescing",
            ));
        }
        store
            .purge_ingest_jobs_for_documents(&documents)
            .map_err(|_| DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::Internal))?;
        if !crate::ocr_worker::cancel_and_wait_for_documents_to_quiesce(
            &documents,
            Duration::from_secs(5),
        ) {
            return Err(DeletionAttemptFailure::unavailable(
                SourceRootDeletionErrorCode::OcrQuiescenceTimeout,
                "source root OCR is quiescing",
            ));
        }
        store
            .set_source_root_deletion_phase(root_id, SourceRootDeletionPhase::Publishing, now)
            .map_err(|_| DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::Internal))?;
        phase = SourceRootDeletionPhase::Publishing;
    }
    if phase == SourceRootDeletionPhase::Publishing {
        publish_root_removal(store, root_id, now)?;
        store
            .set_source_root_deletion_phase(root_id, SourceRootDeletionPhase::Purging, now)
            .map_err(|_| DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::Internal))?;
        phase = SourceRootDeletionPhase::Purging;
    }
    if phase == SourceRootDeletionPhase::Purging {
        purge_root_data(store, root_id, now)?;
        phase = SourceRootDeletionPhase::Verifying;
    }
    if phase != SourceRootDeletionPhase::Verifying {
        return Err(DeletionAttemptFailure::internal(
            SourceRootDeletionErrorCode::Internal,
        ));
    }
    finish_root_data_cleanup(store, root_id, now)?;
    let receipt = store
        .complete_source_root_deletion(root_id, now)
        .map_err(|_| {
            DeletionAttemptFailure::unavailable(
                SourceRootDeletionErrorCode::ReceiptCompletionFailed,
                "receipt_completion",
            )
        })?;
    drop(task_owners);
    Ok(receipt)
}

fn publish_root_removal(
    store: &OwnedMetaStore,
    root_id: &SourceRootId,
    now: meta_store::UnixTimestamp,
) -> Result<(), DeletionAttemptFailure> {
    ensure_search_head_ready_for_privacy_publication(store, now)?;
    let documents = store
        .source_root_deletion_document_ids(root_id)
        .map_err(|_| {
            DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::PublicationFailed)
        })?;
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
        .map_err(|_| {
            DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::PublicationFailed)
        })?;
    }
    store
        .purge_import_tasks_for_deleted_documents(&documents)
        .map_err(|_| {
            DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::PublicationFailed)
        })?;
    Ok(())
}

fn ensure_search_head_ready_for_privacy_publication(
    store: &OwnedMetaStore,
    now: meta_store::UnixTimestamp,
) -> Result<(), DeletionAttemptFailure> {
    let state = store
        .search_projection_state()
        .map_err(|_| DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::Internal))?;
    match (
        state.service_state,
        state.repair_reason,
        state.generation.as_deref(),
    ) {
        (SearchProjectionServiceState::Ready, None, Some(_)) => Ok(()),
        (
            SearchProjectionServiceState::RepairBlocked,
            Some(SearchRepairReason::RuntimeInvariant),
            Some(generation),
        ) => {
            let _ = store
                .reopen_runtime_invariant_for_artifact_repair(
                    generation,
                    state.visible_epoch,
                    now,
                )
                .map_err(|_| {
                    DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::Internal)
                })?;
            Err(DeletionAttemptFailure::unavailable(
                SourceRootDeletionErrorCode::Internal,
                "search artifacts are recovering",
            ))
        }
        (
            SearchProjectionServiceState::Repairing,
            Some(SearchRepairReason::ArtifactUnavailable | SearchRepairReason::MigrationRebuild),
            _,
        ) => Err(DeletionAttemptFailure::unavailable(
            SourceRootDeletionErrorCode::Internal,
            "search artifacts are recovering",
        )),
        (
            SearchProjectionServiceState::RepairBlocked,
            Some(SearchRepairReason::SourceUnavailable),
            _,
        ) => Err(DeletionAttemptFailure::unavailable(
            SourceRootDeletionErrorCode::Internal,
            "search source is unavailable",
        )),
        _ => Err(DeletionAttemptFailure::internal(
            SourceRootDeletionErrorCode::PublicationFailed,
        )),
    }
}

fn purge_root_data(
    store: &OwnedMetaStore,
    root_id: &SourceRootId,
    now: meta_store::UnixTimestamp,
) -> Result<(), DeletionAttemptFailure> {
    if store
        .source_root(root_id)
        .map_err(|_| {
            DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::MetadataPurgeFailed)
        })?
        .is_some()
    {
        store.purge_source_root_data(root_id, now).map_err(|_| {
            DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::MetadataPurgeFailed)
        })?;
    }
    Ok(())
}

fn finish_root_data_cleanup(
    store: &OwnedMetaStore,
    root_id: &SourceRootId,
    now: meta_store::UnixTimestamp,
) -> Result<(), DeletionAttemptFailure> {
    // Re-issue PrivacyRevocation before privacy purge so a Verifying restart after a
    // Publishing crash (or a stuck active projection) converges without reopening
    // Quiescing and without weakening the projection fail-closed gate.
    publish_root_removal(store, root_id, now)?;
    let unreferenced_content_hashes = store
        .source_root_unreferenced_content_hashes(root_id)
        .map_err(|_| {
            DeletionAttemptFailure::unavailable(
                SourceRootDeletionErrorCode::PrivacyCleanupFailed,
                "privacy_cleanup",
            )
        })?;
    store
        .purge_ocr_page_cache_by_content_hashes(&unreferenced_content_hashes)
        .map_err(|_| {
            DeletionAttemptFailure::unavailable(
                SourceRootDeletionErrorCode::PrivacyCleanupFailed,
                "privacy_cleanup",
            )
        })?;
    loop {
        let report = store
            .purge_source_root_deleted_documents(root_id)
            .map_err(|_| {
                DeletionAttemptFailure::unavailable(
                    SourceRootDeletionErrorCode::PrivacyCleanupFailed,
                    "privacy_cleanup",
                )
            })?;
        if report.remaining_tombstones == 0 {
            break;
        }
    }
    store
        .destroy_retained_migration_predecessor()
        .map_err(|_| {
            DeletionAttemptFailure::unavailable(
                SourceRootDeletionErrorCode::PrivacyCleanupFailed,
                "privacy_cleanup",
            )
        })?;
    Ok(())
}

fn acquire_root_task_quiescence(
    data_dir: &Path,
    store: &OwnedMetaStore,
    canonical_root_path: &str,
    now: meta_store::UnixTimestamp,
) -> Result<Vec<ImportTaskOwnerLock>, DeletionAttemptFailure> {
    const QUIESCE_TIMEOUT: Duration = Duration::from_secs(5);
    const POLL_INTERVAL: Duration = Duration::from_millis(25);

    let tasks = store
        .active_import_tasks_for_root_quiescence(canonical_root_path)
        .map_err(|_| DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::Internal))?;
    let mut owners = Vec::with_capacity(tasks.len());
    let deadline = Instant::now() + QUIESCE_TIMEOUT;
    for task in tasks {
        let cancel_at = meta_store::UnixTimestamp::from_unix_seconds(
            now.as_unix_seconds().max(task.updated_at.as_unix_seconds()),
        );
        store
            .cancel_import_task(&task.id, cancel_at)
            .map_err(|_| DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::Internal))?;
        loop {
            match ImportTaskOwnerLock::try_acquire(data_dir, &task.id) {
                Ok(Some(owner)) => {
                    owners.push(owner);
                    break;
                }
                Ok(None) if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
                Ok(None) => {
                    return Err(DeletionAttemptFailure::unavailable(
                        SourceRootDeletionErrorCode::ImportQuiescenceTimeout,
                        "source root deletion is quiescing",
                    ));
                }
                Err(_) => {
                    return Err(DeletionAttemptFailure::internal(
                        SourceRootDeletionErrorCode::Internal,
                    ));
                }
            }
        }
    }
    Ok(owners)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fs;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use import_pipeline::{
        finalize_migration_rebuild, PipelineRunControl, SearchPublicationVectorization,
    };
    use meta_store::{
        DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, ImportProcessingContract,
        SearchProjectionServiceState, SearchProjectionTransitionOutcome, SearchRepairReason,
        SourceRootDeletionErrorCode, SourceRootDeletionPhase, UnixTimestamp, CLASSIFIER_EPOCH,
    };
    use tempfile::tempdir;

    use super::{
        drive_worker_until_complete, ensure_search_head_ready_for_privacy_publication,
        resume_pending, DeletionAttemptFailure, WorkerRegistration,
    };
    use crate::command_failure::CommandFailure;

    #[test]
    fn typed_worker_failures_retry_with_capped_backoff() {
        let attempts = Cell::new(0_u8);
        let mut delays = Vec::new();

        drive_worker_until_complete(
            || {
                attempts.set(attempts.get() + 1);
                (attempts.get() > 6)
                    .then_some(())
                    .ok_or(CommandFailure::ServiceUnavailable("synthetic retry"))
            },
            |delay| delays.push(delay),
        );

        assert_eq!(attempts.get(), 7);
        let expected_ms = [250, 1_000, 4_000, 15_000, 30_000, 30_000];
        assert_eq!(delays, expected_ms.map(Duration::from_millis));
    }

    #[test]
    fn deletion_failure_cause_is_typed_independently_from_transport_copy() {
        let failure = DeletionAttemptFailure::unavailable(
            SourceRootDeletionErrorCode::OcrQuiescenceTimeout,
            "changed transport copy",
        );

        assert_eq!(
            failure.code,
            SourceRootDeletionErrorCode::OcrQuiescenceTimeout
        );
        assert!(matches!(
            failure.transport,
            CommandFailure::ServiceUnavailable("changed transport copy")
        ));
        assert_eq!(
            DeletionAttemptFailure::internal(SourceRootDeletionErrorCode::PublicationFailed).code,
            SourceRootDeletionErrorCode::PublicationFailed
        );
    }

    #[test]
    fn pending_recovery_does_not_bypass_the_active_deletion_owner() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp = std::env::temp_dir().join(format!(
            "resume-ir-source-root-deletion-single-owner-{}-{nonce}",
            std::process::id()
        ));
        let data_dir = temp.join("data");
        let source = temp.join("source");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&source).unwrap();

        let owner = match DataDirectoryOwnerLease::try_acquire(&data_dir).unwrap() {
            DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
            DataDirectoryOwnerAcquisition::Contended => panic!("test data directory contended"),
        };
        let store = owner.open_store().unwrap();
        store.run_migrations().unwrap();
        let contract = import_pipeline::current_import_processing_contract(
            &import_pipeline::ImportOptions::default(),
        )
        .unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_800_000);
        store
            .activate_migration_rebuild_contract(&contract, now)
            .unwrap();
        let canonical_source = fs::canonicalize(&source).unwrap();
        let source_path = canonical_source.to_str().unwrap();
        let root = store
            .register_source_root(source_path, source_path, "Synthetic source", now)
            .unwrap();
        store.begin_source_root_deletion(&root.id, now).unwrap();
        let registration = WorkerRegistration::acquire(&root.id)
            .unwrap_or_else(|_| panic!("deletion worker registry must be available"))
            .expect("synthetic deletion owner must register");

        assert!(resume_pending(&data_dir, &store, &contract).is_ok());
        assert_eq!(
            store.source_root_deletion(&root.id).unwrap().unwrap().phase,
            SourceRootDeletionPhase::Requested,
            "pending recovery must not execute behind the active deletion owner"
        );
        drop(registration);

        drop(store);
        drop(owner);
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn blocked_runtime_invariant_head_waits_for_recovery_instead_of_publication_failed() {
        let directory = tempdir().unwrap();
        let data_dir = directory.path().join("data");
        let owner = match DataDirectoryOwnerLease::try_acquire(&data_dir).unwrap() {
            DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
            DataDirectoryOwnerAcquisition::Contended => panic!("test data directory contended"),
        };
        let store = owner.open_store().unwrap();
        store.run_migrations().unwrap();
        let contract = ImportProcessingContract::new(
            "deletion-recovery-parser-v1",
            "deletion-recovery-ocr-v1",
            "deletion-recovery-schema-v29",
            CLASSIFIER_EPOCH,
        )
        .unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_900_000);
        store
            .activate_migration_rebuild_contract(&contract, now)
            .unwrap();
        finalize_migration_rebuild(
            &store,
            now,
            &contract,
            &SearchPublicationVectorization::default(),
            &PipelineRunControl::default(),
        )
        .unwrap();
        let ready = store.search_projection_state().unwrap();
        let generation = ready.generation.as_deref().unwrap().to_string();
        assert_eq!(
            store
                .begin_artifact_repair(&generation, ready.visible_epoch, now)
                .unwrap(),
            SearchProjectionTransitionOutcome::Applied
        );
        let context = store.artifact_repair_context().unwrap().unwrap();
        assert_eq!(
            store
                .block_artifact_repair(
                    &generation,
                    &context.publication_fingerprint,
                    ready.visible_epoch,
                    UnixTimestamp::from_unix_seconds(1_700_900_001),
                )
                .unwrap(),
            SearchProjectionTransitionOutcome::Applied
        );

        let failure = ensure_search_head_ready_for_privacy_publication(
            &store,
            UnixTimestamp::from_unix_seconds(1_700_900_002),
        )
        .expect_err("blocked head must wait for artifact recovery");
        assert_eq!(failure.code, SourceRootDeletionErrorCode::Internal);
        assert!(matches!(
            failure.transport,
            CommandFailure::ServiceUnavailable("search artifacts are recovering")
        ));
        assert_ne!(
            failure.code,
            SourceRootDeletionErrorCode::PublicationFailed,
            "recovery wait must not be recorded as publication_failed"
        );
        let repairing = store.search_projection_state().unwrap();
        assert_eq!(
            repairing.service_state,
            SearchProjectionServiceState::Repairing
        );
        assert_eq!(
            repairing.repair_reason,
            Some(SearchRepairReason::ArtifactUnavailable)
        );
        assert_eq!(repairing.generation.as_deref(), Some(generation.as_str()));
    }
}
