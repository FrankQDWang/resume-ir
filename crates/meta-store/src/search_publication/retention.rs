use std::collections::BTreeSet;

use rusqlite::{params, TransactionBehavior};

use crate::{MetaStore, MetaStoreError, Result};

use super::{
    model::{SearchPublicationFailure, SearchPublicationState},
    validation::publication_error,
};

impl MetaStore {
    /// Returns the complete metadata-retained artifact generations from one
    /// SQLite read snapshot. The caller must already hold both artifact-store
    /// exclusive publication leases so no unrecorded artifact can appear
    /// between this read and garbage collection.
    pub fn search_artifact_retention_generations(
        &self,
        retain_ready: usize,
    ) -> Result<BTreeSet<String>> {
        let retain_ready = i64::try_from(retain_ready)
            .ok()
            .filter(|value| (1..=256).contains(value))
            .ok_or_else(|| publication_error(SearchPublicationFailure::InvalidDescriptor))?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(MetaStoreError::storage)?;
        let mut retained = BTreeSet::new();
        let head = transaction
            .query_row(
                "SELECT generation FROM search_projection_state WHERE state_key = 'default'",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(MetaStoreError::storage)?;
        retained.extend(head);
        {
            let mut statement = transaction
                .prepare(
                    "SELECT generation FROM search_publication_journal
                     WHERE state IN (?1, ?2)
                     ORDER BY generation",
                )
                .map_err(MetaStoreError::storage)?;
            let generations = statement
                .query_map(
                    params![
                        SearchPublicationState::Preparing.as_str(),
                        SearchPublicationState::Validated.as_str(),
                    ],
                    |row| row.get::<_, String>(0),
                )
                .map_err(MetaStoreError::storage)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(MetaStoreError::storage)?;
            retained.extend(generations);
        }
        {
            let mut statement = transaction
                .prepare(
                    "SELECT generation FROM search_publication_journal
                     WHERE state = ?1
                     ORDER BY updated_at_seconds DESC, generation DESC
                     LIMIT ?2",
                )
                .map_err(MetaStoreError::storage)?;
            let generations = statement
                .query_map(
                    params![SearchPublicationState::Ready.as_str(), retain_ready],
                    |row| row.get::<_, String>(0),
                )
                .map_err(MetaStoreError::storage)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(MetaStoreError::storage)?;
            retained.extend(generations);
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(retained)
    }
}
