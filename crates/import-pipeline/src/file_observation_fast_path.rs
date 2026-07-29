use std::fs;

use fs_crawler::{observe_open_file, observe_path, DiscoveredFile, FileObservation};
use meta_store::{OwnedMetaStore, SourceRevisionId, UnixTimestamp};
use resume_classifier::LinearPromotionPolicy;

use crate::file_processing::{exact_rerun_decision, processed_file_from_exact};
use crate::source_dispositions::ProcessedFile;
use crate::{ImportIoMetrics, ImportMetadataFallbackReason, ImportPipelineError, Result};

const MIN_AUDIT_INTERVAL_SECONDS: i64 = 6 * 60 * 60;
const AUDIT_JITTER_SECONDS: i64 = 18 * 60 * 60;

pub(crate) enum FastPathAttempt {
    Hit(ProcessedFile),
    Fallback,
}

pub(crate) fn attempt_metadata_fast_path(
    store: &OwnedMetaStore,
    file: &DiscoveredFile,
    now: UnixTimestamp,
    linear_promotion: &LinearPromotionPolicy,
    io_metrics: &mut ImportIoMetrics,
) -> Result<FastPathAttempt> {
    let Some(discovered) = file.observation.as_ref() else {
        io_metrics.record_metadata_fallback(ImportMetadataFallbackReason::UnsupportedObservation);
        return Ok(FastPathAttempt::Fallback);
    };
    let Some(persisted) = store
        .source_file_observation_for_document(&file.document_id)
        .map_err(ImportPipelineError::store)?
    else {
        io_metrics.record_metadata_fallback(ImportMetadataFallbackReason::ObservationMissing);
        return Ok(FastPathAttempt::Fallback);
    };
    if now.as_unix_seconds() >= persisted.next_strong_verification_at.as_unix_seconds() {
        io_metrics.record_metadata_fallback(ImportMetadataFallbackReason::AuditDue);
        return Ok(FastPathAttempt::Fallback);
    }
    if !persisted_matches(&persisted, discovered) {
        io_metrics.record_metadata_fallback(ImportMetadataFallbackReason::MetadataMismatch);
        return Ok(FastPathAttempt::Fallback);
    }

    let handle = match fs::File::open(file.normalized_path.as_str()) {
        Ok(handle) => handle,
        Err(_) => {
            io_metrics.record_metadata_fallback(ImportMetadataFallbackReason::MetadataIo);
            return Ok(FastPathAttempt::Fallback);
        }
    };
    io_metrics.metadata_handle_open_count = io_metrics.metadata_handle_open_count.saturating_add(1);
    if observe_open_file(&handle).ok().flatten().as_ref() != Some(discovered) {
        io_metrics.record_metadata_fallback(ImportMetadataFallbackReason::MetadataMismatch);
        return Ok(FastPathAttempt::Fallback);
    }

    let Some(decision) =
        exact_rerun_decision(store, file, &persisted.content_hash, linear_promotion, now)?
    else {
        io_metrics.record_metadata_fallback(ImportMetadataFallbackReason::ProcessingContract);
        return Ok(FastPathAttempt::Fallback);
    };

    let handle_unchanged = observe_open_file(&handle).ok().flatten().as_ref() == Some(discovered);
    let path_unchanged = observe_path(std::path::Path::new(file.normalized_path.as_str()))
        .ok()
        .flatten()
        .as_ref()
        == Some(discovered);
    if !handle_unchanged || !path_unchanged {
        io_metrics.record_metadata_fallback(ImportMetadataFallbackReason::ChangedDuringImport);
        return Ok(FastPathAttempt::Fallback);
    }

    io_metrics.metadata_fast_path_hits = io_metrics.metadata_fast_path_hits.saturating_add(1);
    io_metrics.strong_hashes_skipped = io_metrics.strong_hashes_skipped.saturating_add(1);
    Ok(FastPathAttempt::Hit(processed_file_from_exact(decision)))
}

pub(crate) fn revalidate_discovered_observation(file: &DiscoveredFile) -> bool {
    let Some(expected) = file.observation.as_ref() else {
        return false;
    };
    observe_path(std::path::Path::new(file.normalized_path.as_str()))
        .ok()
        .flatten()
        .as_ref()
        == Some(expected)
}

pub(crate) fn next_strong_verification_at(
    now: UnixTimestamp,
    stable_file_id: &str,
) -> UnixTimestamp {
    let suffix = stable_file_id
        .get(stable_file_id.len().saturating_sub(8)..)
        .and_then(|value| u32::from_str_radix(value, 16).ok())
        .unwrap_or(0);
    let jitter = i64::from(suffix) % AUDIT_JITTER_SECONDS;
    UnixTimestamp::from_unix_seconds(
        now.as_unix_seconds()
            .saturating_add(MIN_AUDIT_INTERVAL_SECONDS)
            .saturating_add(jitter),
    )
}

fn persisted_matches(
    persisted: &meta_store::SourceFileObservation,
    observed: &FileObservation,
) -> bool {
    persisted.stable_file_id == observed.stable_file_id.as_str()
        && persisted.byte_size == observed.byte_size
        && persisted.mtime_seconds == observed.modified.seconds
        && persisted.mtime_nanoseconds == observed.modified.nanoseconds
        && persisted.ctime_seconds == observed.changed.seconds
        && persisted.ctime_nanoseconds == observed.changed.nanoseconds
}

pub(crate) fn strong_store_observation(
    source_revision_id: SourceRevisionId,
    observed: &FileObservation,
    now: UnixTimestamp,
) -> meta_store::StrongSourceFileObservation {
    meta_store::StrongSourceFileObservation {
        source_revision_id,
        stable_file_id: observed.stable_file_id.as_str().to_string(),
        byte_size: observed.byte_size,
        mtime_seconds: observed.modified.seconds,
        mtime_nanoseconds: observed.modified.nanoseconds,
        ctime_seconds: observed.changed.seconds,
        ctime_nanoseconds: observed.changed.nanoseconds,
        strongly_verified_at: now,
        next_strong_verification_at: next_strong_verification_at(
            now,
            observed.stable_file_id.as_str(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_deadline_is_deterministic_and_bounded_to_one_day() {
        let now = UnixTimestamp::from_unix_seconds(1_900_000_000);
        let stable_file_id = "sfi_0123456789abcdef0123456789abcdef";
        let deadline = next_strong_verification_at(now, stable_file_id);
        let delta = deadline.as_unix_seconds() - now.as_unix_seconds();

        assert!((6 * 60 * 60..24 * 60 * 60).contains(&delta));
        assert_eq!(deadline, next_strong_verification_at(now, stable_file_id));
    }

    #[test]
    fn final_path_revalidation_rejects_replacement_after_processing() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("synthetic.txt");
        fs::write(&path, b"first synthetic content").unwrap();
        let file = fs_crawler::crawl_directory(root.path())
            .unwrap()
            .files
            .remove(0);

        fs::remove_file(&path).unwrap();
        fs::write(&path, b"replacement synthetic content").unwrap();

        assert!(!revalidate_discovered_observation(&file));
    }
}
