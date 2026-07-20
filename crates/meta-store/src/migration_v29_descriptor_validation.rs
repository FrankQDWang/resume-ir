use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::{schema_v29, MetaStoreError, Result};

use super::projection_validation::validate_active_projection_snapshot;

#[path = "migration_v29_descriptor_records.rs"]
mod records;

#[cfg(any(test, feature = "migration-test-support"))]
pub(super) use records::legacy_fingerprint;
#[cfg(test)]
pub(super) use records::{
    descriptor_contract, CURRENT_FULLTEXT_MANIFEST, CURRENT_VECTOR_INDEX, CURRENT_VECTOR_MANIFEST,
};
pub(super) use records::{
    parse_projection_digest, publication_descriptor_records, required_u64, DescriptorContract,
    PublicationDescriptorRecord, LEGACY_FULLTEXT_INDEX, LEGACY_FULLTEXT_MANIFEST,
    LEGACY_VECTOR_INDEX, LEGACY_VECTOR_MANIFEST,
};

const CURRENT_REPAIR_AUTHORITY_TRIGGER: &str = "artifact_repair_context_insert_authority";
const MIGRATION_REPAIR_AUTHORITY_TRIGGER: &str =
    "artifact_repair_context_insert_migration_authority";

pub(super) fn apply_contract_migration(transaction: &Transaction<'_>) -> Result<()> {
    let permanent_authority = install_migration_repair_authority(transaction)?;
    let publications = publication_descriptor_records(transaction)?;
    let head = transaction
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
        .map_err(MetaStoreError::storage)?;

    if let Some(generation) = head.1.as_deref() {
        let publication = publications
            .iter()
            .find(|publication| publication.generation == generation)
            .ok_or_else(MetaStoreError::storage_invariant)?;
        validate_active_head(transaction, &head, publication)?;
        let repair_state = matches!(
            (head.0.as_str(), head.3.as_deref()),
            ("repairing", Some("artifact_unavailable"))
                | ("repair_blocked", Some("runtime_invariant"))
        );
        if publication.contract == DescriptorContract::Legacy || repair_state {
            if !matches!(
                publication.contract,
                DescriptorContract::Legacy | DescriptorContract::Current
            ) {
                return Err(MetaStoreError::storage_invariant());
            }
            insert_artifact_repair_context(transaction, publication, head.2)?;
        }
        if publication.contract == DescriptorContract::Legacy && head.0 == "ready" {
            let changed = transaction
                .execute(
                    "UPDATE search_projection_state
                     SET service_state = 'repairing', repair_reason = 'artifact_unavailable'
                     WHERE state_key = 'default' AND service_state = 'ready'
                       AND repair_reason IS NULL AND generation = ?1
                       AND visible_epoch = ?2",
                    params![generation, head.2],
                )
                .map_err(MetaStoreError::storage)?;
            if changed != 1 {
                return Err(MetaStoreError::storage_invariant());
            }
        }
    }

    transaction
        .execute_batch(schema_v29::DROP_LEGACY_ISOLATION_TRIGGERS)
        .map_err(MetaStoreError::migration)?;
    transaction
        .execute(
            "UPDATE search_publication_journal
             SET state = 'abandoned', publication_fingerprint = NULL,
                 fulltext_generation = NULL, fulltext_manifest_schema = NULL,
                 fulltext_index_schema = NULL, fulltext_document_count = NULL,
                 fulltext_projection_digest = NULL,
                 fulltext_logical_content_digest = NULL,
                 vector_generation = NULL, vector_manifest_schema = NULL,
                 vector_index_schema = NULL, vector_mode = NULL,
                 vector_model_id = NULL, vector_dimension = NULL,
                 vector_projection_count = NULL, vector_coverage_digest = NULL,
                 vector_count = NULL, vector_document_count = NULL,
                 vector_resume_version_count = NULL,
                 vector_projection_digest = NULL,
                 vector_logical_content_digest = NULL
             WHERE fulltext_manifest_schema = ?1 AND fulltext_index_schema = ?2
               AND vector_manifest_schema = ?3 AND vector_index_schema = ?4",
            params![
                LEGACY_FULLTEXT_MANIFEST,
                LEGACY_FULLTEXT_INDEX,
                LEGACY_VECTOR_MANIFEST,
                LEGACY_VECTOR_INDEX,
            ],
        )
        .map_err(MetaStoreError::storage)?;
    transaction
        .execute_batch(schema_v29::RESTORE_LEGACY_ISOLATION_TRIGGERS)
        .map_err(MetaStoreError::migration)?;
    restore_current_repair_authority(transaction, &permanent_authority)?;
    Ok(())
}

/// Proves that a completed v29 store carries the exact permanent current-only
/// trigger, with no migration-private authority left behind.
pub(super) fn validate_current_repair_authority_trigger(connection: &Connection) -> Result<()> {
    validated_current_repair_authority_sql(connection).map(drop)
}

fn install_migration_repair_authority(transaction: &Transaction<'_>) -> Result<String> {
    let permanent_authority = validated_current_repair_authority_sql(transaction)?;
    transaction
        .execute_batch(schema_v29::INSTALL_MIGRATION_REPAIR_CONTEXT_AUTHORITY)
        .map_err(MetaStoreError::migration)?;
    Ok(permanent_authority)
}

fn restore_current_repair_authority(
    transaction: &Transaction<'_>,
    permanent_authority: &str,
) -> Result<()> {
    let restore =
        format!("DROP TRIGGER {MIGRATION_REPAIR_AUTHORITY_TRIGGER};\n{permanent_authority};");
    transaction
        .execute_batch(&restore)
        .map_err(MetaStoreError::migration)?;
    validate_current_repair_authority_trigger(transaction)
}

fn validated_current_repair_authority_sql(connection: &Connection) -> Result<String> {
    let mut statement = connection
        .prepare(
            "SELECT name, sql FROM sqlite_master
             WHERE type = 'trigger' AND name IN (?1, ?2)
             ORDER BY name",
        )
        .map_err(MetaStoreError::storage)?;
    let definitions = statement
        .query_map(
            params![
                CURRENT_REPAIR_AUTHORITY_TRIGGER,
                MIGRATION_REPAIR_AUTHORITY_TRIGGER
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(MetaStoreError::storage)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(MetaStoreError::storage)?;
    let [(name, sql)] = definitions.as_slice() else {
        return Err(MetaStoreError::storage_invariant());
    };
    if name != CURRENT_REPAIR_AUTHORITY_TRIGGER
        || canonical_trigger_sql(sql)
            != canonical_trigger_sql(schema_v29::CURRENT_REPAIR_CONTEXT_AUTHORITY)
    {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(sql.clone())
}

fn canonical_trigger_sql(sql: &str) -> String {
    sql.trim()
        .trim_end_matches(';')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn validate_active_head(
    connection: &Connection,
    head: &(String, Option<String>, i64, Option<String>),
    publication: &PublicationDescriptorRecord,
) -> Result<()> {
    let valid_state = matches!(
        (head.0.as_str(), head.3.as_deref()),
        ("ready", None)
            | ("repairing", Some("artifact_unavailable"))
            | ("repair_blocked", Some("runtime_invariant"))
    );
    if !valid_state
        || publication.state != "ready"
        || publication.expected_visible_epoch.checked_add(1) != Some(required_u64(head.2)?)
        || publication.contract == DescriptorContract::None
    {
        return Err(MetaStoreError::storage_invariant());
    }
    validate_active_projection_snapshot(
        connection,
        &publication.generation,
        &publication.classifier_epoch,
        publication
            .fulltext_document_count
            .ok_or_else(MetaStoreError::storage_invariant)?,
        &publication.projection_digest,
    )
}

fn insert_artifact_repair_context(
    connection: &Connection,
    publication: &PublicationDescriptorRecord,
    visible_epoch: i64,
) -> Result<()> {
    let changed = connection
        .execute(
            "INSERT INTO artifact_repair_context (
                state_key, generation, publication_fingerprint, visible_epoch,
                classifier_epoch, projection_digest, projection_count,
                vector_mode, vector_model_id, vector_dimension,
                created_at_seconds, updated_at_seconds
             ) VALUES ('default', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, 0)",
            params![
                publication.generation,
                publication
                    .publication_fingerprint
                    .as_ref()
                    .ok_or_else(MetaStoreError::storage_invariant)?
                    .as_str(),
                visible_epoch,
                publication.classifier_epoch,
                publication.projection_digest.as_str(),
                i64::try_from(
                    publication
                        .fulltext_document_count
                        .ok_or_else(MetaStoreError::storage_invariant)?
                )
                .map_err(|_| MetaStoreError::storage_invariant())?,
                publication.vector_mode,
                publication.vector_model_id,
                publication.vector_dimension.map(i64::from),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if changed != 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

pub(super) fn validate_persisted_repair_context(connection: &Connection) -> Result<()> {
    let context = connection
        .query_row(
            "SELECT generation, publication_fingerprint, visible_epoch,
                    classifier_epoch, projection_digest, projection_count
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
                ))
            },
        )
        .optional()
        .map_err(MetaStoreError::storage)?;
    let Some((generation, _, epoch, classifier_epoch, projection_digest, count)) = context else {
        return Ok(());
    };
    let head_matches = connection
        .query_row(
            "SELECT EXISTS (
                 SELECT 1 FROM search_projection_state
                 WHERE state_key = 'default' AND generation = ?1 AND visible_epoch = ?2
                   AND ((service_state = 'repairing' AND repair_reason = 'artifact_unavailable')
                     OR (service_state = 'repair_blocked' AND repair_reason = 'runtime_invariant'))
             )",
            params![generation, epoch],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?
        == 1;
    if !head_matches {
        return Err(MetaStoreError::storage_invariant());
    }
    validate_active_projection_snapshot(
        connection,
        &generation,
        &classifier_epoch,
        required_u64(count)?,
        &parse_projection_digest(projection_digest)?,
    )
}

#[cfg(test)]
#[path = "migration_v29_descriptor_validation_tests.rs"]
mod tests;
