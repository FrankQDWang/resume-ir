use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use import_pipeline::{import_root_with_options, ImportOptions};
use meta_store::{
    ImportTask, ImportTaskId, ImportTaskStatus, MetaStore, SearchProjectionFilter, UnixTimestamp,
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
    _temp: TestDir,
    data_dir: PathBuf,
    root: PathBuf,
    store: MetaStore,
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
        let store = MetaStore::open_data_dir(&data_dir).unwrap();
        store.run_migrations().unwrap();
        let mut fixture = Self {
            _temp: temp,
            data_dir,
            root,
            store,
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
        let task = ImportTask {
            id: ImportTaskId::from_non_secret_parts(&[&format!("task-{}", self.next_timestamp)]),
            root_path: self.root.to_string_lossy().into_owned(),
            status: ImportTaskStatus::Running,
            queued_at: now,
            started_at: Some(now),
            finished_at: None,
            updated_at: now,
        };
        self.store.insert_import_task(&task).unwrap();
        import_root_with_options(
            &self.data_dir,
            &self.store,
            &task,
            &self.root,
            now,
            ImportOptions::default(),
        )
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
