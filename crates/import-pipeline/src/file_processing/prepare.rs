use std::fs;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use fs_crawler::DiscoveredFile;
use index_fulltext::IndexDocument;
use meta_store::{
    Document, DocumentStatus, FileExtension, OwnedMetaStore, SourceRevision, UnixTimestamp,
};
use parser_common::{ParseInput, ParseStatus, Parser, ParserErrorKind, ResourceBudget};
use parser_doc::DocParser;
use parser_docx::DocxParser;
use parser_pdf::{PdfParser, PdfTextExtractionTimings};
use parser_text::TxtParser;
use resume_classifier::LinearPromotionPolicy;
use sectionizer::Sectionizer;
use text_normalizer::TextNormalizer;

use super::formatting::{
    document_from_discovered_file, file_extension_label, language_set, sections_to_index,
};
use super::model::{
    ExactRerunDecision, ParseWorkItem, ParseWorkItemOutput, ParseWorkOutcome, ParseWorkResult,
    PreparedFile, ProcessedImportFile,
};
use super::persistence::{
    entity_mentions_from_rules, persist_document_failure_without_revision,
    persist_source_revision_failure,
};
use super::rerun::exact_rerun_decision;
use crate::classification::AdmissionDecision;
use crate::immutable_ingest::{resume_version, source_revision};
use crate::source_digest::stream_content_digest;
use crate::source_dispositions::ProcessedFile;
use crate::timing::{measure_result_stage, measure_stage};
use crate::{ImportFailureKind, ImportPostParserTimings, Result, PARSE_VERSION, SCHEMA_VERSION};

pub(crate) fn prepare_file_for_parse(
    store: &OwnedMetaStore,
    index: usize,
    file: DiscoveredFile,
    now: UnixTimestamp,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    db_timing: &mut Duration,
    parse_prepare_timing: &mut Duration,
    content_bytes_read: &mut u64,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<PreparedFile> {
    let started = Instant::now();
    let mut db_elapsed = Duration::ZERO;
    let result = prepare_file_for_parse_inner(
        store,
        index,
        file,
        now,
        ensure_not_cancelled,
        &mut db_elapsed,
        content_bytes_read,
        linear_promotion,
    );
    *db_timing += db_elapsed;
    *parse_prepare_timing += started.elapsed().saturating_sub(db_elapsed);
    result
}

pub(crate) fn prepare_file_for_parse_inner(
    store: &OwnedMetaStore,
    index: usize,
    file: DiscoveredFile,
    now: UnixTimestamp,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    db_elapsed: &mut Duration,
    content_bytes_read: &mut u64,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<PreparedFile> {
    ensure_not_cancelled()?;
    let mut document = document_from_discovered_file(&file, now, DocumentStatus::Discovered);
    let path = PathBuf::from(file.normalized_path.as_str());
    ensure_not_cancelled()?;
    if file.extension == FileExtension::Txt
        && file.byte_size > parser_text::DEFAULT_MAX_BYTES as u64
    {
        let Some((content_hash, byte_size)) = stream_content_digest(&path, ensure_not_cancelled)?
        else {
            document.status = DocumentStatus::FailedRetryable;
            document.updated_at = now;
            measure_result_stage(db_elapsed, || {
                persist_document_failure_without_revision(store, &document)
            })?;
            return Ok(PreparedFile::Ready(ProcessedImportFile {
                file,
                processed: ProcessedFile::Failed {
                    kind: ImportFailureKind::ReadError,
                    source_revision_id: None,
                },
            }));
        };
        *content_bytes_read = content_bytes_read.saturating_add(byte_size);
        let source_revision =
            SourceRevision::for_content(document.id.clone(), content_hash, byte_size);
        document.content_hash = Some(source_revision.content_hash.as_str().to_string());
        document.byte_size = source_revision.byte_size;
        document.status = DocumentStatus::FailedPermanent;
        document.updated_at = now;
        measure_result_stage(db_elapsed, || {
            persist_source_revision_failure(
                store,
                &document,
                &source_revision,
                now,
                linear_promotion,
            )
        })?;
        return Ok(PreparedFile::Ready(ProcessedImportFile {
            file,
            processed: ProcessedFile::Failed {
                kind: ImportFailureKind::TextTooLarge,
                source_revision_id: Some(source_revision.id),
            },
        }));
    }

    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => {
            document.status = DocumentStatus::FailedRetryable;
            document.updated_at = now;
            measure_result_stage(db_elapsed, || {
                persist_document_failure_without_revision(store, &document)
            })?;
            return Ok(PreparedFile::Ready(ProcessedImportFile {
                file,
                processed: ProcessedFile::Failed {
                    kind: ImportFailureKind::ReadError,
                    source_revision_id: None,
                },
            }));
        }
    };
    *content_bytes_read += bytes.len() as u64;
    ensure_not_cancelled()?;

    let source_revision = source_revision(&document, &bytes);
    if let Some(noop_kind) = measure_result_stage(db_elapsed, || {
        exact_rerun_decision(
            store,
            &file,
            &source_revision.content_hash,
            linear_promotion,
            now,
        )
    })? {
        let processed = match noop_kind {
            ExactRerunDecision::UnchangedSearchable {
                source_revision_id,
                resume_version_id,
            } => ProcessedFile::UnchangedSearchable {
                source_revision_id,
                resume_version_id,
            },
            ExactRerunDecision::MetadataChangedSearchable { pending } => {
                ProcessedFile::Searchable { pending }
            }
            ExactRerunDecision::UnchangedOcrRequired { source_revision_id } => {
                ProcessedFile::UnchangedOcrRequired { source_revision_id }
            }
            ExactRerunDecision::UnchangedExcluded {
                source_revision_id,
                resume_version_id,
            } => ProcessedFile::UnchangedExcluded {
                source_revision_id,
                resume_version_id,
            },
        };
        return Ok(PreparedFile::Ready(ProcessedImportFile { file, processed }));
    }

    document.content_hash = Some(source_revision.content_hash.as_str().to_string());
    document.byte_size = source_revision.byte_size;
    ensure_not_cancelled()?;

    Ok(PreparedFile::Parse(ParseWorkItem {
        index,
        file,
        document,
        source_revision,
        bytes,
    }))
}

pub(crate) fn parse_worker_loop(
    work_rx: Arc<Mutex<mpsc::Receiver<ParseWorkItem>>>,
    result_tx: mpsc::SyncSender<ParseWorkResult>,
    linear_promotion: &LinearPromotionPolicy,
) {
    loop {
        let work = match work_rx.lock() {
            Ok(receiver) => receiver.recv(),
            Err(_) => return,
        };
        let Ok(work) = work else {
            return;
        };
        if result_tx
            .send(parse_work_item(work, linear_promotion))
            .is_err()
        {
            return;
        }
    }
}

pub(crate) fn parse_work_item(
    work: ParseWorkItem,
    linear_promotion: &LinearPromotionPolicy,
) -> ParseWorkResult {
    let ParseWorkItem {
        index,
        file,
        document,
        source_revision,
        bytes,
    } = work;
    let parse_started = Instant::now();
    let output =
        parse_work_item_inner(&file, &document, &source_revision, &bytes, linear_promotion);
    let parse_finished = Instant::now();

    ParseWorkResult {
        index,
        file,
        document,
        source_revision,
        parse_elapsed: parse_finished.saturating_duration_since(parse_started),
        parse_started,
        parse_finished,
        pdf_parse_timings: output.pdf_parse_timings,
        post_parser_timings: output.post_parser_timings,
        outcome: output.outcome,
    }
}

pub(crate) fn parse_work_item_inner(
    file: &DiscoveredFile,
    document: &Document,
    source_revision: &SourceRevision,
    bytes: &[u8],
    linear_promotion: &LinearPromotionPolicy,
) -> ParseWorkItemOutput {
    let extension = file_extension_label(&file.extension);
    let mut pdf_parse_timings = PdfTextExtractionTimings::default();
    let mut post_parser_timings = ImportPostParserTimings::default();
    let parse_output = match file.extension {
        FileExtension::Docx => DocxParser.parse(
            ParseInput::from_bytes(Some(extension), bytes),
            ResourceBudget::default(),
        ),
        FileExtension::Doc => DocParser::default().parse(
            ParseInput::from_bytes(Some(extension), bytes),
            ResourceBudget::default(),
        ),
        FileExtension::Pdf => {
            match PdfParser.parse_with_timings(
                ParseInput::from_bytes(Some(extension), bytes),
                ResourceBudget::default(),
            ) {
                Ok((parse_output, timings)) => {
                    pdf_parse_timings = timings;
                    Ok(parse_output)
                }
                Err(error) => Err(error),
            }
        }
        FileExtension::Txt => TxtParser.parse(
            ParseInput::from_bytes(Some(extension), bytes),
            ResourceBudget::default().with_max_bytes(parser_text::DEFAULT_MAX_BYTES),
        ),
        _ => {
            return ParseWorkItemOutput {
                outcome: ParseWorkOutcome::Failed {
                    status: DocumentStatus::FailedPermanent,
                    kind: ImportFailureKind::UnsupportedExtension,
                },
                pdf_parse_timings,
                post_parser_timings,
            };
        }
    };

    let parse_output = match parse_output {
        Ok(parse_output) => parse_output,
        Err(error) => {
            let status = if error.retryable() {
                DocumentStatus::FailedRetryable
            } else if error.kind() == ParserErrorKind::OcrRequired {
                DocumentStatus::OcrRequired
            } else {
                DocumentStatus::FailedPermanent
            };
            let outcome = if status == DocumentStatus::OcrRequired {
                ParseWorkOutcome::OcrRequired
            } else {
                ParseWorkOutcome::Failed {
                    status,
                    kind: ImportFailureKind::from_parser_error(error.kind()),
                }
            };
            return ParseWorkItemOutput {
                outcome,
                pdf_parse_timings,
                post_parser_timings,
            };
        }
    };

    if parse_output.status() == ParseStatus::OcrRequired {
        return ParseWorkItemOutput {
            outcome: ParseWorkOutcome::OcrRequired,
            pdf_parse_timings,
            post_parser_timings,
        };
    }

    let clean_text = measure_stage(&mut post_parser_timings.normalization, || {
        TextNormalizer::normalize_text_only(parse_output.text())
    });
    if clean_text.trim().is_empty() {
        let outcome = if file.extension == FileExtension::Txt {
            ParseWorkOutcome::Failed {
                status: DocumentStatus::FailedPermanent,
                kind: ImportFailureKind::EmptyText,
            }
        } else {
            ParseWorkOutcome::OcrRequired
        };
        return ParseWorkItemOutput {
            outcome,
            pdf_parse_timings,
            post_parser_timings,
        };
    }

    let sections = measure_stage(&mut post_parser_timings.sectionization, || {
        Sectionizer::default().sectionize(&clean_text)
    });
    let decision =
        AdmissionDecision::after_sectionization(&clean_text, &sections, linear_promotion);
    let admitted = decision.admits_search_index();
    let version = resume_version(
        document,
        source_revision,
        clean_text.clone(),
        PARSE_VERSION,
        SCHEMA_VERSION,
        language_set(&clean_text),
        parse_output
            .page_count()
            .and_then(|page_count| u32::try_from(page_count).ok()),
        Some(0.8),
    );
    let version_id = version.id.clone();
    let outcome = if admitted {
        ParseWorkOutcome::Searchable {
            decision,
            version: Box::new(version),
            mentions: entity_mentions_from_rules(&version_id, &clean_text),
            index_document: Box::new(IndexDocument {
                doc_id: document.id.to_string(),
                resume_version_id: version_id.to_string(),
                file_name: file.file_name.clone(),
                clean_text,
                sections: sections_to_index(sections),
            }),
        }
    } else {
        ParseWorkOutcome::Excluded {
            decision,
            version: Box::new(version),
        }
    };

    ParseWorkItemOutput {
        outcome,
        pdf_parse_timings,
        post_parser_timings,
    }
}
