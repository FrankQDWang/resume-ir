use std::{fs, path::Path, process::Command};

use import_pipeline::{current_import_processing_contract, ImportOptions};
use meta_store::{
    migration_test_support::{
        active_projection_count_for_processing_contract,
        completed_import_task_count_for_processing_contract,
        completed_migration_rebuild_task_count, seed_v27_repairing_fixture,
        seed_v28_blocked_processing_contract_fixture,
    },
    EntityType, ImportProcessingContract, ImportTaskPurpose, ImportTaskStatus, ReadMetaStore,
    SearchProjectionServiceState, SearchRepairReason, MAX_ENTITY_MENTIONS_PER_VERSION,
    MAX_ENTITY_MENTION_VALUE_BYTES,
};
use search_runtime::{HitLimit, QueryCoordinator};
use tempfile::tempdir;

const LEGACY_VISIBLE_EPOCH: u64 = 41;
const UNIQUE_SEARCH_TOKEN: &str = "S83V27V28ConvergenceToken";

#[test]
fn migration_fixture_builder_rejects_non_synthetic_roots() {
    let workspace = tempdir().unwrap();
    let data_dir = workspace.path().join("data");
    let non_synthetic_root = workspace.path().join("private-root");
    fs::create_dir_all(&non_synthetic_root).unwrap();

    assert!(seed_v27_repairing_fixture(&data_dir, &non_synthetic_root, 1).is_err());
}

#[test]
fn bounded_daemon_loop_converges_a_failed_v27_rebuild_to_one_ready_v29_publication() {
    let workspace = tempdir().unwrap();
    let data_dir = workspace.path().join("data");
    let source_root = workspace.path().join("resume-ir-synthetic-v27-source");
    fs::create_dir_all(&source_root).unwrap();
    let date_ranges = unique_closed_date_ranges(MAX_ENTITY_MENTIONS_PER_VERSION + 32);
    let synthetic_text = format!(
        "SUMMARY\nSynthetic software engineer resume.\n\
         EXPERIENCE\n{}Built {UNIQUE_SEARCH_TOKEN} local-first search services in Rust.\n\
         EDUCATION\nSynthetic University, Computer Science.\n\
         SKILLS\nRust Java Kubernetes distributed systems.\n",
        date_ranges
    );
    fs::write(source_root.join("synthetic-resume.txt"), &synthetic_text).unwrap();
    let canonical_root = fs::canonicalize(&source_root).unwrap();
    let legacy =
        seed_v27_repairing_fixture(&data_dir, &canonical_root, LEGACY_VISIBLE_EPOCH).unwrap();
    seed_stale_search_artifacts(&data_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-imports",
            "--work-index",
            "--worker-interval-ms",
            "1",
            "--max-worker-ticks",
            "3",
        ])
        .output()
        .expect("run the bounded daemon import loop");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "bounded daemon convergence failed: stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(stderr.is_empty(), "unexpected daemon stderr: {stderr:?}");
    for private_path in [
        path_str(workspace.path()),
        path_str(&data_dir),
        path_str(&canonical_root),
    ] {
        assert!(!stdout.contains(private_path));
        assert!(!stderr.contains(private_path));
    }

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert_eq!(store.schema_version().unwrap(), 29);
    assert!(store
        .import_task_by_id(legacy.legacy_task_id())
        .unwrap()
        .is_none());
    let completed = store
        .latest_import_task_by_root(path_str(&canonical_root))
        .unwrap()
        .unwrap();
    assert_ne!(&completed.id, legacy.legacy_task_id());
    assert_eq!(completed.status, ImportTaskStatus::Completed);
    assert_eq!(
        store.import_task_purpose(&completed.id).unwrap(),
        ImportTaskPurpose::MigrationRebuildFullCorpus
    );
    assert_eq!(completed_migration_rebuild_task_count(&store).unwrap(), 1);
    let completed_scope = store
        .import_scan_scope_by_task_id(&completed.id)
        .unwrap()
        .unwrap();
    assert_eq!(completed_scope.scan_budget_kind, None);
    assert_eq!(completed_scope.scan_budget_limit, None);
    assert_eq!(completed_scope.scan_budget_observed, None);
    assert!(!completed_scope.scan_budget_exhausted);

    let status = store.status_summary().unwrap();
    assert_eq!(status.import_tasks_queued, 0);
    assert_eq!(status.import_tasks_recoverable, 0);
    assert_eq!(status.searchable_documents, 1);
    let projection = store.search_projection_state().unwrap();
    assert_eq!(
        projection.service_state,
        SearchProjectionServiceState::Ready
    );
    assert_eq!(projection.repair_reason, None);
    assert!(projection.generation.is_some());
    assert!(projection.publication.is_some());
    assert_eq!(
        projection.visible_epoch,
        legacy.inherited_visible_epoch() + 1
    );

    assert!(!data_dir
        .join("search-index/fulltext.snapshot.key-v1")
        .exists());
    assert!(!data_dir
        .join("vector-index/vector.snapshot.key-v1")
        .exists());
    let mut coordinator = QueryCoordinator::open(&data_dir).unwrap();
    let hits = coordinator
        .with_query(|scope| {
            scope.fulltext_candidates(UNIQUE_SEARCH_TOKEN, HitLimit::new(10)?, None)
        })
        .unwrap();
    assert_eq!(hits.len(), 1);
    let version_id = hits[0].projection.resume_version_id.clone();
    let mentions = store.entity_mentions_for_version(&version_id).unwrap();
    assert!(mentions.len() <= MAX_ENTITY_MENTIONS_PER_VERSION);
    assert!(mentions.iter().all(|mention| {
        mention.raw_value.len() <= MAX_ENTITY_MENTION_VALUE_BYTES
            && mention
                .normalized_value
                .as_deref()
                .is_none_or(|value| value.len() <= MAX_ENTITY_MENTION_VALUE_BYTES)
    }));
    let years = mentions
        .iter()
        .filter(|mention| mention.entity_type == EntityType::YearsExperience)
        .collect::<Vec<_>>();
    assert_eq!(
        years.len(),
        1,
        "bounded synthetic migration must retain one derived years mention; total_mentions={}",
        mentions.len()
    );
    let years = years[0];
    assert_eq!(years.span_start, None);
    assert_eq!(years.span_end, None);
    assert_eq!(years.raw_value, years.normalized_value.as_deref().unwrap());
    for entity_type in [EntityType::School, EntityType::Skill] {
        assert!(
            mentions
                .iter()
                .any(|mention| mention.entity_type == entity_type),
            "bounded migration dropped {entity_type:?}"
        );
    }
}

#[test]
fn daemon_hard_cuts_a_blocked_old_v28_contract_and_publishes_only_current_rows() {
    let workspace = tempdir().unwrap();
    let data_dir = workspace.path().join("data");
    let source_root = workspace
        .path()
        .join("resume-ir-synthetic-blocked-old-contract");
    fs::create_dir_all(&source_root).unwrap();
    fs::write(
        source_root.join("current-contract-resume.txt"),
        format!(
            "SUMMARY\nSynthetic platform engineer resume.\n\
             EXPERIENCE\nBuilt {UNIQUE_SEARCH_TOKEN} deterministic services in Rust.\n\
             EDUCATION\nSynthetic University.\n\
             SKILLS\nRust distributed systems.\n"
        ),
    )
    .unwrap();
    let canonical_root = fs::canonicalize(&source_root).unwrap();
    let old_contract = ImportProcessingContract::new(
        "parser-v1",
        "ocr-v1",
        "resume-ir-s9-v1",
        meta_store::CLASSIFIER_EPOCH,
    )
    .unwrap();
    let current_contract = current_import_processing_contract(&ImportOptions::default()).unwrap();
    assert_ne!(old_contract.id(), current_contract.id());
    let fixture = seed_v28_blocked_processing_contract_fixture(
        &data_dir,
        &canonical_root,
        LEGACY_VISIBLE_EPOCH,
        &old_contract,
    )
    .unwrap();

    run_bounded_daemon(&data_dir, workspace.path(), &canonical_root);

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert_eq!(store.schema_version().unwrap(), 29);
    assert!(store
        .import_task_by_id(fixture.legacy_task_id())
        .unwrap()
        .is_none());
    assert!(store
        .document_by_id(fixture.immutable_document_id())
        .unwrap()
        .is_some());
    assert!(store
        .source_revision_by_id(fixture.immutable_source_revision_id())
        .unwrap()
        .is_some());
    assert!(store
        .resume_version_by_id(fixture.immutable_resume_version_id())
        .unwrap()
        .is_some());
    assert!(store
        .resume_version_classification(
            fixture.immutable_resume_version_id(),
            old_contract.classifier_epoch(),
        )
        .unwrap()
        .is_some());

    let completed = store
        .latest_import_task_by_root(path_str(&canonical_root))
        .unwrap()
        .unwrap();
    assert_eq!(completed.status, ImportTaskStatus::Completed);
    assert_eq!(
        store.import_task_purpose(&completed.id).unwrap(),
        ImportTaskPurpose::MigrationRebuildFullCorpus
    );
    assert_eq!(
        store
            .import_task_processing_contract_id(&completed.id)
            .unwrap()
            .as_ref(),
        Some(current_contract.id())
    );
    assert_eq!(
        completed_import_task_count_for_processing_contract(&store, old_contract.id()).unwrap(),
        0
    );
    assert_eq!(
        completed_import_task_count_for_processing_contract(&store, current_contract.id()).unwrap(),
        1
    );
    assert_eq!(
        active_projection_count_for_processing_contract(&store, old_contract.id()).unwrap(),
        0
    );
    assert_eq!(
        active_projection_count_for_processing_contract(&store, current_contract.id()).unwrap(),
        1
    );
    assert_eq!(store.status_summary().unwrap().searchable_documents, 1);
    let projection = store.search_projection_state().unwrap();
    assert_eq!(
        projection.service_state,
        SearchProjectionServiceState::Ready
    );
    assert_eq!(projection.repair_reason, None);
    assert_eq!(
        projection.visible_epoch,
        fixture.inherited_visible_epoch() + 1
    );

    let mut coordinator = QueryCoordinator::open(&data_dir).unwrap();
    let hits = coordinator
        .with_query(|scope| {
            scope.fulltext_candidates(UNIQUE_SEARCH_TOKEN, HitLimit::new(10)?, None)
        })
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_ne!(
        &hits[0].projection.resume_version_id,
        fixture.immutable_resume_version_id()
    );
}

#[test]
fn daemon_restart_does_not_reopen_a_blocked_current_v28_contract() {
    let workspace = tempdir().unwrap();
    let data_dir = workspace.path().join("data");
    let source_root = workspace
        .path()
        .join("resume-ir-synthetic-blocked-current-contract");
    fs::create_dir_all(&source_root).unwrap();
    fs::write(
        source_root.join("blocked-current-resume.txt"),
        "SUMMARY\nSynthetic blocked current contract resume.\n",
    )
    .unwrap();
    let canonical_root = fs::canonicalize(&source_root).unwrap();
    let current_contract = current_import_processing_contract(&ImportOptions::default()).unwrap();
    let fixture = seed_v28_blocked_processing_contract_fixture(
        &data_dir,
        &canonical_root,
        LEGACY_VISIBLE_EPOCH,
        &current_contract,
    )
    .unwrap();

    for _ in 0..2 {
        run_bounded_daemon(&data_dir, workspace.path(), &canonical_root);
        let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
        let projection = store.search_projection_state().unwrap();
        assert_eq!(
            projection.service_state,
            SearchProjectionServiceState::RepairBlocked
        );
        assert_eq!(
            projection.repair_reason,
            Some(SearchRepairReason::RuntimeInvariant)
        );
        assert_eq!(projection.generation, None);
        assert_eq!(projection.visible_epoch, fixture.inherited_visible_epoch());
        assert_eq!(store.status_summary().unwrap().searchable_documents, 0);
        assert_eq!(
            store
                .import_task_by_id(fixture.legacy_task_id())
                .unwrap()
                .unwrap()
                .status,
            ImportTaskStatus::Completed
        );
        assert_eq!(
            completed_import_task_count_for_processing_contract(&store, current_contract.id())
                .unwrap(),
            1
        );
        assert_eq!(
            active_projection_count_for_processing_contract(&store, current_contract.id()).unwrap(),
            0
        );
    }
}

fn run_bounded_daemon(data_dir: &Path, workspace: &Path, canonical_root: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(data_dir),
            "run",
            "--foreground",
            "--work-imports",
            "--work-index",
            "--worker-interval-ms",
            "1",
            "--max-worker-ticks",
            "3",
        ])
        .output()
        .expect("run the bounded daemon import loop");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "bounded daemon convergence failed: stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(stderr.is_empty(), "unexpected daemon stderr: {stderr:?}");
    for private_path in [
        path_str(workspace),
        path_str(data_dir),
        path_str(canonical_root),
    ] {
        assert!(!stdout.contains(private_path));
        assert!(!stderr.contains(private_path));
    }
}

fn unique_closed_date_ranges(count: usize) -> String {
    (0..count)
        .map(|ordinal| {
            let start_year = 1980 + ordinal / 12;
            let start_month = ordinal % 12 + 1;
            let (end_year, end_month) = if start_month == 12 {
                (start_year + 1, 1)
            } else {
                (start_year, start_month + 1)
            };
            format!("{start_year}-{start_month:02} - {end_year}-{end_month:02}\n")
        })
        .collect()
}

fn seed_stale_search_artifacts(data_dir: &Path) {
    let fulltext = data_dir.join("search-index");
    let vector = data_dir.join("vector-index");
    fs::create_dir_all(fulltext.join("snapshots/stale-generation")).unwrap();
    fs::create_dir_all(vector.join("staging/stale-generation")).unwrap();
    fs::write(
        fulltext.join("fulltext.snapshot.key-v1"),
        b"synthetic stale",
    )
    .unwrap();
    fs::write(vector.join("vector.snapshot.key-v1"), b"synthetic stale").unwrap();
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}
