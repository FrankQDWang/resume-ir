use rusqlite::{params, Connection, OptionalExtension};

use crate::{
    ImportProcessingContractId, ImportTaskId, MetaStoreError, MetadataStore, MetadataStoreAccess,
    Result,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportTaskPurpose {
    ConfiguredCatchUp,
    MigrationRebuildFullCorpus,
}

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    /// Returns the persisted purpose of an existing task. Tasks without the
    /// migration marker are ordinary configured imports by definition.
    pub fn import_task_purpose(&self, task_id: &ImportTaskId) -> Result<ImportTaskPurpose> {
        let connection = self.connection.borrow();
        connection
            .query_row(
                "SELECT marker.import_task_id IS NOT NULL
                 FROM import_task AS task
                 LEFT JOIN migration_rebuild_full_corpus_task AS marker
                   ON marker.import_task_id = task.id
                 WHERE task.id = ?1",
                params![task_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(MetaStoreError::storage)?
            .map(|marked| {
                if marked == 1 {
                    ImportTaskPurpose::MigrationRebuildFullCorpus
                } else {
                    ImportTaskPurpose::ConfiguredCatchUp
                }
            })
            .ok_or_else(|| MetaStoreError::not_found("import_task"))
    }
}

/// Marks a just-inserted, contract-bound task as a full-corpus migration
/// rebuild task. Callers must invoke this inside the same transaction that
/// inserts the task and its contract binding.
pub(super) fn insert_migration_rebuild_full_corpus_task_marker_in_connection(
    connection: &Connection,
    task_id: &ImportTaskId,
    contract_id: &ImportProcessingContractId,
) -> Result<()> {
    connection
        .execute(
            "INSERT INTO migration_rebuild_full_corpus_task (
                import_task_id, processing_contract_id
             ) VALUES (?1, ?2)",
            params![task_id.as_str(), contract_id.as_str()],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(())
}
