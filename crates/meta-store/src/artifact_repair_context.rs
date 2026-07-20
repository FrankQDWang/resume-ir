use std::{fmt, str::FromStr};

use rusqlite::{params, OptionalExtension, TransactionBehavior};

use crate::{
    ContentDigest, MetaStoreError, MetadataStore, MetadataStoreAccess, MetadataStoreWriteAccess,
    Result, SearchProjectionDigest, SearchProjectionTransitionOutcome, SearchPublicationState,
    UnixTimestamp, FULLTEXT_INDEX_SCHEMA_V3, FULLTEXT_MANIFEST_SCHEMA_V3, VECTOR_INDEX_SCHEMA_V4,
    VECTOR_MANIFEST_SCHEMA_V4,
};

#[derive(Clone, PartialEq, Eq)]
pub enum ArtifactRepairVectorContext {
    Disabled,
    Enabled { model_id: String, dimension: u32 },
}

impl fmt::Debug for ArtifactRepairVectorContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => formatter.write_str("Disabled"),
            Self::Enabled { dimension, .. } => formatter
                .debug_struct("Enabled")
                .field("model_id", &"<redacted>")
                .field("dimension", dimension)
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ArtifactRepairContext {
    pub generation: String,
    pub publication_fingerprint: ContentDigest,
    pub visible_epoch: u64,
    pub classifier_epoch: String,
    pub projection_digest: SearchProjectionDigest,
    pub projection_count: u64,
    pub vector: ArtifactRepairVectorContext,
}

impl fmt::Debug for ArtifactRepairContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArtifactRepairContext")
            .field("generation", &"<redacted>")
            .field("publication_fingerprint", &"<redacted>")
            .field("visible_epoch", &self.visible_epoch)
            .field("classifier_epoch", &"<redacted>")
            .field("projection_digest", &self.projection_digest)
            .field("projection_count", &self.projection_count)
            .field("vector", &self.vector)
            .finish()
    }
}

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    pub fn artifact_repair_context(&self) -> Result<Option<ArtifactRepairContext>> {
        let record = self
            .connection
            .borrow()
            .query_row(
                "SELECT generation, publication_fingerprint, visible_epoch,
                        classifier_epoch, projection_digest, projection_count,
                        vector_mode, vector_model_id, vector_dimension
                 FROM artifact_repair_context WHERE state_key = 'default'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<i64>>(8)?,
                    ))
                },
            )
            .optional()
            .map_err(MetaStoreError::storage)?;
        record
            .map(
                |(
                    generation,
                    fingerprint,
                    epoch,
                    classifier_epoch,
                    projection_digest,
                    projection_count,
                    vector_mode,
                    model_id,
                    dimension,
                )| {
                    let vector = match (vector_mode.as_str(), model_id, dimension) {
                        ("disabled", None, None) => ArtifactRepairVectorContext::Disabled,
                        ("enabled", Some(model_id), Some(dimension)) => {
                            ArtifactRepairVectorContext::Enabled {
                                model_id,
                                dimension: u32::try_from(dimension)
                                    .map_err(|_| MetaStoreError::storage_invariant())?,
                            }
                        }
                        _ => return Err(MetaStoreError::storage_invariant()),
                    };
                    Ok(ArtifactRepairContext {
                        generation,
                        publication_fingerprint: ContentDigest::from_str(&fingerprint)
                            .map_err(|_| MetaStoreError::storage_invariant())?,
                        visible_epoch: u64::try_from(epoch)
                            .map_err(|_| MetaStoreError::storage_invariant())?,
                        classifier_epoch,
                        projection_digest: SearchProjectionDigest::from_str(&projection_digest)
                            .map_err(|_| MetaStoreError::storage_invariant())?,
                        projection_count: u64::try_from(projection_count)
                            .map_err(|_| MetaStoreError::storage_invariant())?,
                        vector,
                    })
                },
            )
            .transpose()
    }

    /// Starts or resumes artifact repair only while the exact published head
    /// observed by the caller is still current. Repair-blocked and migration
    /// states are sticky and cannot be reopened through this transition.
    pub fn begin_artifact_repair(
        &self,
        expected_generation: &str,
        expected_visible_epoch: u64,
        now: UnixTimestamp,
    ) -> Result<SearchProjectionTransitionOutcome>
    where
        Access: MetadataStoreWriteAccess,
    {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let expected_epoch = u64_to_i64(
            expected_visible_epoch,
            "search_projection_state.visible_epoch",
        )?;
        let resumes_bound_context = transaction
            .query_row(
                "SELECT EXISTS (
                     SELECT 1 FROM artifact_repair_context AS context
                     JOIN search_projection_state AS head
                       ON head.state_key = context.state_key
                      AND head.generation = context.generation
                      AND head.visible_epoch = context.visible_epoch
                     WHERE context.state_key = 'default'
                       AND context.generation = ?1 AND context.visible_epoch = ?2
                       AND head.service_state = 'repairing'
                       AND head.repair_reason = 'artifact_unavailable'
                 )",
                params![expected_generation, expected_epoch],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?
            == 1;
        if !resumes_bound_context {
            let publication = crate::search_publication::search_publication_in_connection(
                &transaction,
                expected_generation,
            )?;
            let exact_current_publication = publication.as_ref().is_some_and(|publication| {
                publication.state == SearchPublicationState::Ready
                    && publication.expected_visible_epoch.checked_add(1)
                        == Some(expected_visible_epoch)
                    && publication.publication_fingerprint.is_some()
            });
            if !exact_current_publication {
                transaction.commit().map_err(MetaStoreError::storage)?;
                return Ok(SearchProjectionTransitionOutcome::Superseded);
            }
        }
        transaction
            .execute(
                "INSERT INTO artifact_repair_context (
                    state_key, generation, publication_fingerprint, visible_epoch,
                    classifier_epoch, projection_digest, projection_count,
                    vector_mode, vector_model_id, vector_dimension,
                    created_at_seconds, updated_at_seconds
                 )
                 SELECT 'default', publication.generation,
                        publication.publication_fingerprint, head.visible_epoch,
                        publication.classifier_epoch, publication.projection_digest,
                        publication.fulltext_document_count, publication.vector_mode,
                        publication.vector_model_id, publication.vector_dimension, ?1, ?1
                 FROM search_projection_state AS head
                 JOIN search_publication_journal AS publication
                   ON publication.generation = head.generation
                 WHERE head.state_key = 'default'
                   AND head.service_state = 'ready' AND head.repair_reason IS NULL
                   AND head.generation = ?2 AND head.visible_epoch = ?3
                   AND publication.state = 'ready'
                   AND publication.fulltext_manifest_schema = ?4
                   AND publication.fulltext_index_schema = ?5
                   AND publication.vector_manifest_schema = ?6
                   AND publication.vector_index_schema = ?7
                   AND publication.fulltext_document_count = publication.vector_projection_count
                 ON CONFLICT(state_key) DO NOTHING",
                params![
                    now.as_unix_seconds(),
                    expected_generation,
                    expected_epoch,
                    FULLTEXT_MANIFEST_SCHEMA_V3,
                    FULLTEXT_INDEX_SCHEMA_V3,
                    VECTOR_MANIFEST_SCHEMA_V4,
                    VECTOR_INDEX_SCHEMA_V4,
                ],
            )
            .map_err(MetaStoreError::storage)?;
        let exact_context = transaction
            .query_row(
                "SELECT EXISTS (
                     SELECT 1 FROM artifact_repair_context
                     WHERE state_key = 'default' AND generation = ?1
                       AND visible_epoch = ?2
                 )",
                params![expected_generation, expected_epoch],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?
            == 1;
        if !exact_context {
            transaction.commit().map_err(MetaStoreError::storage)?;
            return Ok(SearchProjectionTransitionOutcome::Superseded);
        }
        let changed = transaction
            .execute(
                "UPDATE search_projection_state
                 SET service_state = 'repairing', repair_reason = 'artifact_unavailable',
                     updated_at_seconds = MAX(updated_at_seconds, ?1)
                 WHERE state_key = 'default'
                   AND generation = ?2
                   AND visible_epoch = ?3
                   AND (
                       (service_state = 'ready' AND repair_reason IS NULL)
                       OR (service_state = 'repairing'
                           AND repair_reason = 'artifact_unavailable')
                   )",
                params![now.as_unix_seconds(), expected_generation, expected_epoch,],
            )
            .map_err(MetaStoreError::storage)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(if changed == 1 {
            SearchProjectionTransitionOutcome::Applied
        } else {
            SearchProjectionTransitionOutcome::Superseded
        })
    }

    /// Blocks only the exact published artifact repair attempt observed by
    /// the caller. The immutable generation and visible epoch are preserved so
    /// a stale worker can never overwrite a newer ready head.
    pub fn block_artifact_repair(
        &self,
        expected_generation: &str,
        expected_publication_fingerprint: &ContentDigest,
        expected_visible_epoch: u64,
        now: UnixTimestamp,
    ) -> Result<SearchProjectionTransitionOutcome>
    where
        Access: MetadataStoreWriteAccess,
    {
        let changed = self
            .connection
            .borrow()
            .execute(
                "UPDATE search_projection_state
                 SET service_state = 'repair_blocked', repair_reason = 'runtime_invariant',
                     updated_at_seconds = MAX(updated_at_seconds, ?1)
                 WHERE state_key = 'default'
                   AND service_state = 'repairing'
                   AND repair_reason = 'artifact_unavailable'
                   AND generation = ?2
                   AND visible_epoch = ?3
                   AND EXISTS (
                       SELECT 1
                       FROM artifact_repair_context context
                       WHERE context.state_key = search_projection_state.state_key
                         AND context.generation = search_projection_state.generation
                         AND context.visible_epoch = search_projection_state.visible_epoch
                         AND context.publication_fingerprint = ?4
                   )",
                params![
                    now.as_unix_seconds(),
                    expected_generation,
                    u64_to_i64(
                        expected_visible_epoch,
                        "search_projection_state.visible_epoch"
                    )?,
                    expected_publication_fingerprint.as_str(),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        Ok(if changed == 1 {
            SearchProjectionTransitionOutcome::Applied
        } else {
            SearchProjectionTransitionOutcome::Superseded
        })
    }
}

fn u64_to_i64(value: u64, field: &'static str) -> Result<i64> {
    i64::try_from(value).map_err(|_| MetaStoreError::invalid_value(field))
}
