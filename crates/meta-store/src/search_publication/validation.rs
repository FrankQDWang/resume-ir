use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension};

use crate::{
    classification::resume_version_has_resume_candidate_classification_at_epoch_in_connection,
    document_status_to_storage, ActiveSearchProjection, ContentDigest, CurrentClassifierEpoch,
    DocumentStatus, MetaStoreError, Result, SearchProjectionDigest,
};

use super::model::{
    FullTextSnapshotDescriptor, SearchPublicationCommit, SearchPublicationDraft,
    SearchPublicationFailure, SearchPublicationRecord, SearchPublicationValidation,
    VectorSnapshotDescriptor, VectorSnapshotMode,
};

const MAX_GENERATION_BYTES: usize = 128;
const MAX_MODEL_ID_CHARS: usize = 128;
const MAX_VECTOR_DIMENSION: u32 = 65_536;

pub fn search_publication_fingerprint(
    classifier_epoch: &str,
    projection_digest: &SearchProjectionDigest,
    fulltext: &FullTextSnapshotDescriptor,
    vector: &VectorSnapshotDescriptor,
) -> ContentDigest {
    let mut canonical = Vec::new();
    append_fingerprint_part(&mut canonical, b"resume-ir.search-publication.v1");
    append_fingerprint_part(&mut canonical, classifier_epoch.as_bytes());
    append_fingerprint_part(&mut canonical, projection_digest.as_str().as_bytes());
    append_fingerprint_part(&mut canonical, fulltext.manifest_schema().as_bytes());
    append_fingerprint_part(&mut canonical, fulltext.index_schema().as_bytes());
    append_fingerprint_part(&mut canonical, &fulltext.document_count().to_le_bytes());
    append_fingerprint_part(
        &mut canonical,
        fulltext.projection_digest().as_str().as_bytes(),
    );
    append_fingerprint_part(
        &mut canonical,
        fulltext.logical_content_digest().as_str().as_bytes(),
    );
    append_fingerprint_part(&mut canonical, vector.manifest_schema().as_bytes());
    append_fingerprint_part(&mut canonical, vector.index_schema().as_bytes());
    append_fingerprint_part(&mut canonical, &vector.projection_count().to_le_bytes());
    append_fingerprint_part(
        &mut canonical,
        vector.projection_digest().as_str().as_bytes(),
    );
    append_fingerprint_part(&mut canonical, vector.coverage_digest().as_str().as_bytes());
    append_fingerprint_part(&mut canonical, &vector.vector_count().to_le_bytes());
    append_fingerprint_part(&mut canonical, &vector.document_count().to_le_bytes());
    append_fingerprint_part(&mut canonical, &vector.resume_version_count().to_le_bytes());
    append_fingerprint_part(
        &mut canonical,
        vector.logical_content_digest().as_str().as_bytes(),
    );
    match vector.mode() {
        VectorSnapshotMode::Disabled => append_fingerprint_part(&mut canonical, b"disabled"),
        VectorSnapshotMode::Enabled {
            model_id,
            dimension,
        } => {
            append_fingerprint_part(&mut canonical, b"enabled");
            append_fingerprint_part(&mut canonical, model_id.as_bytes());
            append_fingerprint_part(&mut canonical, &dimension.to_le_bytes());
        }
    }
    ContentDigest::from_bytes(&canonical)
}

fn append_fingerprint_part(target: &mut Vec<u8>, part: &[u8]) {
    target.extend_from_slice(&(part.len() as u64).to_le_bytes());
    target.extend_from_slice(part);
}

pub(super) fn validate_draft(draft: &SearchPublicationDraft) -> Result<()> {
    if !valid_generation(&draft.generation)
        || draft
            .base_generation
            .as_deref()
            .is_some_and(|base| !valid_generation(base) || base == draft.generation)
    {
        return Err(publication_error(
            SearchPublicationFailure::InvalidGeneration,
        ));
    }
    if CurrentClassifierEpoch::parse(&draft.classifier_epoch).is_none() {
        return Err(publication_error(
            SearchPublicationFailure::InvalidClassifierEpoch,
        ));
    }
    Ok(())
}

pub(super) fn validate_descriptors(validation: &SearchPublicationValidation<'_>) -> Result<()> {
    let fulltext = validation.fulltext;
    let vector = validation.vector;
    if !valid_generation(validation.generation)
        || fulltext.generation() != validation.generation
        || vector.generation() != validation.generation
        || fulltext.projection_digest() != vector.projection_digest()
        || fulltext.document_count() != vector.projection_count()
    {
        return Err(publication_error(
            SearchPublicationFailure::DescriptorMismatch,
        ));
    }
    if vector.document_count() > vector.projection_count()
        || vector.resume_version_count() != vector.document_count()
        || vector.vector_count() < vector.document_count()
    {
        return Err(publication_error(
            SearchPublicationFailure::InvalidDescriptor,
        ));
    }
    let empty_coverage = SearchProjectionDigest::from_pairs::<_, &str, &str>([])
        .map_err(|_| publication_error(SearchPublicationFailure::InvalidDescriptor))?;
    match vector.mode() {
        VectorSnapshotMode::Disabled
            if vector.vector_count() == 0
                && vector.document_count() == 0
                && vector.resume_version_count() == 0
                && vector.coverage_digest() == &empty_coverage => {}
        VectorSnapshotMode::Enabled {
            model_id,
            dimension,
        } if valid_model_id(model_id) && (1..=MAX_VECTOR_DIMENSION).contains(dimension) => {}
        VectorSnapshotMode::Disabled | VectorSnapshotMode::Enabled { .. } => {
            return Err(publication_error(
                SearchPublicationFailure::InvalidDescriptor,
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_commit_shape(commit: &SearchPublicationCommit<'_>) -> Result<()> {
    if !valid_generation(commit.generation) {
        return Err(publication_error(
            SearchPublicationFailure::InvalidGeneration,
        ));
    }
    let active = projection_map(commit.projections)?;
    projection_map(commit.vector_coverage)?;

    let mut documents = BTreeMap::new();
    for document in commit.terminal_documents {
        let is_projected = active.contains_key(document.document_id.as_str());
        let valid_terminal_shape = match document.terminal_status {
            DocumentStatus::Searchable => is_projected && !document.terminal_is_deleted,
            DocumentStatus::Deleted => !is_projected && document.terminal_is_deleted,
            DocumentStatus::Excluded | DocumentStatus::FailedPermanent => {
                !is_projected && !document.terminal_is_deleted
            }
            _ => false,
        };
        if documents
            .insert(document.document_id.as_str(), ())
            .is_some()
            || !valid_terminal_shape
        {
            return Err(publication_error(
                SearchPublicationFailure::InvalidDocumentState,
            ));
        }
    }

    Ok(())
}

pub(super) fn validate_commit_against_publication(
    connection: &Connection,
    commit: &SearchPublicationCommit<'_>,
    publication: &SearchPublicationRecord,
) -> Result<()> {
    let fulltext = publication
        .fulltext
        .as_ref()
        .ok_or_else(|| publication_error(SearchPublicationFailure::InvalidPersistedState))?;
    let vector = publication
        .vector
        .as_ref()
        .ok_or_else(|| publication_error(SearchPublicationFailure::InvalidPersistedState))?;
    let projection_digest = projection_digest(commit.projections)?;
    if projection_digest != publication.projection_digest
        || projection_digest != *fulltext.projection_digest()
        || projection_digest != *vector.projection_digest()
        || fulltext.document_count() != commit.projections.len() as u64
        || vector.projection_count() != commit.projections.len() as u64
    {
        return Err(publication_error(
            SearchPublicationFailure::ProjectionMismatch,
        ));
    }
    let active = projection_map(commit.projections)?;
    let coverage = projection_map(commit.vector_coverage)?;
    if coverage
        .iter()
        .any(|(document, version)| active.get(document) != Some(version))
        || coverage.len() as u64 != vector.document_count()
        || projection_digest_from_map(&coverage)? != *vector.coverage_digest()
    {
        return Err(publication_error(
            SearchPublicationFailure::VectorCoverageMismatch,
        ));
    }
    validate_projection_transitions(
        connection,
        &active,
        commit.terminal_documents,
        publication.base_generation.as_deref(),
    )?;
    for terminal in commit
        .terminal_documents
        .iter()
        .filter(|terminal| terminal.terminal_status == DocumentStatus::Searchable)
    {
        let Some(version_id) = active.get(terminal.document_id.as_str()) else {
            return Err(publication_error(
                SearchPublicationFailure::InvalidDocumentState,
            ));
        };
        let projected_content_hash = connection
            .query_row(
                "SELECT revision.content_hash
                 FROM resume_version AS version
                 JOIN source_revision AS revision
                   ON revision.id = version.source_revision_id
                 WHERE version.id = ?1 AND version.document_id = ?2",
                params![version_id, terminal.document_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(MetaStoreError::storage)?;
        if projected_content_hash.as_deref() != Some(terminal.expected_content_hash.as_str()) {
            return Err(publication_error(
                SearchPublicationFailure::InvalidDocumentState,
            ));
        }
    }
    for projection in commit.projections {
        if !resume_version_has_resume_candidate_classification_at_epoch_in_connection(
            connection,
            &projection.resume_version_id,
            &publication.classifier_epoch,
        )? {
            return Err(publication_error(
                SearchPublicationFailure::ExactClassificationMissing,
            ));
        }
    }
    Ok(())
}

fn validate_projection_transitions(
    connection: &Connection,
    target: &BTreeMap<&str, &str>,
    terminal_documents: &[super::model::TerminalDocumentUpdate],
    base_generation: Option<&str>,
) -> Result<()> {
    let mut statement = connection
        .prepare(
            "SELECT document_id, resume_version_id, generation
             FROM active_search_projection
             ORDER BY document_id",
        )
        .map_err(MetaStoreError::storage)?;
    let mut rows = statement.query([]).map_err(MetaStoreError::storage)?;
    let mut current = BTreeMap::new();
    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        let document_id = row.get::<_, String>(0).map_err(MetaStoreError::storage)?;
        let resume_version_id = row.get::<_, String>(1).map_err(MetaStoreError::storage)?;
        let generation = row.get::<_, String>(2).map_err(MetaStoreError::storage)?;
        if Some(generation.as_str()) != base_generation {
            return Err(publication_error(
                SearchPublicationFailure::InvalidPersistedState,
            ));
        }
        current.insert(document_id, resume_version_id);
    }

    let terminals = terminal_documents
        .iter()
        .map(|terminal| (terminal.document_id.as_str(), terminal.terminal_status))
        .collect::<BTreeMap<_, _>>();
    for (document_id, current_version) in &current {
        let terminal_status = terminals.get(document_id.as_str()).copied();
        let target_version = target.get(document_id.as_str()).copied();
        let valid_transition = match target_version {
            Some(target_version) if target_version == current_version => terminal_status.is_none(),
            Some(_) => terminal_status == Some(DocumentStatus::Searchable),
            None => terminal_status.is_some_and(is_stable_nonsearch_terminal),
        };
        if !valid_transition {
            return Err(publication_error(
                SearchPublicationFailure::InvalidProjectionTransition,
            ));
        }
    }
    for document_id in target.keys() {
        if current.contains_key(*document_id) {
            continue;
        }
        if terminals.get(*document_id).copied() != Some(DocumentStatus::Searchable) {
            return Err(publication_error(
                SearchPublicationFailure::InvalidProjectionTransition,
            ));
        }
    }
    Ok(())
}

fn is_stable_nonsearch_terminal(status: DocumentStatus) -> bool {
    matches!(
        status,
        DocumentStatus::Excluded | DocumentStatus::FailedPermanent | DocumentStatus::Deleted
    )
}

pub(super) fn validate_projected_document_states(
    connection: &Connection,
    projections: &[ActiveSearchProjection],
) -> Result<()> {
    for projection in projections {
        let state = connection
            .query_row(
                "SELECT document.is_deleted, document.status, document.content_hash,
                        revision.content_hash,
                        EXISTS (
                            SELECT 1 FROM active_search_projection AS active
                            WHERE active.document_id = ?1
                              AND active.resume_version_id = ?2
                        )
                 FROM document
                 JOIN resume_version AS version
                   ON version.document_id = document.id AND version.id = ?2
                 JOIN source_revision AS revision
                   ON revision.id = version.source_revision_id
                 WHERE document.id = ?1",
                params![
                    projection.document_id.as_str(),
                    projection.resume_version_id.as_str(),
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(MetaStoreError::storage)?;
        if state.is_none_or(
            |(is_deleted, status, document_hash, revision_hash, retained)| {
                is_deleted != 0
                    || status == document_status_to_storage(DocumentStatus::Deleted)
                    || retained == 0
                        && (status != document_status_to_storage(DocumentStatus::Searchable)
                            || document_hash.as_deref() != Some(revision_hash.as_str()))
            },
        ) {
            return Err(publication_error(
                SearchPublicationFailure::InvalidDocumentState,
            ));
        }
    }
    Ok(())
}

pub(super) fn projection_digest(
    projections: &[ActiveSearchProjection],
) -> Result<SearchProjectionDigest> {
    SearchProjectionDigest::from_pairs(projections.iter().map(|projection| {
        (
            projection.document_id.as_str(),
            projection.resume_version_id.as_str(),
        )
    }))
    .map_err(|_| publication_error(SearchPublicationFailure::ProjectionMismatch))
}

fn projection_map(projections: &[ActiveSearchProjection]) -> Result<BTreeMap<&str, &str>> {
    let mut mapping = BTreeMap::new();
    let mut versions = BTreeMap::new();
    for projection in projections {
        if mapping
            .insert(
                projection.document_id.as_str(),
                projection.resume_version_id.as_str(),
            )
            .is_some()
            || versions
                .insert(projection.resume_version_id.as_str(), ())
                .is_some()
        {
            return Err(publication_error(
                SearchPublicationFailure::ProjectionMismatch,
            ));
        }
    }
    Ok(mapping)
}

fn projection_digest_from_map(mapping: &BTreeMap<&str, &str>) -> Result<SearchProjectionDigest> {
    SearchProjectionDigest::from_pairs(
        mapping
            .iter()
            .map(|(document, version)| (*document, *version)),
    )
    .map_err(|_| publication_error(SearchPublicationFailure::ProjectionMismatch))
}

pub(super) fn vector_mode_storage(
    mode: &VectorSnapshotMode,
) -> (&'static str, Option<&str>, Option<u32>) {
    match mode {
        VectorSnapshotMode::Disabled => ("disabled", None, None),
        VectorSnapshotMode::Enabled {
            model_id,
            dimension,
        } => ("enabled", Some(model_id.as_str()), Some(*dimension)),
    }
}

fn valid_generation(generation: &str) -> bool {
    !generation.is_empty()
        && generation.len() <= MAX_GENERATION_BYTES
        && generation != "."
        && generation != ".."
        && !generation.starts_with('.')
        && generation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_model_id(model_id: &str) -> bool {
    let count = model_id.chars().count();
    (1..=MAX_MODEL_ID_CHARS).contains(&count)
        && !model_id.chars().any(char::is_control)
        && model_id.trim() == model_id
}

pub(super) fn u64_to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| publication_error(SearchPublicationFailure::InvalidDescriptor))
}

pub(super) fn publication_error(failure: SearchPublicationFailure) -> MetaStoreError {
    MetaStoreError::search_publication(failure)
}
