use meta_store::{
    ImportProcessingContract, ImportRootTaskHeadOutcome, ImportRootTaskHeadRequest,
    ImportScanScope, ImportTask, ImportTaskStatus, OwnedMetaStore, UnixTimestamp,
};

use crate::{import_command, DaemonError, Result};

/// Time-based policy for scheduling another scan of a completed import root.
///
/// The interval applies to every observation, including the first worker-loop
/// observation after daemon startup. Callers provide only the observation time;
/// process generations and worker tick counts are deliberately outside this
/// contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CompletedRootRescanSchedule {
    interval_seconds: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InvalidCompletedRootRescanInterval;

impl CompletedRootRescanSchedule {
    pub(crate) fn new(
        interval_seconds: i64,
    ) -> std::result::Result<Self, InvalidCompletedRootRescanInterval> {
        if interval_seconds < 0 {
            return Err(InvalidCompletedRootRescanInterval);
        }
        Ok(Self { interval_seconds })
    }

    pub(crate) fn requeue_due(
        self,
        store: &OwnedMetaStore,
        processing_contract: &ImportProcessingContract,
        now: UnixTimestamp,
    ) -> Result<usize> {
        let finished_at_or_before = UnixTimestamp::from_unix_seconds(
            now.as_unix_seconds().saturating_sub(self.interval_seconds),
        );
        let scopes = store
            .completed_import_scan_scopes_due_for_requeue(finished_at_or_before)
            .map_err(DaemonError::store)?;
        let mut requeued = 0_usize;

        for (index, scope) in scopes.into_iter().enumerate() {
            let task_id = import_command::new_task_id(index)
                .map_err(|_| DaemonError::user("system clock is before unix epoch"))?;
            requeued += usize::from(enqueue_import_from_completed_scope(
                store,
                processing_contract,
                scope,
                task_id,
                now,
            )?);
        }

        Ok(requeued)
    }
}

pub(crate) fn enqueue_import_from_completed_scope(
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    scope: ImportScanScope,
    task_id: meta_store::ImportTaskId,
    now: UnixTimestamp,
) -> Result<bool> {
    let task = ImportTask {
        id: task_id.clone(),
        root_path: scope.canonical_root_path.clone(),
        status: ImportTaskStatus::Queued,
        queued_at: now,
        started_at: None,
        finished_at: None,
        updated_at: now,
    };
    let next_scope = pending_scope_from_completed(scope, task_id, now);
    let outcome = store
        .coordinate_import_root_task_head(ImportRootTaskHeadRequest::Configured {
            task: &task,
            scope: &next_scope,
            processing_contract,
        })
        .map_err(DaemonError::store)?;
    Ok(matches!(
        outcome,
        ImportRootTaskHeadOutcome::HeadInserted { .. }
    ))
}

fn pending_scope_from_completed(
    scope: ImportScanScope,
    import_task_id: meta_store::ImportTaskId,
    now: UnixTimestamp,
) -> ImportScanScope {
    ImportScanScope {
        import_task_id,
        root_kind: scope.root_kind,
        root_preset: scope.root_preset,
        scan_profile: scope.scan_profile,
        requested_root_path: scope.requested_root_path,
        canonical_root_path: scope.canonical_root_path,
        files_discovered: 0,
        ignored_entries: 0,
        scan_errors: 0,
        searchable_documents: 0,
        ocr_required_documents: 0,
        ocr_jobs_queued: 0,
        failed_documents: 0,
        deleted_documents: 0,
        scan_budget_kind: scope.scan_budget_kind,
        scan_budget_limit: scope.scan_budget_limit,
        scan_budget_observed: scope.scan_budget_limit.map(|_| 0),
        scan_budget_exhausted: false,
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use import_pipeline::{
        current_import_processing_contract, finalize_migration_rebuild,
        prepare_migration_rebuild_artifacts, ImportOptions, SearchPublicationVectorization,
    };
    use meta_store::{
        DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, ImportRootKind, ImportScanProfile,
        ImportTaskId,
    };
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn every_observation_respects_the_completed_root_interval() {
        let fixture = completed_root_fixture(UnixTimestamp::from_unix_seconds(100));
        let schedule = CompletedRootRescanSchedule::new(300).unwrap();

        assert_eq!(
            schedule
                .requeue_due(
                    &fixture.store,
                    &fixture.processing_contract,
                    UnixTimestamp::from_unix_seconds(399),
                )
                .unwrap(),
            0
        );
        assert!(fixture
            .store
            .pending_import_task_by_root(fixture.root)
            .unwrap()
            .is_none());

        assert_eq!(
            schedule
                .requeue_due(
                    &fixture.store,
                    &fixture.processing_contract,
                    UnixTimestamp::from_unix_seconds(400),
                )
                .unwrap(),
            1
        );
        let queued = fixture
            .store
            .pending_import_task_by_root(fixture.root)
            .unwrap()
            .unwrap();

        assert_eq!(
            schedule
                .requeue_due(
                    &fixture.store,
                    &fixture.processing_contract,
                    UnixTimestamp::from_unix_seconds(400),
                )
                .unwrap(),
            0
        );
        assert_eq!(
            fixture
                .store
                .pending_import_task_by_root(fixture.root)
                .unwrap()
                .unwrap()
                .id,
            queued.id
        );
    }

    #[test]
    fn negative_intervals_are_not_representable() {
        assert_eq!(
            CompletedRootRescanSchedule::new(-1),
            Err(InvalidCompletedRootRescanInterval)
        );
    }

    struct CompletedRootFixture {
        store: OwnedMetaStore,
        processing_contract: ImportProcessingContract,
        root: &'static str,
        _owner: DataDirectoryOwnerLease,
        _directory: TempDir,
    }

    fn completed_root_fixture(finished_at: UnixTimestamp) -> CompletedRootFixture {
        let directory = tempfile::tempdir().unwrap();
        let owner = match DataDirectoryOwnerLease::try_acquire(directory.path()).unwrap() {
            DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
            DataDirectoryOwnerAcquisition::Contended => {
                panic!("synthetic rescan data directory was contended")
            }
        };
        let store = owner.open_store().unwrap();
        let processing_contract =
            current_import_processing_contract(&ImportOptions::default()).unwrap();
        store
            .activate_migration_rebuild_contract(
                &processing_contract,
                UnixTimestamp::from_unix_seconds(1),
            )
            .unwrap();
        prepare_migration_rebuild_artifacts(
            &store,
            UnixTimestamp::from_unix_seconds(1),
            &import_pipeline::PipelineRunControl::default(),
        )
        .unwrap();
        finalize_migration_rebuild(
            &store,
            UnixTimestamp::from_unix_seconds(1),
            &processing_contract,
            &SearchPublicationVectorization::default(),
            &import_pipeline::PipelineRunControl::default(),
        )
        .unwrap();

        let root = "/synthetic/completed-rescan-root";
        let queued_at = UnixTimestamp::from_unix_seconds(90);
        let task = ImportTask {
            id: ImportTaskId::from_non_secret_parts(&["completed-root-rescan-fixture"]),
            root_path: root.to_string(),
            status: ImportTaskStatus::Queued,
            queued_at,
            started_at: None,
            finished_at: None,
            updated_at: queued_at,
        };
        let mut scope = ImportScanScope {
            import_task_id: task.id.clone(),
            root_kind: ImportRootKind::Explicit,
            root_preset: None,
            scan_profile: ImportScanProfile::Explicit,
            requested_root_path: root.to_string(),
            canonical_root_path: root.to_string(),
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
            updated_at: queued_at,
        };
        store
            .insert_import_task_with_scan_scope(&task, &scope, &processing_contract)
            .unwrap();
        let running = store
            .claim_observed_import_task_for_worker(&task, UnixTimestamp::from_unix_seconds(99))
            .unwrap()
            .unwrap();
        scope.updated_at = finished_at;
        store
            .complete_import_task(&running.id, processing_contract.id(), &scope, finished_at)
            .unwrap();

        CompletedRootFixture {
            store,
            processing_contract,
            root,
            _owner: owner,
            _directory: directory,
        }
    }
}
