use meta_store::{
    DocumentId, ImportProcessingContractId, ImportSourceDispositionKind, ImportTaskId,
    ImportTaskSourceDisposition, OwnedMetaStore, ResumeVersionId, SourceRevisionId,
};

use crate::file_processing::{
    PendingClassifiedDocument, PendingExistingDocument, PendingSearchableDocument,
    PendingSourceTriageDocument,
};
use crate::{ImportFailureKind, ImportPipelineError, Result};

pub(super) enum ProcessedFile {
    Searchable {
        pending: Box<PendingSearchableDocument>,
    },
    UnchangedSearchable {
        source_revision_id: SourceRevisionId,
        resume_version_id: ResumeVersionId,
    },
    UnchangedOcrRequired {
        document: PendingExistingDocument,
        source_revision_id: SourceRevisionId,
    },
    UnchangedExcluded {
        document: PendingExistingDocument,
        source_revision_id: SourceRevisionId,
        resume_version_id: ResumeVersionId,
    },
    Excluded {
        pending: Box<PendingClassifiedDocument>,
    },
    OcrRequired {
        pending: Box<PendingSourceTriageDocument>,
    },
    Failed {
        kind: ImportFailureKind,
        pending: Option<Box<PendingSourceTriageDocument>>,
    },
}

impl ProcessedFile {
    pub(super) fn disposition_staging(&self) -> DispositionStaging {
        if matches!(self, Self::Searchable { .. }) {
            DispositionStaging::SearchableFactsPending
        } else {
            DispositionStaging::Ready
        }
    }

    pub(super) fn source_disposition(
        &self,
        source_ordinal: usize,
        document_id: &DocumentId,
    ) -> Result<ImportTaskSourceDisposition> {
        let source_ordinal =
            u64::try_from(source_ordinal).map_err(|_| ImportPipelineError::store_invariant())?;
        let (source_revision_id, resume_version_id, kind) = match self {
            Self::Searchable { pending } => (
                pending.source_revision.id.clone(),
                Some(pending.version.id.clone()),
                ImportSourceDispositionKind::Searchable,
            ),
            Self::UnchangedSearchable {
                source_revision_id,
                resume_version_id,
            } => (
                source_revision_id.clone(),
                Some(resume_version_id.clone()),
                ImportSourceDispositionKind::Searchable,
            ),
            Self::UnchangedOcrRequired {
                source_revision_id, ..
            } => (
                source_revision_id.clone(),
                None,
                ImportSourceDispositionKind::OcrBacklog,
            ),
            Self::UnchangedExcluded {
                source_revision_id,
                resume_version_id,
                ..
            } => (
                source_revision_id.clone(),
                Some(resume_version_id.clone()),
                ImportSourceDispositionKind::Excluded,
            ),
            Self::Excluded { pending } => (
                pending.source_revision.id.clone(),
                Some(pending.version.id.clone()),
                ImportSourceDispositionKind::Excluded,
            ),
            Self::OcrRequired { pending } => (
                pending.source_revision.id.clone(),
                None,
                ImportSourceDispositionKind::OcrBacklog,
            ),
            Self::Failed {
                pending: Some(pending),
                ..
            } => (
                pending.source_revision.id.clone(),
                None,
                ImportSourceDispositionKind::Failed,
            ),
            Self::Failed { pending: None, .. } => {
                return Err(ImportPipelineError::migration_scan_incomplete())
            }
        };
        Ok(ImportTaskSourceDisposition {
            source_ordinal,
            document_id: document_id.clone(),
            source_revision_id,
            resume_version_id,
            kind,
        })
    }
}

#[derive(Clone, Copy)]
pub(super) enum DispositionStaging {
    Ready,
    SearchableFactsPending,
}

#[derive(Clone, Copy)]
pub(super) enum SearchableStagingState {
    Pending,
    Completed,
}

impl SearchableStagingState {
    pub(super) fn from_pending_documents<T>(pending: &[T]) -> Self {
        if pending.is_empty() {
            Self::Completed
        } else {
            Self::Pending
        }
    }
}

pub(super) struct ImportDispositionBatches {
    task_id: ImportTaskId,
    contract_id: ImportProcessingContractId,
    ready: Vec<ImportTaskSourceDisposition>,
    pending_searchable: Vec<ImportTaskSourceDisposition>,
}

impl ImportDispositionBatches {
    pub(super) fn new(task_id: ImportTaskId, contract_id: ImportProcessingContractId) -> Self {
        Self {
            task_id,
            contract_id,
            ready: Vec::new(),
            pending_searchable: Vec::new(),
        }
    }

    pub(super) fn record(
        &mut self,
        disposition: ImportTaskSourceDisposition,
        staging: DispositionStaging,
    ) {
        match staging {
            DispositionStaging::Ready => self.ready.push(disposition),
            DispositionStaging::SearchableFactsPending => {
                self.pending_searchable.push(disposition);
            }
        }
    }

    pub(super) fn flush_ready_if_full(&mut self, store: &OwnedMetaStore) -> Result<()> {
        if self.ready.len() >= meta_store::IMPORT_SOURCE_DISPOSITION_BATCH_LIMIT {
            self.flush_ready(store)?;
        }
        Ok(())
    }

    pub(super) fn searchable_staging_completed(
        &mut self,
        state: SearchableStagingState,
        store: &OwnedMetaStore,
    ) -> Result<()> {
        if matches!(state, SearchableStagingState::Pending) || self.pending_searchable.is_empty() {
            return Ok(());
        }
        self.ready.append(&mut self.pending_searchable);
        self.ready
            .sort_unstable_by_key(|disposition| disposition.source_ordinal);
        self.flush_ready(store)
    }

    pub(super) fn flush_all(&mut self, store: &OwnedMetaStore) -> Result<()> {
        if !self.pending_searchable.is_empty() {
            return Err(ImportPipelineError::store_invariant());
        }
        self.flush_ready(store)
    }

    fn flush_ready(&mut self, store: &OwnedMetaStore) -> Result<()> {
        for batch in self
            .ready
            .chunks(meta_store::IMPORT_SOURCE_DISPOSITION_BATCH_LIMIT)
        {
            store
                .stage_import_task_source_dispositions(&self.task_id, &self.contract_id, batch)
                .map_err(ImportPipelineError::store)?;
        }
        self.ready.clear();
        Ok(())
    }
}

#[cfg(test)]
#[path = "source_dispositions_tests.rs"]
mod tests;
