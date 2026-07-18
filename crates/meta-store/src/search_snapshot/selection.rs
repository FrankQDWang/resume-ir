use std::{fmt, str::FromStr};

use rusqlite::{params, Connection, OptionalExtension};

use super::{head::active_projection_for_generation, SearchMetadataSnapshot};
use crate::{
    read_document, read_entity_mention, CandidateId, ContentDigest, Document, DocumentId,
    EntityMention, MetaStoreError, Result, ResumeVersionId, SearchSelection,
    SearchSelectionResolution, SourceRevisionId, DOCUMENT_COLUMNS, ENTITY_MENTION_COLUMNS,
};

pub const MAX_SEARCH_SELECTION_MENTIONS: usize = 256;
const MAX_VERSION_LABEL_BYTES: usize = 256;
const MAX_LANGUAGE_SET_BYTES: usize = 4 * 1024;
const MAX_LANGUAGE_COUNT: usize = 64;
const MAX_LANGUAGE_BYTES: usize = 64;
const MAX_DOCUMENT_SOURCE_URI_BYTES: u64 = 128 * 1024;
const MAX_DOCUMENT_NORMALIZED_PATH_BYTES: u64 = 128 * 1024;
const MAX_DOCUMENT_FILE_NAME_BYTES: u64 = 4 * 1024;
const MAX_DOCUMENT_EXTENSION_BYTES: u64 = 256;
pub(crate) const MAX_MENTION_VALUE_BYTES: usize = 4 * 1024;
pub(crate) const MAX_MENTION_EXTRACTOR_BYTES: usize = 256;

#[derive(Clone, PartialEq)]
pub struct SearchSelectionDetails {
    pub selection: SearchSelection,
    pub version: SearchSelectionVersion,
    pub candidate_id: Option<CandidateId>,
    pub mentions: Vec<EntityMention>,
}

/// Immutable, bounded metadata for the exact version named by a search
/// selection. Mutable `Document` state is deliberately excluded.
#[derive(Clone, PartialEq)]
pub struct SearchSelectionVersion {
    pub source_revision_id: SourceRevisionId,
    pub source_content_hash: ContentDigest,
    pub source_byte_size: u64,
    pub normalized_text_hash: ContentDigest,
    pub parse_version: String,
    pub schema_version: String,
    pub language_set: Vec<String>,
    pub page_count: Option<u32>,
    pub quality_score: Option<f32>,
}

impl fmt::Debug for SearchSelectionVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchSelectionVersion")
            .field("source_revision_id", &"<redacted>")
            .field("source_content_hash", &"<redacted>")
            .field("source_byte_size", &self.source_byte_size)
            .field("normalized_text_hash", &"<redacted>")
            .field("parse_version", &self.parse_version)
            .field("schema_version", &self.schema_version)
            .field("language_count", &self.language_set.len())
            .field("page_count", &self.page_count)
            .field("quality_score", &self.quality_score)
            .finish()
    }
}

impl fmt::Debug for SearchSelectionDetails {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchSelectionDetails")
            .field("selection", &self.selection)
            .field("version", &self.version)
            .field(
                "candidate_id",
                &self.candidate_id.as_ref().map(|_| "<redacted>"),
            )
            .field("mention_count", &self.mentions.len())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SearchSelectionDetailsResolution {
    Current(Box<SearchSelectionDetails>),
    Stale,
    NotFound,
    LimitExceeded(SearchSelectionLimit),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchSelectionLimit {
    VersionMetadata,
    Mentions,
}

impl SearchMetadataSnapshot<'_> {
    pub fn resolve_search_selection(
        &self,
        selection: &SearchSelection,
    ) -> Result<SearchSelectionResolution> {
        if selection.visible_epoch > self.head.visible_epoch {
            return Ok(SearchSelectionResolution::NotFound);
        }
        resolve_selection(self.connection, &self.head.generation, selection)
    }

    pub fn selection_details(
        &self,
        selection: &SearchSelection,
    ) -> Result<SearchSelectionDetailsResolution> {
        let selection = match self.resolve_search_selection(selection)? {
            SearchSelectionResolution::Current { selection } => selection,
            SearchSelectionResolution::Stale => {
                return Ok(SearchSelectionDetailsResolution::Stale);
            }
            SearchSelectionResolution::NotFound => {
                return Ok(SearchSelectionDetailsResolution::NotFound);
            }
        };
        let version = match bounded_selection_version(
            self.connection,
            &selection.document_id,
            &selection.resume_version_id,
        )? {
            BoundedSelectionVersion::Version(version) => version,
            BoundedSelectionVersion::LimitExceeded => {
                return Ok(SearchSelectionDetailsResolution::LimitExceeded(
                    SearchSelectionLimit::VersionMetadata,
                ));
            }
        };
        let mentions = match bounded_entity_mentions_for_version(
            self.connection,
            &selection.resume_version_id,
        )? {
            BoundedMentions::Mentions(mentions) => mentions,
            BoundedMentions::LimitExceeded => {
                return Ok(SearchSelectionDetailsResolution::LimitExceeded(
                    SearchSelectionLimit::Mentions,
                ));
            }
        };
        let candidate_id =
            candidate_id_for_current_version(self.connection, &selection.resume_version_id)?;
        Ok(SearchSelectionDetailsResolution::Current(Box::new(
            SearchSelectionDetails {
                selection,
                version,
                candidate_id,
                mentions,
            },
        )))
    }
}

pub(super) fn resolve_selection(
    connection: &Connection,
    generation: &str,
    selection: &SearchSelection,
) -> Result<SearchSelectionResolution> {
    let exact_sealed_pair = connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1
                FROM resume_version AS version
                JOIN resume_version_seal AS seal
                  ON seal.resume_version_id = version.id
                WHERE version.id = ?1 AND version.document_id = ?2
             )",
            params![
                selection.resume_version_id.as_str(),
                selection.document_id.as_str(),
            ],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?
        != 0;
    if !exact_sealed_pair {
        return Ok(SearchSelectionResolution::NotFound);
    }
    let Some(active) =
        active_projection_for_generation(connection, generation, &selection.document_id)?
    else {
        return Ok(SearchSelectionResolution::NotFound);
    };
    if active.resume_version_id != selection.resume_version_id {
        return Ok(SearchSelectionResolution::Stale);
    }
    Ok(SearchSelectionResolution::Current {
        selection: selection.clone(),
    })
}

enum BoundedSelectionVersion {
    Version(SearchSelectionVersion),
    LimitExceeded,
}

fn bounded_selection_version(
    connection: &Connection,
    document_id: &DocumentId,
    version_id: &ResumeVersionId,
) -> Result<BoundedSelectionVersion> {
    let row = connection
        .query_row(
            "SELECT version.source_revision_id,
                    revision.content_hash,
                    revision.byte_size,
                    version.normalized_text_hash,
                    version.parse_version,
                    version.schema_version,
                    version.language_set_json,
                    version.page_count,
                    version.quality_score
             FROM resume_version AS version
             JOIN source_revision AS revision
               ON revision.id = version.source_revision_id
              AND revision.document_id = version.document_id
             WHERE version.id = ?1 AND version.document_id = ?2",
            params![version_id.as_str(), document_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<f64>>(8)?,
                ))
            },
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .ok_or_else(MetaStoreError::storage_invariant)?;
    if row.4.len() > MAX_VERSION_LABEL_BYTES
        || row.5.len() > MAX_VERSION_LABEL_BYTES
        || row.6.len() > MAX_LANGUAGE_SET_BYTES
    {
        return Ok(BoundedSelectionVersion::LimitExceeded);
    }
    let language_set = serde_json::from_str::<Vec<String>>(&row.6)
        .map_err(|_| MetaStoreError::invalid_value("resume_version.language_set"))?;
    if language_set.len() > MAX_LANGUAGE_COUNT
        || language_set
            .iter()
            .any(|language| language.len() > MAX_LANGUAGE_BYTES)
    {
        return Ok(BoundedSelectionVersion::LimitExceeded);
    }
    let source_byte_size = u64::try_from(row.2)
        .map_err(|_| MetaStoreError::invalid_value("source_revision.byte_size"))?;
    let page_count = row
        .7
        .map(|page_count| {
            u32::try_from(page_count)
                .map_err(|_| MetaStoreError::invalid_value("resume_version.page_count"))
        })
        .transpose()?;
    Ok(BoundedSelectionVersion::Version(SearchSelectionVersion {
        source_revision_id: SourceRevisionId::from_str(&row.0)
            .map_err(|_| MetaStoreError::invalid_value("resume_version.source_revision_id"))?,
        source_content_hash: ContentDigest::from_str(&row.1)
            .map_err(|_| MetaStoreError::invalid_value("source_revision.content_hash"))?,
        source_byte_size,
        normalized_text_hash: ContentDigest::from_str(&row.3)
            .map_err(|_| MetaStoreError::invalid_value("resume_version.normalized_text_hash"))?,
        parse_version: row.4,
        schema_version: row.5,
        language_set,
        page_count,
        quality_score: row.8.map(|quality_score| quality_score as f32),
    }))
}

fn document_by_id(connection: &Connection, document_id: &DocumentId) -> Result<Option<Document>> {
    let sql = format!("SELECT {DOCUMENT_COLUMNS} FROM document WHERE id = ?1");
    connection
        .query_row(&sql, params![document_id.as_str()], |row| {
            read_document(row).map_err(|_| rusqlite::Error::InvalidQuery)
        })
        .optional()
        .map_err(MetaStoreError::storage)
}

pub(super) enum BoundedDocument {
    Document(Box<Document>),
    LimitExceeded,
}

pub(super) fn bounded_document_by_id(
    connection: &Connection,
    document_id: &DocumentId,
) -> Result<BoundedDocument> {
    let lengths = connection
        .query_row(
            "SELECT length(CAST(source_uri AS BLOB)),
                    length(CAST(normalized_path AS BLOB)),
                    length(CAST(file_name AS BLOB)),
                    length(CAST(extension AS BLOB))
             FROM document WHERE id = ?1",
            params![document_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .ok_or_else(MetaStoreError::storage_invariant)?;
    let lengths = [
        (lengths.0, MAX_DOCUMENT_SOURCE_URI_BYTES),
        (lengths.1, MAX_DOCUMENT_NORMALIZED_PATH_BYTES),
        (lengths.2, MAX_DOCUMENT_FILE_NAME_BYTES),
        (lengths.3, MAX_DOCUMENT_EXTENSION_BYTES),
    ];
    for (actual, maximum) in lengths {
        let actual = u64::try_from(actual)
            .map_err(|_| MetaStoreError::invalid_value("document.metadata_length"))?;
        if actual > maximum {
            return Ok(BoundedDocument::LimitExceeded);
        }
    }
    Ok(BoundedDocument::Document(Box::new(
        document_by_id(connection, document_id)?.ok_or_else(MetaStoreError::storage_invariant)?,
    )))
}

pub(super) enum BoundedMentions {
    Mentions(Vec<EntityMention>),
    LimitExceeded,
}

pub(super) fn bounded_entity_mentions_for_version(
    connection: &Connection,
    version_id: &ResumeVersionId,
) -> Result<BoundedMentions> {
    let oversized = connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM entity_mention
                WHERE resume_version_id = ?1
                  AND (
                    length(CAST(raw_value AS BLOB)) > ?2
                    OR length(CAST(COALESCE(normalized_value, '') AS BLOB)) > ?2
                    OR length(CAST(extractor AS BLOB)) > ?3
                  )
             )",
            params![
                version_id.as_str(),
                i64::try_from(MAX_MENTION_VALUE_BYTES)
                    .map_err(|_| MetaStoreError::storage_invariant())?,
                i64::try_from(MAX_MENTION_EXTRACTOR_BYTES)
                    .map_err(|_| MetaStoreError::storage_invariant())?,
            ],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?
        != 0;
    if oversized {
        return Ok(BoundedMentions::LimitExceeded);
    }
    let sql = format!(
        "SELECT {ENTITY_MENTION_COLUMNS}
         FROM entity_mention WHERE resume_version_id = ?1
         ORDER BY span_start IS NULL, span_start, rowid
         LIMIT ?2"
    );
    let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
    let mut mentions = statement
        .query_map(
            params![
                version_id.as_str(),
                i64::try_from(MAX_SEARCH_SELECTION_MENTIONS + 1)
                    .map_err(|_| MetaStoreError::storage_invariant())?,
            ],
            |row| read_entity_mention(row).map_err(|_| rusqlite::Error::InvalidQuery),
        )
        .map_err(MetaStoreError::storage)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(MetaStoreError::storage)?;
    if mentions.len() > MAX_SEARCH_SELECTION_MENTIONS {
        return Ok(BoundedMentions::LimitExceeded);
    }
    mentions.shrink_to_fit();
    Ok(BoundedMentions::Mentions(mentions))
}

pub(super) fn candidate_id_for_current_version(
    connection: &Connection,
    version_id: &ResumeVersionId,
) -> Result<Option<CandidateId>> {
    connection
        .query_row(
            "SELECT assignment.candidate_id
             FROM resume_version_candidate AS assignment
             JOIN resume_version_seal AS seal
               ON seal.resume_version_id = assignment.resume_version_id
              AND seal.candidate_id = assignment.candidate_id
             WHERE assignment.resume_version_id = ?1",
            params![version_id.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .map(|candidate| {
            CandidateId::from_str(&candidate)
                .map_err(|_| MetaStoreError::invalid_value("resume_version_candidate.candidate_id"))
        })
        .transpose()
}
