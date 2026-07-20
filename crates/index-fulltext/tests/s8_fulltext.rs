use std::collections::BTreeSet;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};

#[cfg(unix)]
use std::sync::mpsc;

use index_fulltext::{
    commit_snapshot_gc, prepare_snapshot_gc, publish_incremental_snapshot, publish_snapshot,
    staging_orphan_count, try_acquire_snapshot_gc, FullTextError, FullTextIndex,
    FullTextSnapshotGcCommitReport, FullTextSnapshotGcPreparation, IndexDocument, IndexSection,
    SearchQuery, SnapshotPurgeSummary, SnapshotReadLease,
};

#[cfg(unix)]
use index_fulltext::{publish_snapshot_with_control, SnapshotPublishControl, SnapshotPublishPhase};

const SNAPSHOT_TEST_WRITE_RETRY_ATTEMPTS: usize = 100;
const SNAPSHOT_TEST_WRITE_RETRY_DELAY: Duration = Duration::from_millis(50);

fn run_snapshot_gc(
    index_root: &Path,
    retained: &BTreeSet<String>,
) -> Result<Option<FullTextSnapshotGcCommitReport>, FullTextError> {
    let Some(acquisition) = try_acquire_snapshot_gc(index_root)? else {
        return Ok(None);
    };
    match prepare_snapshot_gc(acquisition, retained)? {
        FullTextSnapshotGcPreparation::Deferred => Ok(None),
        FullTextSnapshotGcPreparation::Prepared(prepared) => Ok(Some(commit_snapshot_gc(prepared))),
    }
}

fn complete_gc_summary(report: FullTextSnapshotGcCommitReport) -> SnapshotPurgeSummary {
    match report {
        FullTextSnapshotGcCommitReport::Complete(summary) => summary,
        FullTextSnapshotGcCommitReport::Interrupted(_) => {
            panic!("full-text GC unexpectedly interrupted")
        }
        FullTextSnapshotGcCommitReport::PartialFailure(failure) => {
            panic!(
                "full-text GC unexpectedly failed: {:?}",
                failure.failure_class()
            )
        }
    }
}

#[test]
fn exposes_index_fulltext_crate_identity() {
    assert_eq!(index_fulltext::crate_name(), "index-fulltext");
}

#[test]
fn snapshot_read_lease_does_not_create_runtime_state_when_no_index_exists() {
    let index_root = temp_dir("missing-reader-lease");

    assert!(SnapshotReadLease::acquire(&index_root).unwrap().is_none());
    assert!(fs::read_dir(&index_root).unwrap().next().is_none());

    remove_dir(&index_root);
}

#[test]
fn snapshot_read_lease_treats_absent_root_as_unavailable_without_creating_it() {
    let parent = temp_dir("absent-reader-lease-parent");
    let index_root = parent.join("missing-index");

    assert!(SnapshotReadLease::acquire(&index_root).unwrap().is_none());
    assert!(!index_root.exists());

    remove_dir(&parent);
}

#[cfg(unix)]
#[test]
fn lock_files_are_created_owner_only() {
    let index_root = temp_dir("private-lock-modes");
    publish_snapshot(&index_root, "private-locks", [java_payment_document()]).unwrap();

    for lock_name in [
        "snapshot-readers.lock",
        "snapshot-publication.lock",
        "generation-pins/private-locks.lock",
    ] {
        let mode = fs::symlink_metadata(index_root.join(lock_name))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "{lock_name} must be owner-only");
    }
    remove_dir(&index_root);
}

#[cfg(unix)]
#[test]
fn query_rejects_permissive_reader_lock_without_changing_permissions() {
    let index_root = temp_dir("permissive-reader-lock");
    let lock_path = index_root.join("snapshot-readers.lock");
    fs::write(&lock_path, b"").unwrap();
    fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o644)).unwrap();

    assert!(SnapshotReadLease::acquire(&index_root).is_err());
    assert_eq!(
        fs::symlink_metadata(&lock_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644,
        "query path must not chmod an existing lock"
    );
    remove_dir(&index_root);
}

#[cfg(unix)]
#[test]
fn publication_rejects_permissive_or_symlink_lock_files() {
    let permissive_root = temp_dir("permissive-publication-lock");
    let permissive_lock = permissive_root.join("snapshot-publication.lock");
    fs::write(&permissive_lock, b"").unwrap();
    fs::set_permissions(&permissive_lock, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(publish_snapshot(
        &permissive_root,
        "permissive-lock",
        [java_payment_document()]
    )
    .is_err());
    assert_eq!(
        fs::symlink_metadata(&permissive_lock)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    remove_dir(&permissive_root);

    let symlink_root = temp_dir("symlink-publication-lock");
    let target = symlink_root.join("lock-target");
    fs::write(&target, b"").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    symlink(&target, symlink_root.join("snapshot-publication.lock")).unwrap();
    assert!(publish_snapshot(&symlink_root, "symlink-lock", [java_payment_document()]).is_err());
    assert_eq!(
        fs::symlink_metadata(&target).unwrap().permissions().mode() & 0o777,
        0o600
    );
    remove_dir(&symlink_root);
}

#[cfg(unix)]
#[test]
fn query_rejects_reader_lock_symlink() {
    let index_root = temp_dir("symlink-reader-lock");
    let target = index_root.join("lock-target");
    fs::write(&target, b"").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    symlink(&target, index_root.join("snapshot-readers.lock")).unwrap();

    assert!(SnapshotReadLease::acquire(&index_root).is_err());
    remove_dir(&index_root);
}

#[test]
fn published_documents_are_searchable_after_exact_generation_open() {
    let (index_root, index) =
        published_test_index("published-searchable", [java_payment_document()]);

    let hits = index
        .search(SearchQuery::new("Java payment").with_limit(5))
        .unwrap();

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].rank, 1);
    assert_eq!(hits[0].doc_id, stable_document_id("java-payment"));
    assert_eq!(
        hits[0].resume_version_id,
        stable_resume_version_id("java-payment")
    );
    assert_eq!(hits[0].file_name, "synthetic-java-payment.pdf");
    assert!(hits[0].snippet.contains("Java"));
    assert!(!format!("{:?}", hits[0]).contains("Java payment platform"));
    assert!(!format!("{:?}", java_payment_document()).contains("Java payment platform"));
    assert!(!format!("{:?}", SearchQuery::new("Java payment")).contains("Java payment"));

    remove_dir(&index_root);
}

#[test]
fn exact_snapshot_open_keeps_immutable_generations_independent() {
    let index_root = temp_dir("exact-generation-match");
    let metadata =
        publish_snapshot(&index_root, "generation-a", [java_payment_document()]).unwrap();
    assert_eq!(metadata.generation(), "generation-a");
    assert_eq!(metadata.schema().manifest_schema(), "fulltext.snapshot.v3");
    assert_eq!(metadata.schema().index_schema(), "tantivy.fulltext.v3");
    assert_eq!(metadata.document_count(), 1);

    let generation_a = open_snapshot(&index_root, "generation-a").unwrap().unwrap();
    assert_eq!(generation_a.snapshot_metadata(), Some(&metadata));
    assert!(open_snapshot(&index_root, "generation-b")
        .unwrap()
        .is_none());

    publish_snapshot(
        &index_root,
        "generation-b",
        [IndexDocument {
            doc_id: stable_document_id("doc_generation_b"),
            resume_version_id: stable_resume_version_id("ver_generation_b"),
            file_name: "generation-b.pdf".to_string(),
            clean_text: "Rust generation B".to_string(),
            sections: Vec::new(),
        }],
    )
    .unwrap();

    let previous = open_snapshot(&index_root, "generation-a").unwrap().unwrap();
    assert_eq!(
        previous
            .search(SearchQuery::new("Java payment").with_limit(5))
            .unwrap()
            .len(),
        1
    );
    let index = open_snapshot(&index_root, "generation-b").unwrap().unwrap();
    let hits = index
        .search(SearchQuery::new("Rust generation").with_limit(5))
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].doc_id, stable_document_id("doc_generation_b"));

    remove_dir(&index_root);
}

#[cfg(unix)]
#[test]
fn exact_open_rejects_generation_directory_symlink() {
    let index_root = temp_dir("symlink-generation");
    publish_snapshot(&index_root, "generation-source", [java_payment_document()]).unwrap();
    symlink(
        "generation-source",
        index_root.join("snapshots/generation-alias"),
    )
    .unwrap();

    assert!(open_snapshot(&index_root, "generation-alias").is_err());
    remove_dir(&index_root);
}

#[cfg(unix)]
#[test]
fn exact_open_rejects_manifest_and_envelope_symlinks() {
    for artifact in ["snapshot-manifest.json", "fulltext.snapshot.enc"] {
        let index_root = temp_dir(&format!("symlink-{artifact}"));
        let generation = "artifact-generation";
        publish_snapshot(&index_root, generation, [java_payment_document()]).unwrap();
        let artifact_path = index_root.join("snapshots").join(generation).join(artifact);
        let backup_path = artifact_path.with_extension("regular-backup");
        fs::rename(&artifact_path, &backup_path).unwrap();
        symlink(&backup_path, &artifact_path).unwrap();

        assert!(open_snapshot(&index_root, generation).is_err());
        remove_dir(&index_root);
    }
}

#[cfg(unix)]
#[test]
fn exact_open_rejects_key_symlink() {
    let index_root = temp_dir("symlink-key");
    let generation = "key-generation";
    publish_snapshot(&index_root, generation, [java_payment_document()]).unwrap();
    let key_path = index_root.join("snapshots/key-generation/fulltext.snapshot.key-v3");
    let backup_path = index_root.join("key-regular-backup");
    fs::rename(&key_path, &backup_path).unwrap();
    symlink(&backup_path, &key_path).unwrap();

    assert!(open_snapshot(&index_root, generation).is_err());
    remove_dir(&index_root);
}

#[cfg(unix)]
#[test]
fn exact_open_rejects_permissive_key_without_changing_permissions() {
    let index_root = temp_dir("permissive-key");
    let generation = "permissive-key-generation";
    publish_snapshot(&index_root, generation, [java_payment_document()]).unwrap();
    let key_path = index_root.join("snapshots/permissive-key-generation/fulltext.snapshot.key-v3");
    fs::set_permissions(&key_path, fs::Permissions::from_mode(0o644)).unwrap();

    assert!(open_snapshot(&index_root, generation).is_err());
    assert_eq!(
        fs::symlink_metadata(&key_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644,
        "query path must not chmod the key"
    );
    remove_dir(&index_root);
}

#[cfg(unix)]
#[test]
fn damaged_generation_key_does_not_poison_later_publications() {
    let index_root = temp_dir("generation-local-keys");
    publish_snapshot(&index_root, "generation-a", [java_payment_document()]).unwrap();
    let first_key = index_root.join("snapshots/generation-a/fulltext.snapshot.key-v3");
    let first_key_bytes = fs::read(&first_key).unwrap();
    fs::remove_file(&first_key).unwrap();

    publish_snapshot(&index_root, "generation-b", [java_payment_document()]).unwrap();
    let second_key = index_root.join("snapshots/generation-b/fulltext.snapshot.key-v3");
    let second_key_bytes = fs::read(&second_key).unwrap();
    assert_ne!(second_key_bytes, first_key_bytes);
    fs::write(&second_key, b"corrupt generation key").unwrap();

    publish_snapshot(&index_root, "generation-c", [java_payment_document()]).unwrap();
    let third_key = index_root.join("snapshots/generation-c/fulltext.snapshot.key-v3");
    let third_key_bytes = fs::read(&third_key).unwrap();
    assert_ne!(third_key_bytes, second_key_bytes);
    fs::set_permissions(&third_key, fs::Permissions::from_mode(0o644)).unwrap();

    publish_snapshot(&index_root, "generation-d", [java_payment_document()]).unwrap();
    let fourth_key = index_root.join("snapshots/generation-d/fulltext.snapshot.key-v3");
    let fourth_key_bytes = fs::read(&fourth_key).unwrap();
    assert_ne!(fourth_key_bytes, third_key_bytes);
    let fourth_backup = fourth_key.with_extension("backup");
    fs::rename(&fourth_key, &fourth_backup).unwrap();
    symlink(&fourth_backup, &fourth_key).unwrap();

    publish_snapshot(&index_root, "generation-e", [java_payment_document()]).unwrap();
    assert!(open_snapshot(&index_root, "generation-a").is_err());
    assert!(open_snapshot(&index_root, "generation-b").is_err());
    assert!(open_snapshot(&index_root, "generation-c").is_err());
    assert!(open_snapshot(&index_root, "generation-d").is_err());
    open_snapshot(&index_root, "generation-e").unwrap();
    remove_dir(&index_root);
}

#[test]
fn snapshot_rejects_non_unique_document_and_version_mappings() {
    let index_root = temp_dir("non-unique-mapping");
    let first = java_payment_document();
    let mut second_version = first.clone();
    second_version.resume_version_id = stable_resume_version_id("java-payment-v2");
    assert!(publish_snapshot(
        &index_root,
        "duplicate-document",
        [first.clone(), second_version]
    )
    .is_err());

    let mut second_document = first.clone();
    second_document.doc_id = stable_document_id("java-payment-copy");
    assert!(publish_snapshot(&index_root, "duplicate-version", [first, second_document]).is_err());
    assert!(open_snapshot(&index_root, "duplicate-document")
        .unwrap()
        .is_none());
    assert!(open_snapshot(&index_root, "duplicate-version")
        .unwrap()
        .is_none());
    remove_dir(&index_root);
}

#[test]
fn concurrent_duplicate_generation_has_one_atomic_winner() {
    let index_root = temp_dir("concurrent-generation");
    let barrier = Arc::new(Barrier::new(2));
    let workers = ["first", "second"].map(|seed| {
        let index_root = index_root.clone();
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let mut document = java_payment_document();
            document.doc_id = stable_document_id(seed);
            document.resume_version_id = stable_resume_version_id(seed);
            document.clean_text = format!("winner {seed}");
            barrier.wait();
            publish_snapshot(&index_root, "same-generation", [document])
        })
    });
    let results = workers.map(|worker| worker.join().unwrap());
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
    assert_eq!(staging_orphan_count(&index_root).unwrap(), 0);

    let opened = open_snapshot(&index_root, "same-generation")
        .unwrap()
        .unwrap();
    assert_eq!(opened.snapshot_metadata().unwrap().document_count(), 1);
    assert_eq!(
        opened
            .search(SearchQuery::new("winner").with_limit(2))
            .unwrap()
            .len(),
        1
    );
    remove_dir(&index_root);
}

#[cfg(unix)]
#[test]
fn live_publication_defers_gc_without_blocking_queries() {
    let index_root = temp_dir("live-publication-gc-deferred");
    publish_snapshot(&index_root, "generation-old", [java_payment_document()]).unwrap();
    let entered_publication = Arc::new(Barrier::new(2));
    let resume_publication = Arc::new(Barrier::new(2));
    let publisher_root = index_root.clone();
    let publisher_entered = Arc::clone(&entered_publication);
    let publisher_resume = Arc::clone(&resume_publication);
    let publisher = thread::spawn(move || {
        let paused = std::cell::Cell::new(false);
        let observer = |phase| {
            if phase == SnapshotPublishPhase::DocumentIndexing && !paused.replace(true) {
                publisher_entered.wait();
                publisher_resume.wait();
            }
        };
        publish_snapshot_with_control(
            &publisher_root,
            "generation-new",
            [java_payment_document()],
            SnapshotPublishControl::disabled().with_phase_observer(&observer),
        )
    });
    entered_publication.wait();

    let (gc_result_tx, gc_result_rx) = mpsc::channel();
    let gc_root = index_root.clone();
    let gc_attempt = thread::spawn(move || {
        gc_result_tx
            .send(try_acquire_snapshot_gc(&gc_root).map(|lease| lease.is_none()))
            .unwrap();
    });
    let gc_deferred = match gc_result_rx.recv_timeout(Duration::from_secs(1)) {
        Ok(result) => result.unwrap(),
        Err(error) => {
            resume_publication.wait();
            let _ = publisher.join();
            let _ = gc_attempt.join();
            panic!("GC acquisition blocked behind live publication: {error}");
        }
    };
    gc_attempt.join().unwrap();
    assert!(gc_deferred);
    assert_eq!(
        open_snapshot(&index_root, "generation-old")
            .unwrap()
            .unwrap()
            .search(SearchQuery::new("payment"))
            .unwrap()
            .len(),
        1
    );

    resume_publication.wait();
    publisher.join().unwrap().unwrap();
    let retained = BTreeSet::from(["generation-new".to_string()]);
    let summary = complete_gc_summary(run_snapshot_gc(&index_root, &retained).unwrap().unwrap());
    assert_eq!(summary.removed_snapshots(), 1);
    assert!(!index_root.join("snapshots/generation-old").exists());
    remove_dir(&index_root);
}

#[test]
fn retained_generation_reader_does_not_block_obsolete_snapshot_gc() {
    let index_root = temp_dir("retained-reader-generation-gc");
    publish_snapshot(&index_root, "generation-a", [java_payment_document()]).unwrap();
    publish_snapshot(&index_root, "generation-b", [java_payment_document()]).unwrap();
    let opened_generation_b = open_snapshot(&index_root, "generation-b").unwrap().unwrap();
    let retained = BTreeSet::from(["generation-b".to_string()]);
    let summary = complete_gc_summary(run_snapshot_gc(&index_root, &retained).unwrap().unwrap());
    assert_eq!(summary.removed_snapshots(), 1);
    assert!(!index_root.join("snapshots/generation-a").exists());
    assert!(index_root.join("snapshots/generation-b").exists());
    assert!(!index_root
        .join("generation-pins/generation-a.lock")
        .exists());
    assert!(index_root
        .join("generation-pins/generation-b.lock")
        .exists());
    assert_eq!(
        opened_generation_b
            .search(SearchQuery::new("payment"))
            .unwrap()
            .len(),
        1
    );
    drop(opened_generation_b);
    remove_dir(&index_root);
}

#[test]
fn prepared_gc_releases_root_fence_before_commit() {
    let index_root = temp_dir("prepared-reader-entry");
    publish_snapshot(&index_root, "generation-old", [java_payment_document()]).unwrap();
    publish_snapshot(
        &index_root,
        "generation-retained",
        [java_payment_document()],
    )
    .unwrap();
    let retained = BTreeSet::from(["generation-retained".to_string()]);
    let reader_root = index_root.clone();
    let (reader_ready_tx, reader_ready_rx) = std::sync::mpsc::sync_channel(0);
    let (reader_start_tx, reader_start_rx) = std::sync::mpsc::channel();
    let (lease_acquired_tx, lease_acquired_rx) = std::sync::mpsc::channel();
    let reader = thread::spawn(move || {
        reader_ready_tx.send(()).unwrap();
        reader_start_rx.recv().unwrap();
        let lease = SnapshotReadLease::acquire(&reader_root).unwrap().unwrap();
        lease_acquired_tx.send(()).unwrap();
        let opened =
            FullTextIndex::open_snapshot_with_lease(&reader_root, "generation-retained", lease)
                .unwrap()
                .unwrap();
        assert_eq!(
            opened.snapshot_metadata().unwrap().generation(),
            "generation-retained"
        );
    });
    reader_ready_rx.recv().unwrap();

    let acquisition = try_acquire_snapshot_gc(&index_root).unwrap().unwrap();
    let FullTextSnapshotGcPreparation::Prepared(prepared) =
        prepare_snapshot_gc(acquisition, &retained).unwrap()
    else {
        panic!("GC unexpectedly deferred");
    };

    reader_start_tx.send(()).unwrap();
    lease_acquired_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("snapshot read lease remained blocked after GC preparation");
    reader.join().unwrap();

    let summary = complete_gc_summary(commit_snapshot_gc(prepared));
    assert_eq!(summary.removed_snapshots(), 1);
    remove_dir(&index_root);
}

#[test]
fn obsolete_reader_defers_gc_without_blocking_current_query_or_publication() {
    let index_root = temp_dir("obsolete-generation-reader-gc");
    publish_snapshot(&index_root, "generation-a", [java_payment_document()]).unwrap();
    publish_snapshot(&index_root, "generation-b", [java_payment_document()]).unwrap();
    let opened_generation_a = open_snapshot(&index_root, "generation-a").unwrap().unwrap();
    let retained = BTreeSet::from(["generation-b".to_string()]);
    assert!(run_snapshot_gc(&index_root, &retained).unwrap().is_none());
    assert!(index_root.join("snapshots/generation-a").exists());
    assert_eq!(
        open_snapshot(&index_root, "generation-b")
            .unwrap()
            .unwrap()
            .search(SearchQuery::new("payment"))
            .unwrap()
            .len(),
        1
    );
    publish_snapshot(&index_root, "generation-c", [java_payment_document()]).unwrap();
    drop(opened_generation_a);

    let retained = BTreeSet::from(["generation-b".to_string(), "generation-c".to_string()]);
    let summary = complete_gc_summary(run_snapshot_gc(&index_root, &retained).unwrap().unwrap());
    assert_eq!(summary.removed_snapshots(), 1);
    assert!(!index_root.join("snapshots/generation-a").exists());
    assert!(!index_root
        .join("generation-pins/generation-a.lock")
        .exists());
    remove_dir(&index_root);
}

#[test]
fn busy_late_candidate_defers_gc_without_deleting_an_earlier_candidate() {
    let index_root = temp_dir("busy-late-generation-reader-gc");
    for generation in [
        "generation-a-free",
        "generation-b-retained",
        "generation-z-busy",
    ] {
        publish_snapshot(&index_root, generation, [java_payment_document()]).unwrap();
    }
    let busy_reader = open_snapshot(&index_root, "generation-z-busy")
        .unwrap()
        .unwrap();
    let retained = BTreeSet::from(["generation-b-retained".to_string()]);

    assert!(run_snapshot_gc(&index_root, &retained).unwrap().is_none());
    for generation in ["generation-a-free", "generation-z-busy"] {
        assert!(index_root.join("snapshots").join(generation).exists());
        assert!(index_root
            .join("generation-pins")
            .join(format!("{generation}.lock"))
            .exists());
    }

    drop(busy_reader);
    assert_eq!(
        complete_gc_summary(run_snapshot_gc(&index_root, &retained).unwrap().unwrap())
            .removed_snapshots(),
        2
    );
    remove_dir(&index_root);
}

#[test]
fn exact_open_acquires_generation_pin_before_releasing_root_fence() {
    let index_root = temp_dir("generation-open-gc-fence");
    publish_snapshot(&index_root, "generation-a", [java_payment_document()]).unwrap();
    publish_snapshot(&index_root, "generation-b", [java_payment_document()]).unwrap();
    let acquisition_lease = SnapshotReadLease::acquire(&index_root).unwrap().unwrap();
    let retained = BTreeSet::from(["generation-b".to_string()]);
    assert!(try_acquire_snapshot_gc(&index_root).unwrap().is_none());
    let opened_generation_a =
        FullTextIndex::open_snapshot_with_lease(&index_root, "generation-a", acquisition_lease)
            .unwrap()
            .unwrap();
    assert!(run_snapshot_gc(&index_root, &retained).unwrap().is_none());
    assert!(index_root.join("snapshots/generation-a").exists());
    drop(opened_generation_a);

    assert_eq!(
        complete_gc_summary(run_snapshot_gc(&index_root, &retained).unwrap().unwrap())
            .removed_snapshots(),
        1
    );
    assert!(!index_root.join("snapshots/generation-a").exists());
    assert!(!index_root
        .join("generation-pins/generation-a.lock")
        .exists());
    remove_dir(&index_root);
}

#[test]
fn missing_generation_pin_fails_closed_before_open_or_gc_deletion() {
    let index_root = temp_dir("missing-generation-pin");
    publish_snapshot(&index_root, "generation-a", [java_payment_document()]).unwrap();
    publish_snapshot(&index_root, "generation-b", [java_payment_document()]).unwrap();
    fs::remove_file(index_root.join("generation-pins/generation-a.lock")).unwrap();

    assert!(open_snapshot(&index_root, "generation-a").is_err());
    let retained = BTreeSet::from(["generation-b".to_string()]);
    assert!(run_snapshot_gc(&index_root, &retained).is_err());
    assert!(index_root.join("snapshots/generation-a").exists());
    assert!(index_root.join("snapshots/generation-b").exists());
    remove_dir(&index_root);
}

#[test]
fn snapshot_gc_removes_only_controlled_crash_staging_and_reports_it() {
    let index_root = temp_dir("snapshot-crash-staging-gc");
    publish_snapshot(
        &index_root,
        "generation-retained",
        [java_payment_document()],
    )
    .unwrap();
    publish_snapshot(&index_root, "generation-old", [java_payment_document()]).unwrap();
    let plaintext_staging = index_root
        .join("staging")
        .join(".generation-retained.staging-0000000000000000");
    let encrypted_staging = index_root
        .join("snapshots")
        .join(".generation-retained.tmp-1111111111111111");
    create_private_test_directory(&plaintext_staging);
    create_private_test_directory(&encrypted_staging);

    let retained = BTreeSet::from(["generation-retained".to_string()]);
    let summary = complete_gc_summary(run_snapshot_gc(&index_root, &retained).unwrap().unwrap());
    assert_eq!(summary.removed_snapshots(), 1);
    assert_eq!(summary.removed_staging(), 2);
    assert!(!plaintext_staging.exists());
    assert!(!encrypted_staging.exists());
    remove_dir(&index_root);
}

#[test]
fn snapshot_gc_fails_before_deleting_when_staging_layout_is_abnormal() {
    let index_root = temp_dir("snapshot-abnormal-staging-gc");
    publish_snapshot(
        &index_root,
        "generation-retained",
        [java_payment_document()],
    )
    .unwrap();
    publish_snapshot(&index_root, "generation-old", [java_payment_document()]).unwrap();
    let abnormal = index_root
        .join("staging")
        .join(".generation-retained.staging-0000000000000000");
    fs::write(&abnormal, b"not a directory").unwrap();
    #[cfg(unix)]
    fs::set_permissions(&abnormal, fs::Permissions::from_mode(0o600)).unwrap();

    let retained = BTreeSet::from(["generation-retained".to_string()]);
    assert!(run_snapshot_gc(&index_root, &retained).is_err());
    assert!(index_root.join("snapshots/generation-old").exists());
    assert!(abnormal.exists());
    remove_dir(&index_root);
}

#[cfg(unix)]
#[test]
fn snapshot_gc_rejects_staging_symlink_without_following_it() {
    let index_root = temp_dir("snapshot-staging-symlink-gc");
    publish_snapshot(
        &index_root,
        "generation-retained",
        [java_payment_document()],
    )
    .unwrap();
    let target = temp_dir("snapshot-staging-symlink-target");
    fs::write(target.join("sentinel"), b"keep").unwrap();
    let staging_link = index_root
        .join("staging")
        .join(".generation-retained.staging-0000000000000000");
    symlink(&target, &staging_link).unwrap();

    let retained = BTreeSet::from(["generation-retained".to_string()]);
    assert!(run_snapshot_gc(&index_root, &retained).is_err());
    assert!(target.join("sentinel").exists());
    remove_dir(&index_root);
    remove_dir(&target);
}

#[test]
fn active_projection_omission_keeps_removed_documents_out_of_the_snapshot() {
    let (index_root, index) = published_test_index(
        "projection-omission",
        [IndexDocument {
            doc_id: stable_document_id("visible"),
            resume_version_id: stable_resume_version_id("visible"),
            file_name: "visible-rust.pdf".to_string(),
            clean_text: "Rust local search implementation".to_string(),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: "Rust local search".to_string(),
            }],
        }],
    );

    let hits = index
        .search(SearchQuery::new("Java payment").with_limit(10))
        .unwrap();

    assert!(hits.is_empty());
    remove_dir(&index_root);
}

#[test]
fn top_n_snippets_are_generated_only_for_returned_hits() {
    let (index_root, index) = published_test_index(
        "topn-snippets",
        [
            java_payment_document(),
            IndexDocument {
                doc_id: stable_document_id("doc_java_backend"),
                resume_version_id: stable_resume_version_id("ver_java_backend"),
                file_name: "synthetic-java-backend.pdf".to_string(),
                clean_text: "Java backend search service".to_string(),
                sections: vec![IndexSection {
                    section_type: "skill".to_string(),
                    text: "Java backend".to_string(),
                }],
            },
        ],
    );

    let hits = index
        .search(SearchQuery::new("Java").with_limit(1))
        .unwrap();

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].rank, 1);
    assert!(!hits[0].snippet.is_empty());
    remove_dir(&index_root);
}

#[test]
fn duplicate_sections_do_not_hide_distinct_documents_at_top_n_boundary() {
    let mut section_heavy = java_payment_document();
    section_heavy.sections = (0..12)
        .map(|index| IndexSection {
            section_type: "experience".to_string(),
            text: format!("Java payment repeated section {index}"),
        })
        .collect();

    let (index_root, index) = published_test_index(
        "duplicate-sections",
        [
            section_heavy,
            IndexDocument {
                doc_id: stable_document_id("doc_second_java"),
                resume_version_id: stable_resume_version_id("ver_second_java"),
                file_name: "synthetic-second-java.pdf".to_string(),
                clean_text: "Java payment migration".to_string(),
                sections: vec![IndexSection {
                    section_type: "experience".to_string(),
                    text: "Java payment migration".to_string(),
                }],
            },
        ],
    );

    let hits = index
        .search(SearchQuery::new("Java").with_limit(2))
        .unwrap();

    assert_eq!(hits.len(), 2);
    assert!(hits
        .iter()
        .any(|hit| hit.doc_id == stable_document_id("java-payment")));
    assert!(hits
        .iter()
        .any(|hit| hit.doc_id == stable_document_id("doc_second_java")));
    remove_dir(&index_root);
}

#[test]
fn malformed_query_syntax_returns_safe_result_instead_of_error() {
    let (index_root, index) = published_test_index("malformed-query", [java_payment_document()]);

    let hits = index
        .search(SearchQuery::new("Java \"").with_limit(5))
        .unwrap();

    assert_eq!(hits.len(), 1);
    remove_dir(&index_root);
}

#[test]
fn mixed_chinese_english_query_matches_clean_text() {
    let (index_root, index) = published_test_index(
        "mixed-query",
        [IndexDocument {
            doc_id: stable_document_id("doc_java_pay_cn"),
            resume_version_id: stable_resume_version_id("ver_java_pay_cn"),
            file_name: "synthetic-java-pay-cn.pdf".to_string(),
            clean_text: "Java 支付 平台 本地 搜索".to_string(),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: "Java 支付 平台".to_string(),
            }],
        }],
    );

    let hits = index
        .search(SearchQuery::new("Java 支付").with_limit(5))
        .unwrap();

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].doc_id, stable_document_id("doc_java_pay_cn"));
    assert!(hits[0].snippet.contains("支付"));
    remove_dir(&index_root);
}

#[test]
fn simple_terms_are_required_all_while_explicit_or_and_phrase_remain_explicit() {
    let document = |doc_id: &str, text: &str| IndexDocument {
        doc_id: stable_document_id(doc_id),
        resume_version_id: stable_resume_version_id(doc_id),
        file_name: format!("{doc_id}.pdf"),
        clean_text: text.to_string(),
        sections: Vec::new(),
    };

    let (index_root, index) = published_test_index(
        "required-all-query-semantics",
        [
            document("both", "rust backend platform"),
            document("rust-only", "rust frontend platform"),
            document("backend-only", "python backend platform"),
            document("reversed", "backend rust platform"),
        ],
    );

    let required_all = index
        .search(SearchQuery::new("rust backend").with_limit(10))
        .unwrap()
        .into_iter()
        .map(|hit| hit.doc_id)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        required_all,
        BTreeSet::from([stable_document_id("both"), stable_document_id("reversed"),])
    );

    let reordered = index
        .search(SearchQuery::new("backend rust").with_limit(10))
        .unwrap()
        .into_iter()
        .map(|hit| hit.doc_id)
        .collect::<BTreeSet<_>>();
    assert_eq!(reordered, required_all);

    let explicit_or = index
        .search(SearchQuery::new("rust OR backend").with_limit(10))
        .unwrap();
    assert_eq!(explicit_or.len(), 4);

    let phrase = index
        .search(SearchQuery::new("\"rust backend\"").with_limit(10))
        .unwrap();
    assert_eq!(
        phrase.into_iter().map(|hit| hit.doc_id).collect::<Vec<_>>(),
        vec![stable_document_id("both")]
    );

    remove_dir(&index_root);
}

#[test]
fn snippets_redact_contact_values_near_query_matches() {
    let (index_root, index) = published_test_index(
        "snippet-redaction",
        [IndexDocument {
            doc_id: stable_document_id("doc_contact"),
            resume_version_id: stable_resume_version_id("ver_contact"),
            file_name: "synthetic-contact.pdf".to_string(),
            clean_text: "Java WeChat: Candidate_2026 Email: a@b.test Phone: +14155550132"
                .to_string(),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: "Built Java ranking services".to_string(),
            }],
        }],
    );

    let hits = index
        .search(SearchQuery::new("Java").with_limit(5))
        .unwrap();

    assert_eq!(hits.len(), 1);
    assert!(hits[0].snippet.contains("Java"));
    assert!(hits[0].snippet.contains("<redacted-email>"));
    assert!(hits[0].snippet.contains("<redacted-phone>"));
    assert!(hits[0].snippet.contains("<redacted-wechat>"));
    assert!(!hits[0].snippet.contains("a@b.test"));
    assert!(!hits[0].snippet.contains("Candidate_2026"));
    assert!(!hits[0].snippet.contains("415"));
    remove_dir(&index_root);
}

#[test]
fn published_index_fields_expose_only_redacted_contact_values() {
    let (index_root, index) = published_test_index(
        "stored-contact-redaction",
        [IndexDocument {
            doc_id: stable_document_id("doc_stored_contact"),
            resume_version_id: stable_resume_version_id("ver_stored_contact"),
            file_name: "synthetic-Shared.Candidate@Example.Test.pdf".to_string(),
            clean_text: concat!(
                "Built Java systems. Email: Shared.Candidate@Example.Test ",
                "Phone: (415) 555-0132 Alt: (415)555-0132 Backup: +1(415)555-0132"
            )
            .to_string(),
            sections: vec![IndexSection {
                section_type: "contact".to_string(),
                text: "Contact +14155550132 and Shared.Candidate@Example.Test".to_string(),
            }],
        }],
    );

    let hits = index
        .search(SearchQuery::new("Java systems").with_limit(5))
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].snippet.contains("Java"));
    assert!(hits[0].file_name.contains("<redacted-email>"));
    assert!(!hits[0].file_name.contains("Shared.Candidate"));

    for contact_query in [
        "Shared.Candidate@Example.Test",
        "(415) 555-0132",
        "(415)555-0132",
        "+1(415)555-0132",
        "+14155550132",
    ] {
        let contact_hits = index
            .search(SearchQuery::new(contact_query).with_limit(5))
            .unwrap();
        assert!(
            contact_hits.is_empty(),
            "query should not match: {contact_query}"
        );
    }
    remove_dir(&index_root);
}

#[test]
fn published_snapshot_opens_only_by_explicit_generation_without_reading_staging_orphans() {
    let index_root = temp_dir("published-snapshot");

    publish_snapshot(
        &index_root,
        "fulltext-1800001000-1-0-0",
        [java_payment_document()],
    )
    .unwrap();
    fs::create_dir_all(index_root.join("staging").join("orphan-bad")).unwrap();
    write_snapshot_test_file_with_retry(
        &index_root
            .join("staging")
            .join("orphan-bad")
            .join("meta.json"),
        b"not a valid tantivy index",
    )
    .unwrap();

    assert_eq!(staging_orphan_count(&index_root).unwrap(), 1);
    assert!(!index_root.join("active-snapshot").exists());

    let index = open_snapshot(&index_root, "fulltext-1800001000-1-0-0")
        .unwrap()
        .unwrap();
    let hits = index
        .search(SearchQuery::new("Java payment").with_limit(5))
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].doc_id, stable_document_id("java-payment"));

    remove_dir(&index_root);
}

#[test]
fn published_snapshot_encrypts_payload_at_rest() {
    let index_root = temp_dir("published-encrypted-snapshot");
    let snapshot_name = "fulltext-1800003000-1-0-0";
    let private_payload = "PRIVATE_FULLTEXT_PAYLOAD_SECRET_1800003000";

    publish_snapshot(
        &index_root,
        snapshot_name,
        [IndexDocument {
            doc_id: stable_document_id("doc_private_fulltext"),
            resume_version_id: stable_resume_version_id("ver_private_fulltext"),
            file_name: "synthetic-private-fulltext.pdf".to_string(),
            clean_text: format!("Rust local search {private_payload}"),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: format!("Search evidence {private_payload}"),
            }],
        }],
    )
    .unwrap();

    let snapshot_dir = index_root.join("snapshots").join(snapshot_name);
    let envelope = fs::read(snapshot_dir.join("fulltext.snapshot.enc")).unwrap();
    assert!(envelope.starts_with(b"resume-ir-fulltext-snapshot-encrypted-v3\n"));
    assert!(!snapshot_dir.join("meta.json").exists());
    let snapshot_bytes = recursive_bytes(&snapshot_dir);
    assert!(!String::from_utf8_lossy(&snapshot_bytes).contains(private_payload));

    let reopened = open_snapshot(&index_root, snapshot_name).unwrap().unwrap();
    let hits = reopened
        .search(SearchQuery::new("Rust local search").with_limit(5))
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].doc_id, stable_document_id("doc_private_fulltext"));
    assert!(hits[0].snippet.contains("Rust"));

    remove_dir(&index_root);
}

#[test]
fn v2_manifest_rejects_exact_generation_without_implicit_fallback() {
    let index_root = temp_dir("snapshot-schema-mismatch");
    publish_snapshot(
        &index_root,
        "fulltext-1800003100-1-0-0",
        [java_payment_document()],
    )
    .unwrap();
    publish_snapshot(
        &index_root,
        "fulltext-1800003200-1-0-0",
        [IndexDocument {
            doc_id: stable_document_id("doc_future_schema"),
            resume_version_id: stable_resume_version_id("ver_future_schema"),
            file_name: "synthetic-future-schema.pdf".to_string(),
            clean_text: "future schema active snapshot".to_string(),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: "future schema active snapshot".to_string(),
            }],
        }],
    )
    .unwrap();

    let manifest_path = index_root
        .join("snapshots")
        .join("fulltext-1800003200-1-0-0")
        .join("snapshot-manifest.json");
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    assert!(manifest.contains("\"schema_version\":\"fulltext.snapshot.v3\""));
    assert!(!manifest.contains("Java payment"));
    fs::write(
        &manifest_path,
        manifest.replace("fulltext.snapshot.v3", "fulltext.snapshot.v2"),
    )
    .unwrap();

    assert!(open_snapshot(&index_root, "fulltext-1800003200-1-0-0").is_err());
    let index = open_snapshot(&index_root, "fulltext-1800003100-1-0-0")
        .unwrap()
        .unwrap();
    let recovered_hits = index
        .search(SearchQuery::new("Java payment").with_limit(5))
        .unwrap();
    assert_eq!(recovered_hits.len(), 1);
    assert_eq!(recovered_hits[0].doc_id, stable_document_id("java-payment"));
    assert!(index
        .search(SearchQuery::new("future schema").with_limit(5))
        .unwrap()
        .is_empty());

    remove_dir(&index_root);
}

#[test]
fn manifest_document_count_mismatch_fails_closed() {
    let index_root = temp_dir("snapshot-count-mismatch");
    let generation = "fulltext-count-mismatch";
    publish_snapshot(&index_root, generation, [java_payment_document()]).unwrap();
    let manifest_path = index_root
        .join("snapshots")
        .join(generation)
        .join("snapshot-manifest.json");
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    fs::write(
        &manifest_path,
        manifest.replace("\"document_count\":1", "\"document_count\":2"),
    )
    .unwrap();

    assert!(open_snapshot(&index_root, generation).is_err());
    remove_dir(&index_root);
}

#[test]
fn incremental_snapshot_inherits_replaces_and_excludes_documents() {
    let index_root = temp_dir("incremental-snapshot");

    publish_snapshot(
        &index_root,
        "fulltext-1800004000-1-0-0",
        [
            java_payment_document(),
            IndexDocument {
                doc_id: stable_document_id("doc_backend"),
                resume_version_id: stable_resume_version_id("ver_backend_old"),
                file_name: "synthetic-backend-old.pdf".to_string(),
                clean_text: "Rust backend retiredtoken".to_string(),
                sections: vec![IndexSection {
                    section_type: "experience".to_string(),
                    text: "Rust backend retiredtoken".to_string(),
                }],
            },
        ],
    )
    .unwrap();

    publish_incremental_snapshot(
        &index_root,
        Some("fulltext-1800004000-1-0-0"),
        "fulltext-1800005000-1-0-0",
        [
            IndexDocument {
                doc_id: stable_document_id("doc_backend"),
                resume_version_id: stable_resume_version_id("ver_backend_new"),
                file_name: "synthetic-backend-new.pdf".to_string(),
                clean_text: "Go backend updated snapshot token".to_string(),
                sections: vec![IndexSection {
                    section_type: "experience".to_string(),
                    text: "Go backend updated".to_string(),
                }],
            },
            IndexDocument {
                doc_id: stable_document_id("doc_python"),
                resume_version_id: stable_resume_version_id("ver_python_new"),
                file_name: "synthetic-python-new.pdf".to_string(),
                clean_text: "Python ranking new snapshot token".to_string(),
                sections: vec![IndexSection {
                    section_type: "skill".to_string(),
                    text: "Python ranking".to_string(),
                }],
            },
        ],
        &BTreeSet::from([stable_document_id("java-payment")]),
    )
    .unwrap();

    let index = open_snapshot(&index_root, "fulltext-1800005000-1-0-0")
        .unwrap()
        .unwrap();
    assert!(index
        .search(SearchQuery::new("Java payment").with_limit(5))
        .unwrap()
        .is_empty());
    assert!(index
        .search(SearchQuery::new("retiredtoken").with_limit(5))
        .unwrap()
        .is_empty());

    let updated_hits = index
        .search(SearchQuery::new("Go backend").with_limit(5))
        .unwrap();
    assert_eq!(updated_hits.len(), 1);
    assert_eq!(updated_hits[0].doc_id, stable_document_id("doc_backend"));
    assert_eq!(
        updated_hits[0].resume_version_id,
        stable_resume_version_id("ver_backend_new")
    );

    let new_hits = index
        .search(SearchQuery::new("Python ranking").with_limit(5))
        .unwrap();
    assert_eq!(new_hits.len(), 1);
    assert_eq!(new_hits[0].doc_id, stable_document_id("doc_python"));

    remove_dir(&index_root);
}

#[test]
fn corrupt_generation_never_selects_a_previous_generation_implicitly() {
    let index_root = temp_dir("snapshot-fallback");
    publish_snapshot(
        &index_root,
        "fulltext-1800001000-1-0-0",
        [java_payment_document()],
    )
    .unwrap();
    publish_snapshot(
        &index_root,
        "fulltext-1800002000-1-0-0",
        [IndexDocument {
            doc_id: stable_document_id("doc_rust_snapshot"),
            resume_version_id: stable_resume_version_id("ver_rust_snapshot"),
            file_name: "synthetic-rust-snapshot.pdf".to_string(),
            clean_text: "Rust snapshot that will be corrupted".to_string(),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: "Rust snapshot".to_string(),
            }],
        }],
    )
    .unwrap();
    write_snapshot_test_file_with_retry(
        &index_root
            .join("snapshots")
            .join("fulltext-1800002000-1-0-0")
            .join("fulltext.snapshot.enc"),
        b"not a valid encrypted snapshot",
    )
    .unwrap();

    assert!(open_snapshot(&index_root, "fulltext-1800002000-1-0-0").is_err());
    let index = open_snapshot(&index_root, "fulltext-1800001000-1-0-0")
        .unwrap()
        .unwrap();
    let recovered_hits = index
        .search(SearchQuery::new("Java payment").with_limit(5))
        .unwrap();
    assert_eq!(recovered_hits.len(), 1);
    assert_eq!(recovered_hits[0].doc_id, stable_document_id("java-payment"));
    assert!(index
        .search(SearchQuery::new("corrupted").with_limit(5))
        .unwrap()
        .is_empty());

    remove_dir(&index_root);
}

#[test]
fn snapshot_identity_digests_are_version_exact_and_empty_snapshots_are_stable() {
    let index_root = temp_dir("snapshot-identity-digests");
    let document = java_payment_document();
    let first = publish_snapshot(&index_root, "identity-a", [document.clone()]).unwrap();
    let mut next_version = document;
    next_version.resume_version_id = stable_resume_version_id("java-payment-v2");
    let second = publish_snapshot(&index_root, "identity-b", [next_version]).unwrap();

    assert_ne!(first.projection_digest(), second.projection_digest());
    assert_ne!(
        first.logical_content_digest(),
        second.logical_content_digest()
    );

    let empty_a =
        publish_snapshot(&index_root, "empty-a", std::iter::empty::<IndexDocument>()).unwrap();
    let empty_b =
        publish_snapshot(&index_root, "empty-b", std::iter::empty::<IndexDocument>()).unwrap();
    assert_eq!(empty_a.projection_digest(), empty_b.projection_digest());
    assert_eq!(
        empty_a.logical_content_digest(),
        empty_b.logical_content_digest()
    );
    let opened = open_snapshot(&index_root, "empty-a").unwrap().unwrap();
    let first_projection = opened.exact_identity_pairs().unwrap();
    let second_projection = opened.exact_identity_pairs().unwrap();
    assert!(first_projection.is_empty());
    assert!(std::ptr::eq(first_projection, second_projection));
    remove_dir(&index_root);
}

#[test]
fn manifest_identity_or_artifact_digest_tampering_fails_closed() {
    for field in [
        "projection_digest",
        "logical_content_digest",
        "artifact_digest",
    ] {
        let index_root = temp_dir(&format!("manifest-{field}-tamper"));
        let generation = format!("tamper-{field}");
        publish_snapshot(&index_root, &generation, [java_payment_document()]).unwrap();
        let manifest_path = index_root
            .join("snapshots")
            .join(&generation)
            .join("snapshot-manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest[field] = serde_json::json!(format!("sha256:{}", "0".repeat(64)));
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

        assert!(open_snapshot(&index_root, &generation).is_err());
        remove_dir(&index_root);
    }
}

#[test]
fn encrypted_snapshot_byte_tampering_fails_closed() {
    let index_root = temp_dir("encrypted-byte-tamper");
    let generation = "encrypted-byte-tamper";
    publish_snapshot(&index_root, generation, [java_payment_document()]).unwrap();
    let encrypted_path = index_root
        .join("snapshots")
        .join(generation)
        .join("fulltext.snapshot.enc");
    let mut encrypted = fs::read(&encrypted_path).unwrap();
    let last = encrypted.last_mut().unwrap();
    *last ^= 0x01;
    fs::write(&encrypted_path, encrypted).unwrap();

    assert!(open_snapshot(&index_root, generation).is_err());
    remove_dir(&index_root);
}

#[test]
fn generation_relabel_cannot_replay_a_valid_encrypted_snapshot() {
    let index_root = temp_dir("generation-relabel-replay");
    publish_snapshot(&index_root, "generation-a", [java_payment_document()]).unwrap();
    let source = index_root.join("snapshots/generation-a");
    let replay = index_root.join("snapshots/generation-b");
    fs::rename(source, &replay).unwrap();
    let manifest_path = replay.join("snapshot-manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["generation"] = serde_json::json!("generation-b");
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    assert!(open_snapshot(&index_root, "generation-b").is_err());
    remove_dir(&index_root);
}

#[test]
fn oversized_manifest_fails_closed_before_json_parse() {
    let index_root = temp_dir("oversized-manifest");
    let generation = "oversized-manifest";
    publish_snapshot(&index_root, generation, [java_payment_document()]).unwrap();
    let manifest_path = index_root
        .join("snapshots")
        .join(generation)
        .join("snapshot-manifest.json");
    fs::write(&manifest_path, vec![b'x'; 4 * 1024 + 1]).unwrap();

    assert!(open_snapshot(&index_root, generation).is_err());
    remove_dir(&index_root);
}

#[cfg(unix)]
#[test]
fn publication_rejects_symlink_layout_roots() {
    for layout in ["staging", "snapshots"] {
        let index_root = temp_dir(&format!("layout-symlink-{layout}"));
        let target = temp_dir(&format!("layout-symlink-target-{layout}"));
        symlink(&target, index_root.join(layout)).unwrap();

        assert!(publish_snapshot(&index_root, "generation-a", [java_payment_document()]).is_err());
        assert!(fs::read_dir(&target).unwrap().next().is_none());
        remove_dir(&index_root);
        remove_dir(&target);
    }
}

#[cfg(unix)]
#[test]
fn canonical_root_pins_publication_and_rejects_mismatched_read_lease() {
    use std::cell::Cell;

    let parent = temp_dir("canonical-root-parent");
    let target_a = temp_dir("canonical-root-a");
    let target_b = temp_dir("canonical-root-b");
    let link = parent.join("fulltext-index");
    symlink(&target_a, &link).unwrap();
    let retargeted = Cell::new(false);
    let observer = |phase| {
        if phase == SnapshotPublishPhase::DocumentIndexing && !retargeted.replace(true) {
            fs::remove_file(&link).unwrap();
            symlink(&target_b, &link).unwrap();
        }
    };
    publish_snapshot_with_control(
        &link,
        "generation-a",
        [java_payment_document()],
        SnapshotPublishControl::disabled().with_phase_observer(&observer),
    )
    .unwrap();

    assert!(target_a.join("snapshots/generation-a").exists());
    assert!(!target_b.join("snapshots/generation-a").exists());
    let lease_a = SnapshotReadLease::acquire(&target_a).unwrap().unwrap();
    assert!(FullTextIndex::open_snapshot_with_lease(&link, "generation-a", lease_a).is_err());
    assert!(open_snapshot(&target_a, "generation-a").unwrap().is_some());
    remove_dir(&parent);
    remove_dir(&target_a);
    remove_dir(&target_b);
}

#[cfg(unix)]
#[test]
fn exact_open_rejects_snapshots_intermediate_symlink_after_lease_acquisition() {
    let index_root = temp_dir("snapshots-intermediate-symlink");
    publish_snapshot(&index_root, "generation-a", [java_payment_document()]).unwrap();
    let lease = SnapshotReadLease::acquire(&index_root).unwrap().unwrap();
    let snapshots = index_root.join("snapshots");
    let real_snapshots = index_root.join("snapshots-real");
    fs::rename(&snapshots, &real_snapshots).unwrap();
    symlink(&real_snapshots, &snapshots).unwrap();

    assert!(FullTextIndex::open_snapshot_with_lease(&index_root, "generation-a", lease).is_err());
    assert!(real_snapshots.join("generation-a").exists());
    remove_dir(&index_root);
}

#[cfg(unix)]
#[test]
fn exact_open_rejects_same_path_root_inode_replacement() {
    let parent = temp_dir("root-inode-replacement-parent");
    let index_root = parent.join("index");
    create_private_test_directory(&index_root);
    publish_snapshot(&index_root, "generation-a", [java_payment_document()]).unwrap();
    let lease = SnapshotReadLease::acquire(&index_root).unwrap().unwrap();
    let displaced = parent.join("index-displaced");
    fs::rename(&index_root, &displaced).unwrap();
    create_private_test_directory(&index_root);
    publish_snapshot(&index_root, "generation-b", [java_payment_document()]).unwrap();

    assert!(FullTextIndex::open_snapshot_with_lease(&index_root, "generation-b", lease).is_err());
    assert!(open_snapshot(&index_root, "generation-b")
        .unwrap()
        .is_some());
    remove_dir(&parent);
}

#[cfg(unix)]
#[test]
fn exact_open_rejects_intermediate_directory_inode_replacement() {
    for component in ["snapshots", "generation-pins"] {
        let index_root = temp_dir(&format!("{component}-inode-replacement"));
        publish_snapshot(&index_root, "generation-a", [java_payment_document()]).unwrap();
        let lease = SnapshotReadLease::acquire(&index_root).unwrap().unwrap();
        let original = index_root.join(component);
        let displaced = index_root.join(format!("{component}-displaced"));
        fs::rename(&original, &displaced).unwrap();
        create_private_test_directory(&original);
        if component == "snapshots" {
            fs::rename(
                displaced.join("generation-a"),
                original.join("generation-a"),
            )
            .unwrap();
        } else {
            fs::rename(
                displaced.join("generation-a.lock"),
                original.join("generation-a.lock"),
            )
            .unwrap();
        }

        assert!(
            FullTextIndex::open_snapshot_with_lease(&index_root, "generation-a", lease).is_err()
        );
        remove_dir(&index_root);
    }
}

#[cfg(unix)]
#[test]
fn gc_lease_rejects_same_path_root_inode_replacement_without_deleting_new_root() {
    let parent = temp_dir("gc-root-inode-replacement-parent");
    let index_root = parent.join("index");
    create_private_test_directory(&index_root);
    publish_snapshot(&index_root, "generation-a", [java_payment_document()]).unwrap();
    let acquisition = try_acquire_snapshot_gc(&index_root).unwrap().unwrap();
    let displaced = parent.join("index-displaced");
    fs::rename(&index_root, &displaced).unwrap();
    create_private_test_directory(&index_root);
    publish_snapshot(&index_root, "generation-b", [java_payment_document()]).unwrap();
    let retained = BTreeSet::from(["generation-b".to_string()]);

    assert!(prepare_snapshot_gc(acquisition, &retained).is_err());
    assert!(index_root.join("snapshots/generation-b").exists());
    remove_dir(&parent);
}

#[cfg(unix)]
#[test]
fn publication_rejects_same_path_root_inode_replacement() {
    use std::cell::Cell;

    let parent = temp_dir("publication-root-inode-replacement-parent");
    let index_root = parent.join("index");
    create_private_test_directory(&index_root);
    let displaced = parent.join("index-displaced");
    let replaced = Cell::new(false);
    let observer = |phase| {
        if phase == SnapshotPublishPhase::DocumentIndexing && !replaced.replace(true) {
            fs::rename(&index_root, &displaced).unwrap();
            create_private_test_directory(&index_root);
            publish_snapshot(&index_root, "generation-b", [java_payment_document()]).unwrap();
        }
    };

    assert!(publish_snapshot_with_control(
        &index_root,
        "generation-a",
        [java_payment_document()],
        SnapshotPublishControl::disabled().with_phase_observer(&observer),
    )
    .is_err());
    assert!(index_root.join("snapshots/generation-b").exists());
    assert!(!index_root.join("snapshots/generation-a").exists());
    remove_dir(&parent);
}

fn java_payment_document() -> IndexDocument {
    IndexDocument {
        doc_id: stable_document_id("java-payment"),
        resume_version_id: stable_resume_version_id("java-payment"),
        file_name: "synthetic-java-payment.pdf".to_string(),
        clean_text: "Built a Java payment platform with local search observability.".to_string(),
        sections: vec![
            IndexSection {
                section_type: "experience".to_string(),
                text: "Java payment platform".to_string(),
            },
            IndexSection {
                section_type: "skill".to_string(),
                text: "Java Rust SQLite".to_string(),
            },
        ],
    }
}

fn stable_document_id(seed: &str) -> String {
    stable_id("doc_", seed)
}

fn stable_resume_version_id(seed: &str) -> String {
    stable_id("ver_", seed)
}

fn stable_id(prefix: &str, seed: &str) -> String {
    let mut left = 0xcbf2_9ce4_8422_2325_u64;
    let mut right = 0x6c62_272e_07bb_0142_u64;
    for byte in seed.bytes() {
        left = (left ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        right = (right ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{prefix}{left:016x}{right:016x}")
}

fn published_test_index(
    label: &str,
    documents: impl IntoIterator<Item = IndexDocument>,
) -> (PathBuf, FullTextIndex) {
    let index_root = temp_dir(label);
    let generation = "test-generation";
    publish_snapshot(&index_root, generation, documents).unwrap();
    let index = open_snapshot(&index_root, generation).unwrap().unwrap();
    (index_root, index)
}

fn open_snapshot(
    index_root: &Path,
    generation: &str,
) -> index_fulltext::Result<Option<FullTextIndex>> {
    let Some(lease) = SnapshotReadLease::acquire(index_root)? else {
        return Ok(None);
    };
    FullTextIndex::open_snapshot_with_lease(index_root, generation, lease)
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s8-index-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

fn create_private_test_directory(path: &Path) {
    fs::create_dir(path).unwrap();
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn write_snapshot_test_file_with_retry(path: &Path, bytes: &[u8]) -> io::Result<()> {
    for attempt in 0..SNAPSHOT_TEST_WRITE_RETRY_ATTEMPTS {
        match fs::write(path, bytes) {
            Ok(()) => return Ok(()),
            Err(error)
                if attempt + 1 < SNAPSHOT_TEST_WRITE_RETRY_ATTEMPTS
                    && is_transient_snapshot_test_write_error(&error) =>
            {
                thread::sleep(SNAPSHOT_TEST_WRITE_RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::other("snapshot test write retry exhausted"))
}

fn is_transient_snapshot_test_write_error(error: &io::Error) -> bool {
    if matches!(
        error.kind(),
        ErrorKind::Interrupted | ErrorKind::PermissionDenied | ErrorKind::WouldBlock
    ) {
        return true;
    }

    #[cfg(windows)]
    if matches!(error.raw_os_error(), Some(32 | 33 | 145)) {
        return true;
    }

    let diagnostic = error.to_string().to_ascii_lowercase();
    diagnostic.contains("os error 5")
        || diagnostic.contains("os error 32")
        || diagnostic.contains("os error 33")
        || diagnostic.contains("os error 145")
        || diagnostic.contains("access is denied")
        || diagnostic.contains("permission denied")
        || diagnostic.contains("being used by another process")
        || diagnostic.contains("locked a portion of the file")
}

fn recursive_bytes(root: &Path) -> Vec<u8> {
    let mut output = Vec::new();
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            output.extend(recursive_bytes(&path));
        } else {
            output.extend(fs::read(path).unwrap());
        }
    }
    output
}
