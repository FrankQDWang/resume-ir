use std::str::FromStr;

use rusqlite::{params, OptionalExtension};

use crate::{
    ContentDigest, ImportTaskId, MetaStoreError, MetadataStore, MetadataStoreAccess,
    MetadataStoreWriteAccess, Result, SourceRevisionId, UnixTimestamp,
};

pub const SOURCE_FILE_OBSERVATION_ASSURANCE: &str = "macos_stat_v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceFileObservation {
    pub source_revision_id: SourceRevisionId,
    pub content_hash: ContentDigest,
    pub stable_file_id: String,
    pub byte_size: u64,
    pub mtime_seconds: i64,
    pub mtime_nanoseconds: u32,
    pub ctime_seconds: i64,
    pub ctime_nanoseconds: u32,
    pub strongly_verified_at: UnixTimestamp,
    pub next_strong_verification_at: UnixTimestamp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrongSourceFileObservation {
    pub source_revision_id: SourceRevisionId,
    pub stable_file_id: String,
    pub byte_size: u64,
    pub mtime_seconds: i64,
    pub mtime_nanoseconds: u32,
    pub ctime_seconds: i64,
    pub ctime_nanoseconds: u32,
    pub strongly_verified_at: UnixTimestamp,
    pub next_strong_verification_at: UnixTimestamp,
}

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    pub fn source_file_observation_count(&self) -> Result<u64> {
        let count = self
            .connection
            .borrow()
            .query_row("SELECT COUNT(*) FROM source_file_observation", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(MetaStoreError::storage)?;
        u64::try_from(count).map_err(|_| MetaStoreError::storage_invariant())
    }

    pub fn source_file_observation_for_import_task(
        &self,
        task_id: &ImportTaskId,
        normalized_path: &str,
    ) -> Result<Option<SourceFileObservation>> {
        let Some((root, relative_path)) =
            self.source_root_and_relative_path_for_import_task(task_id, normalized_path)?
        else {
            return Ok(None);
        };
        self.connection
            .borrow()
            .query_row(
                "SELECT observation.source_revision_id, revision.content_hash,
                        observation.stable_file_id, observation.byte_size,
                        observation.mtime_seconds, observation.mtime_nanoseconds,
                        observation.ctime_seconds, observation.ctime_nanoseconds,
                        observation.strongly_verified_at_seconds,
                        observation.next_strong_verification_at_seconds
                 FROM source_file_observation AS observation
                 JOIN source_occurrence AS occurrence
                   ON occurrence.root_id = observation.root_id
                  AND occurrence.relative_path = observation.relative_path
                  AND occurrence.source_revision_id = observation.source_revision_id
                  AND occurrence.state = 'present'
                 JOIN source_revision AS revision
                   ON revision.id = observation.source_revision_id
                 WHERE observation.root_id = ?1
                   AND observation.relative_path = ?2
                   AND observation.assurance_kind = ?3",
                params![
                    root.id.as_str(),
                    relative_path,
                    SOURCE_FILE_OBSERVATION_ASSURANCE
                ],
                source_file_observation_from_row,
            )
            .optional()
            .map_err(MetaStoreError::storage)
    }

    pub fn record_strong_source_file_observation(
        &self,
        task_id: &ImportTaskId,
        normalized_path: &str,
        observation: &StrongSourceFileObservation,
    ) -> Result<()>
    where
        Access: MetadataStoreWriteAccess,
    {
        let Some((root, relative_path)) =
            self.source_root_and_relative_path_for_import_task(task_id, normalized_path)?
        else {
            return Ok(());
        };
        let connection = self.connection.borrow_mut();
        record_strong_source_file_observation_in_connection(
            &connection,
            &root.id,
            &relative_path,
            observation,
        )
    }
}

pub(super) fn record_strong_source_file_observation_in_connection(
    connection: &rusqlite::Connection,
    root_id: &crate::SourceRootId,
    relative_path: &str,
    observation: &StrongSourceFileObservation,
) -> Result<()> {
    validate_observation(observation)?;
    let changed = connection
        .execute(
            "INSERT INTO source_file_observation (
                root_id, relative_path, source_revision_id, assurance_kind,
                stable_file_id, byte_size, mtime_seconds, mtime_nanoseconds,
                ctime_seconds, ctime_nanoseconds, strongly_verified_at_seconds,
                next_strong_verification_at_seconds
             )
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12
             FROM source_occurrence
             WHERE root_id = ?1 AND relative_path = ?2
               AND source_revision_id = ?3 AND state = 'present'
             ON CONFLICT(root_id, relative_path) DO UPDATE SET
                source_revision_id = excluded.source_revision_id,
                assurance_kind = excluded.assurance_kind,
                stable_file_id = excluded.stable_file_id,
                byte_size = excluded.byte_size,
                mtime_seconds = excluded.mtime_seconds,
                mtime_nanoseconds = excluded.mtime_nanoseconds,
                ctime_seconds = excluded.ctime_seconds,
                ctime_nanoseconds = excluded.ctime_nanoseconds,
                strongly_verified_at_seconds = excluded.strongly_verified_at_seconds,
                next_strong_verification_at_seconds =
                    excluded.next_strong_verification_at_seconds",
            params![
                root_id.as_str(),
                relative_path,
                observation.source_revision_id.as_str(),
                SOURCE_FILE_OBSERVATION_ASSURANCE,
                observation.stable_file_id,
                i64::try_from(observation.byte_size)
                    .map_err(|_| MetaStoreError::invalid_value("observation.byte_size"))?,
                observation.mtime_seconds,
                observation.mtime_nanoseconds,
                observation.ctime_seconds,
                observation.ctime_nanoseconds,
                observation.strongly_verified_at.as_unix_seconds(),
                observation.next_strong_verification_at.as_unix_seconds(),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if changed != 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn source_file_observation_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<SourceFileObservation> {
    let source_revision_id = parse_row_value::<SourceRevisionId>(row.get(0)?, 0)?;
    let content_hash = parse_row_value::<ContentDigest>(row.get(1)?, 1)?;
    let byte_size = nonnegative_u64(row.get(3)?, 3)?;
    let mtime_nanoseconds = nanoseconds(row.get(5)?, 5)?;
    let ctime_nanoseconds = nanoseconds(row.get(7)?, 7)?;
    Ok(SourceFileObservation {
        source_revision_id,
        content_hash,
        stable_file_id: row.get(2)?,
        byte_size,
        mtime_seconds: row.get(4)?,
        mtime_nanoseconds,
        ctime_seconds: row.get(6)?,
        ctime_nanoseconds,
        strongly_verified_at: UnixTimestamp::from_unix_seconds(row.get(8)?),
        next_strong_verification_at: UnixTimestamp::from_unix_seconds(row.get(9)?),
    })
}

fn parse_row_value<T>(value: String, column: usize) -> rusqlite::Result<T>
where
    T: FromStr,
{
    value.parse().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            "invalid source file observation identity".into(),
        )
    })
}

fn nonnegative_u64(value: i64, column: usize) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

fn nanoseconds(value: i64, column: usize) -> rusqlite::Result<u32> {
    u32::try_from(value)
        .ok()
        .filter(|value| *value < 1_000_000_000)
        .ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                column,
                rusqlite::types::Type::Integer,
                "invalid source file observation nanoseconds".into(),
            )
        })
}

fn validate_observation(observation: &StrongSourceFileObservation) -> Result<()> {
    if observation.stable_file_id.len() != 36
        || !observation.stable_file_id.starts_with("sfi_")
        || !observation.stable_file_id[4..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || observation.mtime_nanoseconds >= 1_000_000_000
        || observation.ctime_nanoseconds >= 1_000_000_000
        || observation.strongly_verified_at.as_unix_seconds() < 0
        || observation.next_strong_verification_at.as_unix_seconds()
            <= observation.strongly_verified_at.as_unix_seconds()
    {
        return Err(MetaStoreError::invalid_value("source_file_observation"));
    }
    Ok(())
}
