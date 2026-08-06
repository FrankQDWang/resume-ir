use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::*;
use crate::{
    active_store_manifest::{read_manifest, read_manifest_format_version},
    ContentDigest, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, Document, DocumentId,
    DocumentStatus, FileExtension, FullTextSnapshotDescriptor, ImportProcessingContract,
    MetaStoreErrorClass, OwnedMetaStore, ScanTrigger, SearchProjectionDigest,
    SearchPublicationCommit, SearchPublicationDraft, SearchPublicationOutcome,
    SearchPublicationValidation, UnixTimestamp, VectorSnapshotDescriptor, CLASSIFIER_EPOCH,
};

#[test]
fn authority_free_directory_initializes_exact_v38_and_reopens_without_writes() {
    let fixture = OwnedDirectory::new();
    let store = fixture.owner.open_store().unwrap();
    assert_eq!(store.schema_version().unwrap(), schema_v38::VERSION);
    assert_eq!(
        store
            .connection
            .borrow()
            .query_row(
                "SELECT COUNT(*) FROM forward_migration_history",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        9
    );
    let now = UnixTimestamp::from_unix_seconds(1_800_500_600);
    let root = store
        .register_source_root(
            "/synthetic/v38-reopen",
            "/synthetic/v38-reopen",
            "Synthetic v38 reopen",
            now,
        )
        .unwrap();
    store
        .begin_scan(&root.id, "v38-reopen-scan", ScanTrigger::Manual, now)
        .unwrap();
    store.begin_source_root_deletion(&root.id, now).unwrap();
    drop(store);

    let reopened = fixture.owner.open_store().unwrap();
    assert_eq!(
        reopened
            .connection
            .borrow()
            .query_row(
                "SELECT checkpoint_protocol_version
                 FROM source_root_deletion WHERE root_id = ?1",
                [root.id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        schema_v37::SNAPSHOT_INVARIANT_V2
    );
    assert_eq!(
        reopened
            .connection
            .borrow()
            .query_row(
                "SELECT source_root.revocation_epoch,
                        scan_snapshot.root_revocation_epoch
                 FROM source_root
                 JOIN scan_snapshot ON scan_snapshot.root_id = source_root.id
                 WHERE source_root.id = ?1",
                [root.id.as_str()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap(),
        (1, 0)
    );
    drop(reopened);

    let manifest_path = fixture.data_dir().join(MANIFEST_FILE);
    assert_eq!(read_manifest_format_version(&manifest_path).unwrap(), 2);
    let before = snapshot_tree(fixture.data_dir());
    drop(crate::ReadMetaStore::open_data_dir(fixture.data_dir()).unwrap());
    assert_eq!(snapshot_tree(fixture.data_dir()), before);
}

#[test]
fn encrypted_current_logical_authority_is_inspected_without_writes() {
    const GENERATION: &str = "current-authority-generation";
    let fixture = OwnedDirectory::new();
    let store = fixture.owner.open_store().unwrap();
    let contract = ImportProcessingContract::new(
        "current-authority-parser",
        "current-authority-ocr",
        "current-authority-schema",
        CLASSIFIER_EPOCH,
    )
    .unwrap();
    store
        .activate_migration_rebuild_contract(
            &contract,
            UnixTimestamp::from_unix_seconds(1_899_999_998),
        )
        .unwrap();
    let barrier = store
        .acquire_migration_rebuild_barrier_token(contract.id())
        .unwrap()
        .unwrap();
    let projection_digest = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
    let mut session = store.wait_for_search_publication_session().unwrap();
    assert!(matches!(
        session
            .acquire_migration_rebuild_publication_attempt(
                &barrier,
                UnixTimestamp::from_unix_seconds(1_899_999_999),
            )
            .unwrap(),
        crate::MigrationRebuildPublicationAttemptAcquire::Started(_)
    ));
    assert_eq!(
        session
            .begin_search_publication(&SearchPublicationDraft {
                generation: GENERATION.to_string(),
                base_generation: None,
                expected_visible_epoch: 0,
                classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                projection_digest: projection_digest.clone(),
                now: UnixTimestamp::from_unix_seconds(1_900_000_000),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    let fulltext = FullTextSnapshotDescriptor::new(
        GENERATION.to_string(),
        0,
        projection_digest.clone(),
        ContentDigest::from_bytes(b"current-fulltext"),
    );
    let vector = VectorSnapshotDescriptor::disabled(
        GENERATION.to_string(),
        0,
        projection_digest.clone(),
        projection_digest,
        ContentDigest::from_bytes(b"current-vector"),
    );
    session
        .validate_search_publication(&SearchPublicationValidation {
            generation: GENERATION,
            fulltext: &fulltext,
            vector: &vector,
            now: UnixTimestamp::from_unix_seconds(1_900_000_001),
        })
        .unwrap();
    assert_eq!(
        session
            .commit_migration_rebuild_search_publication(
                &SearchPublicationCommit {
                    generation: GENERATION,
                    terminal_documents: &[],
                    projections: &[],
                    projected_documents: &[],
                    vector_coverage: &[],
                    now: UnixTimestamp::from_unix_seconds(1_900_000_002),
                },
                &barrier,
            )
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    drop(session);
    drop(store);
    let before = snapshot_tree(fixture.data_dir());

    let authority = crate::inspect_metadata_logical_authority(fixture.data_dir()).unwrap();

    assert_eq!(authority.generation, GENERATION);
    assert_eq!(authority.visible_epoch, 1);
    assert_eq!(authority.fulltext_document_count, 0);
    assert_eq!(authority.vector_mode, "disabled");
    assert_eq!(authority.vector_model_id, None);
    assert_eq!(snapshot_tree(fixture.data_dir()), before);
}

#[test]
fn exact_v29_migrates_through_cow_without_mutating_predecessor() {
    let fixture = OwnedDirectory::new();
    let source_store = fixture.open_historical_v29();
    let document = synthetic_document();
    source_store.upsert_document(&document).unwrap();
    let source_manifest = read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap();
    assert_eq!(source_manifest.schema_version, schema_v29::VERSION);
    assert_eq!(
        read_manifest_format_version(&fixture.data_dir().join(MANIFEST_FILE)).unwrap(),
        1
    );
    let source_path = fixture.data_dir().join(&source_manifest.file_name);
    drop(source_store);
    let source_ciphertext = sha256_file(&source_path);

    let migrated = fixture.owner.open_store().unwrap();
    assert_eq!(migrated.schema_version().unwrap(), schema_v38::VERSION);
    assert_eq!(
        migrated.document_by_id(&document.id).unwrap(),
        Some(document)
    );
    assert_eq!(
        migrated.source_file_observation_count().unwrap(),
        0,
        "pre-v35 stores must migrate without inventing fast-path observations"
    );
    let target_manifest = read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap();
    assert_eq!(target_manifest.schema_version, schema_v38::VERSION);
    assert_eq!(
        target_manifest.store_id_digest,
        source_manifest.store_id_digest
    );
    assert_ne!(target_manifest.file_name, source_manifest.file_name);
    assert_eq!(
        read_manifest_format_version(&fixture.data_dir().join(MANIFEST_FILE)).unwrap(),
        2
    );
    assert_eq!(sha256_file(&source_path), source_ciphertext);
    assert!(source_path.exists());
    let migration_receipt = receipt::read(&receipt::path(fixture.data_dir())).unwrap();
    assert_eq!(migration_receipt.phase, ReceiptPhase::Published);
    assert_eq!(migration_receipt.source, source_manifest);
    assert_eq!(migration_receipt.target, target_manifest);
}

#[test]
fn future_manifest_fails_closed_without_mutating_authority() {
    let fixture = OwnedDirectory::new();
    drop(fixture.owner.open_store().unwrap());
    let manifest_path = fixture.data_dir().join(MANIFEST_FILE);
    let current = read_manifest(&manifest_path).unwrap();
    fs::write(
        &manifest_path,
        format!(
            "resume-ir.metadata-active.v2\nfile=metadata-v39-{}.sqlite3\nschema=39\ndigest={}\n",
            &current.store_id_digest[..16],
            current.store_id_digest,
        ),
    )
    .unwrap();
    crate::restrict_private_file_permissions(&manifest_path).unwrap();
    let before = snapshot_tree(fixture.data_dir());

    let error = fixture.owner.open_store().unwrap_err();

    assert_eq!(error.class(), MetaStoreErrorClass::UnsupportedStoreSchema);
    assert_eq!(snapshot_tree(fixture.data_dir()), before);
}

#[test]
fn preparing_receipt_discards_only_recorded_unpublished_files_then_retries() {
    let fixture = OwnedDirectory::new();
    drop(fixture.open_historical_v29());
    let source = read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap();
    let interrupted = migration_receipt(&source, "a".repeat(64), ReceiptPhase::Preparing);
    for file_name in [
        interrupted.staging_file.as_str(),
        interrupted.target.file_name.as_str(),
    ] {
        let path = fixture.data_dir().join(file_name);
        fs::write(&path, b"interrupted unpublished bytes").unwrap();
        crate::restrict_private_file_permissions(&path).unwrap();
    }
    receipt::persist(fixture.data_dir(), &interrupted).unwrap();

    let migrated = fixture.owner.open_store().unwrap();

    assert_eq!(migrated.schema_version().unwrap(), schema_v38::VERSION);
    assert!(!fixture.data_dir().join(interrupted.staging_file).exists());
    assert!(!fixture
        .data_dir()
        .join(interrupted.target.file_name)
        .exists());
    let completed = receipt::read(&receipt::path(fixture.data_dir())).unwrap();
    assert_eq!(completed.phase, ReceiptPhase::Published);
    assert_ne!(completed.migration_id, interrupted.migration_id);
}

#[test]
fn ready_receipt_atomically_publishes_the_prevalidated_target() {
    let fixture = OwnedDirectory::new();
    let source_store = fixture.open_historical_v29();
    let document = synthetic_document();
    source_store.upsert_document(&document).unwrap();
    drop(source_store);
    let source_manifest = read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap();
    let source_path = fixture.data_dir().join(&source_manifest.file_name);
    let source_ciphertext = sha256_file(&source_path);
    let key = read_key(fixture.data_dir()).unwrap();
    let source = open_encrypted_read_connection(&source_path, &key).unwrap();
    let mut interrupted = migration_receipt(&source_manifest, "b".repeat(64), ReceiptPhase::Ready);
    let staging_path = fixture.data_dir().join(&interrupted.staging_file);
    copy_encrypted_store(&source, &staging_path, &key).unwrap();
    let mut staging = open_existing_encrypted_writer(&staging_path, &key).unwrap();
    forward_migration::apply_current_schema(&mut staging, schema_v29::VERSION).unwrap();
    validate_current_connection(&staging, &source_manifest.store_id_digest).unwrap();
    drop(staging);
    let target_path = fixture.data_dir().join(&interrupted.target.file_name);
    fs::rename(&staging_path, &target_path).unwrap();
    sync_parent_directory(fixture.data_dir()).unwrap();
    validate_current_store(&target_path, &key, &source_manifest.store_id_digest).unwrap();
    receipt::persist(fixture.data_dir(), &interrupted).unwrap();

    let recovered = fixture.owner.open_store().unwrap();

    assert_eq!(recovered.schema_version().unwrap(), schema_v38::VERSION);
    assert_eq!(
        recovered.document_by_id(&document.id).unwrap(),
        Some(document)
    );
    assert_eq!(
        read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap(),
        interrupted.target
    );
    interrupted.phase = ReceiptPhase::Published;
    assert_eq!(
        receipt::read(&receipt::path(fixture.data_dir())).unwrap(),
        interrupted
    );
    assert_eq!(sha256_file(&source_path), source_ciphertext);
}

#[test]
fn published_v35_receipt_is_retired_before_upgrading_to_v38() {
    let fixture = OwnedDirectory::new();
    let source_store = fixture.open_historical_v29();
    let document = synthetic_document();
    source_store.upsert_document(&document).unwrap();
    drop(source_store);
    let source = read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap();
    let receipt = v35_migration_receipt(&source, "c".repeat(64), ReceiptPhase::Published);
    build_historical_target(
        fixture.data_dir(),
        &source,
        &receipt.staging_file,
        &receipt.target,
    );
    replace_active_store(fixture.data_dir(), &source, &receipt.target, || Ok(())).unwrap();
    persist_exact_forward_receipt(fixture.data_dir(), &receipt);

    let recovered = fixture.owner.open_store().unwrap();

    assert_eq!(recovered.schema_version().unwrap(), schema_v38::VERSION);
    assert_eq!(
        recovered.document_by_id(&document.id).unwrap(),
        Some(document)
    );
    assert!(!fixture.data_dir().join(source.file_name).exists());
    let current_receipt = receipt::read(&receipt::path(fixture.data_dir())).unwrap();
    assert_eq!(current_receipt.phase, ReceiptPhase::Published);
    assert_eq!(current_receipt.source, receipt.target);
    assert_eq!(current_receipt.target.schema_version, schema_v38::VERSION);
}

#[test]
fn preparing_v35_receipt_cleans_exact_old_files_then_upgrades() {
    let fixture = OwnedDirectory::new();
    drop(fixture.open_historical_v29());
    let source = read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap();
    let interrupted = v35_migration_receipt(&source, "d".repeat(64), ReceiptPhase::Preparing);
    for file_name in [
        interrupted.staging_file.as_str(),
        interrupted.target.file_name.as_str(),
    ] {
        let path = fixture.data_dir().join(file_name);
        fs::write(&path, b"historical v35 unpublished bytes").unwrap();
        crate::restrict_private_file_permissions(&path).unwrap();
    }
    persist_exact_forward_receipt(fixture.data_dir(), &interrupted);

    let recovered = fixture.owner.open_store().unwrap();

    assert_eq!(recovered.schema_version().unwrap(), schema_v38::VERSION);
    assert!(!fixture.data_dir().join(interrupted.staging_file).exists());
    assert!(!fixture
        .data_dir()
        .join(interrupted.target.file_name)
        .exists());
    let current_receipt = receipt::read(&receipt::path(fixture.data_dir())).unwrap();
    assert_eq!(current_receipt.phase, ReceiptPhase::Published);
    assert_eq!(current_receipt.target.schema_version, schema_v38::VERSION);
}

#[test]
fn ready_v35_receipt_publishes_then_upgrades_without_orphans() {
    let fixture = OwnedDirectory::new();
    let source_store = fixture.open_historical_v29();
    let document = synthetic_document();
    source_store.upsert_document(&document).unwrap();
    drop(source_store);
    let source = read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap();
    let interrupted = v35_migration_receipt(&source, "e".repeat(64), ReceiptPhase::Ready);
    build_historical_target(
        fixture.data_dir(),
        &source,
        &interrupted.staging_file,
        &interrupted.target,
    );
    persist_exact_forward_receipt(fixture.data_dir(), &interrupted);

    let recovered = fixture.owner.open_store().unwrap();

    assert_eq!(recovered.schema_version().unwrap(), schema_v38::VERSION);
    assert_eq!(
        recovered.document_by_id(&document.id).unwrap(),
        Some(document)
    );
    assert!(!fixture.data_dir().join(source.file_name).exists());
    assert!(!fixture.data_dir().join(interrupted.staging_file).exists());
    let current_receipt = receipt::read(&receipt::path(fixture.data_dir())).unwrap();
    assert_eq!(current_receipt.phase, ReceiptPhase::Published);
    assert_eq!(current_receipt.source, interrupted.target);
}

#[test]
fn preparing_v35_initialization_receipt_cleans_exact_old_files() {
    let fixture = OwnedDirectory::new();
    let initialization_id = "f".repeat(64);
    let old_staging = format!(".metadata-v35-init-{}.sqlite3", &initialization_id[..16]);
    let old_target = format!("metadata-v35-{}.sqlite3", &initialization_id[..16]);
    for file_name in [&old_staging, &old_target] {
        let path = fixture.data_dir().join(file_name);
        fs::write(&path, b"historical v35 initialization bytes").unwrap();
        crate::restrict_private_file_permissions(&path).unwrap();
    }
    persist_exact_initialization_receipt(fixture.data_dir(), "preparing", &initialization_id, None);

    let recovered = fixture.owner.open_store().unwrap();

    assert_eq!(recovered.schema_version().unwrap(), schema_v38::VERSION);
    assert!(!fixture.data_dir().join(old_staging).exists());
    assert!(!fixture.data_dir().join(old_target).exists());
    assert!(!initialization_receipt::path(fixture.data_dir()).exists());
}

#[test]
fn ready_v35_initialization_receipt_publishes_then_upgrades() {
    let fixture = OwnedDirectory::new();
    let source_store = fixture.open_historical_v29();
    let document = synthetic_document();
    source_store.upsert_document(&document).unwrap();
    drop(source_store);
    let source = read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap();
    let initialization_id = "1".repeat(64);
    let old_staging = format!(".metadata-v35-init-{}.sqlite3", &initialization_id[..16]);
    let old_target = ActiveStoreManifest {
        file_name: format!("metadata-v35-{}.sqlite3", &initialization_id[..16]),
        schema_version: schema_v35::VERSION,
        store_id_digest: source.store_id_digest.clone(),
    };
    build_historical_target(fixture.data_dir(), &source, &old_staging, &old_target);
    fs::remove_file(fixture.data_dir().join(&source.file_name)).unwrap();
    fs::remove_file(fixture.data_dir().join(MANIFEST_FILE)).unwrap();
    sync_parent_directory(fixture.data_dir()).unwrap();
    persist_exact_initialization_receipt(
        fixture.data_dir(),
        "ready",
        &initialization_id,
        Some(&old_target.store_id_digest),
    );

    let recovered = fixture.owner.open_store().unwrap();

    assert_eq!(recovered.schema_version().unwrap(), schema_v38::VERSION);
    assert_eq!(
        recovered.document_by_id(&document.id).unwrap(),
        Some(document)
    );
    assert!(!fixture.data_dir().join(old_staging).exists());
    assert!(!initialization_receipt::path(fixture.data_dir()).exists());
    let current_receipt = receipt::read(&receipt::path(fixture.data_dir())).unwrap();
    assert_eq!(current_receipt.phase, ReceiptPhase::Published);
    assert_eq!(current_receipt.source, old_target);
    assert!(fixture
        .data_dir()
        .join(&current_receipt.source.file_name)
        .exists());
}

#[test]
fn published_v35_initialization_ready_receipt_continues_to_v38() {
    let fixture = OwnedDirectory::new();
    let source_store = fixture.open_historical_v29();
    let document = synthetic_document();
    source_store.upsert_document(&document).unwrap();
    drop(source_store);
    let source = read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap();
    let initialization_id = "2".repeat(64);
    let old_staging = format!(".metadata-v35-init-{}.sqlite3", &initialization_id[..16]);
    let old_target = ActiveStoreManifest {
        file_name: format!("metadata-v35-{}.sqlite3", &initialization_id[..16]),
        schema_version: schema_v35::VERSION,
        store_id_digest: source.store_id_digest.clone(),
    };
    build_historical_target(fixture.data_dir(), &source, &old_staging, &old_target);
    replace_active_store(fixture.data_dir(), &source, &old_target, || Ok(())).unwrap();
    fs::remove_file(fixture.data_dir().join(source.file_name)).unwrap();
    sync_parent_directory(fixture.data_dir()).unwrap();
    persist_exact_initialization_receipt(
        fixture.data_dir(),
        "ready",
        &initialization_id,
        Some(&old_target.store_id_digest),
    );

    let recovered = fixture.owner.open_store().unwrap();

    assert_eq!(recovered.schema_version().unwrap(), schema_v38::VERSION);
    assert_eq!(
        recovered.document_by_id(&document.id).unwrap(),
        Some(document)
    );
    assert!(!fixture.data_dir().join(old_staging).exists());
    assert!(!initialization_receipt::path(fixture.data_dir()).exists());
    let current_receipt = receipt::read(&receipt::path(fixture.data_dir())).unwrap();
    assert_eq!(current_receipt.phase, ReceiptPhase::Published);
    assert_eq!(current_receipt.source, old_target);
}

#[test]
fn v36_forward_receipt_phases_converge_before_upgrading_to_v38() {
    for (phase, migration_id) in [
        (ReceiptPhase::Preparing, "3".repeat(64)),
        (ReceiptPhase::Ready, "4".repeat(64)),
        (ReceiptPhase::Published, "5".repeat(64)),
    ] {
        let fixture = OwnedDirectory::new();
        let source_store = fixture.open_historical_v29();
        let document = synthetic_document();
        source_store.upsert_document(&document).unwrap();
        drop(source_store);
        let source = read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap();
        let interrupted =
            historical_migration_receipt(&source, migration_id, phase, schema_v36::VERSION);
        match phase {
            ReceiptPhase::Preparing => {
                for file_name in [
                    interrupted.staging_file.as_str(),
                    interrupted.target.file_name.as_str(),
                ] {
                    let path = fixture.data_dir().join(file_name);
                    fs::write(&path, b"historical v36 unpublished bytes").unwrap();
                    crate::restrict_private_file_permissions(&path).unwrap();
                }
            }
            ReceiptPhase::Ready | ReceiptPhase::Published => {
                build_historical_target(
                    fixture.data_dir(),
                    &source,
                    &interrupted.staging_file,
                    &interrupted.target,
                );
                if phase == ReceiptPhase::Published {
                    replace_active_store(fixture.data_dir(), &source, &interrupted.target, || {
                        Ok(())
                    })
                    .unwrap();
                }
            }
        }
        persist_exact_forward_receipt(fixture.data_dir(), &interrupted);

        let recovered = fixture.owner.open_store().unwrap();

        assert_eq!(recovered.schema_version().unwrap(), schema_v38::VERSION);
        assert_eq!(
            recovered.document_by_id(&document.id).unwrap(),
            Some(document)
        );
        assert!(!fixture.data_dir().join(interrupted.staging_file).exists());
        let current_receipt = receipt::read(&receipt::path(fixture.data_dir())).unwrap();
        assert_eq!(current_receipt.phase, ReceiptPhase::Published);
        assert_eq!(current_receipt.target.schema_version, schema_v38::VERSION);
        if phase != ReceiptPhase::Preparing {
            assert_eq!(current_receipt.source, interrupted.target);
        }
    }
}

#[test]
fn preparing_v36_initialization_receipt_cleans_exact_old_files() {
    let fixture = OwnedDirectory::new();
    let initialization_id = "6".repeat(64);
    let old_staging = format!(".metadata-v36-init-{}.sqlite3", &initialization_id[..16]);
    let old_target = format!("metadata-v36-{}.sqlite3", &initialization_id[..16]);
    for file_name in [&old_staging, &old_target] {
        let path = fixture.data_dir().join(file_name);
        fs::write(&path, b"historical v36 initialization bytes").unwrap();
        crate::restrict_private_file_permissions(&path).unwrap();
    }
    persist_exact_initialization_receipt(fixture.data_dir(), "preparing", &initialization_id, None);

    let recovered = fixture.owner.open_store().unwrap();

    assert_eq!(recovered.schema_version().unwrap(), schema_v38::VERSION);
    assert!(!fixture.data_dir().join(old_staging).exists());
    assert!(!fixture.data_dir().join(old_target).exists());
    assert!(!initialization_receipt::path(fixture.data_dir()).exists());
}

#[test]
fn ready_v36_initialization_receipt_publishes_then_upgrades() {
    let fixture = OwnedDirectory::new();
    let source_store = fixture.open_historical_v29();
    let document = synthetic_document();
    source_store.upsert_document(&document).unwrap();
    drop(source_store);
    let source = read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap();
    let initialization_id = "7".repeat(64);
    let old_staging = format!(".metadata-v36-init-{}.sqlite3", &initialization_id[..16]);
    let old_target = ActiveStoreManifest {
        file_name: format!("metadata-v36-{}.sqlite3", &initialization_id[..16]),
        schema_version: schema_v36::VERSION,
        store_id_digest: source.store_id_digest.clone(),
    };
    build_historical_target(fixture.data_dir(), &source, &old_staging, &old_target);
    fs::remove_file(fixture.data_dir().join(&source.file_name)).unwrap();
    fs::remove_file(fixture.data_dir().join(MANIFEST_FILE)).unwrap();
    sync_parent_directory(fixture.data_dir()).unwrap();
    persist_exact_initialization_receipt(
        fixture.data_dir(),
        "ready",
        &initialization_id,
        Some(&old_target.store_id_digest),
    );

    let recovered = fixture.owner.open_store().unwrap();

    assert_eq!(recovered.schema_version().unwrap(), schema_v38::VERSION);
    assert_eq!(
        recovered.document_by_id(&document.id).unwrap(),
        Some(document)
    );
    assert!(!fixture.data_dir().join(old_staging).exists());
    assert!(!initialization_receipt::path(fixture.data_dir()).exists());
    let current_receipt = receipt::read(&receipt::path(fixture.data_dir())).unwrap();
    assert_eq!(current_receipt.phase, ReceiptPhase::Published);
    assert_eq!(current_receipt.source, old_target);
}

#[test]
fn exact_v37_recovery_receipts_converge_before_v38_upgrade() {
    for (phase, digit) in [
        (ReceiptPhase::Preparing, "a"),
        (ReceiptPhase::Ready, "b"),
        (ReceiptPhase::Published, "c"),
    ] {
        let fixture = OwnedDirectory::new();
        drop(fixture.open_historical_v29());
        let source = read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap();
        let interrupted =
            historical_migration_receipt(&source, digit.repeat(64), phase, schema_v37::VERSION);
        if phase == ReceiptPhase::Preparing {
            for file in [&interrupted.staging_file, &interrupted.target.file_name] {
                fs::write(fixture.data_dir().join(file), b"v37 unpublished").unwrap();
                crate::restrict_private_file_permissions(&fixture.data_dir().join(file)).unwrap();
            }
        } else {
            build_historical_target(
                fixture.data_dir(),
                &source,
                &interrupted.staging_file,
                &interrupted.target,
            );
            if phase == ReceiptPhase::Published {
                replace_active_store(fixture.data_dir(), &source, &interrupted.target, || Ok(()))
                    .unwrap();
            }
        }
        persist_exact_forward_receipt(fixture.data_dir(), &interrupted);
        assert_opens_current_v38(&fixture);
    }

    for (phase, digit) in [("preparing", "d"), ("ready", "e")] {
        let fixture = OwnedDirectory::new();
        let initialization_id = digit.repeat(64);
        let staging = format!(".metadata-v37-init-{}.sqlite3", &initialization_id[..16]);
        let target_file = format!("metadata-v37-{}.sqlite3", &initialization_id[..16]);
        if phase == "preparing" {
            for file in [&staging, &target_file] {
                fs::write(fixture.data_dir().join(file), b"v37 initialization").unwrap();
                crate::restrict_private_file_permissions(&fixture.data_dir().join(file)).unwrap();
            }
        } else {
            drop(fixture.open_historical_v29());
            let source = read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap();
            let target = ActiveStoreManifest {
                file_name: target_file,
                schema_version: schema_v37::VERSION,
                store_id_digest: source.store_id_digest.clone(),
            };
            build_historical_target(fixture.data_dir(), &source, &staging, &target);
            fs::remove_file(fixture.data_dir().join(&source.file_name)).unwrap();
            fs::remove_file(fixture.data_dir().join(MANIFEST_FILE)).unwrap();
            sync_parent_directory(fixture.data_dir()).unwrap();
            persist_exact_initialization_receipt(
                fixture.data_dir(),
                phase,
                &initialization_id,
                Some(target.store_id_digest.as_str()),
            );
        }
        if phase == "preparing" {
            persist_exact_initialization_receipt(
                fixture.data_dir(),
                phase,
                &initialization_id,
                None,
            );
        }
        assert_opens_current_v38(&fixture);
    }
}

fn assert_opens_current_v38(fixture: &OwnedDirectory) {
    assert_eq!(
        fixture
            .owner
            .open_store()
            .unwrap()
            .schema_version()
            .unwrap(),
        schema_v38::VERSION
    );
}

#[test]
fn missing_v29_key_fails_without_touching_the_source_ciphertext() {
    let fixture = OwnedDirectory::new();
    drop(fixture.open_historical_v29());
    fs::remove_file(crate::metadata_encryption_key_path(fixture.data_dir())).unwrap();
    let before = snapshot_tree(fixture.data_dir());

    let error = fixture.owner.open_store().unwrap_err();

    assert_eq!(error.class(), MetaStoreErrorClass::Storage);
    assert_eq!(snapshot_tree(fixture.data_dir()), before);
}

#[test]
fn tampered_forward_history_fails_closed_without_repair() {
    let fixture = OwnedDirectory::new();
    let store = fixture.owner.open_store().unwrap();
    store
        .connection
        .borrow()
        .execute(
            "UPDATE forward_migration_history
             SET migration_checksum = ?1
             WHERE to_version = ?2",
            rusqlite::params!["0".repeat(64), i64::from(schema_v35::VERSION)],
        )
        .unwrap();
    drop(store);
    let before = snapshot_tree(fixture.data_dir());

    let error = fixture.owner.open_store().unwrap_err();

    assert_eq!(error.class(), MetaStoreErrorClass::StorageInvariant);
    assert_eq!(snapshot_tree(fixture.data_dir()), before);
}

#[test]
fn logical_preservation_digest_scans_storage_order_without_temp_sorting() {
    let source = Connection::open_in_memory().unwrap();
    source
        .execute_batch(
            "CREATE TABLE rowid_records (
                 id TEXT NOT NULL UNIQUE,
                 payload BLOB NOT NULL
             );
             CREATE TABLE keyed_records (
                 group_id TEXT NOT NULL,
                 item_id TEXT NOT NULL,
                 payload BLOB NOT NULL,
                 PRIMARY KEY (group_id, item_id)
             ) WITHOUT ROWID;",
        )
        .unwrap();
    for index in (0..64).rev() {
        source
            .execute(
                "INSERT INTO rowid_records (id, payload) VALUES (?1, ?2)",
                rusqlite::params![
                    format!("row-{index:03}"),
                    vec![u8::try_from(index).unwrap(); 64 * 1024]
                ],
            )
            .unwrap();
        source
            .execute(
                "INSERT INTO keyed_records (group_id, item_id, payload)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![
                    format!("group-{}", index % 4),
                    format!("item-{index:03}"),
                    vec![u8::try_from(index).unwrap(); 64 * 1024]
                ],
            )
            .unwrap();
    }
    let tables = vec!["keyed_records".to_string(), "rowid_records".to_string()];
    let source_digest = logical_data_digest(&source, &tables, schema_v38::VERSION).unwrap();

    let destination_path = tempfile::NamedTempFile::new().unwrap();
    let mut destination = Connection::open(destination_path.path()).unwrap();
    let backup = Backup::new(&source, &mut destination).unwrap();
    backup
        .run_to_completion(32, Duration::from_millis(1), None)
        .unwrap();
    drop(backup);

    assert_eq!(
        logical_data_digest(&destination, &tables, schema_v38::VERSION).unwrap(),
        source_digest
    );
    assert_eq!(
        stable_table_order(
            &source,
            "rowid_records",
            &[("id".to_string(), 0), ("payload".to_string(), 0)]
        )
        .unwrap(),
        "_rowid_"
    );
    assert_eq!(
        stable_table_order(
            &source,
            "keyed_records",
            &[
                ("group_id".to_string(), 1),
                ("item_id".to_string(), 2),
                ("payload".to_string(), 0),
            ]
        )
        .unwrap(),
        "\"group_id\", \"item_id\""
    );
    for query in [
        "SELECT * FROM rowid_records ORDER BY _rowid_",
        "SELECT * FROM keyed_records ORDER BY \"group_id\", \"item_id\"",
    ] {
        let mut plan = source
            .prepare(&format!("EXPLAIN QUERY PLAN {query}"))
            .unwrap();
        let details = plan
            .query_map([], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            details
                .iter()
                .all(|detail| !detail.contains("USE TEMP B-TREE")),
            "{details:?}"
        );
    }
}

struct OwnedDirectory {
    _directory: TempDir,
    owner: DataDirectoryOwnerLease,
}

impl OwnedDirectory {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
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

    fn open_historical_v29(&self) -> OwnedMetaStore {
        let owner = self.owner.shared_guard();
        let (path, key) = migration_v29::prepare_active_v29_store(&owner).unwrap();
        OwnedMetaStore::open_owned_encrypted(path, &key, owner).unwrap()
    }
}

fn migration_receipt(
    source: &ActiveStoreManifest,
    migration_id: String,
    phase: ReceiptPhase,
) -> MigrationReceipt {
    MigrationReceipt {
        phase,
        staging_file: format!("{STAGING_PREFIX}{}.sqlite3", &migration_id[..16]),
        target: ActiveStoreManifest {
            file_name: format!("metadata-v38-{}.sqlite3", &migration_id[..16]),
            schema_version: schema_v38::VERSION,
            store_id_digest: source.store_id_digest.clone(),
        },
        source: source.clone(),
        migration_id,
    }
}

fn v35_migration_receipt(
    source: &ActiveStoreManifest,
    migration_id: String,
    phase: ReceiptPhase,
) -> MigrationReceipt {
    MigrationReceipt {
        phase,
        staging_file: format!("{STAGING_PREFIX}{}.sqlite3", &migration_id[..16]),
        target: ActiveStoreManifest {
            file_name: format!("metadata-v35-{}.sqlite3", &migration_id[..16]),
            schema_version: schema_v35::VERSION,
            store_id_digest: source.store_id_digest.clone(),
        },
        source: source.clone(),
        migration_id,
    }
}

fn historical_migration_receipt(
    source: &ActiveStoreManifest,
    migration_id: String,
    phase: ReceiptPhase,
    target_version: u32,
) -> MigrationReceipt {
    MigrationReceipt {
        phase,
        staging_file: format!("{STAGING_PREFIX}{}.sqlite3", &migration_id[..16]),
        target: ActiveStoreManifest {
            file_name: format!("metadata-v{target_version}-{}.sqlite3", &migration_id[..16]),
            schema_version: target_version,
            store_id_digest: source.store_id_digest.clone(),
        },
        source: source.clone(),
        migration_id,
    }
}

fn build_historical_target(
    data_dir: &Path,
    source: &ActiveStoreManifest,
    staging_file: &str,
    target: &ActiveStoreManifest,
) {
    let key = read_key(data_dir).unwrap();
    let source_connection =
        open_encrypted_read_connection(&data_dir.join(&source.file_name), &key).unwrap();
    let staging_path = data_dir.join(staging_file);
    copy_encrypted_store(&source_connection, &staging_path, &key).unwrap();
    let mut staging = open_existing_encrypted_writer(&staging_path, &key).unwrap();
    forward_migration::apply_chain(&mut staging, source.schema_version, target.schema_version)
        .unwrap();
    validate_source_store(&staging, target).unwrap();
    drop(staging);
    let target_path = data_dir.join(&target.file_name);
    fs::rename(&staging_path, &target_path).unwrap();
    sync_parent_directory(data_dir).unwrap();
    validate_store_for_manifest(data_dir, &key, target).unwrap();
}

fn persist_exact_forward_receipt(data_dir: &Path, receipt: &MigrationReceipt) {
    let phase = match receipt.phase {
        ReceiptPhase::Preparing => "preparing",
        ReceiptPhase::Ready => "ready",
        ReceiptPhase::Published => "published",
    };
    let body = format!(
        "resume-ir.metadata-forward-migration-receipt.v1\nphase={phase}\nid={}\nsource_file={}\nsource_schema={}\nsource_digest={}\nstaging_file={}\ntarget_file={}\ntarget_schema={}\ntarget_digest={}\n",
        receipt.migration_id,
        receipt.source.file_name,
        receipt.source.schema_version,
        receipt.source.store_id_digest,
        receipt.staging_file,
        receipt.target.file_name,
        receipt.target.schema_version,
        receipt.target.store_id_digest,
    );
    let path = receipt::path(data_dir);
    fs::write(&path, body).unwrap();
    crate::restrict_private_file_permissions(&path).unwrap();
    sync_parent_directory(data_dir).unwrap();
}

fn persist_exact_initialization_receipt(
    data_dir: &Path,
    phase: &str,
    initialization_id: &str,
    digest: Option<&str>,
) {
    let body = format!(
        "resume-ir.metadata-initialization-receipt.v1\nphase={phase}\nid={initialization_id}\ndigest={}\n",
        digest.unwrap_or("-")
    );
    let path = initialization_receipt::path(data_dir);
    fs::write(&path, body).unwrap();
    crate::restrict_private_file_permissions(&path).unwrap();
    sync_parent_directory(data_dir).unwrap();
}

fn synthetic_document() -> Document {
    let now = UnixTimestamp::from_unix_seconds(1_800_000_000);
    Document {
        id: DocumentId::from_non_secret_parts(&["v33-cow-migration", "preserved"]),
        source_uri: "synthetic://v33-cow-migration/preserved".to_string(),
        normalized_path: "synthetic/v33-cow-migration/preserved.txt".to_string(),
        file_name: "preserved.txt".to_string(),
        extension: FileExtension::Txt,
        byte_size: 128,
        mtime: now,
        content_hash: None,
        text_hash: None,
        is_deleted: false,
        created_at: now,
        updated_at: now,
        status: DocumentStatus::Searchable,
    }
}

fn sha256_file(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}

fn snapshot_tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, current: &Path, snapshot: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if entry.file_type().unwrap().is_dir() {
                walk(root, &path, snapshot);
            } else {
                snapshot.insert(relative, fs::read(path).unwrap());
            }
        }
    }

    let mut snapshot = BTreeMap::new();
    walk(root, root, &mut snapshot);
    snapshot
}
