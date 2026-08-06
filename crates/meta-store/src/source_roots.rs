use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::{
    import_root_head::coordinate_import_root_task_head_in_connection,
    import_task_status_to_storage, validate_import_scan_scope, validate_import_task,
    ClassificationCounts, ContentDigest, CurrentClassifierEpoch, ImportProcessingContract,
    ImportRootTaskHeadOutcome, ImportRootTaskHeadRequest, ImportScanScope, ImportTask,
    ImportTaskId, ImportTaskStatus, MetaStoreError, MetadataStore, MetadataStoreAccess,
    MetadataStoreWriteAccess, Result, SearchSourceFileReference, UnixTimestamp,
};
use core_domain::{DocumentId, SourceRevisionId};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

const MAX_SOURCE_ROOTS: usize = 16;
const MAX_PATH_BYTES: usize = 128 * 1024;
const MAX_RELATIVE_PATH_BYTES: usize = 4 * 1024;
const MAX_DISPLAY_LABEL_CHARS: usize = 80;

#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceRootId(String);

impl SourceRootId {
    pub fn new() -> Result<Self> {
        let mut random = [0_u8; 16];
        getrandom::getrandom(&mut random).map_err(|_| MetaStoreError::random())?;
        Ok(Self(format!("root-{}", crate::encode_hex(&random))))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for SourceRootId {
    type Err = MetaStoreError;

    fn from_str(value: &str) -> Result<Self> {
        if value.len() != 37
            || !value.starts_with("root-")
            || !value[5..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(MetaStoreError::invalid_value("source_root.id"));
        }
        Ok(Self(value.to_string()))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceRootState {
    Active,
    Offline,
}

impl SourceRootState {
    fn storage(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Offline => "offline",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "active" => Ok(Self::Active),
            "offline" => Ok(Self::Offline),
            _ => Err(MetaStoreError::invalid_value("source_root.state")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceWatcherState {
    Active,
    Paused,
    Unavailable,
}

impl SourceWatcherState {
    fn storage(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Unavailable => "unavailable",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "active" => Ok(Self::Active),
            "paused" => Ok(Self::Paused),
            "unavailable" => Ok(Self::Unavailable),
            _ => Err(MetaStoreError::invalid_value("source_root.watcher_state")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRoot {
    pub id: SourceRootId,
    pub canonical_path: String,
    pub requested_path: String,
    pub display_label: String,
    pub state: SourceRootState,
    pub watcher_state: SourceWatcherState,
    pub created_at: UnixTimestamp,
    pub updated_at: UnixTimestamp,
}

/// One validated directory-authority row used by atomic source-root imports.
///
/// Callers must canonicalize the path at the native boundary before creating
/// this value. The store revalidates bounds, overlap, capacity and the complete
/// batch before any row is written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRootRegistration {
    pub canonical_path: String,
    pub requested_path: String,
    pub display_label: String,
    pub availability: SourceRootRegistrationAvailability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceRootRegistrationAvailability {
    Available,
    Offline,
}

impl SourceRootRegistrationAvailability {
    fn root_state(self) -> SourceRootState {
        match self {
            Self::Available => SourceRootState::Active,
            Self::Offline => SourceRootState::Offline,
        }
    }

    fn watcher_state(self) -> SourceWatcherState {
        match self {
            Self::Available => SourceWatcherState::Active,
            Self::Offline => SourceWatcherState::Unavailable,
        }
    }
}

impl std::fmt::Debug for SourceRootId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SourceRootId(<opaque>)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanTrigger {
    Initial,
    Manual,
    Watcher,
    Periodic,
    Recovery,
}

impl ScanTrigger {
    fn storage(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::Manual => "manual",
            Self::Watcher => "watcher",
            Self::Periodic => "periodic",
            Self::Recovery => "recovery",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "initial" => Ok(Self::Initial),
            "manual" => Ok(Self::Manual),
            "watcher" => Ok(Self::Watcher),
            "periodic" => Ok(Self::Periodic),
            "recovery" => Ok(Self::Recovery),
            _ => Err(MetaStoreError::invalid_value("scan_snapshot.trigger")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanPhase {
    Queued,
    Discovering,
    Fingerprinting,
    Classifying,
    Parsing,
    Ocr,
    Publishing,
    Complete,
    Partial,
    Failed,
}

impl ScanPhase {
    fn storage(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Discovering => "discovering",
            Self::Fingerprinting => "fingerprinting",
            Self::Classifying => "classifying",
            Self::Parsing => "parsing",
            Self::Ocr => "ocr",
            Self::Publishing => "publishing",
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "queued" => Ok(Self::Queued),
            "discovering" => Ok(Self::Discovering),
            "fingerprinting" => Ok(Self::Fingerprinting),
            "classifying" => Ok(Self::Classifying),
            "parsing" => Ok(Self::Parsing),
            "ocr" => Ok(Self::Ocr),
            "publishing" => Ok(Self::Publishing),
            "complete" => Ok(Self::Complete),
            "partial" => Ok(Self::Partial),
            "failed" => Ok(Self::Failed),
            _ => Err(MetaStoreError::invalid_value("scan_snapshot.phase")),
        }
    }

    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::Discovering
                | Self::Fingerprinting
                | Self::Classifying
                | Self::Parsing
                | Self::Ocr
                | Self::Publishing
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanCompleteness {
    Unknown,
    Complete,
    Partial,
}

impl ScanCompleteness {
    fn storage(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Complete => "complete",
            Self::Partial => "partial",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "unknown" => Ok(Self::Unknown),
            "complete" => Ok(Self::Complete),
            "partial" => Ok(Self::Partial),
            _ => Err(MetaStoreError::invalid_value("scan_snapshot.completeness")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScanCounts {
    pub discovered: u64,
    pub searchable: u64,
    pub non_resume: u64,
    pub needs_review: u64,
    pub ocr: u64,
    pub failed: u64,
    pub ignored: u64,
    pub processed: u64,
    pub total: Option<u64>,
    pub errors: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScanSnapshot {
    pub id: String,
    pub root_id: SourceRootId,
    pub trigger: ScanTrigger,
    pub phase: ScanPhase,
    pub completeness: ScanCompleteness,
    pub counts: ScanCounts,
    pub rate_per_second: Option<f64>,
    pub eta_seconds: Option<u64>,
    pub started_at: UnixTimestamp,
    pub updated_at: UnixTimestamp,
    pub completed_at: Option<UnixTimestamp>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SourceScanProgress {
    pub discovered: u64,
    pub searchable: u64,
    pub ocr: u64,
    pub failed: u64,
    pub ignored: u64,
    pub processed: u64,
    pub errors: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BeginScanOutcome {
    Started(ScanSnapshot),
    Coalesced(ScanSnapshot),
}

#[derive(Clone, Debug, PartialEq)]
pub enum SourceRootScanCoordination {
    Started {
        snapshot: ScanSnapshot,
        task_head: Box<ImportRootTaskHeadOutcome>,
    },
    Coalesced(ScanSnapshot),
    Rejected(ImportRootTaskHeadOutcome),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OccurrenceChange {
    Inserted,
    Unchanged,
    Replaced,
}

pub(crate) const SOURCE_ROOT_CLASSIFICATION_COUNTS_SQL: &str = "
    SELECT
        COALESCE(SUM(EXISTS (
            SELECT 1
            FROM resume_version AS version INDEXED BY resume_version_document_idx
            JOIN resume_version_classification AS classification
              ON classification.resume_version_id = version.id
            WHERE version.document_id = occurrence.document_id
              AND version.source_revision_id = occurrence.source_revision_id
              AND classification.classifier_epoch = ?2
              AND classification.status = 'resume_candidate'
        )), 0),
        COALESCE(SUM(EXISTS (
            SELECT 1
            FROM resume_version AS version INDEXED BY resume_version_document_idx
            JOIN resume_version_classification AS classification
              ON classification.resume_version_id = version.id
            WHERE version.document_id = occurrence.document_id
              AND version.source_revision_id = occurrence.source_revision_id
              AND classification.classifier_epoch = ?2
              AND classification.status = 'non_resume'
        )), 0),
        COALESCE(SUM(EXISTS (
            SELECT 1
            FROM resume_version AS version INDEXED BY resume_version_document_idx
            JOIN resume_version_classification AS classification
              ON classification.resume_version_id = version.id
            WHERE version.document_id = occurrence.document_id
              AND version.source_revision_id = occurrence.source_revision_id
              AND classification.classifier_epoch = ?2
              AND classification.status = 'needs_review'
        )), 0),
        COALESCE(SUM(EXISTS (
            SELECT 1
            FROM source_revision_triage AS triage
            WHERE triage.source_revision_id = occurrence.source_revision_id
              AND triage.triage_epoch = ?3
              AND triage.status = 'ocr_backlog'
        )), 0),
        COALESCE(SUM(
            EXISTS (
                SELECT 1
                FROM resume_version AS version INDEXED BY resume_version_document_idx
                JOIN resume_version_classification AS classification
                  ON classification.resume_version_id = version.id
                WHERE version.document_id = occurrence.document_id
                  AND version.source_revision_id = occurrence.source_revision_id
                  AND classification.classifier_epoch = ?2
                  AND classification.status = 'failed'
            )
            OR EXISTS (
                SELECT 1
                FROM source_revision_triage AS triage
                WHERE triage.source_revision_id = occurrence.source_revision_id
                  AND triage.triage_epoch = ?3
                  AND triage.status = 'failed'
            )
        ), 0)
     FROM source_occurrence AS occurrence
     WHERE occurrence.root_id = ?1
       AND occurrence.state = 'present'";

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    pub fn source_roots(&self) -> Result<Vec<SourceRoot>> {
        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare(
                "SELECT id, canonical_path, requested_path, display_label, state,
                        watcher_state, created_at_seconds, updated_at_seconds
                 FROM source_root ORDER BY created_at_seconds, id",
            )
            .map_err(MetaStoreError::storage)?;
        let roots = statement
            .query_map([], read_source_root)
            .map_err(MetaStoreError::storage)?
            .map(|row| {
                row.map_err(MetaStoreError::storage)
                    .and_then(validate_source_root)
            })
            .collect();
        roots
    }

    pub fn source_root(&self, id: &SourceRootId) -> Result<Option<SourceRoot>> {
        self.connection
            .borrow()
            .query_row(
                "SELECT id, canonical_path, requested_path, display_label, state,
                        watcher_state, created_at_seconds, updated_at_seconds
                 FROM source_root WHERE id = ?1",
                params![id.as_str()],
                read_source_root,
            )
            .optional()
            .map_err(MetaStoreError::storage)?
            .map(validate_source_root)
            .transpose()
    }

    pub fn source_root_by_canonical_path(
        &self,
        canonical_path: &str,
    ) -> Result<Option<SourceRoot>> {
        self.connection
            .borrow()
            .query_row(
                "SELECT id, canonical_path, requested_path, display_label, state,
                        watcher_state, created_at_seconds, updated_at_seconds
                 FROM source_root WHERE canonical_path = ?1",
                params![canonical_path],
                read_source_root,
            )
            .optional()
            .map_err(MetaStoreError::storage)?
            .map(validate_source_root)
            .transpose()
    }

    pub fn active_source_file_for_revision(
        &self,
        document_id: &DocumentId,
        source_revision_id: &SourceRevisionId,
    ) -> Result<Option<SearchSourceFileReference>> {
        self.connection
            .borrow()
            .query_row(
                "SELECT root.canonical_path, occurrence.relative_path,
                        revision.content_hash, revision.byte_size, document.extension
                 FROM source_occurrence AS occurrence
                 JOIN source_root AS root ON root.id = occurrence.root_id
                 JOIN source_revision AS revision
                   ON revision.id = occurrence.source_revision_id
                  AND revision.document_id = occurrence.document_id
                 JOIN document ON document.id = occurrence.document_id
                 WHERE occurrence.document_id = ?1
                   AND occurrence.source_revision_id = ?2
                   AND occurrence.state = 'present'
                   AND root.state = 'active'
                   AND NOT EXISTS (
                       SELECT 1
                       FROM source_root_deletion AS deletion
                       WHERE deletion.root_id = root.id
                         AND deletion.phase NOT IN ('complete', 'failed')
                   )
                   AND document.is_deleted = 0
                   AND document.status <> 'deleted'
                   AND document.content_hash = revision.content_hash
                 ORDER BY root.id, occurrence.relative_path
                 LIMIT 1",
                params![document_id.as_str(), source_revision_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(MetaStoreError::storage)?
            .map(|(root, relative, hash, byte_size, extension)| {
                Ok(SearchSourceFileReference {
                    root_path: PathBuf::from(root),
                    relative_path: PathBuf::from(relative),
                    source_revision_id: source_revision_id.clone(),
                    content_hash: ContentDigest::from_str(&hash).map_err(|_| {
                        MetaStoreError::invalid_value("source_revision.content_hash")
                    })?,
                    byte_size: u64::try_from(byte_size)
                        .map_err(|_| MetaStoreError::invalid_value("source_revision.byte_size"))?,
                    extension: crate::file_extension_from_storage(&extension),
                })
            })
            .transpose()
    }

    pub fn source_root_classification_counts(
        &self,
        root_id: &SourceRootId,
        classifier_epoch: &str,
    ) -> Result<ClassificationCounts> {
        if CurrentClassifierEpoch::parse(classifier_epoch).is_none() {
            return Err(MetaStoreError::invalid_value(
                "source_root_classification_counts.classifier_epoch",
            ));
        }
        let processing_contract = self
            .active_import_processing_contract()?
            .ok_or_else(|| MetaStoreError::not_found("import_processing_contract"))?;
        if processing_contract.classifier_epoch() != classifier_epoch {
            return Err(MetaStoreError::invalid_value(
                "source_root_classification_counts.classifier_epoch",
            ));
        }
        let source_triage_epoch = processing_contract.source_triage_epoch();
        let counts = self
            .connection
            .borrow()
            .query_row(
                SOURCE_ROOT_CLASSIFICATION_COUNTS_SQL,
                params![
                    root_id.as_str(),
                    classifier_epoch,
                    source_triage_epoch.as_str(),
                ],
                |row| {
                    Ok([
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ])
                },
            )
            .map_err(MetaStoreError::storage)?;
        Ok(ClassificationCounts {
            resume_candidate: non_negative(counts[0]).map_err(MetaStoreError::storage)?,
            non_resume: non_negative(counts[1]).map_err(MetaStoreError::storage)?,
            needs_review: non_negative(counts[2]).map_err(MetaStoreError::storage)?,
            ocr_backlog: non_negative(counts[3]).map_err(MetaStoreError::storage)?,
            failed: non_negative(counts[4]).map_err(MetaStoreError::storage)?,
        })
    }

    pub fn source_root_searchable_count(&self, root_id: &SourceRootId) -> Result<u64> {
        let count = self
            .connection
            .borrow()
            .query_row(
                "SELECT COUNT(*)
                 FROM source_occurrence AS occurrence
                 WHERE occurrence.root_id = ?1
                   AND occurrence.state = 'present'
                   AND EXISTS (
                    SELECT 1
                    FROM active_search_projection AS projection
                    JOIN resume_version AS version
                      ON version.id = projection.resume_version_id
                    WHERE projection.document_id = occurrence.document_id
                      AND version.source_revision_id = occurrence.source_revision_id
                   )",
                params![root_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        non_negative(count).map_err(MetaStoreError::storage)
    }

    pub fn source_root_present_count(&self, root_id: &SourceRootId) -> Result<u64> {
        let count = self
            .connection
            .borrow()
            .query_row(
                "SELECT COUNT(*)
                 FROM source_occurrence
                 WHERE root_id = ?1 AND state = 'present'",
                params![root_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        non_negative(count).map_err(MetaStoreError::storage)
    }

    pub fn latest_scan_snapshot(&self, root_id: &SourceRootId) -> Result<Option<ScanSnapshot>> {
        self.connection
            .borrow()
            .query_row(
                "SELECT id, root_id, trigger, phase, completeness,
                        discovered_count, searchable_count, non_resume_count,
                        needs_review_count, ocr_count, failed_count, ignored_count,
                        processed_count, total_count, rate_per_second, eta_seconds,
                        error_count, started_at_seconds, updated_at_seconds,
                        completed_at_seconds
                 FROM scan_snapshot
                 WHERE root_id = ?1
                 ORDER BY started_at_seconds DESC, id DESC LIMIT 1",
                params![root_id.as_str()],
                read_scan_snapshot,
            )
            .optional()
            .map_err(MetaStoreError::storage)?
            .map(validate_scan_snapshot)
            .transpose()
    }

    pub fn register_source_root(
        &self,
        canonical_path: &str,
        requested_path: &str,
        display_label: &str,
        now: UnixTimestamp,
    ) -> Result<SourceRoot>
    where
        Access: MetadataStoreWriteAccess,
    {
        self.register_source_roots_atomically(
            &[SourceRootRegistration {
                canonical_path: canonical_path.to_string(),
                requested_path: requested_path.to_string(),
                display_label: display_label.to_string(),
                availability: SourceRootRegistrationAvailability::Available,
            }],
            now,
        )?
        .into_iter()
        .next()
        .ok_or_else(MetaStoreError::storage_invariant)
    }

    pub fn register_source_roots_atomically(
        &self,
        registrations: &[SourceRootRegistration],
        now: UnixTimestamp,
    ) -> Result<Vec<SourceRoot>>
    where
        Access: MetadataStoreWriteAccess,
    {
        if registrations.is_empty() || registrations.len() > MAX_SOURCE_ROOTS {
            return Err(MetaStoreError::invalid_value(
                "source_root.registration_count",
            ));
        }
        for registration in registrations {
            validate_canonical_path(&registration.canonical_path)?;
            validate_canonical_path(&registration.requested_path)?;
            validate_display_label(&registration.display_label)?;
        }
        for (index, registration) in registrations.iter().enumerate() {
            let requested = Path::new(&registration.canonical_path);
            if registrations[index + 1..].iter().any(|other| {
                let other = Path::new(&other.canonical_path);
                requested.starts_with(other) || other.starts_with(requested)
            }) {
                return Err(MetaStoreError::invalid_value("source_root.overlap"));
            }
        }

        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let existing = all_source_root_identities(&transaction)?;
        for registration in registrations {
            if super::source_root_commit_fence::source_root_is_deleting(
                &transaction,
                &registration.canonical_path,
            )? {
                return Err(MetaStoreError::invalid_transition());
            }
        }
        for registration in registrations {
            let requested = Path::new(&registration.canonical_path);
            if existing.iter().any(|(_, path)| {
                path != &registration.canonical_path && {
                    let existing = Path::new(path);
                    requested.starts_with(existing) || existing.starts_with(requested)
                }
            }) {
                return Err(MetaStoreError::invalid_value("source_root.overlap"));
            }
        }
        let new_count = registrations
            .iter()
            .filter(|registration| {
                !existing
                    .iter()
                    .any(|(_, path)| path == &registration.canonical_path)
            })
            .count();
        if existing.len().saturating_add(new_count) > MAX_SOURCE_ROOTS {
            return Err(MetaStoreError::invalid_value("source_root.limit"));
        }

        let mut ids = Vec::with_capacity(registrations.len());
        for registration in registrations {
            let id = existing
                .iter()
                .find(|(_, path)| path == &registration.canonical_path)
                .map(|(id, _)| SourceRootId::from_str(id))
                .transpose()?
                .map_or_else(SourceRootId::new, Ok)?;
            upsert_source_root(&transaction, &id, registration, now)?;
            ids.push(id);
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        drop(connection);
        ids.into_iter()
            .map(|id| {
                self.source_root(&id)?
                    .ok_or_else(MetaStoreError::storage_invariant)
            })
            .collect()
    }

    pub fn set_source_root_state(
        &self,
        id: &SourceRootId,
        state: SourceRootState,
        watcher_state: SourceWatcherState,
        now: UnixTimestamp,
    ) -> Result<SourceRoot>
    where
        Access: MetadataStoreWriteAccess,
    {
        let changed = self
            .connection
            .borrow()
            .execute(
                "UPDATE source_root
                 SET state = ?2, watcher_state = ?3,
                     updated_at_seconds = MAX(updated_at_seconds, ?4)
                 WHERE id = ?1",
                params![
                    id.as_str(),
                    state.storage(),
                    watcher_state.storage(),
                    now.as_unix_seconds()
                ],
            )
            .map_err(MetaStoreError::storage)?;
        if changed != 1 {
            return Err(MetaStoreError::not_found("source_root"));
        }
        self.source_root(id)?
            .ok_or_else(MetaStoreError::storage_invariant)
    }

    /// Resumes monitoring for a source root and retires any paused state left
    /// by the predecessor import-root authority.
    ///
    /// `source_root` is the only current directory authority. The legacy
    /// `authorized_import_root` row remains a pipeline implementation detail
    /// until all imported work has been republished, so a migrated paused bit
    /// must be cleared in the same transaction before recovery work is queued.
    pub fn resume_source_root_monitoring(
        &self,
        id: &SourceRootId,
        now: UnixTimestamp,
    ) -> Result<SourceRoot>
    where
        Access: MetadataStoreWriteAccess,
    {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let canonical_path = transaction
            .query_row(
                "SELECT canonical_path FROM source_root WHERE id = ?1",
                params![id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(MetaStoreError::storage)?
            .ok_or_else(|| MetaStoreError::not_found("source_root"))?;
        transaction
            .execute(
                "UPDATE source_root
                 SET state = 'active', watcher_state = 'active',
                     updated_at_seconds = MAX(updated_at_seconds, ?2)
                 WHERE id = ?1",
                params![id.as_str(), now.as_unix_seconds()],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "UPDATE authorized_import_root
                 SET paused = 0,
                     updated_at_seconds = MAX(updated_at_seconds, ?2)
                 WHERE canonical_root_path = ?1",
                params![canonical_path, now.as_unix_seconds()],
            )
            .map_err(MetaStoreError::storage)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        drop(connection);
        self.source_root(id)?
            .ok_or_else(MetaStoreError::storage_invariant)
    }

    /// Clears a predecessor pipeline pause without changing the current
    /// watcher preference.
    ///
    /// A paused watcher still permits an explicit manual scan. This bridge is
    /// intentionally one-way: `source_root.watcher_state` owns the product
    /// preference, while the old pipeline row may only be activated so it
    /// cannot veto current source-root work.
    pub fn activate_source_root_pipeline(&self, id: &SourceRootId, now: UnixTimestamp) -> Result<()>
    where
        Access: MetadataStoreWriteAccess,
    {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let canonical_path = transaction
            .query_row(
                "SELECT canonical_path FROM source_root WHERE id = ?1",
                params![id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(MetaStoreError::storage)?
            .ok_or_else(|| MetaStoreError::not_found("source_root"))?;
        transaction
            .execute(
                "UPDATE authorized_import_root
                 SET paused = 0,
                     updated_at_seconds = MAX(updated_at_seconds, ?2)
                 WHERE canonical_root_path = ?1",
                params![canonical_path, now.as_unix_seconds()],
            )
            .map_err(MetaStoreError::storage)?;
        transaction.commit().map_err(MetaStoreError::storage)
    }

    pub fn begin_scan(
        &self,
        root_id: &SourceRootId,
        scan_id: &str,
        trigger: ScanTrigger,
        now: UnixTimestamp,
    ) -> Result<BeginScanOutcome>
    where
        Access: MetadataStoreWriteAccess,
    {
        validate_scan_id(scan_id)?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let root_revocation_epoch =
            super::source_root_commit_fence::admit_scan(&transaction, root_id)?;
        let root_state = transaction
            .query_row(
                "SELECT state FROM source_root WHERE id = ?1",
                params![root_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(MetaStoreError::storage)?
            .ok_or_else(|| MetaStoreError::not_found("source_root"))?;
        if SourceRootState::parse(&root_state)? != SourceRootState::Active {
            return Err(MetaStoreError::invalid_transition());
        }
        if let Some(snapshot) = active_scan(&transaction, root_id)? {
            return Ok(BeginScanOutcome::Coalesced(snapshot));
        }
        transaction
            .execute(
                "INSERT INTO scan_snapshot (
                    id, root_id, trigger, phase, completeness,
                    started_at_seconds, updated_at_seconds, root_revocation_epoch
                 ) VALUES (?1, ?2, ?3, 'queued', 'unknown', ?4, ?4, ?5)",
                params![
                    scan_id,
                    root_id.as_str(),
                    trigger.storage(),
                    now.as_unix_seconds(),
                    root_revocation_epoch
                ],
            )
            .map_err(MetaStoreError::storage)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        drop(connection);
        let snapshot = self
            .latest_scan_snapshot(root_id)?
            .ok_or_else(MetaStoreError::storage_invariant)?;
        Ok(BeginScanOutcome::Started(snapshot))
    }

    /// Atomically establishes the canonical import task and its directory
    /// progress snapshot.
    ///
    /// A worker cannot observe the task before the corresponding snapshot is
    /// committed. A live scan is coalesced, while an orphaned active snapshot
    /// is closed before a replacement task is created.
    pub fn coordinate_source_root_scan(
        &self,
        root_id: &SourceRootId,
        trigger: ScanTrigger,
        task: &ImportTask,
        scope: &ImportScanScope,
        processing_contract: &ImportProcessingContract,
        now: UnixTimestamp,
    ) -> Result<SourceRootScanCoordination>
    where
        Access: MetadataStoreWriteAccess,
    {
        validate_import_task(task)?;
        validate_import_scan_scope(scope)?;
        if task.status != ImportTaskStatus::Queued
            || task.id != scope.import_task_id
            || task.root_path != scope.canonical_root_path
        {
            return Err(MetaStoreError::invalid_value("source_root_scan.task_scope"));
        }
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let root = transaction
            .query_row(
                "SELECT canonical_path, state FROM source_root WHERE id = ?1",
                params![root_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(MetaStoreError::storage)?
            .ok_or_else(|| MetaStoreError::not_found("source_root"))?;
        if root.0 != task.root_path
            || root.0 != scope.canonical_root_path
            || SourceRootState::parse(&root.1)? != SourceRootState::Active
        {
            return Err(MetaStoreError::invalid_transition());
        }
        if let Some(active) = active_scan(&transaction, root_id)? {
            if active_scan_has_live_task(&transaction, &active.id)? {
                transaction.commit().map_err(MetaStoreError::storage)?;
                return Ok(SourceRootScanCoordination::Coalesced(active));
            }
            close_orphaned_active_scan(&transaction, &active, now)?;
        }
        let root_revocation_epoch =
            super::source_root_commit_fence::read_epoch(&transaction, root_id)?;
        let task_head = coordinate_import_root_task_head_in_connection(
            &transaction,
            ImportRootTaskHeadRequest::Configured {
                task,
                scope,
                processing_contract,
            },
        )?;
        let persisted_task = match &task_head {
            ImportRootTaskHeadOutcome::HeadInserted { task, .. }
            | ImportRootTaskHeadOutcome::HeadPromoted { task, .. }
            | ImportRootTaskHeadOutcome::HeadRetained { task, .. } => task,
            ImportRootTaskHeadOutcome::RunningTaskConflict
            | ImportRootTaskHeadOutcome::RootPaused
            | ImportRootTaskHeadOutcome::MigrationRebuildSuperseded => {
                transaction.commit().map_err(MetaStoreError::storage)?;
                return Ok(SourceRootScanCoordination::Rejected(task_head));
            }
        };
        if let Some(existing) = snapshot_by_id(&transaction, root_id, persisted_task.id.as_str())? {
            if matches!(
                task_head,
                ImportRootTaskHeadOutcome::HeadPromoted { .. }
                    | ImportRootTaskHeadOutcome::HeadRetained { .. }
            ) && matches!(existing.phase, ScanPhase::Partial | ScanPhase::Failed)
            {
                transaction.commit().map_err(MetaStoreError::storage)?;
                return Ok(SourceRootScanCoordination::Coalesced(existing));
            }
            return Err(MetaStoreError::storage_invariant());
        }
        transaction
            .execute(
                "INSERT INTO scan_snapshot (
                    id, root_id, trigger, phase, completeness,
                    started_at_seconds, updated_at_seconds, root_revocation_epoch
                 ) VALUES (?1, ?2, ?3, 'queued', 'unknown', ?4, ?4, ?5)",
                params![
                    persisted_task.id.as_str(),
                    root_id.as_str(),
                    trigger.storage(),
                    now.as_unix_seconds(),
                    root_revocation_epoch
                ],
            )
            .map_err(MetaStoreError::storage)?;
        super::source_root_commit_fence::validate_scan_commit(
            &transaction,
            root_id,
            &persisted_task.id,
        )?;
        let snapshot = ScanSnapshot {
            id: persisted_task.id.as_str().to_string(),
            root_id: root_id.clone(),
            trigger,
            phase: ScanPhase::Queued,
            completeness: ScanCompleteness::Unknown,
            counts: ScanCounts::default(),
            rate_per_second: None,
            eta_seconds: None,
            started_at: now,
            updated_at: now,
            completed_at: None,
        };
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(SourceRootScanCoordination::Started {
            snapshot,
            task_head: Box::new(task_head),
        })
    }

    pub fn update_scan_snapshot(&self, snapshot: &ScanSnapshot) -> Result<()>
    where
        Access: MetadataStoreWriteAccess,
    {
        validate_scan_snapshot(snapshot.clone())?;
        let completed_at = snapshot.completed_at.map(UnixTimestamp::as_unix_seconds);
        let changed = self
            .connection
            .borrow()
            .execute(
                "UPDATE scan_snapshot SET
                    phase = ?3, completeness = ?4,
                    discovered_count = ?5, searchable_count = ?6,
                    non_resume_count = ?7, needs_review_count = ?8,
                    ocr_count = ?9, failed_count = ?10, ignored_count = ?11,
                    processed_count = ?12, total_count = ?13,
                    rate_per_second = ?14, eta_seconds = ?15, error_count = ?16,
                    updated_at_seconds = ?17, completed_at_seconds = ?18
                 WHERE id = ?1 AND root_id = ?2",
                params![
                    snapshot.id,
                    snapshot.root_id.as_str(),
                    snapshot.phase.storage(),
                    snapshot.completeness.storage(),
                    to_i64(snapshot.counts.discovered, "scan_snapshot.discovered")?,
                    to_i64(snapshot.counts.searchable, "scan_snapshot.searchable")?,
                    to_i64(snapshot.counts.non_resume, "scan_snapshot.non_resume")?,
                    to_i64(snapshot.counts.needs_review, "scan_snapshot.needs_review")?,
                    to_i64(snapshot.counts.ocr, "scan_snapshot.ocr")?,
                    to_i64(snapshot.counts.failed, "scan_snapshot.failed")?,
                    to_i64(snapshot.counts.ignored, "scan_snapshot.ignored")?,
                    to_i64(snapshot.counts.processed, "scan_snapshot.processed")?,
                    snapshot
                        .counts
                        .total
                        .map(|value| to_i64(value, "scan_snapshot.total"))
                        .transpose()?,
                    snapshot.rate_per_second,
                    snapshot
                        .eta_seconds
                        .map(|value| to_i64(value, "scan_snapshot.eta"))
                        .transpose()?,
                    to_i64(snapshot.counts.errors, "scan_snapshot.errors")?,
                    snapshot.updated_at.as_unix_seconds(),
                    completed_at
                ],
            )
            .map_err(MetaStoreError::storage)?;
        if changed != 1 {
            return Err(MetaStoreError::not_found("scan_snapshot"));
        }
        Ok(())
    }

    pub fn update_source_scan_progress_for_import_task(
        &self,
        task_id: &ImportTaskId,
        progress: SourceScanProgress,
        now: UnixTimestamp,
    ) -> Result<()>
    where
        Access: MetadataStoreWriteAccess,
    {
        if progress.processed > progress.discovered {
            return Err(MetaStoreError::storage_invariant());
        }
        self.connection
            .borrow()
            .execute(
                "UPDATE scan_snapshot SET
                    discovered_count = ?2,
                    searchable_count = ?3,
                    ocr_count = ?4,
                    failed_count = ?5,
                    ignored_count = ?6,
                    processed_count = ?7,
                    total_count = ?2,
                    error_count = ?8,
                    updated_at_seconds = MAX(updated_at_seconds, ?9)
                 WHERE id = ?1
                   AND phase IN (
                       'queued', 'discovering', 'fingerprinting', 'classifying',
                       'parsing', 'ocr', 'publishing'
                   )",
                params![
                    task_id.as_str(),
                    to_i64(progress.discovered, "scan_snapshot.discovered")?,
                    to_i64(progress.searchable, "scan_snapshot.searchable")?,
                    to_i64(progress.ocr, "scan_snapshot.ocr")?,
                    to_i64(progress.failed, "scan_snapshot.failed")?,
                    to_i64(progress.ignored, "scan_snapshot.ignored")?,
                    to_i64(progress.processed, "scan_snapshot.processed")?,
                    to_i64(progress.errors, "scan_snapshot.errors")?,
                    now.as_unix_seconds(),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        Ok(())
    }

    pub fn observe_source_occurrence(
        &self,
        root_id: &SourceRootId,
        relative_path: &str,
        document_id: &DocumentId,
        source_revision_id: &SourceRevisionId,
        scan_id: &str,
        now: UnixTimestamp,
    ) -> Result<OccurrenceChange>
    where
        Access: MetadataStoreWriteAccess,
    {
        validate_relative_path(relative_path)?;
        validate_scan_id(scan_id)?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let change =
            super::source_root_bound_import_commit::observe_source_occurrence_in_connection(
                &transaction,
                root_id,
                relative_path,
                document_id,
                source_revision_id,
                scan_id,
                now,
            )?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(change)
    }

    pub fn observe_import_task_source_occurrence(
        &self,
        task_id: &ImportTaskId,
        normalized_path: &str,
        document_id: &DocumentId,
        source_revision_id: &SourceRevisionId,
        now: UnixTimestamp,
    ) -> Result<()>
    where
        Access: MetadataStoreWriteAccess,
    {
        let Some((root, relative)) =
            self.source_root_and_relative_path_for_import_task(task_id, normalized_path)?
        else {
            return Ok(());
        };
        self.observe_source_occurrence(
            &root.id,
            &relative,
            document_id,
            source_revision_id,
            task_id.as_str(),
            now,
        )?;
        Ok(())
    }

    /// Marks a previously published occurrence as seen without reading or
    /// replacing its source revision.
    ///
    /// This is used when the operation-specific runtime for a discovered file
    /// is temporarily unavailable. Existing publications remain stable, while
    /// a complete directory scan can still remove genuinely missing sibling
    /// occurrences. A new path is deliberately not inserted until its bytes
    /// can be validated and processed.
    pub fn observe_existing_import_task_source_occurrence(
        &self,
        task_id: &ImportTaskId,
        normalized_path: &str,
        now: UnixTimestamp,
    ) -> Result<bool>
    where
        Access: MetadataStoreWriteAccess,
    {
        let Some((root, relative)) =
            self.source_root_and_relative_path_for_import_task(task_id, normalized_path)?
        else {
            return Ok(false);
        };
        let updated = self
            .connection
            .borrow()
            .execute(
                "UPDATE source_occurrence
                 SET last_seen_scan_id = ?3,
                     observed_at_seconds = MAX(observed_at_seconds, ?4),
                     removed_at_seconds = NULL
                 WHERE root_id = ?1
                   AND relative_path = ?2
                   AND state = 'present'",
                params![
                    root.id.as_str(),
                    relative,
                    task_id.as_str(),
                    now.as_unix_seconds()
                ],
            )
            .map_err(MetaStoreError::storage)?;
        Ok(updated == 1)
    }

    pub fn source_occurrence_documents_for_import_task(
        &self,
        task_id: &ImportTaskId,
    ) -> Result<BTreeMap<String, DocumentId>> {
        let Some(scope) = self.import_scan_scope_by_task_id(task_id)? else {
            return Ok(BTreeMap::new());
        };
        let Some(root) = self.source_root_by_canonical_path(&scope.canonical_root_path)? else {
            return Ok(BTreeMap::new());
        };
        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare(
                "SELECT relative_path, document_id
                 FROM source_occurrence
                 WHERE root_id = ?1 AND state = 'present'
                 ORDER BY relative_path",
            )
            .map_err(MetaStoreError::storage)?;
        let rows = statement
            .query_map(params![root.id.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(MetaStoreError::storage)?;
        let mut documents = BTreeMap::new();
        for row in rows {
            let (relative_path, document_id) = row.map_err(MetaStoreError::storage)?;
            validate_relative_path(&relative_path)?;
            let document_id = DocumentId::from_str(&document_id)
                .map_err(|_| MetaStoreError::invalid_value("document.id"))?;
            documents.insert(relative_path, document_id);
        }
        Ok(documents)
    }

    pub(crate) fn source_root_and_relative_path_for_import_task(
        &self,
        task_id: &ImportTaskId,
        normalized_path: &str,
    ) -> Result<Option<(SourceRoot, String)>> {
        let Some(scope) = self.import_scan_scope_by_task_id(task_id)? else {
            return Ok(None);
        };
        let Some(root) = self.source_root_by_canonical_path(&scope.canonical_root_path)? else {
            return Ok(None);
        };
        let relative = Path::new(normalized_path)
            .strip_prefix(Path::new(&root.canonical_path))
            .map_err(|_| MetaStoreError::storage_invariant())?
            .to_string_lossy()
            .replace('\\', "/");
        validate_relative_path(&relative)?;
        Ok(Some((root, relative)))
    }

    pub fn complete_scan_and_remove_missing(
        &self,
        root_id: &SourceRootId,
        scan_id: &str,
        counts: ScanCounts,
        rate_per_second: Option<f64>,
        now: UnixTimestamp,
    ) -> Result<Vec<DocumentId>>
    where
        Access: MetadataStoreWriteAccess,
    {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let snapshot = snapshot_by_id(&transaction, root_id, scan_id)?
            .ok_or_else(|| MetaStoreError::not_found("scan_snapshot"))?;
        if !snapshot.phase.is_active() || counts.errors != 0 {
            return Err(MetaStoreError::invalid_transition());
        }
        let mut statement = transaction
            .prepare(
                "SELECT document_id
                 FROM source_occurrence
                 WHERE root_id = ?1 AND state = 'present'
                   AND (last_seen_scan_id IS NULL OR last_seen_scan_id <> ?2)
                 ORDER BY document_id",
            )
            .map_err(MetaStoreError::storage)?;
        let stale = statement
            .query_map(params![root_id.as_str(), scan_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(MetaStoreError::storage)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(MetaStoreError::storage)?;
        drop(statement);
        transaction
            .execute(
                "UPDATE source_occurrence
                 SET state = 'removed', removed_at_seconds = ?3
                 WHERE root_id = ?1 AND state = 'present'
                   AND (last_seen_scan_id IS NULL OR last_seen_scan_id <> ?2)",
                params![root_id.as_str(), scan_id, now.as_unix_seconds()],
            )
            .map_err(MetaStoreError::storage)?;
        let mut removed = Vec::new();
        for id in stale {
            super::source_root_bound_import_commit::tombstone_unreferenced_document(
                &transaction,
                &id,
                now,
            )?;
            removed.push(
                DocumentId::from_str(&id)
                    .map_err(|_| MetaStoreError::invalid_value("document.id"))?,
            );
        }
        update_completed_snapshot(&transaction, root_id, scan_id, counts, rate_per_second, now)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(removed)
    }

    pub fn reconcile_complete_source_scan(
        &self,
        root_id: &SourceRootId,
        scan_id: &str,
        counts: ScanCounts,
        rate_per_second: Option<f64>,
        now: UnixTimestamp,
    ) -> Result<Vec<DocumentId>>
    where
        Access: MetadataStoreWriteAccess,
    {
        self.complete_scan_and_remove_missing(root_id, scan_id, counts, rate_per_second, now)
    }

    pub fn fail_or_partial_scan(
        &self,
        root_id: &SourceRootId,
        scan_id: &str,
        counts: ScanCounts,
        phase: ScanPhase,
        now: UnixTimestamp,
    ) -> Result<()>
    where
        Access: MetadataStoreWriteAccess,
    {
        if !matches!(phase, ScanPhase::Partial | ScanPhase::Failed) {
            return Err(MetaStoreError::invalid_value("scan_snapshot.phase"));
        }
        let completeness = if phase == ScanPhase::Partial {
            ScanCompleteness::Partial
        } else {
            ScanCompleteness::Unknown
        };
        let snapshot = ScanSnapshot {
            id: scan_id.to_string(),
            root_id: root_id.clone(),
            trigger: self
                .latest_scan_snapshot(root_id)?
                .filter(|candidate| candidate.id == scan_id)
                .ok_or_else(|| MetaStoreError::not_found("scan_snapshot"))?
                .trigger,
            phase,
            completeness,
            counts,
            rate_per_second: None,
            eta_seconds: None,
            started_at: self
                .latest_scan_snapshot(root_id)?
                .filter(|candidate| candidate.id == scan_id)
                .ok_or_else(|| MetaStoreError::not_found("scan_snapshot"))?
                .started_at,
            updated_at: now,
            completed_at: Some(now),
        };
        self.update_scan_snapshot(&snapshot)
    }
}

fn read_source_root(row: &rusqlite::Row<'_>) -> rusqlite::Result<SourceRoot> {
    Ok(SourceRoot {
        id: SourceRootId(row.get(0)?),
        canonical_path: row.get(1)?,
        requested_path: row.get(2)?,
        display_label: row.get(3)?,
        state: SourceRootState::parse(&row.get::<_, String>(4)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        watcher_state: SourceWatcherState::parse(&row.get::<_, String>(5)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        created_at: UnixTimestamp::from_unix_seconds(row.get(6)?),
        updated_at: UnixTimestamp::from_unix_seconds(row.get(7)?),
    })
}

fn all_source_root_identities(transaction: &Transaction<'_>) -> Result<Vec<(String, String)>> {
    let mut statement = transaction
        .prepare("SELECT id, canonical_path FROM source_root ORDER BY id")
        .map_err(MetaStoreError::storage)?;
    let identities = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(MetaStoreError::storage)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(MetaStoreError::storage);
    identities
}

fn upsert_source_root(
    transaction: &Transaction<'_>,
    id: &SourceRootId,
    registration: &SourceRootRegistration,
    now: UnixTimestamp,
) -> Result<()> {
    transaction
        .execute(
            "INSERT INTO source_root (
                id, canonical_path, requested_path, display_label, state,
                watcher_state, created_at_seconds, updated_at_seconds
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT(canonical_path) DO UPDATE SET
                requested_path = excluded.requested_path,
                display_label = excluded.display_label,
                state = CASE
                    WHEN source_root.state = 'active'
                     AND excluded.state = 'offline'
                    THEN 'active'
                    ELSE excluded.state
                END,
                watcher_state = CASE
                    WHEN source_root.watcher_state = 'paused' THEN 'paused'
                    WHEN source_root.state = 'active'
                     AND excluded.state = 'offline'
                    THEN source_root.watcher_state
                    ELSE excluded.watcher_state
                END,
                updated_at_seconds = MAX(
                    source_root.updated_at_seconds,
                    excluded.updated_at_seconds
                )",
            params![
                id.as_str(),
                registration.canonical_path.as_str(),
                registration.requested_path.as_str(),
                registration.display_label.as_str(),
                registration.availability.root_state().storage(),
                registration.availability.watcher_state().storage(),
                now.as_unix_seconds()
            ],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(())
}

fn validate_source_root(root: SourceRoot) -> Result<SourceRoot> {
    SourceRootId::from_str(root.id.as_str())?;
    validate_canonical_path(&root.canonical_path)?;
    validate_canonical_path(&root.requested_path)?;
    validate_display_label(&root.display_label)?;
    if root.updated_at < root.created_at {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(root)
}

fn read_scan_snapshot(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScanSnapshot> {
    let total = row.get::<_, Option<i64>>(13)?;
    let eta = row.get::<_, Option<i64>>(15)?;
    Ok(ScanSnapshot {
        id: row.get(0)?,
        root_id: SourceRootId(row.get(1)?),
        trigger: ScanTrigger::parse(&row.get::<_, String>(2)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        phase: ScanPhase::parse(&row.get::<_, String>(3)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        completeness: ScanCompleteness::parse(&row.get::<_, String>(4)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        counts: ScanCounts {
            discovered: non_negative(row.get(5)?)?,
            searchable: non_negative(row.get(6)?)?,
            non_resume: non_negative(row.get(7)?)?,
            needs_review: non_negative(row.get(8)?)?,
            ocr: non_negative(row.get(9)?)?,
            failed: non_negative(row.get(10)?)?,
            ignored: non_negative(row.get(11)?)?,
            processed: non_negative(row.get(12)?)?,
            total: total.map(non_negative).transpose()?,
            errors: non_negative(row.get(16)?)?,
        },
        rate_per_second: row.get(14)?,
        eta_seconds: eta.map(non_negative).transpose()?,
        started_at: UnixTimestamp::from_unix_seconds(row.get(17)?),
        updated_at: UnixTimestamp::from_unix_seconds(row.get(18)?),
        completed_at: row
            .get::<_, Option<i64>>(19)?
            .map(UnixTimestamp::from_unix_seconds),
    })
}

fn validate_scan_snapshot(snapshot: ScanSnapshot) -> Result<ScanSnapshot> {
    validate_scan_id(&snapshot.id)?;
    SourceRootId::from_str(snapshot.root_id.as_str())?;
    if snapshot.updated_at < snapshot.started_at
        || snapshot
            .completed_at
            .is_some_and(|completed| completed < snapshot.started_at)
        || snapshot
            .counts
            .total
            .is_some_and(|total| total < snapshot.counts.processed)
        || snapshot
            .rate_per_second
            .is_some_and(|rate| !rate.is_finite() || rate <= 0.0)
        || (snapshot.phase.is_active() && snapshot.completed_at.is_some())
        || (!snapshot.phase.is_active() && snapshot.completed_at.is_none())
    {
        return Err(MetaStoreError::invalid_value("scan_snapshot"));
    }
    Ok(snapshot)
}

fn active_scan(connection: &Connection, root_id: &SourceRootId) -> Result<Option<ScanSnapshot>> {
    connection
        .query_row(
            "SELECT id, root_id, trigger, phase, completeness,
                    discovered_count, searchable_count, non_resume_count,
                    needs_review_count, ocr_count, failed_count, ignored_count,
                    processed_count, total_count, rate_per_second, eta_seconds,
                    error_count, started_at_seconds, updated_at_seconds,
                    completed_at_seconds
             FROM scan_snapshot
             WHERE root_id = ?1 AND phase IN (
                'queued', 'discovering', 'fingerprinting', 'classifying',
                'parsing', 'ocr', 'publishing'
             )
             LIMIT 1",
            params![root_id.as_str()],
            read_scan_snapshot,
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .map(validate_scan_snapshot)
        .transpose()
}

fn active_scan_has_live_task(connection: &Connection, scan_id: &str) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM import_task AS task
                WHERE task.id = ?1
                  AND task.status IN (?2, ?3, ?4)
                  AND NOT EXISTS (
                    SELECT 1 FROM import_task_cancellation AS cancellation
                    WHERE cancellation.import_task_id = task.id
                  )
             )",
            params![
                scan_id,
                import_task_status_to_storage(ImportTaskStatus::Queued),
                import_task_status_to_storage(ImportTaskStatus::Running),
                import_task_status_to_storage(ImportTaskStatus::FailedRetryable),
            ],
            |row| row.get::<_, i64>(0),
        )
        .map(|exists| exists != 0)
        .map_err(MetaStoreError::storage)
}

fn close_orphaned_active_scan(
    connection: &Connection,
    snapshot: &ScanSnapshot,
    now: UnixTimestamp,
) -> Result<()> {
    let completed_at = now
        .as_unix_seconds()
        .max(snapshot.started_at.as_unix_seconds());
    let changed = connection
        .execute(
            "UPDATE scan_snapshot
             SET phase = 'failed', completeness = 'unknown',
                 eta_seconds = NULL,
                 updated_at_seconds = MAX(updated_at_seconds, ?2),
                 completed_at_seconds = ?2
             WHERE id = ?1 AND phase IN (
                'queued', 'discovering', 'fingerprinting', 'classifying',
                'parsing', 'ocr', 'publishing'
             )",
            params![snapshot.id, completed_at],
        )
        .map_err(MetaStoreError::storage)?;
    if changed != 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

pub(super) fn restart_failed_scan_attempt_in_connection(
    connection: &Connection,
    task_id: &ImportTaskId,
    now: UnixTimestamp,
) -> Result<()> {
    let changed = connection
        .execute(
            "UPDATE scan_snapshot
             SET phase = 'queued', completeness = 'unknown',
                 discovered_count = 0, searchable_count = 0,
                 non_resume_count = 0, needs_review_count = 0,
                 ocr_count = 0, failed_count = 0, ignored_count = 0,
                 processed_count = 0, total_count = NULL,
                 rate_per_second = NULL, eta_seconds = NULL, error_count = 0,
                 started_at_seconds = ?2, updated_at_seconds = ?2,
                 completed_at_seconds = NULL
             WHERE id = ?1 AND phase IN ('partial', 'failed')",
            params![task_id.as_str(), now.as_unix_seconds()],
        )
        .map_err(MetaStoreError::storage)?;
    if changed > 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn snapshot_by_id(
    connection: &Connection,
    root_id: &SourceRootId,
    scan_id: &str,
) -> Result<Option<ScanSnapshot>> {
    connection
        .query_row(
            "SELECT id, root_id, trigger, phase, completeness,
                    discovered_count, searchable_count, non_resume_count,
                    needs_review_count, ocr_count, failed_count, ignored_count,
                    processed_count, total_count, rate_per_second, eta_seconds,
                    error_count, started_at_seconds, updated_at_seconds,
                    completed_at_seconds
             FROM scan_snapshot WHERE id = ?1 AND root_id = ?2",
            params![scan_id, root_id.as_str()],
            read_scan_snapshot,
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .map(validate_scan_snapshot)
        .transpose()
}

fn update_completed_snapshot(
    connection: &Connection,
    root_id: &SourceRootId,
    scan_id: &str,
    counts: ScanCounts,
    rate_per_second: Option<f64>,
    now: UnixTimestamp,
) -> Result<()> {
    let changed = connection
        .execute(
            "UPDATE scan_snapshot SET
                phase = 'complete', completeness = 'complete',
                discovered_count = ?3, searchable_count = ?4,
                non_resume_count = ?5, needs_review_count = ?6,
                ocr_count = ?7, failed_count = ?8, ignored_count = ?9,
                processed_count = ?10, total_count = ?11,
                rate_per_second = ?12, eta_seconds = 0, error_count = 0,
                updated_at_seconds = ?13, completed_at_seconds = ?13
             WHERE id = ?1 AND root_id = ?2",
            params![
                scan_id,
                root_id.as_str(),
                to_i64(counts.discovered, "scan_snapshot.discovered")?,
                to_i64(counts.searchable, "scan_snapshot.searchable")?,
                to_i64(counts.non_resume, "scan_snapshot.non_resume")?,
                to_i64(counts.needs_review, "scan_snapshot.needs_review")?,
                to_i64(counts.ocr, "scan_snapshot.ocr")?,
                to_i64(counts.failed, "scan_snapshot.failed")?,
                to_i64(counts.ignored, "scan_snapshot.ignored")?,
                to_i64(counts.processed, "scan_snapshot.processed")?,
                counts
                    .total
                    .map(|value| to_i64(value, "scan_snapshot.total"))
                    .transpose()?,
                rate_per_second,
                now.as_unix_seconds()
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if changed != 1 {
        return Err(MetaStoreError::not_found("scan_snapshot"));
    }
    Ok(())
}

fn validate_canonical_path(value: &str) -> Result<()> {
    let path = Path::new(value);
    if value.is_empty()
        || value.len() > MAX_PATH_BYTES
        || value.as_bytes().contains(&0)
        || !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err(MetaStoreError::invalid_value("source_root.path"));
    }
    Ok(())
}

pub(super) fn validate_relative_path(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_RELATIVE_PATH_BYTES
        || value.starts_with('/')
        || value == ".."
        || value.starts_with("../")
        || value.ends_with("/..")
        || value.contains("/../")
        || value.as_bytes().contains(&0)
    {
        return Err(MetaStoreError::invalid_value(
            "source_occurrence.relative_path",
        ));
    }
    Ok(())
}

fn validate_display_label(value: &str) -> Result<()> {
    let chars = value.chars().count();
    if chars == 0 || chars > MAX_DISPLAY_LABEL_CHARS || value.chars().any(char::is_control) {
        return Err(MetaStoreError::invalid_value("source_root.display_label"));
    }
    Ok(())
}

fn validate_scan_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 80
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(MetaStoreError::invalid_value("scan_snapshot.id"));
    }
    Ok(())
}

fn non_negative(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, value))
}

fn to_i64(value: u64, field: &'static str) -> Result<i64> {
    i64::try_from(value).map_err(|_| MetaStoreError::invalid_value(field))
}
