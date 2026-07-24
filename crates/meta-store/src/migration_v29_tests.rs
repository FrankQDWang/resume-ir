use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use tempfile::{tempdir, TempDir};

use super::descriptor_validation::records::{
    descriptor_contract, CURRENT_FULLTEXT_MANIFEST, CURRENT_VECTOR_INDEX, CURRENT_VECTOR_MANIFEST,
    LEGACY_FULLTEXT_INDEX, LEGACY_FULLTEXT_MANIFEST,
};
use super::*;
use crate::{
    active_store_manifest::read_manifest, ActiveSearchProjection, ClassificationStatus,
    ContentDigest, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, Document, DocumentId,
    DocumentStatus, EnabledVectorSnapshotDescriptor, FileExtension, FullTextSnapshotDescriptor,
    IdentityInsertOutcome, ImportProcessingContract, MetaStoreErrorClass,
    MigrationRebuildPublicationAttemptAcquire, OwnedMetaStore, ProjectedDocumentSnapshot,
    ReasonCode, ResumeVersion, ResumeVersionClassification, ResumeVersionId, ReviewDisposition,
    SearchProjectionDigest, SearchPublicationCommit, SearchPublicationDraft,
    SearchPublicationOutcome, SearchPublicationValidation, SearchSelection,
    SearchSelectionResolution, SourceRevision, TerminalDocumentUpdate, UnixTimestamp,
    VectorSnapshotDescriptor, CLASSIFIER_EPOCH,
};

#[test]
fn descriptor_contract_rejects_mixed_snapshot_versions() {
    assert!(descriptor_contract(
        Some(LEGACY_FULLTEXT_MANIFEST),
        Some(LEGACY_FULLTEXT_INDEX),
        Some(CURRENT_VECTOR_MANIFEST),
        Some(CURRENT_VECTOR_INDEX),
    )
    .is_err());
    assert!(descriptor_contract(
        Some(CURRENT_FULLTEXT_MANIFEST),
        Some(LEGACY_FULLTEXT_INDEX),
        Some(CURRENT_VECTOR_MANIFEST),
        Some(CURRENT_VECTOR_INDEX),
    )
    .is_err());
}

#[test]
fn fresh_owner_directory_initializes_and_reopens_exact_current_v29() {
    let fixture = OwnedDirectory::new();

    let store = fixture.owner.open_store().unwrap();
    assert_eq!(store.schema_version().unwrap(), schema_v29::VERSION);
    let manifest = read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap();
    assert_eq!(manifest.schema_version, schema_v29::VERSION);
    assert_eq!(
        store_identity(&store.connection.borrow()).unwrap(),
        manifest.store_id_digest
    );
    assert_eq!(
        store
            .connection
            .borrow()
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get::<_, u32>(0)
            })
            .unwrap(),
        schema_v29::VERSION
    );
    drop(store);

    let reopened = fixture.owner.open_store().unwrap();
    assert_eq!(reopened.schema_version().unwrap(), schema_v29::VERSION);

    let published_tree = snapshot_tree(fixture.data_dir());
    assert!(!published_tree.keys().any(|path| {
        let name = path.to_string_lossy();
        name == crate::migration_v27::MIGRATION_LOCK_FILE
            || name == LEGACY_CLEANUP_RECEIPT_FILE
            || name == V28_MIGRATION_ATTEMPT_FILE
            || name.starts_with("metadata-v27-")
            || name.starts_with("metadata-v28-")
    }));
}

#[test]
fn current_v29_open_preserves_key_manifest_ciphertext_and_business_data() {
    let fixture = OwnedDirectory::new();
    let projection = {
        let store = fixture.owner.open_store().unwrap();
        seed_published_v29_projection(&store)
    };
    let before_summary = {
        let store = fixture.owner.open_store().unwrap();
        preserved_v29_summary(&store, &projection)
    };
    assert_eq!(before_summary.generation, "v29-preservation-generation");
    assert_eq!(before_summary.visible_epoch, 1);
    assert!(before_summary.selection_is_current);
    assert_eq!(
        before_summary.fulltext_artifact_digest,
        ContentDigest::from_bytes(b"preserved-v29-fulltext-artifact")
    );
    assert_eq!(
        before_summary.vector_artifact_digest,
        ContentDigest::from_bytes(b"preserved-v29-vector-artifact")
    );
    let before = snapshot_tree(fixture.data_dir());

    {
        let reopened = fixture.owner.open_store().unwrap();
        assert_eq!(
            preserved_v29_summary(&reopened, &projection),
            before_summary
        );
    }

    assert_eq!(snapshot_tree(fixture.data_dir()), before);
}

#[test]
fn current_v29_open_accepts_retained_current_ready_history() {
    let fixture = OwnedDirectory::new();
    let store = fixture.owner.open_store().unwrap();
    let contract = ImportProcessingContract::new(
        "v29-history-parser",
        "v29-history-ocr",
        "v29-history-schema",
        CLASSIFIER_EPOCH,
    )
    .unwrap();
    store
        .activate_migration_rebuild_contract(&contract, UnixTimestamp::from_unix_seconds(10))
        .unwrap();
    let barrier = store
        .acquire_migration_rebuild_barrier_token(contract.id())
        .unwrap()
        .unwrap();
    let mut session = store.wait_for_search_publication_session().unwrap();
    assert!(matches!(
        session
            .acquire_migration_rebuild_publication_attempt(
                &barrier,
                UnixTimestamp::from_unix_seconds(11),
            )
            .unwrap(),
        MigrationRebuildPublicationAttemptAcquire::Started(_)
            | MigrationRebuildPublicationAttemptAcquire::InProgress
    ));
    publish_empty_v29_generation(
        &session,
        "v29-history-first",
        None,
        0,
        Some(&barrier),
        UnixTimestamp::from_unix_seconds(12),
    );
    publish_empty_v29_generation(
        &session,
        "v29-history-second",
        Some("v29-history-first"),
        1,
        None,
        UnixTimestamp::from_unix_seconds(13),
    );
    drop(session);
    drop(store);
    let before = snapshot_tree(fixture.data_dir());

    let reopened = fixture.owner.open_store().unwrap();
    let head = reopened.search_projection_state().unwrap();
    assert_eq!(head.generation.as_deref(), Some("v29-history-second"));
    assert_eq!(head.visible_epoch, 2);
    assert_eq!(
        reopened.recent_ready_search_publications(8).unwrap().len(),
        2
    );
    drop(reopened);
    drop(crate::ReadMetaStore::open_data_dir(fixture.data_dir()).unwrap());

    assert_eq!(snapshot_tree(fixture.data_dir()), before);
}

#[test]
fn current_v29_missing_key_fails_without_repair_or_other_writes() {
    let fixture = OwnedDirectory::new();
    drop(fixture.owner.open_store().unwrap());
    fs::remove_file(crate::metadata_encryption_key_path(fixture.data_dir())).unwrap();
    let before = snapshot_tree(fixture.data_dir());

    assert!(fixture.owner.open_store().is_err());

    assert_eq!(snapshot_tree(fixture.data_dir()), before);
    assert!(!crate::metadata_encryption_key_path(fixture.data_dir()).exists());
}

#[test]
fn current_v29_manifest_identity_mismatch_is_byte_stable() {
    let fixture = OwnedDirectory::new();
    drop(fixture.owner.open_store().unwrap());
    let manifest_path = fixture.data_dir().join(MANIFEST_FILE);
    let manifest = read_manifest(&manifest_path).unwrap();
    fs::write(
        &manifest_path,
        format!(
            "resume-ir.metadata-active.v1\nfile={}\nschema={}\ndigest={}\n",
            manifest.file_name,
            schema_v29::VERSION,
            "b".repeat(64)
        ),
    )
    .unwrap();
    let before = snapshot_tree(fixture.data_dir());

    assert!(fixture.owner.open_store().is_err());

    assert_eq!(snapshot_tree(fixture.data_dir()), before);
}

#[test]
fn current_v29_ciphertext_integrity_failure_is_byte_stable() {
    let fixture = OwnedDirectory::new();
    drop(fixture.owner.open_store().unwrap());
    let store_path = active_store_path(fixture.data_dir()).unwrap();
    let mut ciphertext = fs::read(&store_path).unwrap();
    ciphertext[0] ^= 0xff;
    fs::write(&store_path, ciphertext).unwrap();
    let before = snapshot_tree(fixture.data_dir());

    assert!(fixture.owner.open_store().is_err());

    assert_eq!(snapshot_tree(fixture.data_dir()), before);
}

#[test]
fn current_v29_publication_fingerprint_corruption_is_rejected_byte_stably() {
    let fixture = OwnedDirectory::new();
    let store = fixture.owner.open_store().unwrap();
    seed_published_v29_projection(&store);
    {
        let connection = store.connection.borrow();
        connection
            .execute_batch(schema_v29::DROP_LEGACY_ISOLATION_TRIGGERS)
            .unwrap();
        assert_eq!(
            connection
                .execute(
                    "UPDATE search_publication_journal
                     SET publication_fingerprint = ?1
                     WHERE generation = 'v29-preservation-generation'",
                    [format!("sha256:{}", "a".repeat(64))],
                )
                .unwrap(),
            1
        );
        connection
            .execute_batch(schema_v29::RESTORE_LEGACY_ISOLATION_TRIGGERS)
            .unwrap();
    }
    drop(store);
    let store_path = active_store_path(fixture.data_dir()).unwrap();
    sync_validated_store(&store_path).unwrap();
    let before = snapshot_tree(fixture.data_dir());

    assert!(fixture.owner.open_store().is_err());
    assert!(crate::ReadMetaStore::open_data_dir(fixture.data_dir()).is_err());

    assert_eq!(snapshot_tree(fixture.data_dir()), before);
}

#[test]
fn current_v29_active_head_epoch_corruption_is_rejected_byte_stably() {
    let fixture = OwnedDirectory::new();
    let store = fixture.owner.open_store().unwrap();
    seed_published_v29_projection(&store);
    {
        let connection = store.connection.borrow();
        let restore = trigger_restore_sql(
            &connection,
            &[
                "ready_projection_head_matches_journal",
                "search_projection_head_change_requires_commit_guard",
            ],
        );
        connection
            .execute_batch(
                "DROP TRIGGER ready_projection_head_matches_journal;
                 DROP TRIGGER search_projection_head_change_requires_commit_guard;",
            )
            .unwrap();
        assert_eq!(
            connection
                .execute(
                    "UPDATE search_projection_state SET visible_epoch = 2
                 WHERE state_key = 'default'",
                    [],
                )
                .unwrap(),
            1
        );
        connection.execute_batch(&restore).unwrap();
    }
    drop(store);
    let store_path = active_store_path(fixture.data_dir()).unwrap();
    sync_validated_store(&store_path).unwrap();
    let before = snapshot_tree(fixture.data_dir());

    assert!(fixture.owner.open_store().is_err());
    assert!(crate::ReadMetaStore::open_data_dir(fixture.data_dir()).is_err());

    assert_eq!(snapshot_tree(fixture.data_dir()), before);
}

#[cfg(unix)]
#[test]
fn current_v29_symlinked_key_directory_is_rejected_without_following_it() {
    use std::os::unix::fs::symlink;

    let fixture = OwnedDirectory::new();
    drop(fixture.owner.open_store().unwrap());
    let external = tempdir().unwrap();
    let key_directory = fixture.data_dir().join("metadata-secrets");
    let external_key_directory = external.path().join("metadata-secrets");
    fs::rename(&key_directory, &external_key_directory).unwrap();
    symlink(&external_key_directory, &key_directory).unwrap();
    let before = snapshot_tree(fixture.data_dir());
    let external_before = snapshot_tree(external.path());

    assert!(fixture.owner.open_store().is_err());
    assert!(crate::ReadMetaStore::open_data_dir(fixture.data_dir()).is_err());

    assert_eq!(snapshot_tree(fixture.data_dir()), before);
    assert_eq!(snapshot_tree(external.path()), external_before);
}

#[cfg(unix)]
#[test]
fn current_v29_permissive_key_directory_is_rejected_without_chmod_repair() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = OwnedDirectory::new();
    drop(fixture.owner.open_store().unwrap());
    let key_directory = fixture.data_dir().join("metadata-secrets");
    fs::set_permissions(&key_directory, fs::Permissions::from_mode(0o755)).unwrap();
    let before = snapshot_tree(fixture.data_dir());

    assert!(fixture.owner.open_store().is_err());
    assert!(crate::ReadMetaStore::open_data_dir(fixture.data_dir()).is_err());

    assert_eq!(snapshot_tree(fixture.data_dir()), before);
    assert_eq!(
        fs::symlink_metadata(&key_directory)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
}

#[cfg(unix)]
#[test]
fn current_v29_symlinked_database_is_rejected_without_following_it() {
    use std::os::unix::fs::symlink;

    let fixture = OwnedDirectory::new();
    drop(fixture.owner.open_store().unwrap());
    let store_path = active_store_path(fixture.data_dir()).unwrap();
    let external = tempdir().unwrap();
    let external_store = external.path().join("external.sqlite3");
    fs::rename(&store_path, &external_store).unwrap();
    symlink(&external_store, &store_path).unwrap();
    let before = snapshot_tree(fixture.data_dir());
    let external_before = fs::read(&external_store).unwrap();

    assert!(fixture.owner.open_store().is_err());
    assert!(crate::ReadMetaStore::open_data_dir(fixture.data_dir()).is_err());

    assert_eq!(snapshot_tree(fixture.data_dir()), before);
    assert_eq!(fs::read(external_store).unwrap(), external_before);
}

#[test]
fn published_v28_is_rejected_without_mutating_any_authority_or_ciphertext() {
    let fixture = OwnedDirectory::new();
    let (source_path, _) =
        crate::migration_v28::prepare_active_v28_store(&fixture.owner.shared_guard()).unwrap();
    let before = snapshot_tree(fixture.data_dir());

    let Err(error) = fixture.owner.open_store() else {
        panic!("v28 must not enter the production v29 open path");
    };

    assert_eq!(error.class(), MetaStoreErrorClass::UnsupportedStoreSchema);
    assert_eq!(snapshot_tree(fixture.data_dir()), before);
    assert!(source_path.exists());
    assert_eq!(
        read_manifest(&fixture.data_dir().join(MANIFEST_FILE))
            .unwrap()
            .schema_version,
        crate::schema_v28::VERSION
    );
}

#[test]
fn v27_and_unknown_manifests_are_typed_unsupported_and_byte_stable() {
    for version in [27_u32, 99_u32] {
        let fixture = OwnedDirectory::new();
        let manifest_path = fixture.data_dir().join(MANIFEST_FILE);
        let digest = "a".repeat(64);
        fs::write(
            &manifest_path,
            format!(
                "resume-ir.metadata-active.v1\nfile=metadata-v{version}-{}.sqlite3\nschema={version}\ndigest={digest}\n",
                &digest[..16]
            ),
        )
        .unwrap();
        crate::restrict_private_file_permissions(&manifest_path).unwrap();
        let before = snapshot_tree(fixture.data_dir());

        let Err(error) = fixture.owner.open_store() else {
            panic!("schema v{version} must not enter the v29 owner path");
        };

        assert_eq!(error.class(), MetaStoreErrorClass::UnsupportedStoreSchema);
        assert_eq!(snapshot_tree(fixture.data_dir()), before);
    }
}

#[test]
fn legacy_database_authority_is_rejected_without_creating_key_or_manifest() {
    let fixture = OwnedDirectory::new();
    let legacy = fixture.data_dir().join(crate::METADATA_STORE_FILE);
    fs::write(&legacy, b"synthetic legacy authority").unwrap();
    crate::restrict_private_file_permissions(&legacy).unwrap();
    let before = snapshot_tree(fixture.data_dir());

    let Err(error) = fixture.owner.open_store() else {
        panic!("legacy authority must not be migrated");
    };

    assert_eq!(error.class(), MetaStoreErrorClass::UnsupportedStoreSchema);
    assert_eq!(snapshot_tree(fixture.data_dir()), before);
    assert!(!fixture.data_dir().join(MANIFEST_FILE).exists());
    assert!(!crate::metadata_encryption_key_path(fixture.data_dir()).exists());
}

#[test]
fn key_only_directory_is_not_treated_as_a_fresh_store() {
    let fixture = OwnedDirectory::new();
    crate::load_or_create_metadata_encryption_key(fixture.data_dir()).unwrap();
    let before = snapshot_tree(fixture.data_dir());

    let Err(error) = fixture.owner.open_store() else {
        panic!("a key without a current manifest is not an empty directory");
    };

    assert_eq!(error.class(), MetaStoreErrorClass::UnsupportedStoreSchema);
    assert_eq!(snapshot_tree(fixture.data_dir()), before);
    assert!(!fixture.data_dir().join(MANIFEST_FILE).exists());
}

#[test]
fn current_v29_open_never_deletes_a_v28_predecessor_named_file() {
    let fixture = OwnedDirectory::new();
    drop(fixture.owner.open_store().unwrap());
    let manifest = read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap();
    let predecessor = fixture.data_dir().join(format!(
        "metadata-v28-{}.sqlite3",
        &manifest.store_id_digest[..16]
    ));
    fs::write(&predecessor, b"synthetic retained predecessor").unwrap();
    crate::restrict_private_file_permissions(&predecessor).unwrap();
    let before = fs::read(&predecessor).unwrap();

    drop(fixture.owner.open_store().unwrap());

    assert_eq!(fs::read(predecessor).unwrap(), before);
}

#[test]
fn old_migration_attempt_authority_is_rejected_without_cleanup() {
    let fixture = OwnedDirectory::new();
    let attempt = fixture.data_dir().join("metadata-v28-migration-attempt.v1");
    fs::write(&attempt, b"synthetic old migration attempt").unwrap();
    crate::restrict_private_file_permissions(&attempt).unwrap();
    let before = snapshot_tree(fixture.data_dir());

    let Err(error) = fixture.owner.open_store() else {
        panic!("an old migration authority must not be resumed");
    };

    assert_eq!(error.class(), MetaStoreErrorClass::UnsupportedStoreSchema);
    assert_eq!(snapshot_tree(fixture.data_dir()), before);
}

#[test]
fn orphan_versioned_store_authorities_are_typed_unsupported_and_byte_stable() {
    for name in [
        "metadata-v26-0123456789abcdef.sqlite3",
        "metadata-v30-fedcba9876543210.sqlite3",
        "metadata-v30-future-format.sqlite3",
        "metadata-v30-0123456789abcdef.sqlite3-wal",
        "metadata-v29-.sqlite3",
        "metadata-v29-!.sqlite3",
        "metadata-v4294967296-deadbeef.sqlite3",
        "metadata-vnext-user-notes.sqlite3",
        "metadata-active.v2",
        crate::migration_v27::MIGRATION_LOCK_FILE,
    ] {
        let fixture = OwnedDirectory::new();
        let orphan = fixture.data_dir().join(name);
        fs::write(&orphan, format!("synthetic orphan authority: {name}")).unwrap();
        crate::restrict_private_file_permissions(&orphan).unwrap();
        let before = snapshot_tree(fixture.data_dir());

        let Err(error) = fixture.owner.open_store() else {
            panic!("version-shaped orphan {name} must not be replaced by a fresh v29 store");
        };

        assert_eq!(error.class(), MetaStoreErrorClass::UnsupportedStoreSchema);
        assert_eq!(snapshot_tree(fixture.data_dir()), before);
        assert!(!fixture.data_dir().join(MANIFEST_FILE).exists());
        assert!(!crate::metadata_encryption_key_path(fixture.data_dir()).exists());
    }
}

#[test]
fn similarly_named_non_authority_file_does_not_block_fresh_v29_creation() {
    let fixture = OwnedDirectory::new();
    let user_files = [
        fixture.data_dir().join("metadata-v30-user-notes.txt"),
        fixture.data_dir().join("notes-metadata-v30-user.sqlite3"),
    ];
    for user_file in &user_files {
        fs::write(user_file, b"synthetic non-authority user file").unwrap();
    }

    let store = fixture.owner.open_store().unwrap();

    assert_eq!(store.schema_version().unwrap(), schema_v29::VERSION);
    for user_file in user_files {
        assert_eq!(
            fs::read(user_file).unwrap(),
            b"synthetic non-authority user file"
        );
    }
}

#[cfg(unix)]
#[test]
fn unsafe_reserved_authority_objects_are_rejected_without_following_or_deleting_them() {
    use std::os::unix::fs::symlink;

    for kind in ["directory", "symlink"] {
        let fixture = OwnedDirectory::new();
        let authority = fixture.data_dir().join("metadata-active.v2");
        let external = tempdir().unwrap();
        if kind == "directory" {
            fs::create_dir(&authority).unwrap();
            fs::write(authority.join("retained"), b"directory authority").unwrap();
        } else {
            let target = external.path().join("retained");
            fs::write(&target, b"external authority").unwrap();
            symlink(&target, &authority).unwrap();
        }
        let before = snapshot_tree(fixture.data_dir());
        let external_before = snapshot_tree(external.path());

        assert!(fixture.owner.open_store().is_err());

        assert_eq!(snapshot_tree(fixture.data_dir()), before);
        assert_eq!(snapshot_tree(external.path()), external_before);
        assert!(!fixture.data_dir().join(MANIFEST_FILE).exists());
        assert!(!crate::metadata_encryption_key_path(fixture.data_dir()).exists());
    }
}

#[test]
fn identity_guard_does_not_delete_a_replacement_path() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("owned");
    let moved = directory.path().join("moved-owned");
    let guard = create_owned_private_file(&path, b"owned bytes").unwrap();
    fs::rename(&path, &moved).unwrap();
    fs::write(&path, b"replacement bytes").unwrap();
    crate::restrict_private_file_permissions(&path).unwrap();

    drop(guard);

    assert_eq!(fs::read(path).unwrap(), b"replacement bytes");
    assert_eq!(fs::read(moved).unwrap(), b"owned bytes\n");
}

#[test]
fn fresh_key_create_new_never_overwrites_an_existing_key() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("key");
    fs::write(&path, b"foreign key").unwrap();
    crate::restrict_private_file_permissions(&path).unwrap();

    assert!(create_owned_private_file(&path, b"new key").is_err());

    assert_eq!(fs::read(path).unwrap(), b"foreign key");
}

#[test]
fn fresh_target_link_never_overwrites_an_existing_target() {
    let directory = tempdir().unwrap();
    let staging_path = directory.path().join("staging");
    let target_path = directory.path().join("target");
    let staging = create_owned_private_file(&staging_path, b"staging bytes").unwrap();
    fs::write(&target_path, b"foreign target").unwrap();
    crate::restrict_private_file_permissions(&target_path).unwrap();

    assert!(link_owned_regular_file(&staging, &target_path).is_err());

    assert_eq!(fs::read(target_path).unwrap(), b"foreign target");
}

struct OwnedDirectory {
    _directory: TempDir,
    owner: DataDirectoryOwnerLease,
}

impl OwnedDirectory {
    fn new() -> Self {
        let directory = tempdir().unwrap();
        let data_dir = directory.path().join("data");
        let owner = match DataDirectoryOwnerLease::try_acquire(&data_dir).unwrap() {
            DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
            DataDirectoryOwnerAcquisition::Contended => panic!("synthetic owner contended"),
        };
        Self {
            _directory: directory,
            owner,
        }
    }

    fn data_dir(&self) -> &Path {
        self.owner.canonical_data_dir()
    }
}

fn synthetic_document(label: &str) -> Document {
    let now = UnixTimestamp::from_unix_seconds(1_800_000_000);
    let id = DocumentId::from_non_secret_parts(&["v29-hard-cut", label]);
    Document {
        id,
        source_uri: format!("synthetic://document/{label}"),
        normalized_path: format!("synthetic/root/{label}.txt"),
        file_name: format!("{label}.txt"),
        extension: FileExtension::Txt,
        byte_size: 128,
        mtime: now,
        content_hash: Some(
            ContentDigest::from_bytes(label.as_bytes())
                .as_str()
                .to_string(),
        ),
        text_hash: None,
        is_deleted: false,
        created_at: now,
        updated_at: now,
        status: DocumentStatus::Searchable,
    }
}

fn publish_empty_v29_generation(
    session: &crate::SearchPublicationSession,
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
        projection_digest.clone(),
        projection_digest,
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
    let outcome = migration_barrier.map_or_else(
        || session.commit_search_publication(&commit),
        |barrier| session.commit_migration_rebuild_search_publication(&commit, barrier),
    );
    assert_eq!(outcome.unwrap(), SearchPublicationOutcome::Applied);
}

fn trigger_restore_sql(connection: &rusqlite::Connection, names: &[&str]) -> String {
    names
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
        .map(|sql| format!("{sql};\n"))
        .collect()
}

fn seed_published_v29_projection(store: &OwnedMetaStore) -> ActiveSearchProjection {
    const GENERATION: &str = "v29-preservation-generation";
    let mut document = synthetic_document("preserved-v29");
    let revision = SourceRevision::for_content(
        document.id.clone(),
        ContentDigest::from_bytes(b"preserved-v29-source"),
        b"preserved-v29-source".len() as u64,
    );
    document.content_hash = Some(revision.content_hash.as_str().to_string());
    document.byte_size = revision.byte_size;
    document.status = DocumentStatus::FieldsExtracted;
    store.upsert_document(&document).unwrap();
    assert_eq!(
        store.insert_source_revision(&revision).unwrap(),
        IdentityInsertOutcome::Inserted
    );

    let normalized_text_hash = ContentDigest::from_bytes(b"preserved v29 normalized text");
    let version = ResumeVersion {
        id: ResumeVersionId::from_content_identity(
            &document.id,
            &revision.id,
            &normalized_text_hash,
            "v29-preservation-parser",
            "v29-preservation-schema",
        ),
        document_id: document.id.clone(),
        source_revision_id: revision.id.clone(),
        normalized_text_hash,
        parse_version: "v29-preservation-parser".to_string(),
        schema_version: "v29-preservation-schema".to_string(),
        language_set: vec!["en".to_string()],
        page_count: Some(1),
        raw_text: Some("preserved v29 normalized text".to_string()),
        clean_text: Some("preserved v29 normalized text".to_string()),
        quality_score: Some(0.95),
    };
    assert_eq!(
        store.insert_resume_version(&version).unwrap(),
        IdentityInsertOutcome::Inserted
    );
    assert_eq!(
        store
            .insert_resume_version_classification(&ResumeVersionClassification {
                resume_version_id: version.id.clone(),
                status: ClassificationStatus::ResumeCandidate,
                classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
                classified_at: UnixTimestamp::from_unix_seconds(1_800_000_001),
                review_disposition: ReviewDisposition::NotRequired,
            })
            .unwrap(),
        IdentityInsertOutcome::Inserted
    );

    let contract = ImportProcessingContract::new(
        "v29-preservation-parser",
        "v29-preservation-ocr",
        "v29-preservation-schema",
        CLASSIFIER_EPOCH,
    )
    .unwrap();
    store
        .activate_migration_rebuild_contract(
            &contract,
            UnixTimestamp::from_unix_seconds(1_800_000_002),
        )
        .unwrap();
    let barrier = store
        .acquire_migration_rebuild_barrier_token(contract.id())
        .unwrap()
        .unwrap();
    let mut session = store.wait_for_search_publication_session().unwrap();
    assert!(matches!(
        session
            .acquire_migration_rebuild_publication_attempt(
                &barrier,
                UnixTimestamp::from_unix_seconds(1_800_000_003),
            )
            .unwrap(),
        MigrationRebuildPublicationAttemptAcquire::Started(_)
            | MigrationRebuildPublicationAttemptAcquire::InProgress
    ));

    let projection = ActiveSearchProjection {
        document_id: document.id.clone(),
        resume_version_id: version.id,
    };
    let projection_digest = SearchProjectionDigest::from_pairs([(
        projection.document_id.as_str(),
        projection.resume_version_id.as_str(),
    )])
    .unwrap();
    assert_eq!(
        session
            .begin_search_publication(&SearchPublicationDraft {
                generation: GENERATION.to_string(),
                base_generation: None,
                expected_visible_epoch: 0,
                classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                projection_digest: projection_digest.clone(),
                now: UnixTimestamp::from_unix_seconds(1_800_000_004),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    let fulltext = FullTextSnapshotDescriptor::new(
        GENERATION.to_string(),
        1,
        projection_digest.clone(),
        ContentDigest::from_bytes(b"preserved-v29-fulltext-artifact"),
    );
    let vector = VectorSnapshotDescriptor::enabled(EnabledVectorSnapshotDescriptor {
        generation: GENERATION.to_string(),
        model_id: "preserved-v29-vector-model".to_string(),
        dimension: 3,
        projection_count: 1,
        projection_digest: projection_digest.clone(),
        coverage_digest: projection_digest,
        vector_count: 1,
        document_count: 1,
        resume_version_count: 1,
        logical_content_digest: ContentDigest::from_bytes(b"preserved-v29-vector-artifact"),
    });
    session
        .validate_search_publication(&SearchPublicationValidation {
            generation: GENERATION,
            fulltext: &fulltext,
            vector: &vector,
            now: UnixTimestamp::from_unix_seconds(1_800_000_005),
        })
        .unwrap();

    let commit_time = UnixTimestamp::from_unix_seconds(1_800_000_006);
    let terminal_document = TerminalDocumentUpdate {
        document_id: document.id.clone(),
        expected_status: DocumentStatus::FieldsExtracted,
        expected_is_deleted: false,
        expected_content_hash: revision.content_hash,
        terminal_status: DocumentStatus::Searchable,
        terminal_is_deleted: false,
    };
    document.status = DocumentStatus::Searchable;
    document.updated_at = commit_time;
    let projected_document = ProjectedDocumentSnapshot::Replacement {
        projection: projection.clone(),
        document,
    };
    assert_eq!(
        session
            .commit_migration_rebuild_search_publication(
                &SearchPublicationCommit {
                    generation: GENERATION,
                    terminal_documents: &[terminal_document],
                    projections: std::slice::from_ref(&projection),
                    projected_documents: &[projected_document],
                    vector_coverage: std::slice::from_ref(&projection),
                    now: commit_time,
                },
                &barrier,
            )
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    projection
}

#[derive(Debug, PartialEq)]
struct PreservedV29Summary {
    document: Document,
    source_revision: SourceRevision,
    resume_version: ResumeVersion,
    classification: ResumeVersionClassification,
    projection: ActiveSearchProjection,
    active_document: Document,
    generation: String,
    visible_epoch: u64,
    selection: SearchSelection,
    selection_is_current: bool,
    projection_digest: SearchProjectionDigest,
    publication_fingerprint: ContentDigest,
    fulltext_artifact_digest: ContentDigest,
    vector_artifact_digest: ContentDigest,
}

fn preserved_v29_summary(
    store: &OwnedMetaStore,
    projection: &ActiveSearchProjection,
) -> PreservedV29Summary {
    let state = store.search_projection_state().unwrap();
    let publication = state.publication.as_deref().unwrap();
    let selection = SearchSelection {
        document_id: projection.document_id.clone(),
        resume_version_id: projection.resume_version_id.clone(),
        visible_epoch: state.visible_epoch,
    };
    let selection_resolution = store
        .with_search_metadata_snapshot(|snapshot| {
            Ok::<_, ()>(snapshot.resolve_search_selection(&selection).unwrap())
        })
        .unwrap();
    let selection_is_current = matches!(
        selection_resolution,
        SearchSelectionResolution::Current {
            selection: resolved
        } if resolved == selection
    );
    let resume_version = store
        .resume_version_by_id(&projection.resume_version_id)
        .unwrap()
        .unwrap();

    PreservedV29Summary {
        document: store
            .document_by_id(&projection.document_id)
            .unwrap()
            .unwrap(),
        source_revision: store
            .source_revision_by_id(&resume_version.source_revision_id)
            .unwrap()
            .unwrap(),
        classification: store
            .resume_version_classification(&projection.resume_version_id, CLASSIFIER_EPOCH)
            .unwrap()
            .unwrap(),
        projection: store
            .active_search_projection_for_document(&projection.document_id)
            .unwrap()
            .unwrap(),
        active_document: store.active_search_document(projection).unwrap().unwrap(),
        generation: state.generation.unwrap(),
        visible_epoch: state.visible_epoch,
        selection,
        selection_is_current,
        projection_digest: publication.projection_digest.clone(),
        publication_fingerprint: publication.publication_fingerprint.clone().unwrap(),
        fulltext_artifact_digest: publication
            .fulltext
            .as_ref()
            .unwrap()
            .logical_content_digest()
            .clone(),
        vector_artifact_digest: publication
            .vector
            .as_ref()
            .unwrap()
            .logical_content_digest()
            .clone(),
        resume_version,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum SnapshotEntry {
    Directory,
    File(Vec<u8>),
    Other,
}

fn snapshot_tree(root: &Path) -> BTreeMap<PathBuf, SnapshotEntry> {
    let mut snapshot = BTreeMap::new();
    snapshot_directory(root, root, &mut snapshot);
    snapshot
}

fn snapshot_directory(
    root: &Path,
    directory: &Path,
    snapshot: &mut BTreeMap<PathBuf, SnapshotEntry>,
) {
    let mut entries = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap())
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap().to_path_buf();
        let file_type = entry.file_type().unwrap();
        if file_type.is_dir() {
            snapshot.insert(relative, SnapshotEntry::Directory);
            snapshot_directory(root, &path, snapshot);
        } else if file_type.is_file() {
            snapshot.insert(relative, SnapshotEntry::File(fs::read(path).unwrap()));
        } else {
            snapshot.insert(relative, SnapshotEntry::Other);
        }
    }
}
