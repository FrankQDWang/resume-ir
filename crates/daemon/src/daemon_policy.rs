pub(crate) const IMPORT_RETRY_BACKOFF_SECONDS: i64 = 60;
pub(crate) const DEFAULT_IMPORT_RESCAN_MIN_AGE_SECONDS: i64 = 300;
pub(crate) const STALE_IMPORT_TASK_SECONDS: i64 = 15 * 60;
pub(crate) const STALE_INGEST_JOB_SECONDS: i64 = 15 * 60;
pub(crate) const SEARCH_RESULT_FILE_NAME_MAX_BYTES: usize = 160;
pub(crate) const IPC_METADATA_READ_ATTEMPTS: usize = 40;
pub(crate) const IPC_METADATA_READ_RETRY_MS: u64 = 25;
pub(crate) const IMPORT_PROGRESS_STREAM_EVENTS: usize = 3;
pub(crate) const IMPORT_PROGRESS_STREAM_INTERVAL_MS: u64 = 25;
pub(crate) const DEFAULT_OCR_JOBS_PER_TICK: usize = 1;
pub(crate) const OCR_PAGE_BUDGET_REMEDIATION: &str =
    "raise OCR max pages per document or skip oversized scanned PDFs";
pub(crate) const OCR_LANGUAGE_REMEDIATION: &str =
    "install requested OCR language packs or choose an installed OCR language";
pub(crate) const FIELD_CONFIDENCE_THRESHOLD: f32 = 0.75;
