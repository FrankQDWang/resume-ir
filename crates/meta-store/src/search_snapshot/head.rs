use std::{fmt, str::FromStr};

use rusqlite::{params, Connection, OptionalExtension};

use super::SearchMetadataSnapshot;
use crate::{
    search_publication::search_publication_in_connection, ActiveSearchProjection, DocumentId,
    MetaStoreError, Result, ResumeVersionId, SearchProjectionDigest, SearchPublicationRecord,
    SearchPublicationState, SearchRepairReason,
};

#[derive(Clone, PartialEq, Eq)]
pub struct SearchMetadataHead {
    pub generation: String,
    pub visible_epoch: u64,
    pub publication: SearchPublicationRecord,
}

impl fmt::Debug for SearchMetadataHead {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchMetadataHead")
            .field("generation", &"<redacted>")
            .field("visible_epoch", &self.visible_epoch)
            .field("publication", &self.publication)
            .finish()
    }
}

pub(super) enum SearchMetadataOpenError {
    Unavailable(super::SearchMetadataUnavailable),
    Store(MetaStoreError),
}

impl SearchMetadataSnapshot<'_> {
    pub fn active_projection_for_document(
        &self,
        document_id: &DocumentId,
    ) -> Result<Option<ActiveSearchProjection>> {
        active_projection_for_generation(self.connection, &self.head.generation, document_id)
    }

    /// Runs the O(n) exact projection audit and returns its ordered mapping.
    /// Composite query runtimes should call this only on a generation cache
    /// miss while opening artifact readers in this same metadata transaction;
    /// it is intentionally forbidden on per-request cache hits.
    pub fn validated_active_projections(&self) -> Result<Vec<ActiveSearchProjection>> {
        audit_active_projection(self.connection, &self.head)
    }
}

pub(super) fn read_ready_head(
    connection: &Connection,
) -> std::result::Result<SearchMetadataHead, SearchMetadataOpenError> {
    let (service_state, generation, visible_epoch, repair_reason) = connection
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
        .map_err(MetaStoreError::storage)
        .map_err(SearchMetadataOpenError::Store)?;
    if service_state != "ready" {
        let reason = repair_reason
            .as_deref()
            .and_then(search_repair_reason_from_storage)
            .ok_or_else(|| SearchMetadataOpenError::Store(MetaStoreError::storage_invariant()))?;
        let unavailable = match service_state.as_str() {
            "repairing" => super::SearchMetadataUnavailable::Repairing(reason),
            "repair_blocked" => super::SearchMetadataUnavailable::RepairBlocked(reason),
            _ => {
                return Err(SearchMetadataOpenError::Store(
                    MetaStoreError::storage_invariant(),
                ));
            }
        };
        return Err(SearchMetadataOpenError::Unavailable(unavailable));
    }
    if repair_reason.is_some() {
        return Err(SearchMetadataOpenError::Store(
            MetaStoreError::storage_invariant(),
        ));
    }
    let generation = generation
        .ok_or_else(|| SearchMetadataOpenError::Store(MetaStoreError::storage_invariant()))?;
    let publication = search_publication_in_connection(connection, &generation)
        .map_err(SearchMetadataOpenError::Store)?
        .filter(|publication| publication.state == SearchPublicationState::Ready)
        .ok_or_else(MetaStoreError::storage_invariant)
        .map_err(SearchMetadataOpenError::Store)?;
    let visible_epoch = u64::try_from(visible_epoch)
        .map_err(|_| SearchMetadataOpenError::Store(MetaStoreError::storage_invariant()))?;
    if publication.expected_visible_epoch.checked_add(1) != Some(visible_epoch) {
        return Err(SearchMetadataOpenError::Store(
            MetaStoreError::storage_invariant(),
        ));
    }
    Ok(SearchMetadataHead {
        generation,
        visible_epoch,
        publication,
    })
}

pub(super) fn audit_active_projection(
    connection: &Connection,
    head: &SearchMetadataHead,
) -> Result<Vec<ActiveSearchProjection>> {
    let mismatched = connection
        .query_row(
            "SELECT COUNT(*) FROM active_search_projection WHERE generation <> ?1",
            params![head.generation],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    let invalid_active = connection
        .query_row(
            "SELECT COUNT(*)
             FROM active_search_projection AS projection
             JOIN resume_version AS version
               ON version.id = projection.resume_version_id
              AND version.document_id = projection.document_id
             JOIN source_revision AS revision
               ON revision.id = version.source_revision_id
             LEFT JOIN resume_version_seal AS seal
               ON seal.resume_version_id = projection.resume_version_id
             LEFT JOIN resume_version_classification AS classification
               ON classification.resume_version_id = projection.resume_version_id
              AND classification.classifier_epoch = ?2
              AND classification.status = 'resume_candidate'
             WHERE projection.generation = ?1
               AND (
                   seal.resume_version_id IS NULL
                   OR classification.resume_version_id IS NULL
                   OR projection.is_deleted <> 0
                   OR projection.status <> 'searchable'
                   OR projection.content_hash <> revision.content_hash
                   OR projection.byte_size <> revision.byte_size
               )",
            params![head.generation, head.publication.classifier_epoch],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    let projections = active_projection_pairs(connection, &head.generation)?;
    let projection_digest = SearchProjectionDigest::from_pairs(
        projections
            .iter()
            .map(|(document, version)| (document.as_str(), version.as_str())),
    )
    .map_err(|_| MetaStoreError::storage_invariant())?;
    if mismatched != 0
        || invalid_active != 0
        || projections.len() as u64
            != head
                .publication
                .fulltext
                .as_ref()
                .map(|fulltext| fulltext.document_count())
                .ok_or_else(MetaStoreError::storage_invariant)?
        || projection_digest != head.publication.projection_digest
    {
        return Err(MetaStoreError::storage_invariant());
    }
    projections
        .into_iter()
        .map(|(document_id, resume_version_id)| {
            Ok(ActiveSearchProjection {
                document_id: DocumentId::from_str(&document_id).map_err(|_| {
                    MetaStoreError::invalid_value("active_search_projection.document_id")
                })?,
                resume_version_id: ResumeVersionId::from_str(&resume_version_id).map_err(|_| {
                    MetaStoreError::invalid_value("active_search_projection.resume_version_id")
                })?,
            })
        })
        .collect()
}

fn active_projection_pairs(
    connection: &Connection,
    generation: &str,
) -> Result<Vec<(String, String)>> {
    let mut statement = connection
        .prepare(
            "SELECT document_id, resume_version_id
             FROM active_search_projection
             WHERE generation = ?1
             ORDER BY document_id, resume_version_id",
        )
        .map_err(MetaStoreError::storage)?;
    let pairs = statement
        .query_map(params![generation], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(MetaStoreError::storage)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(MetaStoreError::storage)?;
    Ok(pairs)
}

fn search_repair_reason_from_storage(value: &str) -> Option<SearchRepairReason> {
    match value {
        "migration_rebuild" => Some(SearchRepairReason::MigrationRebuild),
        "artifact_unavailable" => Some(SearchRepairReason::ArtifactUnavailable),
        "source_unavailable" => Some(SearchRepairReason::SourceUnavailable),
        "runtime_invariant" => Some(SearchRepairReason::RuntimeInvariant),
        _ => None,
    }
}

pub(super) fn active_projection_for_generation(
    connection: &Connection,
    generation: &str,
    document_id: &DocumentId,
) -> Result<Option<ActiveSearchProjection>> {
    connection
        .query_row(
            "SELECT document_id, resume_version_id
             FROM active_search_projection
             WHERE generation = ?1 AND document_id = ?2",
            params![generation, document_id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .map(|(document, version)| {
            Ok(ActiveSearchProjection {
                document_id: DocumentId::from_str(&document).map_err(|_| {
                    MetaStoreError::invalid_value("active_search_projection.document_id")
                })?,
                resume_version_id: ResumeVersionId::from_str(&version).map_err(|_| {
                    MetaStoreError::invalid_value("active_search_projection.resume_version_id")
                })?,
            })
        })
        .transpose()
}
