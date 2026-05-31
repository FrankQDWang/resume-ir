//! Daemon lifecycle skeleton for local resume indexing.

use import_worker::{run_import_root, ImportSummary};
use meta_store::MetadataStore;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const IMPORT_LEASE_MS: i64 = 5 * 60 * 1_000;
const IMPORT_FAILURE_DIAGNOSTIC: &str = "local import worker failed";
const STALE_IMPORT_CLAIM_ERROR: &str = "Import task claim was no longer current.";

/// Returns the crate name for smoke tests and workspace metadata.
#[must_use]
pub fn crate_name() -> &'static str {
    "resume-daemon"
}

/// Runs the daemon command with explicit arguments and output sink.
pub fn run_with_args<I, S, W>(args: I, output: &mut W) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
    W: Write,
{
    let options = DaemonOptions::parse(args)?;
    if !options.foreground {
        return Err(
            "Daemon currently supports foreground mode only. Use --foreground.".to_string(),
        );
    }

    let store = open_store(&options.data_dir)?;
    let status = store
        .status()
        .map_err(|error| error.user_message().to_string())?;
    writeln!(output, "daemon foreground started").map_err(|error| error.to_string())?;
    writeln!(output, "metadata schema: {}", status.schema_version)
        .map_err(|error| error.to_string())?;
    writeln!(
        output,
        "queued imports: {}",
        status.queued_import_task_count
    )
    .map_err(|error| error.to_string())?;

    if options.once {
        run_once_import_drain(&store, &options.data_dir, output)?;
        writeln!(output, "daemon foreground stopped").map_err(|error| error.to_string())?;
    } else {
        writeln!(output, "daemon foreground skeleton exited").map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn run_once_import_drain<W: Write>(
    store: &MetadataStore,
    data_dir: &Path,
    output: &mut W,
) -> Result<(), String> {
    run_once_import_drain_with_worker(store, data_dir, output, run_import_root)
}

fn run_once_import_drain_with_worker<W, F>(
    store: &MetadataStore,
    data_dir: &Path,
    output: &mut W,
    worker: F,
) -> Result<(), String>
where
    W: Write,
    F: FnOnce(&MetadataStore, &Path, &Path) -> Result<ImportSummary, String>,
{
    let now_ms = current_time_ms()?;
    let claim_token = local_import_claim_token(now_ms);
    let lease_expires_at_ms = import_lease_expires_at_ms(now_ms);
    let Some(claim) = store
        .claim_next_import_task(&claim_token, now_ms, lease_expires_at_ms)
        .map_err(|error| error.user_message().to_string())?
    else {
        writeln!(output, "claimed imports: 0").map_err(|error| error.to_string())?;
        write_import_summary(output, &ImportSummary::default())?;
        return Ok(());
    };

    let summary = match worker(store, data_dir, Path::new(claim.root_path.as_str())) {
        Ok(summary) => {
            complete_current_import_claim(store, claim.task_id, claim.claim_token.as_str())?;
            summary
        }
        Err(_) => {
            fail_current_import_claim(store, claim.task_id, claim.claim_token.as_str())?;
            ImportSummary::default()
        }
    };

    writeln!(output, "claimed imports: 1").map_err(|error| error.to_string())?;
    write_import_summary(output, &summary)
}

fn write_import_summary<W: Write>(output: &mut W, summary: &ImportSummary) -> Result<(), String> {
    writeln!(
        output,
        "discovered documents: {}",
        summary.discovered_documents
    )
    .map_err(|error| error.to_string())?;
    writeln!(
        output,
        "searchable documents: {}",
        summary.searchable_documents
    )
    .map_err(|error| error.to_string())?;
    writeln!(
        output,
        "ocr required documents: {}",
        summary.ocr_required_documents
    )
    .map_err(|error| error.to_string())?;
    writeln!(output, "skipped documents: {}", summary.skipped_documents)
        .map_err(|error| error.to_string())
}

fn complete_current_import_claim(
    store: &MetadataStore,
    task_id: i64,
    claim_token: &str,
) -> Result<(), String> {
    let completed = store
        .complete_claimed_import_task(task_id, claim_token)
        .map_err(|error| error.user_message().to_string())?;
    if completed {
        Ok(())
    } else {
        Err(STALE_IMPORT_CLAIM_ERROR.to_string())
    }
}

fn fail_current_import_claim(
    store: &MetadataStore,
    task_id: i64,
    claim_token: &str,
) -> Result<(), String> {
    let failed = store
        .fail_claimed_import_task(task_id, claim_token, IMPORT_FAILURE_DIAGNOSTIC)
        .map_err(|error| error.user_message().to_string())?;
    if failed {
        Ok(())
    } else {
        Err(STALE_IMPORT_CLAIM_ERROR.to_string())
    }
}

fn current_time_ms() -> Result<i64, String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "System clock is before the Unix epoch.".to_string())?
        .as_millis();
    i64::try_from(elapsed).map_err(|_| "System clock timestamp is too large.".to_string())
}

fn import_lease_expires_at_ms(now_ms: i64) -> i64 {
    now_ms.saturating_add(IMPORT_LEASE_MS)
}

fn local_import_claim_token(now_ms: i64) -> String {
    format!("resume-daemon-local-import-{now_ms}")
}

struct DaemonOptions {
    data_dir: PathBuf,
    foreground: bool,
    once: bool,
}

impl DaemonOptions {
    fn parse<I, S>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut data_dir = PathBuf::from("local-data");
        let mut foreground = false;
        let mut once = false;
        let mut args = args.into_iter();
        let _program = args.next();

        while let Some(arg) = args.next() {
            match arg.as_ref() {
                "--data-dir" => {
                    let Some(value) = args.next() else {
                        return Err("Missing value for --data-dir.".to_string());
                    };
                    data_dir = PathBuf::from(value.as_ref());
                }
                "--foreground" => foreground = true,
                "--once" => once = true,
                _ => {
                    return Err(
                        "Usage: resume-daemon [--data-dir <path>] --foreground [--once]"
                            .to_string(),
                    );
                }
            }
        }

        Ok(Self {
            data_dir,
            foreground,
            once,
        })
    }
}

fn open_store(data_dir: &Path) -> Result<MetadataStore, String> {
    fs::create_dir_all(data_dir)
        .map_err(|error| format!("Could not create local data directory: {error}"))?;
    let store = MetadataStore::open(data_dir.join("metadata.sqlite"))
        .map_err(|error| error.user_message().to_string())?;
    store
        .run_migrations()
        .map_err(|error| error.user_message().to_string())?;
    Ok(store)
}

#[cfg(test)]
mod tests {
    use super::{
        complete_current_import_claim, fail_current_import_claim,
        run_once_import_drain_with_worker, run_with_args, STALE_IMPORT_CLAIM_ERROR,
    };
    use import_worker::ImportSummary;
    use meta_store::MetadataStore;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn foreground_once_initializes_store_and_exits() -> Result<(), String> {
        let data_dir = unique_data_dir("daemon")?;
        let mut output = Vec::new();

        run_with_args(
            [
                "resume-daemon",
                "--data-dir",
                data_dir.as_str(),
                "--foreground",
                "--once",
            ],
            &mut output,
        )?;

        let text = String::from_utf8(output).map_err(|error| error.to_string())?;
        assert!(text.contains("daemon foreground started"));
        assert!(text.contains("metadata schema: 3"));
        assert!(text.contains("daemon foreground stopped"));
        assert!(data_dir.join("metadata.sqlite").is_file());
        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn foreground_without_once_is_supported_by_skeleton() -> Result<(), String> {
        let data_dir = unique_data_dir("daemon-foreground")?;
        let mut output = Vec::new();

        run_with_args(
            [
                "resume-daemon",
                "--data-dir",
                data_dir.as_str(),
                "--foreground",
            ],
            &mut output,
        )?;

        let text = String::from_utf8(output).map_err(|error| error.to_string())?;
        assert!(text.contains("daemon foreground started"));
        assert!(text.contains("daemon foreground skeleton exited"));
        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn foreground_once_claims_one_queued_import_and_prints_only_aggregate_counts(
    ) -> Result<(), String> {
        let data_dir = unique_data_dir("daemon-import-drain")?;
        let import_root = data_dir.join("private-root");
        fs::create_dir_all(import_root.as_ref()).map_err(|error| error.to_string())?;
        fs::write(
            import_root.join("private-searchable.pdf").as_ref(),
            text_layer_pdf_bytes_with("Daemon synthetic Java engineer with PDF text"),
        )
        .map_err(|error| error.to_string())?;
        fs::write(
            import_root.join("private-scan.pdf").as_ref(),
            image_only_pdf_bytes(),
        )
        .map_err(|error| error.to_string())?;

        let store = open_test_store(&data_dir)?;
        store
            .enqueue_import_root(import_root.as_ref())
            .map_err(|error| error.user_message().to_string())?;

        let mut output = Vec::new();
        run_with_args(
            [
                "resume-daemon",
                "--data-dir",
                data_dir.as_str(),
                "--foreground",
                "--once",
            ],
            &mut output,
        )?;

        let text = String::from_utf8(output).map_err(|error| error.to_string())?;
        assert!(text.contains("claimed imports: 1"));
        assert!(text.contains("discovered documents: 2"));
        assert!(text.contains("searchable documents: 1"));
        assert!(text.contains("ocr required documents: 1"));
        assert!(text.contains("skipped documents: 0"));
        assert!(!text.contains(import_root.as_str()));
        assert!(!text.contains("private-root"));
        assert!(!text.contains("private-searchable.pdf"));
        assert!(!text.contains("private-scan.pdf"));

        let reopened = open_test_store(&data_dir)?;
        let status = reopened
            .status()
            .map_err(|error| error.user_message().to_string())?;
        assert_eq!(status.queued_import_task_count, 0);
        assert_eq!(status.searchable_document_count, 1);
        assert_eq!(status.ocr_required_document_count, 1);
        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn foreground_once_failed_import_is_retryable_without_printing_root_path() -> Result<(), String>
    {
        let data_dir = unique_data_dir("daemon-import-failure")?;
        let missing_root = data_dir.join("missing-private-root");
        let store = open_test_store(&data_dir)?;
        let task_id = store
            .enqueue_import_root(missing_root.as_ref())
            .map_err(|error| error.user_message().to_string())?;

        let mut output = Vec::new();
        run_with_args(
            [
                "resume-daemon",
                "--data-dir",
                data_dir.as_str(),
                "--foreground",
                "--once",
            ],
            &mut output,
        )?;

        let text = String::from_utf8(output).map_err(|error| error.to_string())?;
        assert!(text.contains("claimed imports: 1"));
        assert!(text.contains("discovered documents: 0"));
        assert!(text.contains("searchable documents: 0"));
        assert!(text.contains("ocr required documents: 0"));
        assert!(text.contains("skipped documents: 0"));
        assert!(!text.contains(missing_root.as_str()));
        assert!(!text.contains("missing-private-root"));
        assert!(!text.contains("local import worker failed"));

        let reopened = open_test_store(&data_dir)?;
        let retry = reopened
            .claim_import_task(task_id, "test-retry-token", 2_000, 3_000)
            .map_err(|error| error.user_message().to_string())?;
        assert!(retry.is_some());
        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn stale_import_claim_finalization_returns_redacted_error() -> Result<(), String> {
        let data_dir = unique_data_dir("daemon-stale-claim")?;
        let private_root = data_dir.join("private-root");
        let store = open_test_store(&data_dir)?;
        let complete_task = store
            .enqueue_import_root(private_root.as_ref())
            .map_err(|error| error.user_message().to_string())?;
        let fail_task = store
            .enqueue_import_root(private_root.as_ref())
            .map_err(|error| error.user_message().to_string())?;

        store
            .claim_import_task(complete_task, "current-complete-token", 1_000, 2_000)
            .map_err(|error| error.user_message().to_string())?;
        store
            .claim_import_task(fail_task, "current-fail-token", 1_000, 2_000)
            .map_err(|error| error.user_message().to_string())?;

        let complete_error =
            complete_current_import_claim(&store, complete_task, "stale-private-token")
                .err()
                .ok_or_else(|| "stale complete claim should fail".to_string())?;
        let fail_error = fail_current_import_claim(&store, fail_task, "stale-private-token")
            .err()
            .ok_or_else(|| "stale fail claim should fail".to_string())?;

        for error in [complete_error, fail_error] {
            assert_eq!(error, STALE_IMPORT_CLAIM_ERROR);
            assert!(!error.contains("stale-private-token"));
            assert!(!error.contains(private_root.as_str()));
            assert!(!error.contains("local import worker failed"));
        }

        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn once_drain_stops_when_import_claim_was_reclaimed_before_completion() -> Result<(), String> {
        let data_dir = unique_data_dir("daemon-reclaimed-claim")?;
        let private_root = data_dir.join("private-root");
        fs::create_dir_all(private_root.as_ref()).map_err(|error| error.to_string())?;
        let store = open_test_store(&data_dir)?;
        store
            .enqueue_import_root(private_root.as_ref())
            .map_err(|error| error.user_message().to_string())?;
        let mut output = Vec::new();

        let error = run_once_import_drain_with_worker(
            &store,
            data_dir.as_ref(),
            &mut output,
            |store, _data_dir, _root| {
                let stolen = store
                    .claim_next_import_task("stale-takeover-token", i64::MAX - 1, i64::MAX)
                    .map_err(|error| error.user_message().to_string())?;
                if stolen.is_none() {
                    return Err("claim takeover failed".to_string());
                }
                Ok(ImportSummary {
                    discovered_documents: 1,
                    searchable_documents: 1,
                    ocr_required_documents: 0,
                    skipped_documents: 0,
                })
            },
        )
        .err()
        .ok_or_else(|| "reclaimed claim should stop daemon drain".to_string())?;

        assert_eq!(error, STALE_IMPORT_CLAIM_ERROR);
        let text = String::from_utf8(output).map_err(|error| error.to_string())?;
        assert!(!text.contains("claimed imports: 1"));
        assert!(!text.contains(private_root.as_str()));
        assert!(!text.contains("private-root"));
        assert!(!text.contains("stale-takeover-token"));

        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    fn open_test_store(data_dir: &TestPath) -> Result<MetadataStore, String> {
        let store = MetadataStore::open(data_dir.join("metadata.sqlite").as_ref())
            .map_err(|error| error.user_message().to_string())?;
        store
            .run_migrations()
            .map_err(|error| error.user_message().to_string())?;
        Ok(store)
    }

    fn text_layer_pdf_bytes_with(text: &str) -> Vec<u8> {
        format!(
            "%PDF-1.4
1 0 obj
<< /Type /Page /Contents 2 0 R /Resources << /Font << /F1 3 0 R >> >> >>
endobj
2 0 obj
<< /Length 90 >>
stream
BT
/F1 12 Tf
72 720 Td
({text}) Tj
ET
endstream
endobj
3 0 obj
<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>
endobj
%%EOF"
        )
        .into_bytes()
    }

    fn image_only_pdf_bytes() -> Vec<u8> {
        b"%PDF-1.4
1 0 obj
<< /Type /Page /Resources << /XObject << /Im1 2 0 R >> >> /Contents 3 0 R >>
endobj
2 0 obj
<< /Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 4 >>
stream
0000
endstream
endobj
3 0 obj
<< /Length 24 >>
stream
q 10 0 0 10 0 0 cm /Im1 Do Q
endstream
endobj
%%EOF"
            .to_vec()
    }

    fn unique_data_dir(label: &str) -> Result<TestPath, String> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "resume-daemon-{label}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        Ok(TestPath(path))
    }

    struct TestPath(std::path::PathBuf);

    impl TestPath {
        fn join(&self, path: &str) -> Self {
            Self(self.0.join(path))
        }

        fn as_str(&self) -> &str {
            self.0.to_str().unwrap_or("")
        }

        fn is_file(&self) -> bool {
            self.0.is_file()
        }
    }

    impl AsRef<std::path::Path> for TestPath {
        fn as_ref(&self) -> &std::path::Path {
            &self.0
        }
    }
}
