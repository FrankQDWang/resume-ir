use std::str::FromStr;

use rusqlite::{params, Connection};

use crate::{ContentDigest, MetaStoreError, Result, SearchProjectionDigest, UnixTimestamp};

use super::{
    model::{
        EnabledVectorSnapshotDescriptor, FullTextSnapshotDescriptor, SearchPublicationFailure,
        SearchPublicationRecord, SearchPublicationState, SearchPublicationValidation,
        VectorSnapshotDescriptor, FULLTEXT_INDEX_SCHEMA_V3, FULLTEXT_MANIFEST_SCHEMA_V3,
        VECTOR_INDEX_SCHEMA_V4, VECTOR_MANIFEST_SCHEMA_V4,
    },
    validation::{publication_error, search_publication_fingerprint, validate_descriptors},
};

const PUBLICATION_COLUMNS: &str = "generation, base_generation, expected_visible_epoch,
    classifier_epoch, projection_digest, publication_fingerprint, state,
    fulltext_generation, fulltext_manifest_schema, fulltext_index_schema,
    fulltext_document_count, fulltext_projection_digest, fulltext_logical_content_digest,
    vector_generation, vector_manifest_schema, vector_index_schema, vector_mode,
    vector_model_id, vector_dimension, vector_projection_count, vector_coverage_digest,
    vector_count, vector_document_count, vector_resume_version_count,
    vector_projection_digest, vector_logical_content_digest,
    created_at_seconds, updated_at_seconds";

pub(crate) fn search_publication_in_connection(
    connection: &Connection,
    generation: &str,
) -> Result<Option<SearchPublicationRecord>> {
    let sql = format!(
        "SELECT {PUBLICATION_COLUMNS} FROM search_publication_journal WHERE generation = ?1"
    );
    let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
    let mut rows = statement
        .query(params![generation])
        .map_err(MetaStoreError::storage)?;
    match rows.next().map_err(MetaStoreError::storage)? {
        Some(row) => read_publication(row).map(Some),
        None => Ok(None),
    }
}

pub(super) fn query_publications<P>(
    connection: &Connection,
    suffix: &str,
    params: P,
) -> Result<Vec<SearchPublicationRecord>>
where
    P: rusqlite::Params,
{
    let sql = format!("SELECT {PUBLICATION_COLUMNS} FROM search_publication_journal {suffix}");
    let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
    let mut rows = statement.query(params).map_err(MetaStoreError::storage)?;
    let mut publications = Vec::new();
    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        publications.push(read_publication(row)?);
    }
    Ok(publications)
}

fn read_publication(row: &rusqlite::Row<'_>) -> Result<SearchPublicationRecord> {
    let generation = row.get::<_, String>(0).map_err(MetaStoreError::storage)?;
    let state = parse_state(&row.get::<_, String>(6).map_err(MetaStoreError::storage)?)?;
    let fulltext_generation = row
        .get::<_, Option<String>>(7)
        .map_err(MetaStoreError::storage)?;
    let vector_generation = row
        .get::<_, Option<String>>(13)
        .map_err(MetaStoreError::storage)?;
    let fulltext = fulltext_generation
        .map(|generation| {
            let manifest_schema = row.get::<_, String>(8).map_err(MetaStoreError::storage)?;
            let index_schema = row.get::<_, String>(9).map_err(MetaStoreError::storage)?;
            if manifest_schema != FULLTEXT_MANIFEST_SCHEMA_V3
                || index_schema != FULLTEXT_INDEX_SCHEMA_V3
            {
                return Err(publication_error(
                    SearchPublicationFailure::InvalidPersistedState,
                ));
            }
            Ok(FullTextSnapshotDescriptor::new(
                generation,
                i64_to_u64(row.get::<_, i64>(10).map_err(MetaStoreError::storage)?)?,
                parse_projection_digest(
                    &row.get::<_, String>(11).map_err(MetaStoreError::storage)?,
                )?,
                parse_content_digest(&row.get::<_, String>(12).map_err(MetaStoreError::storage)?)?,
            ))
        })
        .transpose()?;
    let vector = vector_generation
        .map(|generation| read_vector_descriptor(row, generation))
        .transpose()?;
    let persisted_fingerprint = row
        .get::<_, Option<String>>(5)
        .map_err(MetaStoreError::storage)?
        .map(|value| parse_content_digest(&value))
        .transpose()?;
    if fulltext.is_some() != vector.is_some()
        || persisted_fingerprint.is_some() != fulltext.is_some()
        || matches!(state, SearchPublicationState::Preparing) && fulltext.is_some()
        || matches!(
            state,
            SearchPublicationState::Validated | SearchPublicationState::Ready
        ) && fulltext.is_none()
    {
        return Err(publication_error(
            SearchPublicationFailure::InvalidPersistedState,
        ));
    }
    let classifier_epoch = row.get::<_, String>(3).map_err(MetaStoreError::storage)?;
    let projection_digest =
        parse_projection_digest(&row.get::<_, String>(4).map_err(MetaStoreError::storage)?)?;
    if let (Some(persisted), Some(fulltext), Some(vector)) =
        (&persisted_fingerprint, &fulltext, &vector)
    {
        validate_descriptors(&SearchPublicationValidation {
            generation: &generation,
            fulltext,
            vector,
            now: UnixTimestamp::from_unix_seconds(0),
        })
        .map_err(|_| publication_error(SearchPublicationFailure::InvalidPersistedState))?;
        if projection_digest != *fulltext.projection_digest() {
            return Err(publication_error(
                SearchPublicationFailure::InvalidPersistedState,
            ));
        }
        if *persisted
            != search_publication_fingerprint(
                &classifier_epoch,
                &projection_digest,
                fulltext,
                vector,
            )
        {
            return Err(publication_error(
                SearchPublicationFailure::InvalidPersistedState,
            ));
        }
    }
    Ok(SearchPublicationRecord {
        generation,
        base_generation: row.get(1).map_err(MetaStoreError::storage)?,
        expected_visible_epoch: i64_to_u64(row.get::<_, i64>(2).map_err(MetaStoreError::storage)?)?,
        classifier_epoch,
        projection_digest,
        publication_fingerprint: persisted_fingerprint,
        state,
        fulltext,
        vector,
        created_at: UnixTimestamp::from_unix_seconds(row.get(26).map_err(MetaStoreError::storage)?),
        updated_at: UnixTimestamp::from_unix_seconds(row.get(27).map_err(MetaStoreError::storage)?),
    })
}

fn read_vector_descriptor(
    row: &rusqlite::Row<'_>,
    generation: String,
) -> Result<VectorSnapshotDescriptor> {
    let manifest_schema = row.get::<_, String>(14).map_err(MetaStoreError::storage)?;
    let index_schema = row.get::<_, String>(15).map_err(MetaStoreError::storage)?;
    if manifest_schema != VECTOR_MANIFEST_SCHEMA_V4 || index_schema != VECTOR_INDEX_SCHEMA_V4 {
        return Err(publication_error(
            SearchPublicationFailure::InvalidPersistedState,
        ));
    }
    let projection_count = i64_to_u64(row.get::<_, i64>(19).map_err(MetaStoreError::storage)?)?;
    let coverage_digest =
        parse_projection_digest(&row.get::<_, String>(20).map_err(MetaStoreError::storage)?)?;
    let vector_count = i64_to_u64(row.get::<_, i64>(21).map_err(MetaStoreError::storage)?)?;
    let document_count = i64_to_u64(row.get::<_, i64>(22).map_err(MetaStoreError::storage)?)?;
    let resume_version_count = i64_to_u64(row.get::<_, i64>(23).map_err(MetaStoreError::storage)?)?;
    let projection_digest =
        parse_projection_digest(&row.get::<_, String>(24).map_err(MetaStoreError::storage)?)?;
    let logical_content_digest =
        parse_content_digest(&row.get::<_, String>(25).map_err(MetaStoreError::storage)?)?;
    match row
        .get::<_, String>(16)
        .map_err(MetaStoreError::storage)?
        .as_str()
    {
        "disabled" if vector_count == 0 && document_count == 0 && resume_version_count == 0 => {
            if row
                .get::<_, Option<String>>(17)
                .map_err(MetaStoreError::storage)?
                .is_some()
                || row
                    .get::<_, Option<i64>>(18)
                    .map_err(MetaStoreError::storage)?
                    .is_some()
            {
                return Err(publication_error(
                    SearchPublicationFailure::InvalidPersistedState,
                ));
            }
            Ok(VectorSnapshotDescriptor::disabled(
                generation,
                projection_count,
                projection_digest,
                coverage_digest,
                logical_content_digest,
            ))
        }
        "enabled" => Ok(VectorSnapshotDescriptor::enabled(
            EnabledVectorSnapshotDescriptor {
                generation,
                model_id: row.get::<_, String>(17).map_err(MetaStoreError::storage)?,
                dimension: u32::try_from(row.get::<_, i64>(18).map_err(MetaStoreError::storage)?)
                    .map_err(|_| {
                    publication_error(SearchPublicationFailure::InvalidPersistedState)
                })?,
                projection_count,
                projection_digest,
                coverage_digest,
                vector_count,
                document_count,
                resume_version_count,
                logical_content_digest,
            },
        )),
        _ => Err(publication_error(
            SearchPublicationFailure::InvalidPersistedState,
        )),
    }
}

fn parse_state(value: &str) -> Result<SearchPublicationState> {
    match value {
        "preparing" => Ok(SearchPublicationState::Preparing),
        "validated" => Ok(SearchPublicationState::Validated),
        "ready" => Ok(SearchPublicationState::Ready),
        "abandoned" => Ok(SearchPublicationState::Abandoned),
        _ => Err(publication_error(
            SearchPublicationFailure::InvalidPersistedState,
        )),
    }
}

fn parse_projection_digest(value: &str) -> Result<SearchProjectionDigest> {
    SearchProjectionDigest::from_str(value)
        .map_err(|_| publication_error(SearchPublicationFailure::InvalidPersistedState))
}

fn parse_content_digest(value: &str) -> Result<ContentDigest> {
    ContentDigest::from_str(value)
        .map_err(|_| publication_error(SearchPublicationFailure::InvalidPersistedState))
}

fn i64_to_u64(value: i64) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| publication_error(SearchPublicationFailure::InvalidPersistedState))
}
