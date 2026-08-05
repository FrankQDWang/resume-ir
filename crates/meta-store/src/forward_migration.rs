use std::path::Path;

use rusqlite::{params, Connection, Transaction};
use sha2::{Digest, Sha256};

use crate::{
    schema_v29, schema_v30, schema_v31, schema_v32, schema_v33, schema_v34, schema_v35, schema_v36,
    MetaStoreError, Result, SourceRootId,
};

const V29_TO_V30_NAME: &str = "metadata-forward-migration-history";
const V30_TO_V31_NAME: &str = "source-root-path-truth";
const V31_TO_V32_NAME: &str = "source-root-durable-deletion";
const V32_TO_V33_NAME: &str = "pdfium-parser-reprocessing";
const V33_TO_V34_NAME: &str = "processing-contract-upgrade-coordinator";
const V34_TO_V35_NAME: &str = "source-file-observation";
const V35_TO_V36_NAME: &str = "source-root-deletion-attempt-evidence";
const PDFIUM_PARSER_CONTRACT: &str = "parser-pdfium-v2";
const PDF_REPROCESS_LOOKUP_INDEX: &str = "__migration_pdf_reprocess_resume_lookup";

struct MigrationStep {
    from: u32,
    to: u32,
    name: &'static str,
    schema: &'static str,
    apply: fn(&Transaction<'_>) -> Result<()>,
    validate: fn(&Connection) -> Result<()>,
}

impl MigrationStep {
    fn checksum(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(b"resume-ir.metadata-forward-migration.v1");
        digest.update(self.from.to_be_bytes());
        digest.update(self.to.to_be_bytes());
        digest.update(self.name.as_bytes());
        digest.update(self.schema.as_bytes());
        format!("{:x}", digest.finalize())
    }
}

pub(super) fn apply_chain(connection: &mut Connection, from: u32, to: u32) -> Result<()> {
    if from > to {
        return Err(MetaStoreError::unsupported_store_schema());
    }
    let mut current = from;
    while current < to {
        let step = registry()
            .into_iter()
            .find(|step| step.from == current)
            .ok_or_else(MetaStoreError::unsupported_store_schema)?;
        if step.to != current + 1 || step.to > to {
            return Err(MetaStoreError::storage_invariant());
        }
        apply_step(connection, &step)?;
        current = step.to;
    }
    validate_chain(connection, schema_v29::VERSION, to)
}

pub(super) fn validate_chain(connection: &Connection, from: u32, to: u32) -> Result<()> {
    if from > to {
        return Err(MetaStoreError::unsupported_store_schema());
    }
    let mut current = from;
    while current < to {
        let step = registry()
            .into_iter()
            .find(|step| step.from == current)
            .ok_or_else(MetaStoreError::unsupported_store_schema)?;
        validate_history(connection, &step)?;
        (step.validate)(connection)?;
        current = step.to;
    }
    if current != to {
        return Err(MetaStoreError::storage_invariant());
    }
    let history_count = connection
        .query_row(
            "SELECT COUNT(*) FROM forward_migration_history
             WHERE to_version > ?1 AND to_version <= ?2",
            params![i64::from(from), i64::from(to)],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    if history_count != i64::from(to - from) {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

pub(super) fn apply_current_schema(connection: &mut Connection, from: u32) -> Result<()> {
    apply_chain(connection, from, schema_v36::VERSION)
}

fn apply_step(connection: &mut Connection, step: &MigrationStep) -> Result<()> {
    let transaction = connection
        .transaction()
        .map_err(MetaStoreError::migration)?;
    (step.apply)(&transaction)?;
    transaction
        .execute(
            "INSERT INTO forward_migration_history (
                to_version, from_version, migration_name,
                migration_checksum, applied_at_seconds
             ) VALUES (?1, ?2, ?3, ?4, 0)",
            params![
                i64::from(step.to),
                i64::from(step.from),
                step.name,
                step.checksum(),
            ],
        )
        .map_err(MetaStoreError::migration)?;
    transaction
        .execute(
            "INSERT INTO schema_migrations (version, applied_at_seconds)
             VALUES (?1, 0)",
            params![i64::from(step.to)],
        )
        .map_err(MetaStoreError::migration)?;
    transaction.commit().map_err(MetaStoreError::migration)?;
    (step.validate)(connection)?;
    validate_history(connection, step)
}

fn validate_history(connection: &Connection, step: &MigrationStep) -> Result<()> {
    let row = connection
        .query_row(
            "SELECT from_version, migration_name, migration_checksum
             FROM forward_migration_history
             WHERE to_version = ?1",
            params![i64::from(step.to)],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(MetaStoreError::storage)?;
    if row != (i64::from(step.from), step.name.to_string(), step.checksum()) {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn apply_v29_to_v30(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(schema_v30::SCHEMA)
        .map_err(MetaStoreError::migration)
}

fn validate_v30(connection: &Connection) -> Result<()> {
    let table_count = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name = 'forward_migration_history'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    if table_count != 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn apply_v30_to_v31(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(schema_v31::SCHEMA)
        .map_err(MetaStoreError::migration)?;
    let mut roots = transaction
        .prepare(
            "SELECT canonical_root_path, requested_root_path, paused, updated_at_seconds
             FROM authorized_import_root
             ORDER BY canonical_root_path",
        )
        .map_err(MetaStoreError::migration)?;
    let roots = roots
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(MetaStoreError::migration)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(MetaStoreError::migration)?;
    for (index, (canonical, _, _, _)) in roots.iter().enumerate() {
        let canonical = Path::new(canonical);
        if roots[index + 1..].iter().any(|(other, _, _, _)| {
            let other = Path::new(other);
            canonical.starts_with(other) || other.starts_with(canonical)
        }) {
            return Err(MetaStoreError::invalid_value(
                "source_root.migration_overlap",
            ));
        }
    }
    for (canonical, requested, paused, updated_at) in roots {
        let root_id = SourceRootId::new()?;
        let display_label = Path::new(&canonical)
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("已授权目录");
        let display_label = bounded_display_label(display_label);
        let watcher = if paused == 1 { "paused" } else { "active" };
        transaction
            .execute(
                "INSERT INTO source_root (
                    id, canonical_path, requested_path, display_label, state,
                    watcher_state, created_at_seconds, updated_at_seconds
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![
                    root_id.as_str(),
                    canonical,
                    requested,
                    display_label,
                    "active",
                    watcher,
                    updated_at.max(0)
                ],
            )
            .map_err(MetaStoreError::migration)?;
        backfill_occurrences(transaction, &root_id, &canonical)?;
    }
    Ok(())
}

fn bounded_display_label(label: &str) -> String {
    const MAX_CHARS: usize = 80;
    let count = label.chars().count();
    if count <= MAX_CHARS {
        return label.to_string();
    }
    let mut bounded = label
        .chars()
        .take(MAX_CHARS.saturating_sub(3))
        .collect::<String>();
    bounded.push_str("...");
    bounded
}

fn backfill_occurrences(
    transaction: &Transaction<'_>,
    root_id: &SourceRootId,
    canonical_root: &str,
) -> Result<()> {
    let root = Path::new(canonical_root);
    let mut documents = transaction
        .prepare(
            "SELECT document.id, document.normalized_path, revision.id,
                    document.updated_at_seconds
             FROM document
             JOIN source_revision AS revision
               ON revision.document_id = document.id
              AND revision.content_hash = document.content_hash
             WHERE document.is_deleted = 0
             ORDER BY document.normalized_path, revision.id",
        )
        .map_err(MetaStoreError::migration)?;
    let documents = documents
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(MetaStoreError::migration)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(MetaStoreError::migration)?;
    for (document_id, normalized_path, revision_id, observed_at) in documents {
        let Ok(relative) = Path::new(&normalized_path).strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        if relative.is_empty() {
            continue;
        }
        transaction
            .execute(
                "INSERT OR IGNORE INTO source_occurrence (
                    root_id, relative_path, document_id, source_revision_id,
                    state, observed_at_seconds
                 ) VALUES (?1, ?2, ?3, ?4, 'present', ?5)",
                params![
                    root_id.as_str(),
                    relative,
                    document_id,
                    revision_id,
                    observed_at.max(0)
                ],
            )
            .map_err(MetaStoreError::migration)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO source_occurrence_revision (
                    root_id, relative_path, source_revision_id, observed_at_seconds
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![root_id.as_str(), relative, revision_id, observed_at.max(0)],
            )
            .map_err(MetaStoreError::migration)?;
    }
    Ok(())
}

fn validate_v31(connection: &Connection) -> Result<()> {
    let tables = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name IN (
                'source_root', 'source_occurrence',
                'source_occurrence_revision', 'scan_snapshot'
             )",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    if tables != 4 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn apply_v31_to_v32(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(schema_v32::SCHEMA)
        .map_err(MetaStoreError::migration)
}

fn validate_v32(connection: &Connection) -> Result<()> {
    let tables = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name IN (
                 'source_root_deletion',
                 'source_root_deletion_document'
             )",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    if tables != 2 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn apply_v32_to_v33(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(schema_v33::SCHEMA)
        .map_err(MetaStoreError::migration)?;
    create_pdf_reprocess_lookup_index(transaction)?;
    transaction
        .execute(
            "INSERT INTO pdf_reprocess_job (
                source_revision_id, root_id, relative_path, parser_contract,
                state, attempts, queued_at_seconds, updated_at_seconds
             )
             SELECT occurrence.source_revision_id, occurrence.root_id,
                    occurrence.relative_path, ?1, 'queued', 0, 0, 0
             FROM source_occurrence AS occurrence
             JOIN source_root AS root ON root.id = occurrence.root_id
             WHERE occurrence.state = 'present'
               AND root.state IN ('active', 'offline')
               AND lower(occurrence.relative_path) LIKE '%.pdf'
               AND NOT EXISTS (
                   SELECT 1
                   FROM resume_version AS version
                   WHERE version.source_revision_id = occurrence.source_revision_id
                     AND version.parse_version = ?1
               )
             ON CONFLICT(source_revision_id) DO NOTHING",
            params![PDFIUM_PARSER_CONTRACT],
        )
        .map_err(MetaStoreError::migration)?;
    transaction
        .execute(&format!("DROP INDEX {PDF_REPROCESS_LOOKUP_INDEX}"), [])
        .map_err(MetaStoreError::migration)?;
    Ok(())
}

fn create_pdf_reprocess_lookup_index(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute(
            &format!(
                "CREATE INDEX {PDF_REPROCESS_LOOKUP_INDEX}
                 ON resume_version(source_revision_id, parse_version)"
            ),
            [],
        )
        .map_err(MetaStoreError::migration)?;
    Ok(())
}

fn validate_v33(connection: &Connection) -> Result<()> {
    let tables = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name = 'pdf_reprocess_job'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    if tables != 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn apply_v33_to_v34(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(schema_v34::SCHEMA)
        .map_err(MetaStoreError::migration)?;
    // Seed committed writer authority from the existing rebuild contract head
    // without activating a transition. Slice B stays dormant.
    transaction
        .execute(
            "UPDATE writer_authority_state
             SET committed_contract_id = (
                     SELECT active_contract_id
                     FROM migration_rebuild_contract_state
                     WHERE state_key = 'default'
                 ),
                 updated_at_seconds = MAX(updated_at_seconds, 0)
             WHERE state_key = 'default'",
            [],
        )
        .map_err(MetaStoreError::migration)?;
    Ok(())
}

fn validate_v34(connection: &Connection) -> Result<()> {
    let tables = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table'
               AND name IN (
                   'writer_contract_transition',
                   'writer_authority_state',
                   'reprocessing_campaign'
               )",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    if tables != 3 {
        return Err(MetaStoreError::storage_invariant());
    }
    let authority_rows = connection
        .query_row(
            "SELECT COUNT(*) FROM writer_authority_state WHERE state_key = 'default'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    if authority_rows != 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn apply_v34_to_v35(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(schema_v35::SCHEMA)
        .map_err(MetaStoreError::migration)
}

fn validate_v35(connection: &Connection) -> Result<()> {
    let tables = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name = 'source_file_observation'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    if tables != 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn apply_v35_to_v36(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(schema_v36::SCHEMA)
        .map_err(MetaStoreError::migration)
}

fn validate_v36(connection: &Connection) -> Result<()> {
    let tables = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table'
               AND name = 'source_root_deletion_attempt_evidence'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    if tables != 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    let missing_evidence = connection
        .query_row(
            "SELECT COUNT(*)
             FROM source_root_deletion AS deletion
             LEFT JOIN source_root_deletion_attempt_evidence AS evidence
               ON evidence.root_id = deletion.root_id
             WHERE evidence.root_id IS NULL",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    if missing_evidence != 0 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn registry() -> [MigrationStep; 7] {
    [
        MigrationStep {
            from: schema_v29::VERSION,
            to: schema_v30::VERSION,
            name: V29_TO_V30_NAME,
            schema: schema_v30::SCHEMA,
            apply: apply_v29_to_v30,
            validate: validate_v30,
        },
        MigrationStep {
            from: schema_v30::VERSION,
            to: schema_v31::VERSION,
            name: V30_TO_V31_NAME,
            schema: schema_v31::SCHEMA,
            apply: apply_v30_to_v31,
            validate: validate_v31,
        },
        MigrationStep {
            from: schema_v31::VERSION,
            to: schema_v32::VERSION,
            name: V31_TO_V32_NAME,
            schema: schema_v32::SCHEMA,
            apply: apply_v31_to_v32,
            validate: validate_v32,
        },
        MigrationStep {
            from: schema_v32::VERSION,
            to: schema_v33::VERSION,
            name: V32_TO_V33_NAME,
            schema: schema_v33::SCHEMA,
            apply: apply_v32_to_v33,
            validate: validate_v33,
        },
        MigrationStep {
            from: schema_v33::VERSION,
            to: schema_v34::VERSION,
            name: V33_TO_V34_NAME,
            schema: schema_v34::SCHEMA,
            apply: apply_v33_to_v34,
            validate: validate_v34,
        },
        MigrationStep {
            from: schema_v34::VERSION,
            to: schema_v35::VERSION,
            name: V34_TO_V35_NAME,
            schema: schema_v35::SCHEMA,
            apply: apply_v34_to_v35,
            validate: validate_v35,
        },
        MigrationStep {
            from: schema_v35::VERSION,
            to: schema_v36::VERSION,
            name: V35_TO_V36_NAME,
            schema: schema_v36::SCHEMA,
            apply: apply_v35_to_v36,
            validate: validate_v36,
        },
    ]
}

#[cfg(test)]
#[path = "forward_migration_tests.rs"]
mod tests;
