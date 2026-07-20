use rusqlite::{params, Connection, TransactionBehavior};

use crate::{schema_v29, MetaStoreError, Result};

use super::descriptor_validation::{
    legacy_fingerprint, publication_descriptor_records, validate_active_head,
    LEGACY_FULLTEXT_INDEX, LEGACY_FULLTEXT_MANIFEST, LEGACY_VECTOR_INDEX, LEGACY_VECTOR_MANIFEST,
};

/// Closed synthetic head state accepted by the cross-crate v28 fixture seam.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LegacyArtifactFixtureHead {
    Ready,
    Repairing,
    Blocked,
}

/// Rewrites one validated current v28 publication to the exact legacy
/// descriptor contract consumed by the v29 COW migration.
pub(crate) fn rewrite_current_publication_as_legacy_fixture(
    connection: &mut Connection,
    generation: &str,
    head: LegacyArtifactFixtureHead,
) -> Result<()> {
    let records = publication_descriptor_records(connection)?;
    if records.len() != 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    let fingerprint = legacy_fingerprint(&records[0])?;
    let restore_triggers = publication_trigger_restore_sql(connection)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(MetaStoreError::storage)?;
    transaction
        .execute_batch(schema_v29::DROP_LEGACY_ISOLATION_TRIGGERS)
        .map_err(MetaStoreError::migration)?;
    let changed = transaction
        .execute(
            "UPDATE search_publication_journal
             SET publication_fingerprint = ?1,
                 fulltext_manifest_schema = ?2, fulltext_index_schema = ?3,
                 vector_manifest_schema = ?4, vector_index_schema = ?5
             WHERE generation = ?6",
            params![
                fingerprint.as_str(),
                LEGACY_FULLTEXT_MANIFEST,
                LEGACY_FULLTEXT_INDEX,
                LEGACY_VECTOR_MANIFEST,
                LEGACY_VECTOR_INDEX,
                generation,
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if changed != 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    transaction
        .execute_batch(&restore_triggers)
        .map_err(MetaStoreError::migration)?;
    transaction.commit().map_err(MetaStoreError::storage)?;

    let (service_state, repair_reason) = match head {
        LegacyArtifactFixtureHead::Ready => ("ready", None),
        LegacyArtifactFixtureHead::Repairing => ("repairing", Some("artifact_unavailable")),
        LegacyArtifactFixtureHead::Blocked => ("repair_blocked", Some("runtime_invariant")),
    };
    let changed = connection
        .execute(
            "UPDATE search_projection_state
             SET service_state = ?1, repair_reason = ?2
             WHERE state_key = 'default' AND generation = ?3
               AND visible_epoch = 1",
            params![service_state, repair_reason, generation],
        )
        .map_err(MetaStoreError::storage)?;
    if changed != 1 {
        return Err(MetaStoreError::storage_invariant());
    }

    let records = publication_descriptor_records(connection)?;
    if records.len() != 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    let raw_head = connection
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
    validate_active_head(connection, &raw_head, &records[0])
}

fn publication_trigger_restore_sql(connection: &Connection) -> Result<String> {
    let mut statement = connection
        .prepare(
            "SELECT sql FROM sqlite_master
             WHERE type = 'trigger' AND name IN (
                 'search_publication_payload_immutable_after_validation',
                 'search_publication_same_state_immutable',
                 'search_publication_transition',
                 'ready_search_publication_immutable_update'
             ) ORDER BY name",
        )
        .map_err(MetaStoreError::storage)?;
    let sql = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(MetaStoreError::storage)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(MetaStoreError::storage)?;
    if sql.len() != 4 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(sql.into_iter().map(|sql| format!("{sql};\n")).collect())
}
