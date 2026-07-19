use std::{fmt, str::FromStr};

use core_domain::{ContentDigest, Document, ResumeVersion, UnixTimestamp};
use rusqlite::{params, Connection, TransactionBehavior};

use crate::{
    document_status_from_storage, file_extension_from_storage, import_task_status_to_storage,
    ImportProcessingContractId, ImportTaskId, ImportTaskStatus, MetaStoreError, MetadataStore,
    MetadataStoreAccess, Result,
};

#[derive(Clone, PartialEq, Eq)]
struct MigrationRebuildRootHead {
    authorized_root_rowid: i64,
    latest_task_rowid: i64,
    latest_task_id: ImportTaskId,
    completion_rowid: i64,
    source_manifest_digest: ContentDigest,
}

/// Opaque compare-and-swap token for an all-root migration rebuild publication.
///
/// A token captures the exact active authorized-root set and each root's latest
/// completed import task. Callers acquire it before constructing index
/// snapshots and present it when committing. It intentionally exposes neither
/// source paths nor task identifiers.
#[derive(Clone, PartialEq, Eq)]
pub struct MigrationRebuildBarrierToken {
    generation: Option<String>,
    visible_epoch: u64,
    processing_contract_id: ImportProcessingContractId,
    root_heads: Vec<MigrationRebuildRootHead>,
}

impl MigrationRebuildBarrierToken {
    pub(super) fn processing_contract_id(&self) -> &ImportProcessingContractId {
        &self.processing_contract_id
    }

    pub(super) fn identity_digest(&self) -> ContentDigest {
        let mut identity = Vec::new();
        append_identity_field(&mut identity, self.processing_contract_id.as_str());
        append_identity_field(
            &mut identity,
            self.generation.as_deref().unwrap_or("<unpublished>"),
        );
        identity.extend_from_slice(&self.visible_epoch.to_be_bytes());
        identity.extend_from_slice(&(self.root_heads.len() as u64).to_be_bytes());
        for head in &self.root_heads {
            identity.extend_from_slice(&head.authorized_root_rowid.to_be_bytes());
            identity.extend_from_slice(&head.latest_task_rowid.to_be_bytes());
            append_identity_field(&mut identity, head.latest_task_id.as_str());
            identity.extend_from_slice(&head.completion_rowid.to_be_bytes());
            append_identity_field(&mut identity, head.source_manifest_digest.as_str());
        }
        ContentDigest::from_bytes(&identity)
    }
}

fn append_identity_field(output: &mut Vec<u8>, value: &str) {
    output.extend_from_slice(&(value.len() as u64).to_be_bytes());
    output.extend_from_slice(value.as_bytes());
}

#[derive(Clone, PartialEq)]
pub struct MigrationRebuildProjectionRow {
    pub document: Document,
    pub resume_version: ResumeVersion,
}

impl fmt::Debug for MigrationRebuildProjectionRow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MigrationRebuildProjectionRow")
            .field("document_id", &self.document.id)
            .field("resume_version_id", &self.resume_version.id)
            .finish()
    }
}

impl fmt::Debug for MigrationRebuildBarrierToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MigrationRebuildBarrierToken")
            .field(
                "generation",
                &self.generation.as_ref().map(|_| "<redacted>"),
            )
            .field("visible_epoch", &self.visible_epoch)
            .field("processing_contract_id", &self.processing_contract_id)
            .field("active_root_count", &self.root_heads.len())
            .finish()
    }
}

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    /// Captures a closed migration-rebuild barrier when every currently active
    /// authorized root has a latest completed, non-cancelled import task.
    /// Paused roots and their cancelled tasks are outside this snapshot.
    pub fn acquire_migration_rebuild_barrier_token(
        &self,
        contract_id: &ImportProcessingContractId,
    ) -> Result<Option<MigrationRebuildBarrierToken>> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(MetaStoreError::storage)?;
        let token = migration_rebuild_barrier_token_in_connection(&transaction, contract_id)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(token)
    }

    /// Reads the exact searchable document/version pairs sealed by a barrier.
    /// The token is revalidated in the same read transaction; no latest-version
    /// or globally-visible document lookup participates in this snapshot.
    pub fn migration_rebuild_projection_rows(
        &self,
        token: &MigrationRebuildBarrierToken,
    ) -> Result<Vec<MigrationRebuildProjectionRow>> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(MetaStoreError::storage)?;
        if !migration_rebuild_barrier_token_matches(&transaction, token)? {
            transaction.commit().map_err(MetaStoreError::storage)?;
            return Err(MetaStoreError::invalid_transition());
        }
        let projection_rows = migration_rebuild_projection_rows_in_connection(&transaction, token)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(projection_rows)
    }
}

pub(super) fn migration_rebuild_barrier_token_matches(
    connection: &Connection,
    expected: &MigrationRebuildBarrierToken,
) -> Result<bool> {
    Ok(
        migration_rebuild_barrier_token_in_connection(
            connection,
            &expected.processing_contract_id,
        )?
        .as_ref()
            == Some(expected),
    )
}

fn migration_rebuild_barrier_token_in_connection(
    connection: &Connection,
    expected_contract_id: &ImportProcessingContractId,
) -> Result<Option<MigrationRebuildBarrierToken>> {
    let (service_state, generation, visible_epoch, repair_reason, active_contract_id) = connection
        .query_row(
            "SELECT projection.service_state, projection.generation,
                    projection.visible_epoch, projection.repair_reason,
                    rebuild.active_contract_id
             FROM search_projection_state AS projection
             JOIN migration_rebuild_contract_state AS rebuild
               ON rebuild.state_key = projection.state_key
             WHERE projection.state_key = 'default'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .map_err(MetaStoreError::storage)?;
    if service_state != "repairing"
        || repair_reason.as_deref() != Some("migration_rebuild")
        || generation.is_some()
        || active_contract_id.as_deref() != Some(expected_contract_id.as_str())
    {
        return Ok(None);
    }

    let unfinished_in_scope_or_unknown_task = connection
        .query_row(
            "SELECT EXISTS (
                 SELECT 1
                 FROM import_task AS task
                 LEFT JOIN authorized_import_root AS root
                   ON root.canonical_root_path = task.root_path
                 WHERE task.status IN (?1, ?2, ?3)
                   AND NOT EXISTS (
                       SELECT 1
                       FROM import_task_cancellation AS cancellation
                       WHERE cancellation.import_task_id = task.id
                   )
                   AND (root.rowid IS NULL OR root.paused = 0)
             )",
            params![
                import_task_status_to_storage(ImportTaskStatus::Queued),
                import_task_status_to_storage(ImportTaskStatus::Running),
                import_task_status_to_storage(ImportTaskStatus::FailedRetryable),
            ],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    if unfinished_in_scope_or_unknown_task != 0 {
        return Ok(None);
    }

    let mut statement = connection
        .prepare(
            "SELECT root.rowid, task.rowid, task.id, task.status,
                    EXISTS (
                        SELECT 1
                        FROM import_task_cancellation AS cancellation
                        WHERE cancellation.import_task_id = task.id
                    ), scope.scan_errors, scope.scan_budget_exhausted,
                    scope.scan_budget_kind, scope.scan_budget_limit,
                    scope.scan_budget_observed,
                    binding.processing_contract_id,
                    purpose.processing_contract_id,
                    completion.rowid, completion.processing_contract_id,
                    completion.source_disposition_count,
                    completion.source_manifest_digest,
                    scope.files_discovered
             FROM authorized_import_root AS root
             LEFT JOIN import_task AS task
               ON task.rowid = (
                   SELECT latest.rowid
                   FROM import_task AS latest
                   WHERE latest.root_path = root.canonical_root_path
                   ORDER BY latest.rowid DESC
                   LIMIT 1
               )
             LEFT JOIN import_scan_scope AS scope
               ON scope.import_task_id = task.id
             LEFT JOIN import_task_contract_binding AS binding
               ON binding.import_task_id = task.id
             LEFT JOIN migration_rebuild_full_corpus_task AS purpose
               ON purpose.import_task_id = task.id
             LEFT JOIN import_task_completion AS completion
               ON completion.import_task_id = task.id
             WHERE root.paused = 0
             ORDER BY root.rowid",
        )
        .map_err(MetaStoreError::storage)?;
    let mut rows = statement.query([]).map_err(MetaStoreError::storage)?;
    let mut root_heads = Vec::new();
    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        let latest_task_rowid = row
            .get::<_, Option<i64>>(1)
            .map_err(MetaStoreError::storage)?;
        let latest_task_id = row
            .get::<_, Option<String>>(2)
            .map_err(MetaStoreError::storage)?;
        let latest_task_status = row
            .get::<_, Option<String>>(3)
            .map_err(MetaStoreError::storage)?;
        let cancelled = row.get::<_, i64>(4).map_err(MetaStoreError::storage)?;
        let scan_errors = row
            .get::<_, Option<i64>>(5)
            .map_err(MetaStoreError::storage)?;
        let scan_budget_exhausted = row
            .get::<_, Option<i64>>(6)
            .map_err(MetaStoreError::storage)?;
        let scan_budget_kind = row
            .get::<_, Option<String>>(7)
            .map_err(MetaStoreError::storage)?;
        let scan_budget_limit = row
            .get::<_, Option<i64>>(8)
            .map_err(MetaStoreError::storage)?;
        let scan_budget_observed = row
            .get::<_, Option<i64>>(9)
            .map_err(MetaStoreError::storage)?;
        let bound_contract_id = row
            .get::<_, Option<String>>(10)
            .map_err(MetaStoreError::storage)?;
        let purpose_contract_id = row
            .get::<_, Option<String>>(11)
            .map_err(MetaStoreError::storage)?;
        let completion_rowid = row
            .get::<_, Option<i64>>(12)
            .map_err(MetaStoreError::storage)?;
        let completion_contract_id = row
            .get::<_, Option<String>>(13)
            .map_err(MetaStoreError::storage)?;
        let source_disposition_count = row
            .get::<_, Option<i64>>(14)
            .map_err(MetaStoreError::storage)?;
        let source_manifest_digest = row
            .get::<_, Option<String>>(15)
            .map_err(MetaStoreError::storage)?;
        let files_discovered = row
            .get::<_, Option<i64>>(16)
            .map_err(MetaStoreError::storage)?;
        let (Some(latest_task_rowid), Some(latest_task_id), Some(latest_task_status)) =
            (latest_task_rowid, latest_task_id, latest_task_status)
        else {
            return Ok(None);
        };
        if latest_task_status != import_task_status_to_storage(ImportTaskStatus::Completed)
            || cancelled != 0
            || scan_errors != Some(0)
            || scan_budget_exhausted != Some(0)
            || scan_budget_kind.is_some()
            || scan_budget_limit.is_some()
            || scan_budget_observed.is_some()
            || bound_contract_id.as_deref() != Some(expected_contract_id.as_str())
            || purpose_contract_id.as_deref() != Some(expected_contract_id.as_str())
            || completion_contract_id.as_deref() != Some(expected_contract_id.as_str())
            || source_disposition_count != files_discovered
        {
            return Ok(None);
        }
        let (Some(completion_rowid), Some(source_manifest_digest)) =
            (completion_rowid, source_manifest_digest)
        else {
            return Ok(None);
        };
        root_heads.push(MigrationRebuildRootHead {
            authorized_root_rowid: row.get(0).map_err(MetaStoreError::storage)?,
            latest_task_rowid,
            latest_task_id: ImportTaskId::from_str(&latest_task_id)
                .map_err(|_| MetaStoreError::invalid_value("import_task.id"))?,
            completion_rowid,
            source_manifest_digest: ContentDigest::from_str(&source_manifest_digest).map_err(
                |_| MetaStoreError::invalid_value("import_task_completion.source_manifest_digest"),
            )?,
        });
    }

    Ok(Some(MigrationRebuildBarrierToken {
        generation,
        visible_epoch: u64::try_from(visible_epoch)
            .map_err(|_| MetaStoreError::invalid_value("search_projection_state.visible_epoch"))?,
        processing_contract_id: expected_contract_id.clone(),
        root_heads,
    }))
}

fn migration_rebuild_projection_rows_in_connection(
    connection: &Connection,
    token: &MigrationRebuildBarrierToken,
) -> Result<Vec<MigrationRebuildProjectionRow>> {
    let mut statement = connection
        .prepare(
            "SELECT document.id, document.source_uri, document.normalized_path,
                    document.file_name, document.extension, document.byte_size,
                    document.mtime_seconds, document.content_hash, document.text_hash,
                    document.is_deleted, document.created_at_seconds,
                    document.updated_at_seconds, document.status,
                    version.id, version.document_id, version.source_revision_id,
                    version.normalized_text_hash, version.parse_version,
                    version.schema_version, version.language_set_json, version.page_count,
                    version.raw_text, version.clean_text, version.quality_score
             FROM authorized_import_root AS root
             JOIN import_task AS task
               ON task.rowid = (
                   SELECT latest.rowid FROM import_task AS latest
                   WHERE latest.root_path = root.canonical_root_path
                   ORDER BY latest.rowid DESC LIMIT 1
               )
             JOIN migration_rebuild_full_corpus_task AS purpose
               ON purpose.import_task_id = task.id
              AND purpose.processing_contract_id = ?1
             JOIN import_task_completion AS completion
               ON completion.import_task_id = task.id
              AND completion.processing_contract_id = ?1
             JOIN import_task_source_disposition AS disposition
               ON disposition.import_task_id = task.id
              AND disposition.processing_contract_id = ?1
              AND disposition.disposition = 'searchable'
             JOIN document
               ON document.id = disposition.document_id
             JOIN resume_version AS version
               ON version.id = disposition.resume_version_id
              AND version.document_id = disposition.document_id
              AND version.source_revision_id = disposition.source_revision_id
             JOIN import_processing_contract AS contract
               ON contract.id = ?1
             JOIN resume_version_classification AS classification
               ON classification.resume_version_id = version.id
              AND classification.classifier_epoch = contract.classifier_epoch
              AND classification.status = 'resume_candidate'
             WHERE root.paused = 0
             ORDER BY document.id, version.id",
        )
        .map_err(MetaStoreError::storage)?;
    let mut rows = statement
        .query(params![token.processing_contract_id.as_str()])
        .map_err(MetaStoreError::storage)?;
    let mut projection_rows = Vec::<MigrationRebuildProjectionRow>::new();
    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        let document = Document {
            id: parse_id(
                row.get::<_, String>(0).map_err(MetaStoreError::storage)?,
                "document.id",
            )?,
            source_uri: row.get(1).map_err(MetaStoreError::storage)?,
            normalized_path: row.get(2).map_err(MetaStoreError::storage)?,
            file_name: row.get(3).map_err(MetaStoreError::storage)?,
            extension: file_extension_from_storage(
                &row.get::<_, String>(4).map_err(MetaStoreError::storage)?,
            ),
            byte_size: parse_u64(
                row.get(5).map_err(MetaStoreError::storage)?,
                "document.byte_size",
            )?,
            mtime: UnixTimestamp::from_unix_seconds(row.get(6).map_err(MetaStoreError::storage)?),
            content_hash: row.get(7).map_err(MetaStoreError::storage)?,
            text_hash: row.get(8).map_err(MetaStoreError::storage)?,
            is_deleted: row.get::<_, i64>(9).map_err(MetaStoreError::storage)? == 1,
            created_at: UnixTimestamp::from_unix_seconds(
                row.get(10).map_err(MetaStoreError::storage)?,
            ),
            updated_at: UnixTimestamp::from_unix_seconds(
                row.get(11).map_err(MetaStoreError::storage)?,
            ),
            status: document_status_from_storage(
                &row.get::<_, String>(12).map_err(MetaStoreError::storage)?,
            )?,
        };
        let language_set_json = row.get::<_, String>(19).map_err(MetaStoreError::storage)?;
        let resume_version = ResumeVersion {
            id: parse_id(
                row.get::<_, String>(13).map_err(MetaStoreError::storage)?,
                "resume_version.id",
            )?,
            document_id: parse_id(
                row.get::<_, String>(14).map_err(MetaStoreError::storage)?,
                "resume_version.document_id",
            )?,
            source_revision_id: parse_id(
                row.get::<_, String>(15).map_err(MetaStoreError::storage)?,
                "resume_version.source_revision_id",
            )?,
            normalized_text_hash: parse_id(
                row.get::<_, String>(16).map_err(MetaStoreError::storage)?,
                "resume_version.normalized_text_hash",
            )?,
            parse_version: row.get(17).map_err(MetaStoreError::storage)?,
            schema_version: row.get(18).map_err(MetaStoreError::storage)?,
            language_set: serde_json::from_str(&language_set_json)
                .map_err(|_| MetaStoreError::invalid_value("resume_version.language_set"))?,
            page_count: row
                .get::<_, Option<i64>>(20)
                .map_err(MetaStoreError::storage)?
                .map(|value| {
                    u32::try_from(value)
                        .map_err(|_| MetaStoreError::invalid_value("resume_version.page_count"))
                })
                .transpose()?,
            raw_text: row.get(21).map_err(MetaStoreError::storage)?,
            clean_text: row.get(22).map_err(MetaStoreError::storage)?,
            quality_score: row
                .get::<_, Option<f64>>(23)
                .map_err(MetaStoreError::storage)?
                .map(|value| value as f32),
        };
        if document.id != resume_version.document_id {
            return Err(MetaStoreError::storage_invariant());
        }
        if let Some(previous) = projection_rows.last() {
            if previous.document.id == document.id {
                if previous.resume_version.id == resume_version.id {
                    continue;
                }
                return Err(MetaStoreError::storage_invariant());
            }
        }
        projection_rows.push(MigrationRebuildProjectionRow {
            document,
            resume_version,
        });
    }
    Ok(projection_rows)
}

fn parse_id<T: FromStr>(value: String, field: &'static str) -> Result<T> {
    value
        .parse()
        .map_err(|_| MetaStoreError::invalid_value(field))
}

fn parse_u64(value: i64, field: &'static str) -> Result<u64> {
    u64::try_from(value).map_err(|_| MetaStoreError::invalid_value(field))
}
