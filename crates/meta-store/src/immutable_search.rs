use std::{collections::BTreeMap, fmt, str::FromStr};

use rusqlite::{params, Connection, OptionalExtension};

use super::{
    entity_mention_normalized_value_for_storage, entity_mention_raw_value_for_storage,
    entity_type_to_storage, read_entity_mention, resume_version_by_id_from_connection,
    search_publication::search_publication_in_connection,
    search_snapshot::{
        MAX_MENTION_EXTRACTOR_BYTES, MAX_MENTION_VALUE_BYTES, MAX_SEARCH_SELECTION_MENTIONS,
    },
    validate_entity_mention, ActiveSearchProjection, CandidateId, ContentDigest, DocumentId,
    EntityMention, MetaStore, MetaStoreError, Result, ResumeVersion, ResumeVersionId,
    SearchPublicationRecord, SearchSelection, SectionId, SourceRevision, SourceRevisionId,
    UnixTimestamp, ENTITY_MENTION_COLUMNS,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityInsertOutcome {
    Inserted,
    AlreadyPresent,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SearchSelectionResolution {
    Current { selection: SearchSelection },
    Stale,
    NotFound,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchProjectionServiceState {
    Repairing,
    Ready,
    RepairBlocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchRepairReason {
    MigrationRebuild,
    ArtifactUnavailable,
    SourceUnavailable,
    RuntimeInvariant,
}

impl SearchRepairReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::MigrationRebuild => "migration_rebuild",
            Self::ArtifactUnavailable => "artifact_unavailable",
            Self::SourceUnavailable => "source_unavailable",
            Self::RuntimeInvariant => "runtime_invariant",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SearchProjectionState {
    pub service_state: SearchProjectionServiceState,
    pub generation: Option<String>,
    pub visible_epoch: u64,
    pub repair_reason: Option<SearchRepairReason>,
    pub publication: Option<Box<SearchPublicationRecord>>,
    pub updated_at: UnixTimestamp,
}

impl fmt::Debug for SearchProjectionState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchProjectionState")
            .field("service_state", &self.service_state)
            .field(
                "generation",
                &self.generation.as_ref().map(|_| "<redacted>"),
            )
            .field("visible_epoch", &self.visible_epoch)
            .field("repair_reason", &self.repair_reason)
            .field("publication", &self.publication)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

impl MetaStore {
    pub fn insert_source_revision(
        &self,
        revision: &SourceRevision,
    ) -> Result<IdentityInsertOutcome> {
        insert_source_revision_in_connection(&self.connection.borrow(), revision)
    }

    pub fn source_revision_by_id(&self, id: &SourceRevisionId) -> Result<Option<SourceRevision>> {
        source_revision_by_id_from_connection(&self.connection.borrow(), id)
    }

    pub fn insert_resume_version(&self, version: &ResumeVersion) -> Result<IdentityInsertOutcome> {
        insert_resume_version_in_connection(&self.connection.borrow(), version)
    }

    pub fn insert_entity_mentions(
        &self,
        version_id: &ResumeVersionId,
        mentions: &[EntityMention],
    ) -> Result<IdentityInsertOutcome> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        let outcome = insert_entity_mentions_in_connection(&transaction, version_id, mentions)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(outcome)
    }

    pub fn insert_candidate_assignment(
        &self,
        version_id: &ResumeVersionId,
        candidate_id: &CandidateId,
    ) -> Result<IdentityInsertOutcome> {
        insert_candidate_assignment_in_connection(
            &self.connection.borrow(),
            version_id,
            candidate_id,
        )
    }

    pub fn candidate_assignment_for_version(
        &self,
        version_id: &ResumeVersionId,
    ) -> Result<Option<CandidateId>> {
        candidate_id_for_version_from_connection(&self.connection.borrow(), version_id)
    }

    pub fn active_search_projection_for_document(
        &self,
        document_id: &DocumentId,
    ) -> Result<Option<ActiveSearchProjection>> {
        active_projection_from_connection(&self.connection.borrow(), document_id)
    }

    pub fn search_projection_state(&self) -> Result<SearchProjectionState> {
        let connection = self.connection.borrow();
        let (state, generation, epoch, repair_reason, updated_at) = connection
            .query_row(
                "SELECT service_state, generation, visible_epoch, repair_reason,
                        updated_at_seconds
                 FROM search_projection_state WHERE state_key = 'default'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .map_err(MetaStoreError::storage)?;
        let service_state = projection_service_state_from_storage(&state)?;
        let publication = generation
            .as_deref()
            .map(|generation| search_publication_in_connection(&connection, generation))
            .transpose()?
            .flatten()
            .map(Box::new);
        if service_state == SearchProjectionServiceState::Ready
            && publication
                .as_ref()
                .is_none_or(|publication| publication.state != super::SearchPublicationState::Ready)
        {
            return Err(MetaStoreError::storage_invariant());
        }
        Ok(SearchProjectionState {
            service_state,
            generation,
            visible_epoch: u64::try_from(epoch).map_err(|_| {
                MetaStoreError::invalid_value("search_projection_state.visible_epoch")
            })?,
            repair_reason: repair_reason
                .as_deref()
                .map(repair_reason_from_storage)
                .transpose()?,
            publication,
            updated_at: UnixTimestamp::from_unix_seconds(updated_at),
        })
    }

    pub fn mark_search_repairing(
        &self,
        reason: SearchRepairReason,
        now: UnixTimestamp,
    ) -> Result<()> {
        mark_search_service_state(
            &self.connection.borrow(),
            SearchProjectionServiceState::Repairing,
            reason,
            now,
        )
    }

    pub fn mark_search_repair_blocked(
        &self,
        reason: SearchRepairReason,
        now: UnixTimestamp,
    ) -> Result<()> {
        mark_search_service_state(
            &self.connection.borrow(),
            SearchProjectionServiceState::RepairBlocked,
            reason,
            now,
        )
    }
}

pub(super) fn insert_source_revision_in_connection(
    connection: &Connection,
    revision: &SourceRevision,
) -> Result<IdentityInsertOutcome> {
    validate_source_revision(revision)?;
    let changed = connection
        .execute(
            "INSERT OR IGNORE INTO source_revision (id, document_id, content_hash, byte_size)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                revision.id.as_str(),
                revision.document_id.as_str(),
                revision.content_hash.as_str(),
                u64_to_i64(revision.byte_size, "source_revision.byte_size")?,
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if changed == 1 {
        return Ok(IdentityInsertOutcome::Inserted);
    }
    match source_revision_by_id_from_connection(connection, &revision.id)? {
        Some(existing) if existing == *revision => Ok(IdentityInsertOutcome::AlreadyPresent),
        Some(_) => Err(MetaStoreError::immutable_identity_conflict(
            "source_revision",
        )),
        None => Err(MetaStoreError::storage_invariant()),
    }
}

pub(super) fn insert_resume_version_in_connection(
    connection: &Connection,
    version: &ResumeVersion,
) -> Result<IdentityInsertOutcome> {
    validate_resume_version_identity(version)?;
    let language_set_json = serde_json::to_string(&version.language_set)
        .map_err(|_| MetaStoreError::invalid_value("resume_version.language_set"))?;
    let changed = connection
        .execute(
            "INSERT OR IGNORE INTO resume_version (
                id, document_id, source_revision_id, normalized_text_hash,
                parse_version, schema_version, language_set_json, page_count,
                raw_text, clean_text, quality_score
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                version.id.as_str(),
                version.document_id.as_str(),
                version.source_revision_id.as_str(),
                version.normalized_text_hash.as_str(),
                version.parse_version,
                version.schema_version,
                language_set_json,
                version.page_count.map(i64::from),
                version.raw_text,
                version.clean_text,
                version.quality_score.map(f64::from),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if changed == 1 {
        return Ok(IdentityInsertOutcome::Inserted);
    }
    match resume_version_by_id_from_connection(connection, &version.id)? {
        Some(existing) if existing == *version => Ok(IdentityInsertOutcome::AlreadyPresent),
        Some(_) => Err(MetaStoreError::immutable_identity_conflict(
            "resume_version",
        )),
        None => Err(MetaStoreError::storage_invariant()),
    }
}

pub(super) fn insert_entity_mentions_in_connection(
    connection: &Connection,
    version_id: &ResumeVersionId,
    mentions: &[EntityMention],
) -> Result<IdentityInsertOutcome> {
    let mut unique = BTreeMap::new();
    for mention in mentions {
        validate_entity_mention(version_id, mention)?;
        let stored = stored_mention(mention);
        if stored.raw_value.len() > MAX_MENTION_VALUE_BYTES
            || stored
                .normalized_value
                .as_deref()
                .is_some_and(|value| value.len() > MAX_MENTION_VALUE_BYTES)
            || stored.extractor.len() > MAX_MENTION_EXTRACTOR_BYTES
        {
            return Err(MetaStoreError::invalid_value("entity_mention.size"));
        }
        if let Some(existing) = unique.insert(stored.id.clone(), stored.clone()) {
            if existing != stored {
                return Err(MetaStoreError::immutable_identity_conflict(
                    "entity_mention",
                ));
            }
        }
    }
    let existing_count = connection
        .query_row(
            "SELECT COUNT(*) FROM entity_mention WHERE resume_version_id = ?1",
            params![version_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    let existing_count =
        usize::try_from(existing_count).map_err(|_| MetaStoreError::storage_invariant())?;
    if existing_count > MAX_SEARCH_SELECTION_MENTIONS {
        return Err(MetaStoreError::storage_invariant());
    }
    let mut additions = 0_usize;
    for stored in unique.values() {
        match entity_mention_by_id_from_connection(connection, &stored.id)? {
            Some(existing) if existing == *stored => {}
            Some(_) => {
                return Err(MetaStoreError::immutable_identity_conflict(
                    "entity_mention",
                ));
            }
            None => additions += 1,
        }
    }
    if existing_count.saturating_add(additions) > MAX_SEARCH_SELECTION_MENTIONS {
        return Err(MetaStoreError::invalid_value("entity_mention.count"));
    }

    let mut outcome = IdentityInsertOutcome::AlreadyPresent;
    for stored in unique.into_values() {
        if let Some(existing) = entity_mention_by_id_from_connection(connection, &stored.id)? {
            if existing != stored {
                return Err(MetaStoreError::immutable_identity_conflict(
                    "entity_mention",
                ));
            }
            continue;
        }
        require_unsealed_version(connection, version_id)?;
        connection
            .execute(
                "INSERT INTO entity_mention (
                    id, resume_version_id, section_id, entity_type, raw_value,
                    normalized_value, span_start, span_end, confidence, extractor
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    stored.id.as_str(),
                    stored.resume_version_id.as_str(),
                    stored.section_id.as_ref().map(SectionId::as_str),
                    entity_type_to_storage(&stored.entity_type),
                    stored.raw_value,
                    stored.normalized_value,
                    stored.span_start.map(|value| value as i64),
                    stored.span_end.map(|value| value as i64),
                    f64::from(stored.confidence),
                    stored.extractor,
                ],
            )
            .map_err(MetaStoreError::storage)?;
        outcome = IdentityInsertOutcome::Inserted;
    }
    Ok(outcome)
}

pub(super) fn insert_candidate_assignment_in_connection(
    connection: &Connection,
    version_id: &ResumeVersionId,
    candidate_id: &CandidateId,
) -> Result<IdentityInsertOutcome> {
    let existing = candidate_id_for_version_from_connection(connection, version_id)?;
    if existing.as_ref() == Some(candidate_id) {
        return Ok(IdentityInsertOutcome::AlreadyPresent);
    } else if existing.is_some() {
        return Err(MetaStoreError::immutable_identity_conflict(
            "resume_version_candidate",
        ));
    }
    require_unsealed_version(connection, version_id)?;
    connection
        .execute(
            "INSERT INTO resume_version_candidate (resume_version_id, candidate_id)
             VALUES (?1, ?2)",
            params![version_id.as_str(), candidate_id.as_str()],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(IdentityInsertOutcome::Inserted)
}

pub(super) fn require_unsealed_version(
    connection: &Connection,
    version_id: &ResumeVersionId,
) -> Result<()> {
    let sealed = connection
        .query_row(
            "SELECT 1 FROM resume_version_seal WHERE resume_version_id = ?1",
            params![version_id.as_str()],
            |_| Ok(()),
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .is_some();
    if sealed {
        return Err(MetaStoreError::invalid_transition());
    }
    Ok(())
}

pub(super) fn seal_resume_version(
    connection: &Connection,
    version_id: &ResumeVersionId,
    now: UnixTimestamp,
) -> Result<()> {
    let mention_count = connection
        .query_row(
            "SELECT COUNT(*) FROM entity_mention WHERE resume_version_id = ?1",
            params![version_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    let candidate_id = candidate_id_for_version_from_connection(connection, version_id)?;
    let changed = connection
        .execute(
            "INSERT OR IGNORE INTO resume_version_seal (
                resume_version_id, sealed_at_seconds, entity_mention_count, candidate_id
             ) VALUES (?1, ?2, ?3, ?4)",
            params![
                version_id.as_str(),
                now.as_unix_seconds(),
                mention_count,
                candidate_id.as_ref().map(CandidateId::as_str),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if changed == 1 {
        return Ok(());
    }
    let existing = connection
        .query_row(
            "SELECT entity_mention_count, candidate_id FROM resume_version_seal
             WHERE resume_version_id = ?1",
            params![version_id.as_str()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(MetaStoreError::storage)?;
    if existing
        == Some((
            mention_count,
            candidate_id.as_ref().map(ToString::to_string),
        ))
    {
        Ok(())
    } else {
        Err(MetaStoreError::storage_invariant())
    }
}

fn validate_source_revision(revision: &SourceRevision) -> Result<()> {
    if SourceRevisionId::from_content_identity(&revision.document_id, &revision.content_hash)
        != revision.id
    {
        return Err(MetaStoreError::invalid_value("source_revision.id"));
    }
    Ok(())
}

fn validate_resume_version_identity(version: &ResumeVersion) -> Result<()> {
    let Some(clean_text) = version.clean_text.as_deref() else {
        return Err(MetaStoreError::invalid_value("resume_version.clean_text"));
    };
    if clean_text.contains('\0')
        || ContentDigest::from_bytes(clean_text.as_bytes()) != version.normalized_text_hash
    {
        return Err(MetaStoreError::invalid_value(
            "resume_version.normalized_text_hash",
        ));
    }
    let expected = ResumeVersionId::from_content_identity(
        &version.document_id,
        &version.source_revision_id,
        &version.normalized_text_hash,
        &version.parse_version,
        &version.schema_version,
    );
    if expected != version.id
        || version.parse_version.trim().is_empty()
        || version.schema_version.trim().is_empty()
    {
        return Err(MetaStoreError::invalid_value("resume_version.identity"));
    }
    Ok(())
}

fn source_revision_by_id_from_connection(
    connection: &Connection,
    id: &SourceRevisionId,
) -> Result<Option<SourceRevision>> {
    connection
        .query_row(
            "SELECT id, document_id, content_hash, byte_size FROM source_revision WHERE id = ?1",
            params![id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .map(|(id, document_id, content_hash, byte_size)| {
            Ok(SourceRevision {
                id: SourceRevisionId::from_str(&id)
                    .map_err(|_| MetaStoreError::invalid_value("source_revision.id"))?,
                document_id: DocumentId::from_str(&document_id)
                    .map_err(|_| MetaStoreError::invalid_value("source_revision.document_id"))?,
                content_hash: ContentDigest::from_str(&content_hash)
                    .map_err(|_| MetaStoreError::invalid_value("source_revision.content_hash"))?,
                byte_size: u64::try_from(byte_size)
                    .map_err(|_| MetaStoreError::invalid_value("source_revision.byte_size"))?,
            })
        })
        .transpose()
}

fn entity_mention_by_id_from_connection(
    connection: &Connection,
    id: &core_domain::EntityMentionId,
) -> Result<Option<EntityMention>> {
    let sql = format!("SELECT {ENTITY_MENTION_COLUMNS} FROM entity_mention WHERE id = ?1");
    connection
        .query_row(&sql, params![id.as_str()], |row| {
            read_entity_mention(row).map_err(|_| rusqlite::Error::InvalidQuery)
        })
        .optional()
        .map_err(MetaStoreError::storage)
}

fn stored_mention(mention: &EntityMention) -> EntityMention {
    let mut stored = mention.clone();
    stored.raw_value = entity_mention_raw_value_for_storage(mention).to_string();
    stored.normalized_value =
        entity_mention_normalized_value_for_storage(mention).map(str::to_string);
    stored
}

fn candidate_id_for_version_from_connection(
    connection: &Connection,
    version_id: &ResumeVersionId,
) -> Result<Option<CandidateId>> {
    connection
        .query_row(
            "SELECT candidate_id FROM resume_version_candidate WHERE resume_version_id = ?1",
            params![version_id.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .map(|value| {
            CandidateId::from_str(&value)
                .map_err(|_| MetaStoreError::invalid_value("resume_version_candidate.candidate_id"))
        })
        .transpose()
}

fn active_projection_from_connection(
    connection: &Connection,
    document_id: &DocumentId,
) -> Result<Option<ActiveSearchProjection>> {
    connection
        .query_row(
            "SELECT document_id, resume_version_id
             FROM active_search_projection WHERE document_id = ?1",
            params![document_id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .map(|(document, version)| {
            Ok(ActiveSearchProjection {
                document_id: DocumentId::from_str(&document).map_err(|_| {
                    MetaStoreError::invalid_value("active_search_projection.document_id")
                })?,
                resume_version_id: ResumeVersionId::from_str(&version).map_err(|_| {
                    MetaStoreError::invalid_value("active_search_projection.resume_version_id")
                })?,
            })
        })
        .transpose()
}

fn projection_service_state_from_storage(value: &str) -> Result<SearchProjectionServiceState> {
    match value {
        "repairing" => Ok(SearchProjectionServiceState::Repairing),
        "ready" => Ok(SearchProjectionServiceState::Ready),
        "repair_blocked" => Ok(SearchProjectionServiceState::RepairBlocked),
        _ => Err(MetaStoreError::invalid_value(
            "search_projection_state.service_state",
        )),
    }
}

fn mark_search_service_state(
    connection: &Connection,
    state: SearchProjectionServiceState,
    reason: SearchRepairReason,
    now: UnixTimestamp,
) -> Result<()> {
    let state = match state {
        SearchProjectionServiceState::Repairing => "repairing",
        SearchProjectionServiceState::RepairBlocked => "repair_blocked",
        SearchProjectionServiceState::Ready => return Err(MetaStoreError::invalid_transition()),
    };
    let changed = connection
        .execute(
            "UPDATE search_projection_state
             SET service_state = ?1, repair_reason = ?2, updated_at_seconds = ?3
             WHERE state_key = 'default'",
            params![state, reason.as_str(), now.as_unix_seconds()],
        )
        .map_err(MetaStoreError::storage)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(MetaStoreError::storage_invariant())
    }
}

fn repair_reason_from_storage(value: &str) -> Result<SearchRepairReason> {
    match value {
        "migration_rebuild" => Ok(SearchRepairReason::MigrationRebuild),
        "artifact_unavailable" => Ok(SearchRepairReason::ArtifactUnavailable),
        "source_unavailable" => Ok(SearchRepairReason::SourceUnavailable),
        "runtime_invariant" => Ok(SearchRepairReason::RuntimeInvariant),
        _ => Err(MetaStoreError::invalid_value(
            "search_projection_state.repair_reason",
        )),
    }
}

fn u64_to_i64(value: u64, field: &'static str) -> Result<i64> {
    i64::try_from(value).map_err(|_| MetaStoreError::invalid_value(field))
}
