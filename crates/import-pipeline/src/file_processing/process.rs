use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fs_crawler::DiscoveredFile;
use index_fulltext::IndexDocument;
use meta_store::{DocumentStatus, FileExtension, OwnedMetaStore, SourceRevision, UnixTimestamp};
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
use super::model::ExactRerunDecision;
use super::persistence::{
    entity_mentions_from_rules, mark_ocr_required_and_enqueue,
    persist_document_failure_without_revision, persist_non_searchable,
    persist_source_revision_failure, prepare_pending_searchable_document,
};
use super::rerun::exact_rerun_decision;
use crate::classification::AdmissionDecision;
use crate::immutable_ingest::{resume_version, source_revision};
use crate::source_digest::stream_content_digest;
use crate::source_dispositions::ProcessedFile;
use crate::timing::{measure_result_stage, measure_stage};
use crate::{
    ImportFailureKind, ImportPostParserTimings, ImportStageTimings, ImportWorkerMetrics, Result,
    PARSE_VERSION, SCHEMA_VERSION,
};

pub(crate) fn process_file(
    data_dir: &Path,
    store: &OwnedMetaStore,
    file: &DiscoveredFile,
    sectionizer: &Sectionizer,
    now: UnixTimestamp,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    stage_timings: &mut ImportStageTimings,
    worker_metrics: &mut ImportWorkerMetrics,
    content_bytes_read: &mut u64,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<ProcessedFile> {
    let started = Instant::now();
    let mut db_elapsed = Duration::ZERO;
    let result = process_file_inner(
        data_dir,
        store,
        file,
        sectionizer,
        now,
        ensure_not_cancelled,
        &mut db_elapsed,
        worker_metrics,
        content_bytes_read,
        linear_promotion,
    );
    stage_timings.db += db_elapsed;
    stage_timings.parse += started.elapsed().saturating_sub(db_elapsed);
    result
}

fn process_file_inner(
    data_dir: &Path,
    store: &OwnedMetaStore,
    file: &DiscoveredFile,
    sectionizer: &Sectionizer,
    now: UnixTimestamp,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    db_elapsed: &mut Duration,
    worker_metrics: &mut ImportWorkerMetrics,
    content_bytes_read: &mut u64,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<ProcessedFile> {
    ensure_not_cancelled()?;
    let mut document = document_from_discovered_file(file, now, DocumentStatus::Discovered);
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
            return Ok(ProcessedFile::Failed {
                kind: ImportFailureKind::ReadError,
                source_revision_id: None,
            });
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
        return Ok(ProcessedFile::Failed {
            kind: ImportFailureKind::TextTooLarge,
            source_revision_id: Some(source_revision.id),
        });
    }

    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => {
            document.status = DocumentStatus::FailedRetryable;
            document.updated_at = now;
            measure_result_stage(db_elapsed, || {
                persist_document_failure_without_revision(store, &document)
            })?;
            return Ok(ProcessedFile::Failed {
                kind: ImportFailureKind::ReadError,
                source_revision_id: None,
            });
        }
    };
    *content_bytes_read += bytes.len() as u64;
    ensure_not_cancelled()?;

    let source_revision = source_revision(&document, &bytes);
    if let Some(noop_kind) = measure_result_stage(db_elapsed, || {
        exact_rerun_decision(
            store,
            file,
            &source_revision.content_hash,
            linear_promotion,
            now,
        )
    })? {
        return Ok(match noop_kind {
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
        });
    }

    document.content_hash = Some(source_revision.content_hash.as_str().to_string());
    document.byte_size = source_revision.byte_size;
    ensure_not_cancelled()?;

    let extension = file_extension_label(&file.extension);
    ensure_not_cancelled()?;
    let mut pdf_parse_timings = PdfTextExtractionTimings::default();
    let mut post_parser_timings = ImportPostParserTimings::default();
    let parse_output = match file.extension {
        FileExtension::Docx => DocxParser
            .parse(
                ParseInput::from_bytes(Some(extension), &bytes),
                ResourceBudget::default(),
            )
            .map_err(|error| (error, document.clone())),
        FileExtension::Doc => DocParser::default()
            .parse(
                ParseInput::from_bytes(Some(extension), &bytes),
                ResourceBudget::default(),
            )
            .map_err(|error| (error, document.clone())),
        FileExtension::Pdf => match PdfParser.parse_with_timings(
            ParseInput::from_bytes(Some(extension), &bytes),
            ResourceBudget::default(),
        ) {
            Ok((parse_output, timings)) => {
                pdf_parse_timings = timings;
                Ok(parse_output)
            }
            Err(error) => Err((error, document.clone())),
        },
        FileExtension::Txt => TxtParser
            .parse(
                ParseInput::from_bytes(Some(extension), &bytes),
                ResourceBudget::default().with_max_bytes(parser_text::DEFAULT_MAX_BYTES),
            )
            .map_err(|error| (error, document.clone())),
        _ => {
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
            return Ok(ProcessedFile::Failed {
                kind: ImportFailureKind::UnsupportedExtension,
                source_revision_id: Some(source_revision.id),
            });
        }
    };
    worker_metrics
        .pdf_parse_timings
        .add_assign(&pdf_parse_timings);
    ensure_not_cancelled()?;

    let parse_output = match parse_output {
        Ok(parse_output) => parse_output,
        Err((error, mut document)) => {
            document.status = if error.retryable() {
                DocumentStatus::FailedRetryable
            } else if error.kind() == ParserErrorKind::OcrRequired {
                DocumentStatus::OcrRequired
            } else {
                DocumentStatus::FailedPermanent
            };
            return Ok(if document.status == DocumentStatus::OcrRequired {
                ProcessedFile::OcrRequired {
                    ocr_job_queued: measure_result_stage(db_elapsed, || {
                        mark_ocr_required_and_enqueue(
                            store,
                            &mut document,
                            &source_revision,
                            now,
                            linear_promotion,
                        )
                    })?,
                    source_revision_id: source_revision.id,
                }
            } else {
                measure_result_stage(db_elapsed, || {
                    persist_source_revision_failure(
                        store,
                        &document,
                        &source_revision,
                        now,
                        linear_promotion,
                    )
                })?;
                ProcessedFile::Failed {
                    kind: ImportFailureKind::from_parser_error(error.kind()),
                    source_revision_id: Some(source_revision.id),
                }
            });
        }
    };

    if parse_output.status() == ParseStatus::OcrRequired {
        ensure_not_cancelled()?;
        return Ok(ProcessedFile::OcrRequired {
            ocr_job_queued: measure_result_stage(db_elapsed, || {
                mark_ocr_required_and_enqueue(
                    store,
                    &mut document,
                    &source_revision,
                    now,
                    linear_promotion,
                )
            })?,
            source_revision_id: source_revision.id,
        });
    }

    ensure_not_cancelled()?;
    let clean_text = measure_stage(&mut post_parser_timings.normalization, || {
        TextNormalizer::normalize_text_only(parse_output.text())
    });
    worker_metrics
        .post_parser_timings
        .add_assign(&post_parser_timings);
    if clean_text.trim().is_empty() {
        if file.extension == FileExtension::Txt {
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
            return Ok(ProcessedFile::Failed {
                kind: ImportFailureKind::EmptyText,
                source_revision_id: Some(source_revision.id),
            });
        }

        ensure_not_cancelled()?;
        return Ok(ProcessedFile::OcrRequired {
            ocr_job_queued: measure_result_stage(db_elapsed, || {
                mark_ocr_required_and_enqueue(
                    store,
                    &mut document,
                    &source_revision,
                    now,
                    linear_promotion,
                )
            })?,
            source_revision_id: source_revision.id,
        });
    }

    ensure_not_cancelled()?;
    document.status = DocumentStatus::TextCleaned;
    let sections = measure_stage(&mut post_parser_timings.sectionization, || {
        sectionizer.sectionize(&clean_text)
    });
    worker_metrics.post_parser_timings.sectionization += post_parser_timings.sectionization;
    let decision =
        AdmissionDecision::after_sectionization(&clean_text, &sections, linear_promotion);
    let admitted = decision.admits_search_index();
    let version = resume_version(
        &document,
        &source_revision,
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
    if !admitted {
        document.status = DocumentStatus::Excluded;
        measure_result_stage(db_elapsed, || {
            persist_non_searchable(store, &document, &source_revision, &version, decision, now)
        })?;
        return Ok(ProcessedFile::Excluded {
            document: Box::new(document),
            source_revision_id: source_revision.id,
            resume_version_id: version.id,
        });
    }
    let mentions = entity_mentions_from_rules(&version_id, &clean_text);
    let index_document = IndexDocument {
        doc_id: document.id.to_string(),
        resume_version_id: version_id.to_string(),
        file_name: file.file_name.clone(),
        clean_text,
        sections: sections_to_index(sections),
    };
    ensure_not_cancelled()?;
    measure_result_stage(db_elapsed, || {
        prepare_pending_searchable_document(
            data_dir,
            document,
            source_revision,
            decision,
            version,
            mentions,
            index_document,
            now,
        )
    })
}
