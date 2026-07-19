use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use import_pipeline::{
    current_import_processing_contract, import_root_with_options, ImportOptions,
    ImportTaskOwnerLock,
};
use meta_store::{
    DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, ImportRootKind,
    ImportRootTaskHeadOutcome, ImportRootTaskHeadRequest, ImportScanProfile, ImportScanScope,
    ImportTask, ImportTaskId, ImportTaskStatus, OwnedMetaStore, SearchProjectionFilter,
    UnixTimestamp,
};
use search_runtime::{
    HitLimit, QueryCoordinator, SearchRuntimeErrorCode, SelectionLimit, SemanticContract,
    SemanticQueryVector,
};

#[test]
fn composite_scope_reads_one_exact_disabled_generation() {
    let fixture = Fixture::new("composite-exact-generation", 2);
    let mut coordinator = QueryCoordinator::open(&fixture.data_dir).unwrap();
    coordinator
        .with_query(|scope| {
            assert_eq!(scope.semantic_contract(), SemanticContract::Disabled);
            let selection = scope.filter_selection(
                &SearchProjectionFilter::new(Vec::new()).unwrap(),
                SelectionLimit::new(16).unwrap(),
            )?;
            let candidates = scope.fulltext_candidates(
                "Rust systems",
                HitLimit::new(10).unwrap(),
                Some(&selection),
            )?;
            assert_eq!(candidates.len(), 2);
            let projections = candidates
                .iter()
                .map(|candidate| candidate.projection.clone())
                .collect::<Vec<_>>();
            let hydrated = scope.hydrate_exact_hits(&projections)?;
            assert_eq!(hydrated.len(), 2);
            assert!(hydrated.iter().all(|hit| {
                hit.selection.visible_epoch == scope.visible_epoch()
                    && hit.selection.document_id == hit.document.id
            }));
            Ok(())
        })
        .unwrap();
}

#[test]
fn bounded_filter_rejects_oversized_selection() {
    let fixture = Fixture::new("bounded-filter", 2);
    let mut coordinator = QueryCoordinator::open(&fixture.data_dir).unwrap();
    let error = coordinator
        .with_query(|scope| {
            scope.filter_selection(
                &SearchProjectionFilter::new(Vec::new()).unwrap(),
                SelectionLimit::new(1).unwrap(),
            )?;
            Ok(())
        })
        .unwrap_err();
    assert_eq!(error.code(), SearchRuntimeErrorCode::SelectionTooLarge);
}

#[test]
fn disabled_vector_never_degrades_semantic_to_fulltext() {
    let fixture = Fixture::new("semantic-disabled", 1);
    let mut coordinator = QueryCoordinator::open(&fixture.data_dir).unwrap();
    let error = coordinator
        .with_query(|scope| {
            let query = SemanticQueryVector::new(vec![1.0]).unwrap();
            scope.semantic_candidates(query, HitLimit::new(10).unwrap(), None)?;
            Ok(())
        })
        .unwrap_err();
    assert_eq!(error.code(), SearchRuntimeErrorCode::SemanticDisabled);
}

#[test]
fn invalid_new_generation_is_not_served_from_the_old_cache() {
    let mut fixture = Fixture::new("invalid-new-generation", 1);
    let mut coordinator = QueryCoordinator::open(&fixture.data_dir).unwrap();
    coordinator
        .with_query(|scope| {
            assert_eq!(
                scope
                    .fulltext_candidates("Rust", HitLimit::new(10).unwrap(), None)?
                    .len(),
                1
            );
            Ok(())
        })
        .unwrap();

    fixture.replace_first_resume("Go");
    let generation = fixture.publish_next();
    fs::remove_file(
        fixture
            .data_dir
            .join("search-index/snapshots")
            .join(generation)
            .join("snapshot-manifest.json"),
    )
    .unwrap();
    let error = coordinator.with_query(|_| Ok(())).unwrap_err();
    assert_eq!(error.code(), SearchRuntimeErrorCode::Integrity);
}

struct Fixture {
    store: OwnedMetaStore,
    _owner: DataDirectoryOwnerLease,
    _temp: TestDir,
    data_dir: PathBuf,
    root: PathBuf,
    next_timestamp: i64,
}

impl Fixture {
    fn new(label: &str, count: usize) -> Self {
        let temp = TestDir::new(label);
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        for index in 0..count {
            fs::write(
                root.join(format!("candidate-{index}.txt")),
                resume_text(index, "Rust"),
            )
            .unwrap();
        }
        let owner = match DataDirectoryOwnerLease::try_acquire(&data_dir).unwrap() {
            DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
            DataDirectoryOwnerAcquisition::Contended => panic!("synthetic fixture owner contended"),
        };
        let store = owner.open_store().unwrap();
        let mut fixture = Self {
            store,
            _owner: owner,
            _temp: temp,
            data_dir,
            root,
            next_timestamp: 1_900_000_000,
        };
        fixture.publish_next();
        fixture
    }

    fn replace_first_resume(&self, skill: &str) {
        fs::write(self.root.join("candidate-0.txt"), resume_text(0, skill)).unwrap();
    }

    fn publish_next(&mut self) -> String {
        let now = UnixTimestamp::from_unix_seconds(self.next_timestamp);
        self.next_timestamp += 1;
        let options = ImportOptions::default();
        let processing_contract = current_import_processing_contract(&options).unwrap();
        self.store
            .activate_migration_rebuild_contract(&processing_contract, now)
            .unwrap();
        let queued = ImportTask {
            id: ImportTaskId::from_non_secret_parts(&[&format!("task-{}", self.next_timestamp)]),
            root_path: self.root.to_string_lossy().into_owned(),
            status: ImportTaskStatus::Queued,
            queued_at: now,
            started_at: None,
            finished_at: None,
            updated_at: now,
        };
        let scope = ImportScanScope {
            import_task_id: queued.id.clone(),
            root_kind: ImportRootKind::Explicit,
            root_preset: None,
            scan_profile: ImportScanProfile::Explicit,
            requested_root_path: queued.root_path.clone(),
            canonical_root_path: queued.root_path.clone(),
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
            updated_at: now,
        };
        let observed = match self
            .store
            .coordinate_import_root_task_head(ImportRootTaskHeadRequest::Configured {
                task: &queued,
                scope: &scope,
                processing_contract: &processing_contract,
            })
            .unwrap()
        {
            ImportRootTaskHeadOutcome::HeadInserted { task, .. }
            | ImportRootTaskHeadOutcome::HeadPromoted { task, .. }
            | ImportRootTaskHeadOutcome::HeadRetained { task, .. } => task,
            outcome => panic!("synthetic fixture head was rejected: {outcome:?}"),
        };
        let _owner_lock = ImportTaskOwnerLock::acquire(&self.data_dir, &observed.id).unwrap();
        let task = self
            .store
            .claim_observed_import_task_for_worker(&observed, now)
            .unwrap()
            .unwrap();
        import_root_with_options(&self.data_dir, &self.store, &task, &self.root, now, options)
            .unwrap();
        self.store
            .with_search_metadata_snapshot(|snapshot| {
                Ok::<_, ()>(snapshot.head().generation.clone())
            })
            .unwrap()
    }
}

fn resume_text(index: usize, skill: &str) -> String {
    format!(
        "SUMMARY\nSynthetic Candidate {index}\nEXPERIENCE\nBuilt {skill} systems\nSKILLS\n{skill}"
    )
}

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("resume-ir-search-runtime-{label}-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
