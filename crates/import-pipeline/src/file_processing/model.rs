use std::time::{Duration, Instant};

use fs_crawler::DiscoveredFile;
use index_fulltext::IndexDocument;
use meta_store::{
    ContactHash, Document, DocumentStatus, EntityMention, ResumeVersion,
    ResumeVersionClassification, ResumeVersionId, SourceRevision, SourceRevisionId,
};
use parser_pdf::PdfTextExtractionTimings;

use crate::classification::AdmissionDecision;
use crate::source_dispositions::ProcessedFile;
use crate::{ImportFailureKind, ImportPostParserTimings};

pub(crate) struct PendingSearchableDocument {
    pub(crate) document: Document,
    pub(crate) source_revision: SourceRevision,
    pub(crate) classification: ResumeVersionClassification,
    pub(crate) version: ResumeVersion,
    pub(crate) mentions: Vec<EntityMention>,
    pub(crate) email_hash: Option<ContactHash>,
    pub(crate) phone_hash: Option<ContactHash>,
    pub(crate) index_document: IndexDocument,
    pub(crate) publication_kind: PendingSearchablePublicationKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PendingSearchablePublicationKind {
    MetadataChanged,
    Replacement,
}

pub(crate) enum PreparedFile {
    Ready(ProcessedImportFile),
    Parse(ParseWorkItem),
}

pub(crate) struct ProcessedImportFile {
    pub(crate) file: DiscoveredFile,
    pub(crate) processed: ProcessedFile,
}

pub(crate) enum ImportFileResult {
    Processed(ProcessedImportFile),
    Parsed(ParseWorkResult),
}

pub(crate) struct ParseWorkItem {
    pub(crate) index: usize,
    pub(crate) file: DiscoveredFile,
    pub(crate) document: Document,
    pub(crate) source_revision: SourceRevision,
    pub(crate) bytes: Vec<u8>,
}

pub(crate) struct ParseWorkResult {
    pub(crate) index: usize,
    pub(crate) file: DiscoveredFile,
    pub(crate) document: Document,
    pub(crate) source_revision: SourceRevision,
    pub(crate) parse_elapsed: Duration,
    pub(crate) parse_started: Instant,
    pub(crate) parse_finished: Instant,
    pub(crate) pdf_parse_timings: PdfTextExtractionTimings,
    pub(crate) post_parser_timings: ImportPostParserTimings,
    pub(crate) outcome: ParseWorkOutcome,
}

pub(crate) struct ParseWorkItemOutput {
    pub(crate) outcome: ParseWorkOutcome,
    pub(crate) pdf_parse_timings: PdfTextExtractionTimings,
    pub(crate) post_parser_timings: ImportPostParserTimings,
}

pub(crate) enum ParseWorkOutcome {
    Searchable {
        decision: AdmissionDecision,
        version: Box<ResumeVersion>,
        mentions: Vec<EntityMention>,
        index_document: Box<IndexDocument>,
    },
    Excluded {
        decision: AdmissionDecision,
        version: Box<ResumeVersion>,
    },
    OcrRequired,
    Failed {
        status: DocumentStatus,
        kind: ImportFailureKind,
    },
}

#[derive(Default)]
pub(crate) struct ParseWorkerClock {
    first_started: Option<Instant>,
    last_finished: Option<Instant>,
    active_elapsed: Duration,
}

impl ParseWorkerClock {
    pub(crate) fn record_result(&mut self, result: &ParseWorkResult) {
        self.active_elapsed += result.parse_elapsed;
        self.first_started = Some(match self.first_started {
            Some(first_started) => first_started.min(result.parse_started),
            None => result.parse_started,
        });
        self.last_finished = Some(match self.last_finished {
            Some(last_finished) => last_finished.max(result.parse_finished),
            None => result.parse_finished,
        });
    }

    pub(crate) fn worker_wall_elapsed(&self) -> Duration {
        match (self.first_started, self.last_finished) {
            (Some(started), Some(finished)) => finished.saturating_duration_since(started),
            _ => Duration::ZERO,
        }
    }

    pub(crate) fn active_elapsed(&self) -> Duration {
        self.active_elapsed
    }
}

pub(crate) enum ExactRerunDecision {
    UnchangedSearchable {
        source_revision_id: SourceRevisionId,
        resume_version_id: ResumeVersionId,
    },
    MetadataChangedSearchable {
        pending: Box<PendingSearchableDocument>,
    },
    UnchangedOcrRequired {
        source_revision_id: SourceRevisionId,
    },
    UnchangedExcluded {
        source_revision_id: SourceRevisionId,
        resume_version_id: ResumeVersionId,
    },
}
