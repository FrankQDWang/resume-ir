use rusqlite::{params, Connection};

use crate::{MetaStoreError, Result, SearchProjectionDigest};

/// Validates the immutable active projection snapshot independently from the
/// artifact descriptor parser. This remains valid after v29 isolates legacy
/// descriptor payloads from ordinary readers.
pub(super) fn validate_active_projection_snapshot(
    connection: &Connection,
    generation: &str,
    classifier_epoch: &str,
    expected_count: u64,
    expected_digest: &SearchProjectionDigest,
) -> Result<()> {
    let mismatched = connection
        .query_row(
            "SELECT COUNT(*) FROM active_search_projection WHERE generation <> ?1",
            params![generation],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    let invalid = connection
        .query_row(
            "SELECT COUNT(*)
             FROM active_search_projection AS projection
             JOIN resume_version AS version
               ON version.id = projection.resume_version_id
              AND version.document_id = projection.document_id
             JOIN source_revision AS revision
               ON revision.id = version.source_revision_id
              AND revision.document_id = version.document_id
             LEFT JOIN resume_version_seal AS seal
               ON seal.resume_version_id = projection.resume_version_id
             LEFT JOIN resume_version_classification AS classification
               ON classification.resume_version_id = projection.resume_version_id
              AND classification.classifier_epoch = ?2
              AND classification.status = 'resume_candidate'
             WHERE projection.generation = ?1
               AND (seal.resume_version_id IS NULL
                 OR classification.resume_version_id IS NULL
                 OR projection.is_deleted <> 0 OR projection.status <> 'searchable'
                 OR projection.content_hash <> revision.content_hash
                 OR projection.byte_size <> revision.byte_size)",
            params![generation, classifier_epoch],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    let pairs = {
        let mut statement = connection
            .prepare(
                "SELECT document_id, resume_version_id FROM active_search_projection
                 WHERE generation = ?1 ORDER BY document_id, resume_version_id",
            )
            .map_err(MetaStoreError::storage)?;
        let pairs = statement
            .query_map(params![generation], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(MetaStoreError::storage)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(MetaStoreError::storage)?;
        pairs
    };
    let digest = SearchProjectionDigest::from_pairs(
        pairs
            .iter()
            .map(|(document, version)| (document.as_str(), version.as_str())),
    )
    .map_err(|_| MetaStoreError::storage_invariant())?;
    if mismatched != 0
        || invalid != 0
        || u64::try_from(pairs.len()).map_err(|_| MetaStoreError::storage_invariant())?
            != expected_count
        || digest != *expected_digest
    {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}
