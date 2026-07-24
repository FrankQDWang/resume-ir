use std::fs::{self, OpenOptions};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use rusqlite::{params, Connection, Error as SqliteError, ErrorCode};
use tempfile::TempDir;

use super::{
    cleanup::{
        recover_migration_attempt, recover_migration_attempt_at_cleanup_cut,
        MigrationCleanupFailpoint, ATTEMPT_JOURNAL_FILE, ATTEMPT_TEMP_PREFIX,
    },
    *,
};
use crate::active_store_manifest::{publish_new_active_store, read_manifest};
use crate::{
    ContentDigest, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease,
    FullTextSnapshotDescriptor, ImportProcessingContract, ImportTaskId, ReadMetaStore,
    SearchProjectionDigest, SearchPublicationCommit, SearchPublicationDraft,
    SearchPublicationOutcome, SearchPublicationSession, SearchPublicationValidation, UnixTimestamp,
    VectorSnapshotDescriptor, CLASSIFIER_EPOCH,
};

const TEST_KEY: [u8; METADATA_ENCRYPTION_KEY_LEN] = [0x35; METADATA_ENCRYPTION_KEY_LEN];
const V27_DIGEST: &str = "1111111111111111111111111111111111111111111111111111111111111111";

#[derive(Clone, Copy)]
enum V27SourceState {
    Repairing,
    Ready,
}

#[test]
fn fresh_store_uses_a_versioned_v28_manifest() {
    let directory = TempDir::new().unwrap();

    let active =
        ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap();

    let manifest = read_manifest(&directory.path().join(MANIFEST_FILE)).unwrap();
    assert_eq!(manifest.schema_version, schema_v28::VERSION);
    assert!(manifest.file_name.starts_with("metadata-v28-"));
    assert_ne!(manifest.file_name, METADATA_STORE_FILE);
    assert_eq!(
        active,
        canonical_data_dir(directory.path()).join(&manifest.file_name)
    );
    assert_clean_attempt_namespace(directory.path());
    assert_eq!(v28_store_files(directory.path()), vec![manifest.file_name]);
}

#[test]
fn every_prepublication_crash_cut_is_recovered_without_orphan_accumulation() {
    for failpoint in [
        MigrationFailpoint::AfterAttemptWriteCrash,
        MigrationFailpoint::AfterTargetCreateCrash,
        MigrationFailpoint::AfterSourceCopyCrash,
        MigrationFailpoint::AfterTargetValidationCrash,
    ] {
        let directory = TempDir::new().unwrap();
        ensure_active_v28_store(directory.path(), &TEST_KEY, failpoint).unwrap_err();
        assert!(!directory.path().join(MANIFEST_FILE).exists());
        assert!(directory.path().join(ATTEMPT_JOURNAL_FILE).exists());

        let active =
            ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap();
        let manifest = read_manifest(&directory.path().join(MANIFEST_FILE)).unwrap();
        assert_eq!(
            active,
            canonical_data_dir(directory.path()).join(&manifest.file_name)
        );
        assert_clean_attempt_namespace(directory.path());
        assert_eq!(v28_store_files(directory.path()), vec![manifest.file_name]);
    }
}

#[test]
fn a_controlled_pre_manifest_failure_removes_the_attempt_and_target() {
    let directory = TempDir::new().unwrap();

    ensure_active_v28_store(
        directory.path(),
        &TEST_KEY,
        MigrationFailpoint::BeforeManifest,
    )
    .unwrap_err();

    assert!(!directory.path().join(MANIFEST_FILE).exists());
    assert_clean_attempt_namespace(directory.path());
    assert!(v28_store_files(directory.path()).is_empty());
}

#[test]
fn post_manifest_crash_reopens_the_exact_committed_target() {
    for failpoint in [
        MigrationFailpoint::AfterManifestRename,
        MigrationFailpoint::AfterManifest,
    ] {
        let directory = TempDir::new().unwrap();
        ensure_active_v28_store(directory.path(), &TEST_KEY, failpoint).unwrap_err();
        let committed = read_manifest(&directory.path().join(MANIFEST_FILE)).unwrap();
        assert_eq!(committed.schema_version, schema_v28::VERSION);
        assert!(directory.path().join(ATTEMPT_JOURNAL_FILE).exists());

        let reopened =
            ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap();
        assert_eq!(
            read_manifest(&directory.path().join(MANIFEST_FILE)).unwrap(),
            committed
        );
        assert_eq!(
            reopened,
            canonical_data_dir(directory.path()).join(&committed.file_name)
        );
        assert_clean_attempt_namespace(directory.path());
        assert_eq!(v28_store_files(directory.path()), vec![committed.file_name]);
    }
}

#[test]
fn unpublished_cleanup_crash_cuts_never_lose_artifact_recovery_authority() {
    for failpoint in cleanup_crash_failpoints() {
        let directory = TempDir::new().unwrap();
        ensure_active_v28_store(
            directory.path(),
            &TEST_KEY,
            MigrationFailpoint::AfterTargetCreateCrash,
        )
        .unwrap_err();
        let target_name = v28_store_files(directory.path())
            .into_iter()
            .next()
            .unwrap();
        let target_path = directory.path().join(target_name);
        let journal_path = directory.path().join(ATTEMPT_JOURNAL_FILE);
        let journal_bytes = fs::read(&journal_path).unwrap();

        recover_migration_attempt_at_cleanup_cut(directory.path(), None, failpoint).unwrap_err();

        assert_sqlite_artifacts_absent(&target_path);
        match failpoint {
            MigrationCleanupFailpoint::AfterArtifactRemoval => {
                assert!(journal_path.exists());
                crate::write_new_private_file(&target_path, b"replayed-unpublished-target")
                    .unwrap();
            }
            MigrationCleanupFailpoint::AfterArtifactSync => {
                assert!(journal_path.exists());
            }
            MigrationCleanupFailpoint::AfterJournalRemoval => {
                assert!(!journal_path.exists());
                crate::write_new_private_file(
                    &journal_path,
                    journal_bytes.strip_suffix(b"\n").unwrap(),
                )
                .unwrap();
            }
            MigrationCleanupFailpoint::None => unreachable!(),
        }

        recover_migration_attempt(directory.path(), None)
            .unwrap_or_else(|error| panic!("{failpoint:?}: {error:?}"));

        assert_sqlite_artifacts_absent(&target_path);
        assert_clean_attempt_namespace(directory.path());
        assert!(v28_store_files(directory.path()).is_empty());
    }
}

#[test]
fn published_cleanup_crash_cuts_never_orphan_the_recorded_predecessor() {
    for failpoint in cleanup_crash_failpoints() {
        let directory = TempDir::new().unwrap();
        let predecessor = seed_v27_manifest(directory.path(), V27SourceState::Repairing);
        let predecessor_path = directory.path().join(predecessor.file_name);
        ensure_active_v28_store(
            directory.path(),
            &TEST_KEY,
            MigrationFailpoint::AfterManifest,
        )
        .unwrap_err();
        let manifest_path = directory.path().join(MANIFEST_FILE);
        let committed = read_manifest(&manifest_path).unwrap();
        let active_path = directory.path().join(&committed.file_name);
        let journal_path = directory.path().join(ATTEMPT_JOURNAL_FILE);
        let journal_bytes = fs::read(&journal_path).unwrap();

        recover_migration_attempt_at_cleanup_cut(directory.path(), Some(&committed), failpoint)
            .unwrap_err();

        assert!(active_path.exists());
        assert_sqlite_artifacts_absent(&predecessor_path);
        match failpoint {
            MigrationCleanupFailpoint::AfterArtifactRemoval => {
                assert!(journal_path.exists());
                crate::write_new_private_file(&predecessor_path, b"replayed-v27-predecessor")
                    .unwrap();
            }
            MigrationCleanupFailpoint::AfterArtifactSync => {
                assert!(journal_path.exists());
            }
            MigrationCleanupFailpoint::AfterJournalRemoval => {
                assert!(!journal_path.exists());
                crate::write_new_private_file(
                    &journal_path,
                    journal_bytes.strip_suffix(b"\n").unwrap(),
                )
                .unwrap();
            }
            MigrationCleanupFailpoint::None => unreachable!(),
        }

        recover_migration_attempt(directory.path(), Some(&committed))
            .unwrap_or_else(|error| panic!("{failpoint:?}: {error:?}"));

        assert_eq!(read_manifest(&manifest_path).unwrap(), committed);
        assert!(active_path.exists());
        assert_sqlite_artifacts_absent(&predecessor_path);
        assert_clean_attempt_namespace(directory.path());
        assert_eq!(v28_store_files(directory.path()), vec![committed.file_name]);
    }
}

fn cleanup_crash_failpoints() -> [MigrationCleanupFailpoint; 3] {
    [
        MigrationCleanupFailpoint::AfterArtifactRemoval,
        MigrationCleanupFailpoint::AfterArtifactSync,
        MigrationCleanupFailpoint::AfterJournalRemoval,
    ]
}

#[test]
fn truncated_attempt_temporary_is_removed_before_a_new_attempt() {
    let directory = TempDir::new().unwrap();
    let truncated = directory.path().join(format!("{ATTEMPT_TEMP_PREFIX}crash"));
    crate::write_new_private_file(&truncated, b"truncated").unwrap();

    ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap();

    assert!(!truncated.exists());
    assert_clean_attempt_namespace(directory.path());
}

#[test]
fn malformed_fixed_attempt_journal_fails_closed_without_publishing() {
    let directory = TempDir::new().unwrap();
    crate::write_new_private_file(&directory.path().join(ATTEMPT_JOURNAL_FILE), b"truncated")
        .unwrap();

    ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap_err();

    assert!(!directory.path().join(MANIFEST_FILE).exists());
    assert!(directory.path().join(ATTEMPT_JOURNAL_FILE).exists());
    assert!(v28_store_files(directory.path()).is_empty());
}

#[test]
fn attempt_journal_cannot_authorize_deleting_a_v28_file_as_a_legacy_predecessor() {
    let directory = TempDir::new().unwrap();
    let malicious_previous = "metadata-v28-3333333333333333.sqlite3";
    crate::write_new_private_file(&directory.path().join(malicious_previous), b"owned").unwrap();
    let target = ActiveStoreManifest {
        file_name: "metadata-v28-4444444444444444.sqlite3".to_string(),
        schema_version: schema_v28::VERSION,
        store_id_digest: "4".repeat(64),
    };
    let journal = format!(
        "resume-ir.metadata-v28-migration-attempt.v1\n\
         expected_file=none\nexpected_schema=0\nexpected_digest=none\n\
         previous_file={malicious_previous}\nprevious_schema=26\nprevious_digest=none\n\
         target_file={}\ntarget_schema=28\ntarget_digest={}",
        target.file_name, target.store_id_digest,
    );
    crate::write_new_private_file(
        &directory.path().join(ATTEMPT_JOURNAL_FILE),
        journal.as_bytes(),
    )
    .unwrap();

    recover_migration_attempt(directory.path(), Some(&target)).unwrap_err();

    assert!(directory.path().join(malicious_previous).exists());
    assert!(directory.path().join(ATTEMPT_JOURNAL_FILE).exists());
}

#[test]
fn empty_legacy_store_is_retired_after_v28_publication() {
    let directory = TempDir::new().unwrap();
    let legacy = directory.path().join(METADATA_STORE_FILE);
    create_private_empty_file(&legacy).unwrap();

    let active =
        ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap();

    assert!(!legacy.exists());
    assert!(active.exists());
    assert_clean_attempt_namespace(directory.path());
}

#[test]
fn fenced_empty_legacy_store_rejects_a_delayed_schema_migration() {
    let directory = TempDir::new().unwrap();
    let legacy = directory.path().join(METADATA_STORE_FILE);
    create_private_empty_file(&legacy).unwrap();
    let legacy_writer = Connection::open(&legacy).unwrap();
    apply_sqlcipher_key(&legacy_writer, &TEST_KEY).unwrap();

    ensure_active_v28_store(
        directory.path(),
        &TEST_KEY,
        MigrationFailpoint::AfterPredecessorFence,
    )
    .unwrap_err();

    assert!(!directory.path().join(MANIFEST_FILE).exists());
    let fence = read_predecessor_write_fence(&legacy_writer)
        .unwrap()
        .unwrap();
    assert_eq!(fence.source_schema_version, 0);
    assert_eq!(fence.target.schema_version, schema_v28::VERSION);
    let migration_rows_before = count(&legacy_writer, "schema_migrations");
    let delayed_migration = legacy_writer
        .execute(
            "INSERT INTO schema_migrations (version, applied_at_seconds) VALUES (1, 0)",
            [],
        )
        .unwrap_err();
    assert_sqlite_constraint(delayed_migration);
    assert_eq!(
        count(&legacy_writer, "schema_migrations"),
        migration_rows_before
    );
    drop(legacy_writer);

    let active =
        ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap();
    assert!(active.exists());
    assert!(!legacy.exists());
    assert_clean_attempt_namespace(directory.path());
}

#[test]
fn unsupported_legacy_schema_fails_closed_without_pointer_mutation() {
    let directory = TempDir::new().unwrap();
    let legacy = directory.path().join(METADATA_STORE_FILE);
    let connection = Connection::open(&legacy).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at_seconds INTEGER NOT NULL
             );
             INSERT INTO schema_migrations (version, applied_at_seconds) VALUES (25, 0);",
        )
        .unwrap();
    drop(connection);
    crate::restrict_private_file_permissions(&legacy).unwrap();

    ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap_err();

    assert!(legacy.exists());
    assert!(!directory.path().join(MANIFEST_FILE).exists());
    assert!(!directory.path().join(ATTEMPT_JOURNAL_FILE).exists());
    assert!(v28_store_files(directory.path()).is_empty());
}

#[test]
fn generic_metadata_file_cannot_be_published_as_v28() {
    let directory = TempDir::new().unwrap();
    let desired = ActiveStoreManifest {
        file_name: METADATA_STORE_FILE.to_string(),
        schema_version: schema_v28::VERSION,
        store_id_digest: "2".repeat(64),
    };

    publish_new_active_store(directory.path(), &desired, || Ok(())).unwrap_err();

    assert!(!directory.path().join(MANIFEST_FILE).exists());
}

#[test]
fn v27_copy_on_write_preserves_only_source_authority_and_epoch() {
    let directory = TempDir::new().unwrap();
    let source_manifest = seed_v27_manifest(directory.path(), V27SourceState::Repairing);
    let source_path = directory.path().join(&source_manifest.file_name);

    let active =
        ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap();

    assert!(!source_path.exists());
    let manifest = read_manifest(&directory.path().join(MANIFEST_FILE)).unwrap();
    assert_eq!(manifest.schema_version, schema_v28::VERSION);
    assert_eq!(
        active,
        canonical_data_dir(directory.path()).join(&manifest.file_name)
    );
    let target = open_encrypted_connection(&active, &TEST_KEY).unwrap();
    let documents = target
        .query_row(
            "SELECT COUNT(*),
                    SUM(content_hash IS NULL AND text_hash IS NULL),
                    SUM(is_deleted = 0 AND status = 'discovered'),
                    SUM(is_deleted = 1 AND status = 'deleted')
             FROM document",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(documents, (2, 2, 1, 1));
    assert_eq!(count(&target, "authorized_import_root"), 1);
    assert_eq!(
        target
            .query_row(
                "SELECT service_state, generation, visible_epoch, repair_reason
                 FROM search_projection_state WHERE state_key = 'default'",
                [],
                |row| Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?
                )),
            )
            .unwrap(),
        (
            "repairing".to_string(),
            None,
            9,
            Some("migration_rebuild".to_string()),
        )
    );
    for table in [
        "source_revision",
        "resume_version",
        "resume_version_classification",
        "import_task",
        "import_scan_scope",
        "import_processing_contract",
        "import_task_source_disposition",
        "import_task_completion",
        "active_search_projection",
        "search_publication_journal",
    ] {
        assert_eq!(
            count(&target, table),
            0,
            "unexpected retained rows in {table}"
        );
    }
    assert_clean_attempt_namespace(directory.path());
}

#[test]
fn read_only_open_of_v27_is_unsupported_without_mutating_the_manifest() {
    let directory = TempDir::new().unwrap();
    let source_manifest = seed_v27_manifest(directory.path(), V27SourceState::Repairing);
    let key_path = seed_test_key(directory.path());
    let manifest_path = directory.path().join(MANIFEST_FILE);
    let source_path = directory.path().join(&source_manifest.file_name);
    let manifest_bytes = fs::read(&manifest_path).unwrap();
    let source_bytes = fs::read(&source_path).unwrap();
    let key_bytes = fs::read(&key_path).unwrap();
    let manifest_modified = fs::metadata(&manifest_path).unwrap().modified().unwrap();
    let source_modified = fs::metadata(&source_path).unwrap().modified().unwrap();
    let key_modified = fs::metadata(&key_path).unwrap().modified().unwrap();
    let names_before = directory_entry_names(directory.path());

    let error = ReadMetaStore::open_data_dir(directory.path()).unwrap_err();

    assert_eq!(
        error.class(),
        crate::MetaStoreErrorClass::UnsupportedStoreSchema
    );
    assert_eq!(fs::read(&manifest_path).unwrap(), manifest_bytes);
    assert_eq!(fs::read(&source_path).unwrap(), source_bytes);
    assert_eq!(fs::read(&key_path).unwrap(), key_bytes);
    assert_eq!(
        fs::metadata(&manifest_path).unwrap().modified().unwrap(),
        manifest_modified
    );
    assert_eq!(
        fs::metadata(&source_path).unwrap().modified().unwrap(),
        source_modified
    );
    assert_eq!(
        fs::metadata(&key_path).unwrap().modified().unwrap(),
        key_modified
    );
    assert_eq!(directory_entry_names(directory.path()), names_before);
    assert!(!directory.path().join(ATTEMPT_JOURNAL_FILE).exists());
    assert!(v28_store_files(directory.path()).is_empty());
}

#[test]
fn read_only_open_and_queries_leave_the_published_current_tree_byte_for_byte_unchanged() {
    let directory = TempDir::new().unwrap();
    let owner = match DataDirectoryOwnerLease::try_acquire(directory.path()).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("test data directory was contended"),
    };
    let store = owner.open_store().unwrap();
    assert_eq!(store.schema_version().unwrap(), crate::schema_v29::VERSION);
    drop(store);
    drop(owner);
    let before = directory_tree_snapshot(directory.path());

    let reader = ReadMetaStore::open_data_dir(directory.path()).unwrap();
    assert_eq!(reader.schema_version().unwrap(), crate::schema_v29::VERSION);
    let state = reader.search_projection_state().unwrap();
    assert_eq!(
        state.service_state,
        crate::SearchProjectionServiceState::Repairing
    );
    assert!(reader.active_authorized_import_roots().unwrap().is_empty());
    drop(reader);

    let after = directory_tree_snapshot(directory.path());
    assert_eq!(
        after
            .iter()
            .map(|entry| &entry.relative_path)
            .collect::<Vec<_>>(),
        before
            .iter()
            .map(|entry| &entry.relative_path)
            .collect::<Vec<_>>()
    );
    for (after, before) in after.iter().zip(&before) {
        assert_eq!(
            after.kind, before.kind,
            "kind changed: {:?}",
            after.relative_path
        );
        assert_eq!(
            after.modified, before.modified,
            "mtime changed: {:?}",
            after.relative_path
        );
        assert_eq!(
            after.bytes.as_deref(),
            before.bytes.as_deref(),
            "bytes changed: {:?}",
            after.relative_path
        );
    }
}

#[test]
fn rollback_snapshot_is_all_old_until_release_then_a_new_reader_sees_the_commit() {
    let directory = TempDir::new().unwrap();
    let owner = match DataDirectoryOwnerLease::try_acquire(directory.path()).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("test data directory was contended"),
    };
    let store = owner.open_store().unwrap();
    let contract = ImportProcessingContract::new(
        "rollback-primary-v28",
        "rollback-ocr-v28",
        "rollback-derived-v28",
        CLASSIFIER_EPOCH,
    )
    .unwrap();
    store
        .activate_migration_rebuild_contract(&contract, timestamp(1))
        .unwrap();
    let barrier = store
        .acquire_migration_rebuild_barrier_token(contract.id())
        .unwrap()
        .unwrap();
    let mut initial_session = store.wait_for_search_publication_session().unwrap();
    let _attempt = match initial_session
        .acquire_migration_rebuild_publication_attempt(&barrier, timestamp(2))
        .unwrap()
    {
        crate::MigrationRebuildPublicationAttemptAcquire::Started(attempt) => attempt,
        other => panic!("expected migration attempt, got {other:?}"),
    };
    publish_empty_generation(
        &initial_session,
        "rollback-generation-1",
        None,
        0,
        Some(&barrier),
        timestamp(2),
    );
    drop(initial_session);

    let reader = ReadMetaStore::open_data_dir(directory.path()).unwrap();
    let writer_store = store.open_sibling().unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let writer = thread::spawn(move || {
        let writer_session = writer_store.wait_for_search_publication_session().unwrap();
        started_tx.send(()).unwrap();
        publish_empty_generation(
            &writer_session,
            "rollback-generation-2",
            Some("rollback-generation-1"),
            1,
            None,
            timestamp(3),
        );
        finished_tx.send(()).unwrap();
    });

    reader
        .with_search_metadata_snapshot(|snapshot| {
            assert_eq!(snapshot.head().generation, "rollback-generation-1");
            assert_eq!(snapshot.head().visible_epoch, 1);
            started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            assert!(finished_rx
                .recv_timeout(Duration::from_millis(150))
                .is_err());
            assert_eq!(snapshot.head().generation, "rollback-generation-1");
            assert_eq!(snapshot.head().visible_epoch, 1);
            Ok::<_, ()>(())
        })
        .unwrap();

    finished_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    writer.join().unwrap();
    let new_reader = ReadMetaStore::open_data_dir(directory.path()).unwrap();
    new_reader
        .with_search_metadata_snapshot(|snapshot| {
            assert_eq!(snapshot.head().generation, "rollback-generation-2");
            assert_eq!(snapshot.head().visible_epoch, 2);
            Ok::<_, ()>(())
        })
        .unwrap();
}

#[cfg(unix)]
#[test]
fn read_only_v28_open_is_unsupported_without_repairing_unsafe_key_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let directory = TempDir::new().unwrap();
    ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap();
    let key_path = seed_test_key(directory.path());
    fs::set_permissions(&key_path, fs::Permissions::from_mode(0o644)).unwrap();
    let key_bytes = fs::read(&key_path).unwrap();
    let key_modified = fs::metadata(&key_path).unwrap().modified().unwrap();

    let error = ReadMetaStore::open_data_dir(directory.path()).unwrap_err();

    assert_eq!(
        error.class(),
        crate::MetaStoreErrorClass::UnsupportedStoreSchema
    );
    assert_eq!(fs::read(&key_path).unwrap(), key_bytes);
    assert_eq!(
        fs::metadata(&key_path).unwrap().modified().unwrap(),
        key_modified
    );
    assert_eq!(
        fs::metadata(&key_path).unwrap().permissions().mode() & 0o777,
        0o644
    );
}

#[test]
fn live_legacy_task_owner_prevents_any_cow_attempt_until_release() {
    let directory = TempDir::new().unwrap();
    let source_manifest = seed_v27_manifest(directory.path(), V27SourceState::Repairing);
    let manifest_path = directory.path().join(MANIFEST_FILE);
    let source_path = directory.path().join(&source_manifest.file_name);
    let manifest_bytes = fs::read(&manifest_path).unwrap();
    let source_bytes = fs::read(&source_path).unwrap();
    let task_id = ImportTaskId::from_non_secret_parts(&["synthetic-v27-task"]);
    let legacy_owner = crate::ImportTaskOwnerLock::acquire(directory.path(), &task_id).unwrap();

    let error =
        ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap_err();

    assert_eq!(
        error.class(),
        crate::MetaStoreErrorClass::MigrationOwnershipRequired
    );
    assert_eq!(fs::read(&manifest_path).unwrap(), manifest_bytes);
    assert_eq!(fs::read(&source_path).unwrap(), source_bytes);
    assert!(!directory.path().join(ATTEMPT_JOURNAL_FILE).exists());
    assert!(v28_store_files(directory.path()).is_empty());

    drop(legacy_owner);
    let active =
        ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap();
    let published = read_manifest(&manifest_path).unwrap();
    assert_eq!(published.schema_version, schema_v28::VERSION);
    assert_eq!(
        active,
        canonical_data_dir(directory.path()).join(published.file_name)
    );
    assert!(!source_path.exists());
    assert_clean_attempt_namespace(directory.path());
}

#[test]
fn live_legacy_publication_namespace_prevents_any_cow_attempt_until_release() {
    let directory = TempDir::new().unwrap();
    let source_manifest = seed_v27_manifest(directory.path(), V27SourceState::Repairing);
    let manifest_path = directory.path().join(MANIFEST_FILE);
    let source_path = directory.path().join(&source_manifest.file_name);
    let manifest_bytes = fs::read(&manifest_path).unwrap();
    let source_bytes = fs::read(&source_path).unwrap();
    let lock_path = directory.path().join("search-publication.lock");
    crate::write_new_private_file(&lock_path, b"").unwrap();
    let legacy_publication = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    fs4::fs_std::FileExt::lock_exclusive(&legacy_publication).unwrap();

    let error =
        ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap_err();

    assert_eq!(
        error.class(),
        crate::MetaStoreErrorClass::MigrationOwnershipRequired
    );
    assert_eq!(fs::read(&manifest_path).unwrap(), manifest_bytes);
    assert_eq!(fs::read(&source_path).unwrap(), source_bytes);
    assert!(!directory.path().join(ATTEMPT_JOURNAL_FILE).exists());
    assert!(v28_store_files(directory.path()).is_empty());

    drop(legacy_publication);
    let active =
        ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap();
    assert!(active.exists());
    assert!(!source_path.exists());
    assert_clean_attempt_namespace(directory.path());
}

#[test]
fn post_fence_crash_recovery_rejects_an_already_open_tail_writer() {
    let directory = TempDir::new().unwrap();
    let source_manifest = seed_v27_manifest(directory.path(), V27SourceState::Repairing);
    let manifest_path = directory.path().join(MANIFEST_FILE);
    let source_path = directory.path().join(&source_manifest.file_name);
    let legacy_writer = open_encrypted_connection(&source_path, &TEST_KEY).unwrap();

    ensure_active_v28_store(
        directory.path(),
        &TEST_KEY,
        MigrationFailpoint::AfterPredecessorFence,
    )
    .unwrap_err();

    assert_eq!(read_manifest(&manifest_path).unwrap(), source_manifest);
    assert!(source_path.exists());
    assert!(directory.path().join(ATTEMPT_JOURNAL_FILE).exists());
    let fence = read_predecessor_write_fence(&legacy_writer)
        .unwrap()
        .unwrap();
    assert_eq!(fence.source_schema_version, schema_v27::VERSION);
    assert_eq!(fence.target.schema_version, schema_v28::VERSION);
    let task_rows_before = count(&legacy_writer, "import_task");
    let tail_write = legacy_writer
        .execute(
            "INSERT INTO import_task (id) VALUES ('post-enumeration-tail')",
            [],
        )
        .unwrap_err();
    assert_sqlite_constraint(tail_write);
    assert_eq!(count(&legacy_writer, "import_task"), task_rows_before);
    drop(legacy_writer);

    let recovered =
        ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap();
    let committed = read_manifest(&manifest_path).unwrap();
    assert_eq!(committed.schema_version, schema_v28::VERSION);
    assert_eq!(
        recovered,
        canonical_data_dir(directory.path()).join(committed.file_name)
    );
    assert!(!source_path.exists());
    assert_clean_attempt_namespace(directory.path());
    assert_eq!(v28_store_files(directory.path()).len(), 1);
}

#[test]
fn post_manifest_recovery_fences_a_legacy_writer_before_retiring_predecessor() {
    let directory = TempDir::new().unwrap();
    let source_manifest = seed_v27_manifest(directory.path(), V27SourceState::Repairing);
    let source_path = directory.path().join(&source_manifest.file_name);
    ensure_active_v28_store(
        directory.path(),
        &TEST_KEY,
        MigrationFailpoint::AfterManifest,
    )
    .unwrap_err();
    let committed = read_manifest(&directory.path().join(MANIFEST_FILE)).unwrap();
    assert_eq!(committed.schema_version, schema_v28::VERSION);
    assert!(source_path.exists());
    assert!(directory.path().join(ATTEMPT_JOURNAL_FILE).exists());

    let task_id = ImportTaskId::from_non_secret_parts(&["synthetic-v27-task"]);
    let legacy_owner = crate::ImportTaskOwnerLock::acquire(directory.path(), &task_id).unwrap();
    let error =
        ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap_err();
    assert_eq!(
        error.class(),
        crate::MetaStoreErrorClass::MigrationOwnershipRequired
    );
    assert!(source_path.exists());
    assert!(directory.path().join(ATTEMPT_JOURNAL_FILE).exists());

    drop(legacy_owner);
    let reopened =
        ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap();
    assert_eq!(
        reopened,
        canonical_data_dir(directory.path()).join(committed.file_name)
    );
    assert!(!source_path.exists());
    assert_clean_attempt_namespace(directory.path());
}

#[test]
fn v27_ready_and_repairing_sources_both_rebuild_as_repairing() {
    for source_state in [V27SourceState::Ready, V27SourceState::Repairing] {
        let directory = TempDir::new().unwrap();
        seed_v27_manifest(directory.path(), source_state);

        let active =
            ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap();
        assert_repairing_epoch(&active, 9);
    }
}

#[test]
fn v27_prepublication_crashes_keep_the_exact_old_store_until_reopen() {
    for failpoint in [
        MigrationFailpoint::AfterAttemptWriteCrash,
        MigrationFailpoint::AfterTargetCreateCrash,
        MigrationFailpoint::AfterSourceCopyCrash,
        MigrationFailpoint::AfterTargetValidationCrash,
    ] {
        let directory = TempDir::new().unwrap();
        let source_manifest = seed_v27_manifest(directory.path(), V27SourceState::Repairing);
        let manifest_path = directory.path().join(MANIFEST_FILE);
        let source_path = directory.path().join(&source_manifest.file_name);
        let manifest_bytes = fs::read(&manifest_path).unwrap();
        let source_bytes = fs::read(&source_path).unwrap();

        ensure_active_v28_store(directory.path(), &TEST_KEY, failpoint).unwrap_err();

        assert_eq!(fs::read(&manifest_path).unwrap(), manifest_bytes);
        assert_eq!(fs::read(&source_path).unwrap(), source_bytes);
        assert_eq!(read_manifest(&manifest_path).unwrap(), source_manifest);
        validate_active_v27_store(&source_path, &TEST_KEY, V27_DIGEST).unwrap();

        let reopened =
            ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap();
        assert!(!source_path.exists());
        assert!(reopened.exists());
        assert_clean_attempt_namespace(directory.path());
        assert_eq!(v28_store_files(directory.path()).len(), 1);
    }
}

#[test]
fn v27_post_manifest_crash_reopen_retires_the_recorded_predecessor() {
    for failpoint in [
        MigrationFailpoint::AfterManifestRename,
        MigrationFailpoint::AfterManifest,
    ] {
        let directory = TempDir::new().unwrap();
        let source_manifest = seed_v27_manifest(directory.path(), V27SourceState::Repairing);
        let source_path = directory.path().join(source_manifest.file_name);

        ensure_active_v28_store(directory.path(), &TEST_KEY, failpoint).unwrap_err();
        let committed = read_manifest(&directory.path().join(MANIFEST_FILE)).unwrap();
        assert_eq!(committed.schema_version, schema_v28::VERSION);
        assert!(source_path.exists());

        let reopened =
            ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap();
        assert_eq!(
            reopened,
            canonical_data_dir(directory.path()).join(committed.file_name)
        );
        assert!(!source_path.exists());
        assert_clean_attempt_namespace(directory.path());
        assert_eq!(v28_store_files(directory.path()).len(), 1);
    }
}

#[test]
fn encrypted_v26_copy_on_write_preserves_source_authority_and_epoch_only() {
    let directory = TempDir::new().unwrap();
    let legacy = seed_encrypted_v26_store(directory.path());

    let active =
        ensure_active_v28_store(directory.path(), &TEST_KEY, MigrationFailpoint::None).unwrap();

    assert!(!legacy.exists());
    let target = open_encrypted_connection(&active, &TEST_KEY).unwrap();
    assert_eq!(count(&target, "document"), 1);
    assert_eq!(count(&target, "authorized_import_root"), 1);
    assert_eq!(count(&target, "import_task"), 0);
    assert_eq!(count(&target, "import_scan_scope"), 0);
    assert_eq!(
        target
            .query_row(
                "SELECT status, content_hash, text_hash FROM document
                 WHERE id = 'synthetic-v26-document'",
                [],
                |row| Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?
                )),
            )
            .unwrap(),
        ("discovered".to_string(), None, None),
    );
    assert_repairing_epoch(&active, 7);
    assert_clean_attempt_namespace(directory.path());
}

fn seed_v27_manifest(
    data_dir: &std::path::Path,
    source_state: V27SourceState,
) -> ActiveStoreManifest {
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("test data directory was contended"),
    };
    let file_name = "metadata-v27-1111111111111111.sqlite3";
    let path = data_dir.join(file_name);
    create_private_empty_file(&path).unwrap();
    let connection = Connection::open(&path).unwrap();
    apply_sqlcipher_key(&connection, &TEST_KEY).unwrap();
    let store = OwnedMetaStore::from_owned_connection(
        connection,
        MetadataEncryptionState::SqlCipher,
        owner.shared_guard(),
    )
    .unwrap();
    store.migrate_staging_store_to_v27(V27_DIGEST).unwrap();
    {
        let connection = store.connection.borrow();
        let content_hash = format!("sha256:{}", "a".repeat(64));
        let text_hash = format!("sha256:{}", "b".repeat(64));
        connection
            .execute(
                "INSERT INTO document (
                    id, source_uri, normalized_path, file_name, extension, byte_size,
                    mtime_seconds, content_hash, text_hash, is_deleted,
                    created_at_seconds, updated_at_seconds, status
                 ) VALUES (?1, ?2, ?3, ?4, 'txt', 128, 10, ?5, ?6, 0, 10, 11, 'searchable')",
                params![
                    "synthetic-v27-document",
                    "synthetic://v27/document",
                    "synthetic/v27/document.txt",
                    "document.txt",
                    content_hash,
                    text_hash,
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO document (
                    id, source_uri, normalized_path, file_name, extension, byte_size,
                    mtime_seconds, content_hash, text_hash, is_deleted,
                    created_at_seconds, updated_at_seconds, status
                 ) VALUES (?1, ?2, ?3, ?4, 'txt', 0, 10, NULL, NULL, 1, 10, 11, 'deleted')",
                params![
                    "synthetic-v27-deleted",
                    "synthetic://v27/deleted",
                    "synthetic/v27/deleted.txt",
                    "deleted.txt",
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO source_revision (id, document_id, content_hash, byte_size)
                 VALUES ('synthetic-v27-revision', 'synthetic-v27-document', ?1, 128)",
                [format!("sha256:{}", "a".repeat(64))],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO resume_version (
                    id, document_id, source_revision_id, normalized_text_hash,
                    parse_version, schema_version, language_set_json, page_count,
                    raw_text, clean_text, quality_score
                 ) VALUES (
                    'synthetic-v27-version', 'synthetic-v27-document',
                    'synthetic-v27-revision', ?1, 'parser_v27', 'schema_v27',
                    '[\"en\"]', 1, 'synthetic text', 'synthetic text', 1.0
                 )",
                [format!("sha256:{}", "b".repeat(64))],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO resume_version_classification (
                    resume_version_id, status, classifier_epoch,
                    classified_at_seconds, review_disposition
                 ) VALUES (
                    'synthetic-v27-version', 'resume_candidate',
                    'synthetic_epoch', 12, 'not_required'
                 )",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO authorized_import_root (
                    canonical_root_path, requested_root_path, root_kind, root_preset,
                    scan_profile, scan_budget_kind, scan_budget_limit, paused,
                    updated_at_seconds
                 ) VALUES (
                    'synthetic/import/root', 'synthetic/import/root', 'explicit', NULL,
                    'explicit', 'files', 100, 0, 13
                 )",
                [],
            )
            .unwrap();
        let legacy_task_id = ImportTaskId::from_non_secret_parts(&["synthetic-v27-task"]);
        connection
            .execute(
                "INSERT INTO import_task (
                    id, root_path, status, queued_at_seconds, started_at_seconds,
                    finished_at_seconds, updated_at_seconds
                 ) VALUES (
                    ?1, 'synthetic/import/root', 'queued', 13, NULL, NULL, 13
                 )",
                [legacy_task_id.as_str()],
            )
            .unwrap();
        set_v27_projection_fixture(&connection, source_state, 9);
    }
    drop(store);
    sync_validated_store(&path).unwrap();
    let manifest = ActiveStoreManifest {
        file_name: file_name.to_string(),
        schema_version: schema_v27::VERSION,
        store_id_digest: V27_DIGEST.to_string(),
    };
    publish_new_active_store(data_dir, &manifest, || Ok(())).unwrap();
    manifest
}

fn set_v27_projection_fixture(
    connection: &Connection,
    source_state: V27SourceState,
    visible_epoch: i64,
) {
    let trigger_names = [
        "ready_projection_head_matches_journal",
        "search_projection_head_change_requires_commit_guard",
    ];
    let trigger_sql = trigger_names
        .iter()
        .map(|name| {
            connection
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
                    [name],
                    |row| row.get::<_, String>(0),
                )
                .unwrap()
        })
        .collect::<Vec<_>>();
    for name in trigger_names {
        connection
            .execute_batch(&format!("DROP TRIGGER {name};"))
            .unwrap();
    }
    match source_state {
        V27SourceState::Repairing => {
            connection
                .execute(
                    "UPDATE search_projection_state
                     SET service_state = 'repairing', generation = NULL,
                         visible_epoch = ?1, repair_reason = 'migration_rebuild'
                     WHERE state_key = 'default'",
                    [visible_epoch],
                )
                .unwrap();
        }
        V27SourceState::Ready => {
            connection
                .execute(
                    "INSERT INTO search_publication_journal (
                        generation, base_generation, expected_visible_epoch,
                        classifier_epoch, projection_digest, state,
                        created_at_seconds, updated_at_seconds
                     ) VALUES (
                        'synthetic-ready-generation', NULL, ?1, 'synthetic_epoch',
                        ?2, 'abandoned', 14, 14
                     )",
                    params![visible_epoch - 1, format!("sha256:{}", "c".repeat(64)),],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE search_projection_state
                     SET service_state = 'ready', generation = 'synthetic-ready-generation',
                         visible_epoch = ?1, repair_reason = NULL
                     WHERE state_key = 'default'",
                    [visible_epoch],
                )
                .unwrap();
        }
    }
    for sql in trigger_sql {
        connection.execute_batch(&sql).unwrap();
    }
}

fn seed_encrypted_v26_store(data_dir: &std::path::Path) -> std::path::PathBuf {
    let path = data_dir.join(METADATA_STORE_FILE);
    create_private_empty_file(&path).unwrap();
    let mut connection = Connection::open(&path).unwrap();
    apply_sqlcipher_key(&connection, &TEST_KEY).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at_seconds INTEGER NOT NULL
             );",
        )
        .unwrap();
    for (version, schema) in crate::legacy_migrations() {
        let transaction = connection.transaction().unwrap();
        transaction.execute_batch(schema).unwrap();
        transaction
            .execute(
                "INSERT INTO schema_migrations (version, applied_at_seconds)
                 VALUES (?1, 0)",
                [i64::from(version)],
            )
            .unwrap();
        transaction.commit().unwrap();
    }
    connection
        .execute(
            "INSERT INTO document (
                id, source_uri, normalized_path, file_name, extension, byte_size,
                mtime_seconds, content_hash, text_hash, is_deleted,
                created_at_seconds, updated_at_seconds, status
             ) VALUES (
                'synthetic-v26-document', 'synthetic://v26/document',
                'synthetic/v26/document.txt', 'document.txt', 'txt', 64, 10,
                ?1, ?2, 0, 10, 11, 'searchable'
             )",
            params![
                format!("sha256:{}", "d".repeat(64)),
                format!("sha256:{}", "e".repeat(64)),
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO import_task (
                id, root_path, status, queued_at_seconds, started_at_seconds,
                finished_at_seconds, updated_at_seconds
             ) VALUES (
                'synthetic-v26-task', 'synthetic/import/v26', 'queued',
                12, NULL, NULL, 12
             )",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO import_scan_scope (
                import_task_id, root_kind, root_preset, scan_profile,
                requested_root_path, canonical_root_path, files_discovered,
                ignored_entries, scan_errors, searchable_documents,
                ocr_required_documents, ocr_jobs_queued, failed_documents,
                deleted_documents, scan_budget_kind, scan_budget_limit,
                scan_budget_observed, scan_budget_exhausted, updated_at_seconds
             ) VALUES (
                'synthetic-v26-task', 'explicit', NULL, 'explicit',
                'synthetic/import/v26', 'synthetic/import/v26', 1,
                0, 0, 1, 0, 0, 0, 0, 'files', 100, 1, 0, 12
             )",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO index_state (
                state_key, manifest_version, snapshot_token, status,
                updated_at_seconds, visible_epoch, manifest_document_count
             ) VALUES ('default', 'v26', NULL, 'stale', 12, 7, 1)",
            [],
        )
        .unwrap();
    drop(connection);
    sync_validated_store(&path).unwrap();
    path
}

fn assert_repairing_epoch(path: &std::path::Path, visible_epoch: i64) {
    let connection = open_encrypted_connection(path, &TEST_KEY).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT service_state, generation, visible_epoch, repair_reason
                 FROM search_projection_state WHERE state_key = 'default'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .unwrap(),
        (
            "repairing".to_string(),
            None,
            visible_epoch,
            Some("migration_rebuild".to_string()),
        )
    );
}

fn count(connection: &Connection, table: &str) -> i64 {
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn assert_sqlite_constraint(error: SqliteError) {
    let SqliteError::SqliteFailure(failure, _) = error else {
        panic!("expected SQLite constraint rejection, got {error:?}");
    };
    assert_eq!(failure.code, ErrorCode::ConstraintViolation);
}

fn v28_store_files(data_dir: &std::path::Path) -> Vec<String> {
    let mut files = fs::read_dir(data_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with("metadata-v28-") && name.ends_with(".sqlite3"))
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn assert_clean_attempt_namespace(data_dir: &std::path::Path) {
    let names = fs::read_dir(data_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    assert!(!names.iter().any(|name| name == ATTEMPT_JOURNAL_FILE));
    assert!(!names
        .iter()
        .any(|name| name.starts_with(ATTEMPT_TEMP_PREFIX)));
}

fn assert_sqlite_artifacts_absent(path: &std::path::Path) {
    assert!(!path.exists());
    for suffix in ["-journal", "-wal", "-shm"] {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        assert!(!std::path::PathBuf::from(sidecar).exists());
    }
}

fn seed_test_key(data_dir: &std::path::Path) -> std::path::PathBuf {
    let path = crate::metadata_encryption_key_path(data_dir);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    if !path.exists() {
        crate::write_new_private_file(&path, crate::encode_hex(&TEST_KEY).as_bytes()).unwrap();
    }
    path
}

fn directory_entry_names(data_dir: &std::path::Path) -> Vec<String> {
    let mut names = fs::read_dir(data_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

#[derive(Debug, PartialEq, Eq)]
struct PathSnapshot {
    relative_path: std::path::PathBuf,
    kind: &'static str,
    modified: std::time::SystemTime,
    bytes: Option<Vec<u8>>,
}

fn directory_tree_snapshot(data_dir: &std::path::Path) -> Vec<PathSnapshot> {
    fn visit(
        root: &std::path::Path,
        directory: &std::path::Path,
        snapshots: &mut Vec<PathSnapshot>,
    ) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            let kind = if metadata.is_dir() {
                "directory"
            } else if metadata.is_file() {
                "file"
            } else {
                "other"
            };
            snapshots.push(PathSnapshot {
                relative_path: path.strip_prefix(root).unwrap().to_path_buf(),
                kind,
                modified: metadata.modified().unwrap(),
                bytes: metadata.is_file().then(|| fs::read(&path).unwrap()),
            });
            if metadata.is_dir() {
                visit(root, &path, snapshots);
            }
        }
    }

    let mut snapshots = Vec::new();
    visit(data_dir, data_dir, &mut snapshots);
    snapshots.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    snapshots
}

fn publish_empty_generation(
    session: &SearchPublicationSession,
    generation: &str,
    base_generation: Option<&str>,
    expected_visible_epoch: u64,
    migration_barrier: Option<&crate::MigrationRebuildBarrierToken>,
    now: UnixTimestamp,
) {
    let projection_digest = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
    assert_eq!(
        session
            .begin_search_publication(&SearchPublicationDraft {
                generation: generation.to_string(),
                base_generation: base_generation.map(str::to_string),
                expected_visible_epoch,
                classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                projection_digest: projection_digest.clone(),
                now,
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    let fulltext = FullTextSnapshotDescriptor::new(
        generation.to_string(),
        0,
        projection_digest.clone(),
        ContentDigest::from_bytes(format!("fulltext:{generation}").as_bytes()),
    );
    let vector = VectorSnapshotDescriptor::disabled(
        generation.to_string(),
        0,
        projection_digest,
        SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap(),
        ContentDigest::from_bytes(format!("vector:{generation}").as_bytes()),
    );
    session
        .validate_search_publication(&SearchPublicationValidation {
            generation,
            fulltext: &fulltext,
            vector: &vector,
            now,
        })
        .unwrap();
    let commit = SearchPublicationCommit {
        generation,
        terminal_documents: &[],
        projections: &[],
        projected_documents: &[],
        vector_coverage: &[],
        now,
    };
    let outcome = if let Some(barrier) = migration_barrier {
        session
            .commit_migration_rebuild_search_publication(&commit, barrier)
            .unwrap()
    } else {
        session.commit_search_publication(&commit).unwrap()
    };
    assert_eq!(outcome, SearchPublicationOutcome::Applied);
}

fn timestamp(seconds: i64) -> UnixTimestamp {
    UnixTimestamp::from_unix_seconds(seconds)
}

fn canonical_data_dir(data_dir: &std::path::Path) -> std::path::PathBuf {
    fs::canonicalize(data_dir).unwrap()
}
