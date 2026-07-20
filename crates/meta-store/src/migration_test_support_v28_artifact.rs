use std::{fmt, fs, path::Path, sync::Arc};

use crate::migration_v29::fixture_support::{
    rewrite_current_publication_as_legacy_fixture, LegacyArtifactFixtureHead,
};
use crate::{
    active_store_manifest::{read_manifest, MANIFEST_FILE},
    migration_v27::{open_encrypted_connection, sync_validated_store},
    migration_v28, schema_v28, ActiveSearchProjection, ClassificationStatus, ContentDigest,
    DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, Document, DocumentId, DocumentStatus,
    FileExtension, FullTextSnapshotDescriptor, ImmutableIngestStage, ImportProcessingContract,
    MetadataEncryptionState, MigrationRebuildContractActivation, OwnedMetaStore,
    ProjectedDocumentSnapshot, ReasonCode, Result, ResumeVersion, ResumeVersionClassification,
    ResumeVersionId, ReviewDisposition, SearchProjectionDigest, SearchPublicationCommit,
    SearchPublicationDraft, SearchPublicationOutcome, SearchPublicationValidation, SourceRevision,
    TerminalDocumentUpdate, UnixTimestamp, VectorSnapshotDescriptor, CLASSIFIER_EPOCH,
};

const FIXTURE_GENERATION: &str = "synthetic-v28-legacy-artifact-generation";

/// Exact active-head shape to seed before the v28-to-v29 COW migration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum V28ArtifactRepairHead {
    Ready,
    Repairing,
    Blocked,
}

/// Opaque synthetic identities retained across the artifact repair cut.
pub struct V28LegacyArtifactRepairFixtureFacts {
    generation: String,
    document_id: DocumentId,
    resume_version_id: ResumeVersionId,
    inherited_visible_epoch: u64,
}

impl V28LegacyArtifactRepairFixtureFacts {
    pub fn generation(&self) -> &str {
        &self.generation
    }

    pub fn document_id(&self) -> &DocumentId {
        &self.document_id
    }

    pub fn resume_version_id(&self) -> &ResumeVersionId {
        &self.resume_version_id
    }

    pub fn inherited_visible_epoch(&self) -> u64 {
        self.inherited_visible_epoch
    }
}

impl fmt::Debug for V28LegacyArtifactRepairFixtureFacts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("V28LegacyArtifactRepairFixtureFacts")
            .field("generation", &"<synthetic>")
            .field("document_id", &"<synthetic>")
            .field("resume_version_id", &"<synthetic>")
            .field("inherited_visible_epoch", &self.inherited_visible_epoch)
            .finish()
    }
}

/// Seeds one encrypted v28 store with an exact, internally consistent legacy
/// artifact publication. The fixture is synthetic-only and releases ownership
/// before returning so a daemon process can exercise the real v29 COW path.
pub fn seed_v28_legacy_artifact_repair_fixture(
    data_dir: &Path,
    head: V28ArtifactRepairHead,
) -> Result<V28LegacyArtifactRepairFixtureFacts> {
    fs::create_dir_all(data_dir).map_err(crate::MetaStoreError::io_storage)?;
    let owner = acquire_owner(data_dir)?;
    let store = open_v28_fixture_store(&owner)?;
    if store.schema_version()? != schema_v28::VERSION {
        return Err(crate::MetaStoreError::storage_invariant());
    }

    let (document, revision, version, classification) = classified_fixture();
    store.stage_immutable_ingest(ImmutableIngestStage::ClassifiedResume {
        document: &document,
        source_revision: &revision,
        version: &version,
        classification: &classification,
        mentions: &[],
        email_hash: None,
        phone_hash: None,
    })?;
    let contract = ImportProcessingContract::new(
        "synthetic-v28-artifact-parser",
        "synthetic-v28-artifact-ocr",
        "synthetic-v28-artifact-schema",
        CLASSIFIER_EPOCH,
    )?;
    if store.activate_migration_rebuild_contract(&contract, timestamp(10))?
        != MigrationRebuildContractActivation::Activated
    {
        return Err(crate::MetaStoreError::storage_invariant());
    }
    let barrier = store
        .acquire_migration_rebuild_barrier_token(contract.id())?
        .ok_or_else(crate::MetaStoreError::storage_invariant)?;
    let projection = ActiveSearchProjection {
        document_id: document.id.clone(),
        resume_version_id: version.id.clone(),
    };
    let session = store.into_search_publication_session_without_prepare_for_test()?;
    publish_initial_projection(
        &session,
        &document,
        std::slice::from_ref(&projection),
        &barrier,
    )?;
    rewrite_current_publication_as_legacy_fixture(
        &mut session.owned_store().connection.borrow_mut(),
        FIXTURE_GENERATION,
        match head {
            V28ArtifactRepairHead::Ready => LegacyArtifactFixtureHead::Ready,
            V28ArtifactRepairHead::Repairing => LegacyArtifactFixtureHead::Repairing,
            V28ArtifactRepairHead::Blocked => LegacyArtifactFixtureHead::Blocked,
        },
    )?;

    let manifest = read_manifest(&data_dir.join(MANIFEST_FILE))?;
    let store_path = data_dir.join(manifest.file_name);
    drop(session);
    drop(owner);
    sync_validated_store(&store_path)?;

    Ok(V28LegacyArtifactRepairFixtureFacts {
        generation: FIXTURE_GENERATION.to_string(),
        document_id: document.id,
        resume_version_id: version.id,
        inherited_visible_epoch: 1,
    })
}

pub(super) fn open_v28_fixture_store(owner: &DataDirectoryOwnerLease) -> Result<OwnedMetaStore> {
    let owner_guard = owner.shared_guard();
    let (store_path, key) = migration_v28::prepare_active_v28_store(&owner_guard)?;
    let connection = open_encrypted_connection(&store_path, &key)?;
    OwnedMetaStore::from_owned_connection(
        connection,
        MetadataEncryptionState::SqlCipher,
        Arc::clone(&owner_guard),
    )
}

fn acquire_owner(data_dir: &Path) -> Result<DataDirectoryOwnerLease> {
    match DataDirectoryOwnerLease::try_acquire(data_dir)
        .map_err(|_| crate::MetaStoreError::storage_invariant())?
    {
        DataDirectoryOwnerAcquisition::Acquired(owner) => Ok(owner),
        DataDirectoryOwnerAcquisition::Contended => {
            Err(crate::MetaStoreError::migration_ownership_required())
        }
    }
}

fn classified_fixture() -> (
    Document,
    SourceRevision,
    ResumeVersion,
    ResumeVersionClassification,
) {
    let source = b"synthetic v28 legacy artifact resume";
    let now = timestamp(1);
    let mut document = Document {
        id: DocumentId::from_non_secret_parts(&["migration-test-support", "v28-artifact"]),
        source_uri: "synthetic://migration-test-support/v28-artifact.txt".to_string(),
        normalized_path: "synthetic/v28-artifact.txt".to_string(),
        file_name: "v28-artifact.txt".to_string(),
        extension: FileExtension::Txt,
        byte_size: source.len() as u64,
        mtime: now,
        content_hash: None,
        text_hash: None,
        is_deleted: false,
        created_at: now,
        updated_at: now,
        status: DocumentStatus::FieldsExtracted,
    };
    let revision = SourceRevision::for_content(
        document.id.clone(),
        ContentDigest::from_bytes(source),
        source.len() as u64,
    );
    document.content_hash = Some(revision.content_hash.as_str().to_string());
    let normalized_text = "synthetic normalized v28 artifact resume";
    let normalized_text_hash = ContentDigest::from_bytes(normalized_text.as_bytes());
    let version = ResumeVersion {
        id: ResumeVersionId::from_content_identity(
            &document.id,
            &revision.id,
            &normalized_text_hash,
            "synthetic-v28-artifact-parser",
            "synthetic-v28-artifact-schema",
        ),
        document_id: document.id.clone(),
        source_revision_id: revision.id.clone(),
        normalized_text_hash,
        parse_version: "synthetic-v28-artifact-parser".to_string(),
        schema_version: "synthetic-v28-artifact-schema".to_string(),
        language_set: vec!["en".to_string()],
        page_count: Some(1),
        raw_text: None,
        clean_text: Some(normalized_text.to_string()),
        quality_score: Some(0.9),
    };
    let classification = ResumeVersionClassification {
        resume_version_id: version.id.clone(),
        status: ClassificationStatus::ResumeCandidate,
        classifier_epoch: CLASSIFIER_EPOCH.to_string(),
        reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
        classified_at: timestamp(2),
        review_disposition: ReviewDisposition::NotRequired,
    };
    (document, revision, version, classification)
}

fn publish_initial_projection(
    session: &crate::SearchPublicationSession,
    document: &Document,
    projections: &[ActiveSearchProjection],
    barrier: &crate::MigrationRebuildBarrierToken,
) -> Result<()> {
    let digest = projection_digest(projections)?;
    if session.begin_legacy_v28_search_publication_for_test(&SearchPublicationDraft {
        generation: FIXTURE_GENERATION.to_string(),
        base_generation: None,
        expected_visible_epoch: 0,
        classifier_epoch: CLASSIFIER_EPOCH.to_string(),
        projection_digest: digest.clone(),
        now: timestamp(20),
    })? != SearchPublicationOutcome::Applied
    {
        return Err(crate::MetaStoreError::storage_invariant());
    }
    let fulltext = FullTextSnapshotDescriptor::new(
        FIXTURE_GENERATION.to_string(),
        projections.len() as u64,
        digest.clone(),
        ContentDigest::from_bytes(b"synthetic-v28-artifact-fulltext"),
    );
    let vector = VectorSnapshotDescriptor::disabled(
        FIXTURE_GENERATION.to_string(),
        projections.len() as u64,
        digest,
        SearchProjectionDigest::from_pairs::<_, &str, &str>([])
            .map_err(|_| crate::MetaStoreError::storage_invariant())?,
        ContentDigest::from_bytes(b"synthetic-v28-artifact-vector"),
    );
    session.validate_search_publication(&SearchPublicationValidation {
        generation: FIXTURE_GENERATION,
        fulltext: &fulltext,
        vector: &vector,
        now: timestamp(21),
    })?;
    let terminal = [TerminalDocumentUpdate {
        document_id: document.id.clone(),
        expected_status: DocumentStatus::FieldsExtracted,
        expected_is_deleted: false,
        expected_content_hash: ContentDigest::from_bytes(b"synthetic v28 legacy artifact resume"),
        terminal_status: DocumentStatus::Searchable,
        terminal_is_deleted: false,
    }];
    let mut searchable = document.clone();
    searchable.status = DocumentStatus::Searchable;
    searchable.updated_at = timestamp(22);
    let projected = [ProjectedDocumentSnapshot::Replacement {
        projection: projections[0].clone(),
        document: searchable,
    }];
    if session.commit_migration_rebuild_search_publication(
        &SearchPublicationCommit {
            generation: FIXTURE_GENERATION,
            terminal_documents: &terminal,
            projections,
            projected_documents: &projected,
            vector_coverage: &[],
            now: timestamp(22),
        },
        barrier,
    )? != SearchPublicationOutcome::Applied
    {
        return Err(crate::MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn projection_digest(projections: &[ActiveSearchProjection]) -> Result<SearchProjectionDigest> {
    SearchProjectionDigest::from_pairs(projections.iter().map(|projection| {
        (
            projection.document_id.as_str(),
            projection.resume_version_id.as_str(),
        )
    }))
    .map_err(|_| crate::MetaStoreError::storage_invariant())
}

fn timestamp(seconds: i64) -> UnixTimestamp {
    UnixTimestamp::from_unix_seconds(seconds)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::{schema_v29, SearchProjectionServiceState, SearchRepairReason};

    #[test]
    fn public_v28_legacy_fixture_covers_each_migratable_head_shape() {
        for (head, expected_state, expected_reason) in [
            (
                V28ArtifactRepairHead::Ready,
                SearchProjectionServiceState::Repairing,
                Some(SearchRepairReason::ArtifactUnavailable),
            ),
            (
                V28ArtifactRepairHead::Repairing,
                SearchProjectionServiceState::Repairing,
                Some(SearchRepairReason::ArtifactUnavailable),
            ),
            (
                V28ArtifactRepairHead::Blocked,
                SearchProjectionServiceState::RepairBlocked,
                Some(SearchRepairReason::RuntimeInvariant),
            ),
        ] {
            let directory = tempdir().unwrap();
            let data_dir = directory.path().join("data");
            let facts = seed_v28_legacy_artifact_repair_fixture(&data_dir, head).unwrap();
            let owner = acquire_owner(&data_dir).unwrap();
            let store = owner.open_store().unwrap();

            assert_eq!(store.schema_version().unwrap(), schema_v29::VERSION);
            let state = store.search_projection_state().unwrap();
            assert_eq!(state.generation.as_deref(), Some(facts.generation()));
            assert_eq!(state.visible_epoch, facts.inherited_visible_epoch());
            assert_eq!(state.service_state, expected_state);
            assert_eq!(state.repair_reason, expected_reason);
            assert!(store.artifact_repair_context().unwrap().is_some());
        }
    }
}
