use ocr_client::{
    CancellationToken, DisabledOcrWorkerClient, LocalOcrCommandClient, LocalOcrCommandSpec,
    LocalPdfRenderCommandClient, LocalPdfRenderCommandSpec, OcrCacheKey, OcrClient, OcrErrorKind,
    OcrOptions, OcrPage, OcrPageRequest, OcrWorkerBudget, PdftoppmPdfRenderer, PdftoppmRenderSpec,
    RenderedPage,
};

#[cfg(unix)]
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn exposes_ocr_client_crate_identity() {
    assert_eq!(ocr_client::crate_name(), "ocr-client");
}

#[test]
fn cache_key_and_page_result_keep_ocr_payloads_out_of_debug() {
    let cache_key =
        OcrCacheKey::new("0123456789abcdef", 3, 300, "eng+chi_sim", "balanced").unwrap();
    assert_eq!(cache_key.page_no(), 3);
    assert!(!format!("{cache_key:?}").contains("0123456789abcdef"));

    let page = OcrPage::new(3, "Synthetic OCR text", 0.88, "balanced", 42).unwrap();
    assert_eq!(page.page_no(), 3);
    assert_eq!(page.text(), "Synthetic OCR text");
    assert!(!format!("{page:?}").contains("Synthetic OCR text"));
}

#[test]
fn rejects_invalid_page_and_timeout_inputs() {
    assert!(OcrCacheKey::new("content", 0, 300, "eng", "balanced").is_err());
    assert!(RenderedPage::new(0, 300, b"bytes".to_vec()).is_err());
    assert!(OcrPage::new(1, "text", 1.5, "balanced", 1).is_err());
    assert!(OcrWorkerBudget::new(0).is_err());
}

#[test]
fn disabled_worker_never_runs_heavy_ocr_and_honors_cancellation() {
    let client = DisabledOcrWorkerClient;
    let request = OcrPageRequest::new(
        RenderedPage::new(1, 300, b"SYNTHETIC IMAGE BYTES".to_vec()).unwrap(),
        OcrOptions::new("eng", "economy").unwrap(),
    )
    .unwrap();

    let cancelled = client
        .recognize_page(
            request.clone(),
            OcrWorkerBudget::new(500).unwrap(),
            &CancellationToken::new_cancelled(),
        )
        .unwrap_err();
    assert_eq!(cancelled.kind(), OcrErrorKind::Cancelled);
    assert!(!format!("{cancelled:?}").contains("SYNTHETIC"));

    let disabled = client
        .recognize_page(
            request,
            OcrWorkerBudget::new(500).unwrap(),
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert_eq!(disabled.kind(), OcrErrorKind::Disabled);
}

#[cfg(unix)]
#[test]
fn local_command_worker_runs_configured_binary_and_parses_structured_output() {
    let command = write_fixture_executable(
        "fixture-ocr",
        r#"#!/bin/sh
input_size="$(wc -c < "$RESUME_IR_OCR_INPUT_PATH" | tr -d ' ')"
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.82\n'
printf 'text:\n'
printf 'page=%s dpi=%s lang=%s profile=%s bytes=%s\n' \
  "$RESUME_IR_OCR_PAGE_NO" \
  "$RESUME_IR_OCR_RENDER_DPI" \
  "$RESUME_IR_OCR_LANG" \
  "$RESUME_IR_OCR_PROFILE" \
  "$input_size"
"#,
    );
    let client = LocalOcrCommandClient::new(
        LocalOcrCommandSpec::new(command, Vec::<String>::new(), "fixture-engine").unwrap(),
    );

    let page = client
        .recognize_page(
            ocr_request(2, b"SYNTHETIC IMAGE BYTES".to_vec()),
            OcrWorkerBudget::new(5_000).unwrap(),
            &CancellationToken::new(),
        )
        .unwrap();

    assert_eq!(page.page_no(), 2);
    assert_eq!(
        page.text(),
        "page=2 dpi=300 lang=eng profile=balanced bytes=21\n"
    );
    assert_eq!(page.confidence(), 0.82);
    assert_eq!(page.engine_profile(), "fixture-engine");
    assert!(!format!("{page:?}").contains("SYNTHETIC IMAGE BYTES"));
}

#[cfg(unix)]
#[test]
fn local_pdf_render_command_returns_page_bytes_without_payload_debug_leaks() {
    let command = write_fixture_executable(
        "fixture-pdf-render",
        r#"#!/bin/sh
input_size="$(wc -c < "$RESUME_IR_PDF_RENDER_INPUT_PATH" | tr -d ' ')"
printf 'rendered-page=%s dpi=%s pdf-bytes=%s' \
  "$RESUME_IR_PDF_RENDER_PAGE_NO" \
  "$RESUME_IR_PDF_RENDER_DPI" \
  "$input_size"
"#,
    );
    let client = LocalPdfRenderCommandClient::new(
        LocalPdfRenderCommandSpec::new(command, Vec::<String>::new()).unwrap(),
    );

    let rendered = client
        .render_page(
            b"SYNTHETIC PDF BYTES",
            2,
            300,
            OcrWorkerBudget::new(5_000).unwrap(),
            &CancellationToken::new(),
        )
        .unwrap();

    assert_eq!(rendered.page_no(), 2);
    assert_eq!(rendered.render_dpi(), 300);
    assert_eq!(rendered.bytes(), b"rendered-page=2 dpi=300 pdf-bytes=19");
    assert!(!format!("{rendered:?}").contains("SYNTHETIC PDF BYTES"));
    assert!(!format!("{rendered:?}").contains("rendered-page=2"));
}

#[cfg(unix)]
#[test]
fn pdftoppm_renderer_renders_valid_pdf_page_to_ppm_without_payload_debug_leaks() {
    let Some(pdftoppm) = find_command("pdftoppm") else {
        eprintln!("skipping pdftoppm renderer witness because pdftoppm is not installed");
        return;
    };
    let renderer =
        PdftoppmPdfRenderer::new(PdftoppmRenderSpec::new(pdftoppm).expect("pdftoppm spec"));
    let pdf_bytes = valid_blank_pdf_bytes();

    let rendered = renderer
        .render_page(
            &pdf_bytes,
            1,
            72,
            OcrWorkerBudget::new(5_000).unwrap(),
            &CancellationToken::new(),
        )
        .unwrap();

    assert_eq!(rendered.page_no(), 1);
    assert_eq!(rendered.render_dpi(), 72);
    assert!(
        rendered.bytes().starts_with(b"P6\n72 72\n255\n"),
        "unexpected PPM header: {:?}",
        &rendered.bytes()[..rendered.bytes().len().min(32)]
    );
    assert!(!format!("{rendered:?}").contains("%PDF"));
    assert!(!format!("{rendered:?}").contains("P6\n72 72"));
}

#[test]
fn local_command_worker_reports_missing_binary_as_worker_unavailable_without_payload_leaks() {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let missing_command = std::env::temp_dir().join(format!(
        "resume-ir-ocr-client-missing-binary-{}-{suffix}",
        std::process::id()
    ));
    let client = LocalOcrCommandClient::new(
        LocalOcrCommandSpec::new(missing_command, Vec::<String>::new(), "missing-engine").unwrap(),
    );

    let error = client
        .recognize_page(
            ocr_request(1, b"SYNTHETIC IMAGE BYTES".to_vec()),
            OcrWorkerBudget::new(500).unwrap(),
            &CancellationToken::new(),
        )
        .unwrap_err();

    assert_eq!(error.kind(), OcrErrorKind::WorkerUnavailable);
    assert!(!format!("{error:?}").contains("SYNTHETIC IMAGE BYTES"));
}

#[cfg(unix)]
#[test]
fn local_command_worker_times_out_and_does_not_report_late_output() {
    let command = write_fixture_executable(
        "slow-ocr",
        r#"#!/bin/sh
sleep 2
printf 'resume-ir-ocr-v1\nconfidence=0.99\ntext:\nlate output\n'
"#,
    );
    let client = LocalOcrCommandClient::new(
        LocalOcrCommandSpec::new(command, Vec::<String>::new(), "slow-engine").unwrap(),
    );

    let error = client
        .recognize_page(
            ocr_request(1, b"SYNTHETIC IMAGE BYTES".to_vec()),
            OcrWorkerBudget::new(50).unwrap(),
            &CancellationToken::new(),
        )
        .unwrap_err();

    assert_eq!(error.kind(), OcrErrorKind::Timeout);
    assert!(!format!("{error:?}").contains("late output"));
}

#[cfg(unix)]
#[test]
fn local_command_worker_terminates_descendants_that_keep_output_pipes_open() {
    let command = write_fixture_executable(
        "descendant-ocr",
        r#"#!/bin/sh
(trap "" HUP; sleep 2; printf 'resume-ir-ocr-v1\nconfidence=0.99\ntext:\nlate output\n') &
sleep 2
"#,
    );
    let client = LocalOcrCommandClient::new(
        LocalOcrCommandSpec::new(command, Vec::<String>::new(), "descendant-engine").unwrap(),
    );

    let started_at = Instant::now();
    let error = client
        .recognize_page(
            ocr_request(1, b"SYNTHETIC IMAGE BYTES".to_vec()),
            OcrWorkerBudget::new(50).unwrap(),
            &CancellationToken::new(),
        )
        .unwrap_err();

    assert_eq!(error.kind(), OcrErrorKind::Timeout);
    assert!(
        started_at.elapsed() < Duration::from_millis(750),
        "timeout returned only after descendant closed inherited pipes"
    );
}

#[cfg(unix)]
#[test]
fn local_command_worker_can_cancel_a_running_process() {
    let command = write_fixture_executable(
        "cancellable-ocr",
        r#"#!/bin/sh
sleep 2
printf 'resume-ir-ocr-v1\nconfidence=0.99\ntext:\nlate output\n'
"#,
    );
    let client = LocalOcrCommandClient::new(
        LocalOcrCommandSpec::new(command, Vec::<String>::new(), "cancellable-engine").unwrap(),
    );
    let cancellation = CancellationToken::new();
    let cancellation_trigger = cancellation.clone();
    let trigger = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        cancellation_trigger.cancel();
    });

    let error = client
        .recognize_page(
            ocr_request(1, b"SYNTHETIC IMAGE BYTES".to_vec()),
            OcrWorkerBudget::new(5_000).unwrap(),
            &cancellation,
        )
        .unwrap_err();
    trigger.join().unwrap();

    assert_eq!(error.kind(), OcrErrorKind::Cancelled);
    assert!(!format!("{error:?}").contains("late output"));
}

#[cfg(unix)]
#[test]
fn local_command_worker_creates_owner_only_input_file() {
    let command = write_fixture_executable(
        "stat-input-mode-ocr",
        r#"#!/bin/sh
mode="$(stat -c %a "$RESUME_IR_OCR_INPUT_PATH" 2>/dev/null || stat -f %Lp "$RESUME_IR_OCR_INPUT_PATH")"
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.91\n'
printf 'text:\n'
printf 'mode=%s\n' "$mode"
"#,
    );
    let client = LocalOcrCommandClient::new(
        LocalOcrCommandSpec::new(command, Vec::<String>::new(), "secure-temp-engine").unwrap(),
    );

    let page = client
        .recognize_page(
            ocr_request(1, b"SYNTHETIC IMAGE BYTES".to_vec()),
            OcrWorkerBudget::new(5_000).unwrap(),
            &CancellationToken::new(),
        )
        .unwrap();

    assert_eq!(page.text(), "mode=600\n");
}

#[cfg(unix)]
#[test]
fn local_command_worker_accepts_crlf_structured_output() {
    let command = write_fixture_executable(
        "crlf-ocr",
        r#"#!/bin/sh
printf 'resume-ir-ocr-v1\r\nconfidence=0.77\r\ntext:\r\ncrlf text\r\n'
"#,
    );
    let client = LocalOcrCommandClient::new(
        LocalOcrCommandSpec::new(command, Vec::<String>::new(), "crlf-engine").unwrap(),
    );

    let page = client
        .recognize_page(
            ocr_request(1, b"SYNTHETIC IMAGE BYTES".to_vec()),
            OcrWorkerBudget::new(5_000).unwrap(),
            &CancellationToken::new(),
        )
        .unwrap();

    assert_eq!(page.confidence(), 0.77);
    assert_eq!(page.text(), "crlf text\n");
}

#[cfg(unix)]
#[test]
fn local_command_worker_rejects_unstructured_success_output() {
    let command = write_fixture_executable(
        "plain-output-ocr",
        r#"#!/bin/sh
printf 'plain text from wrong binary\n'
"#,
    );
    let client = LocalOcrCommandClient::new(
        LocalOcrCommandSpec::new(command, Vec::<String>::new(), "plain-output-engine").unwrap(),
    );

    let error = client
        .recognize_page(
            ocr_request(1, b"SYNTHETIC IMAGE BYTES".to_vec()),
            OcrWorkerBudget::new(5_000).unwrap(),
            &CancellationToken::new(),
        )
        .unwrap_err();

    assert_eq!(error.kind(), OcrErrorKind::EngineFailed);
}

#[cfg(unix)]
#[test]
fn local_command_worker_rejects_out_of_range_confidence() {
    let command = write_fixture_executable(
        "bad-confidence-ocr",
        r#"#!/bin/sh
printf 'resume-ir-ocr-v1\nconfidence=1.5\ntext:\ntext\n'
"#,
    );
    let client = LocalOcrCommandClient::new(
        LocalOcrCommandSpec::new(command, Vec::<String>::new(), "bad-confidence-engine").unwrap(),
    );

    let error = client
        .recognize_page(
            ocr_request(1, b"SYNTHETIC IMAGE BYTES".to_vec()),
            OcrWorkerBudget::new(5_000).unwrap(),
            &CancellationToken::new(),
        )
        .unwrap_err();

    assert_eq!(error.kind(), OcrErrorKind::EngineFailed);
}

#[cfg(unix)]
#[test]
fn local_command_worker_rejects_malformed_structured_output_as_engine_failure() {
    let command = write_fixture_executable(
        "malformed-ocr",
        r#"#!/bin/sh
printf 'resume-ir-ocr-v1\nconfidence=not-a-number\ntext:\ntext\n'
"#,
    );
    let client = LocalOcrCommandClient::new(
        LocalOcrCommandSpec::new(command, Vec::<String>::new(), "malformed-engine").unwrap(),
    );

    let error = client
        .recognize_page(
            ocr_request(1, b"SYNTHETIC IMAGE BYTES".to_vec()),
            OcrWorkerBudget::new(5_000).unwrap(),
            &CancellationToken::new(),
        )
        .unwrap_err();

    assert_eq!(error.kind(), OcrErrorKind::EngineFailed);
}

fn ocr_request(page_no: u32, bytes: Vec<u8>) -> OcrPageRequest {
    OcrPageRequest::new(
        RenderedPage::new(page_no, 300, bytes).unwrap(),
        OcrOptions::new("eng", "balanced").unwrap(),
    )
    .unwrap()
}

#[cfg(unix)]
fn find_command(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|path| path.join(name))
            .find(|path| path.exists())
    })
}

#[cfg(unix)]
fn valid_blank_pdf_bytes() -> Vec<u8> {
    let mut output = Vec::new();
    output.extend_from_slice(b"%PDF-1.4\n");
    let object_1 = output.len();
    output.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    let object_2 = output.len();
    output.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
    let object_3 = output.len();
    output.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] /Resources << >> >>\nendobj\n",
    );
    let xref = output.len();
    output.extend_from_slice(b"xref\n0 4\n");
    output.extend_from_slice(b"0000000000 65535 f \n");
    for offset in [object_1, object_2, object_3] {
        output.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    output.extend_from_slice(
        format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    output
}

#[cfg(unix)]
fn write_fixture_executable(name: &str, body: &str) -> PathBuf {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(name);
    fs::write(&path, body).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

#[cfg(unix)]
fn unique_temp_dir() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "resume-ir-ocr-client-s12-{}-{suffix}-{counter}",
        std::process::id()
    ));
    if Path::new(&path).exists() {
        fs::remove_dir_all(&path).unwrap();
    }
    path
}
