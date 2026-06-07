use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn release_readiness_reports_blocked_evidence_without_local_path_leaks() {
    let data_dir = temp_path("release-readiness-private-data");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "release-readiness"])
        .output()
        .expect("run release readiness gate");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("resume-ir release readiness"));
    assert!(stdout.contains("stable release: blocked"));
    assert!(stdout.contains("signing certificates: blocked"));
    assert!(stdout.contains("production signing certificates"));
    assert!(stdout.contains("certificate chain"));
    assert!(stdout.contains("private key custody"));
    assert!(stdout.contains("signature verification evidence"));
    assert!(stdout.contains("macOS notarization: blocked"));
    assert!(stdout.contains("Apple Developer ID"));
    assert!(stdout.contains("notarization credentials"));
    assert!(stdout.contains("notarization ticket"));
    assert!(stdout.contains("Gatekeeper validation"));
    assert!(stdout.contains("Windows installer lifecycle: blocked"));
    assert!(stdout.contains("MSI install"));
    assert!(stdout.contains("upgrade"));
    assert!(stdout.contains("uninstall"));
    assert!(stdout.contains("rollback"));
    assert!(stdout.contains("release Windows runner"));
    assert!(stdout.contains("Windows service lifecycle: blocked"));
    assert!(stdout.contains("install/start/stop/status/uninstall/recovery"));
    assert!(stdout.contains("release Windows runner"));
    assert!(stdout.contains("macOS installer lifecycle: blocked"));
    assert!(stdout.contains("signed pkg/dmg"));
    assert!(stdout.contains("install/upgrade/uninstall/rollback"));
    assert!(stdout.contains("Gatekeeper validation"));
    assert!(stdout.contains("private real-corpus performance evidence: blocked"));
    assert!(stdout.contains("hot-index hybrid"));
    assert!(stdout.contains("available private corpus"));
    assert!(stdout.contains("500 query samples"));
    assert!(stdout.contains("external 100k/1M scale validation"));
    assert!(!stdout.contains("100k/1M real-corpus benchmarks: blocked"));
    assert!(!stdout.contains("--require-million-scale"));
    assert!(stdout.contains("field extraction quality: blocked"));
    assert!(stdout.contains("min-samples 1000"));
    assert!(stdout.contains("precision/recall/F1 >= 0.93"));
    assert!(stdout.contains("dedupe quality: blocked"));
    assert!(stdout.contains("min-pairs 1000"));
    assert!(stdout.contains("min-positive-pairs 100"));
    assert!(stdout.contains("precision/recall/F1 >= 0.90"));
    assert!(stdout.contains("vector quality: blocked"));
    assert!(stdout.contains("recall@k >= 0.90"));
    assert!(stdout.contains("MRR >= 0.85"));
    assert!(stdout.contains("NDCG@k >= 0.90"));
    assert!(stdout.contains("OCR throughput: blocked"));
    assert!(stdout.contains("min-pages 500"));
    assert!(stdout.contains("OCR p95 <= 1000ms"));
    assert!(stdout.contains("pages_per_second >= 1"));
    assert!(stdout.contains("OCR engine license/distribution: blocked"));
    assert!(stdout.contains("reviewed OCR runtime manifest"));
    assert!(stdout.contains("engine distribution license"));
    assert!(stdout.contains("language-pack distribution license"));
    assert!(stdout.contains("offline packaging evidence"));
    assert!(stdout.contains("embedding model license/distribution: blocked"));
    assert!(stdout.contains("reviewed licensed embedding model"));
    assert!(stdout.contains("model manifest"));
    assert!(stdout.contains("offline distribution"));
    assert!(stdout.contains("license review"));
    assert!(stdout.contains("cross-platform release validation: blocked"));
    assert!(stdout.contains("Windows and macOS release platforms"));
    assert!(stdout.contains("fresh release artifacts"));
    assert!(stdout.contains("install/upgrade/uninstall"));
    assert!(stdout.contains("service lifecycle"));
    assert!(stdout.contains("hardware fault drills: blocked"));
    assert!(stdout.contains("actual ENOSPC"));
    assert!(stdout.contains("service-level daemon kill"));
    assert!(stderr.contains("release readiness blocked"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stdout.contains("PRIVATE"));
}

#[test]
fn release_readiness_json_reports_blockers_without_local_path_leaks() {
    let data_dir = temp_path("release-readiness-json-private-data");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
        ])
        .output()
        .expect("run release readiness json gate");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let report: serde_json::Value =
        serde_json::from_str(&stdout).expect("release readiness json report");

    assert_eq!(report["schema_version"], "release-readiness.v1");
    assert_eq!(report["stable_release"], "blocked");
    assert_eq!(report["local_dry_run_artifacts"], "evidence_only");
    assert_eq!(
        report["next_gate"],
        "keep release blocked until every item has current local evidence"
    );

    let blockers = report["blockers"].as_array().expect("blockers array");
    assert_eq!(blockers.len(), 14);
    let labels = blockers
        .iter()
        .map(|blocker| blocker["label"].as_str().expect("blocker label"))
        .collect::<Vec<_>>();
    assert!(labels.contains(&"signing certificates"));
    assert!(labels.contains(&"macOS notarization"));
    assert!(labels.contains(&"Windows installer lifecycle"));
    assert!(labels.contains(&"Windows service lifecycle"));
    assert!(labels.contains(&"macOS installer lifecycle"));
    assert!(labels.contains(&"private real-corpus performance evidence"));
    assert!(labels.contains(&"field extraction quality"));
    assert!(labels.contains(&"dedupe quality"));
    assert!(labels.contains(&"vector quality"));
    assert!(labels.contains(&"OCR throughput"));
    assert!(labels.contains(&"OCR engine license/distribution"));
    assert!(labels.contains(&"embedding model license/distribution"));
    assert!(labels.contains(&"cross-platform release validation"));
    assert!(labels.contains(&"hardware fault drills"));
    for blocker in blockers {
        assert_eq!(blocker["status"], "blocked");
        assert!(blocker["detail"].as_str().expect("blocker detail").len() > 12);
    }
    let fault_drill_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "hardware fault drills")
        .expect("hardware fault drills blocker");
    let fault_drill_detail = fault_drill_blocker["detail"].as_str().unwrap();
    assert!(fault_drill_detail.contains("actual ENOSPC"));
    assert!(fault_drill_detail.contains("service-level daemon kill"));

    let benchmark_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "private real-corpus performance evidence")
        .expect("benchmark blocker");
    let benchmark_detail = benchmark_blocker["detail"].as_str().unwrap();
    assert!(benchmark_detail.contains("hot-index hybrid"));
    assert!(benchmark_detail.contains("available private corpus"));
    assert!(benchmark_detail.contains("500 query samples"));
    assert!(benchmark_detail.contains("external 100k/1M scale validation"));
    assert!(!benchmark_detail.contains("--require-million-scale"));

    let signing_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "signing certificates")
        .expect("signing blocker");
    let signing_detail = signing_blocker["detail"].as_str().unwrap();
    assert!(signing_detail.contains("production signing certificates"));
    assert!(signing_detail.contains("certificate chain"));
    assert!(signing_detail.contains("private key custody"));
    assert!(signing_detail.contains("signature verification evidence"));

    let notarization_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "macOS notarization")
        .expect("notarization blocker");
    let notarization_detail = notarization_blocker["detail"].as_str().unwrap();
    assert!(notarization_detail.contains("Apple Developer ID"));
    assert!(notarization_detail.contains("notarization credentials"));
    assert!(notarization_detail.contains("notarization ticket"));
    assert!(notarization_detail.contains("Gatekeeper validation"));

    let windows_installer_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "Windows installer lifecycle")
        .expect("Windows installer blocker");
    let windows_installer_detail = windows_installer_blocker["detail"].as_str().unwrap();
    assert!(windows_installer_detail.contains("MSI install"));
    assert!(windows_installer_detail.contains("upgrade"));
    assert!(windows_installer_detail.contains("uninstall"));
    assert!(windows_installer_detail.contains("rollback"));
    assert!(windows_installer_detail.contains("release Windows runner"));

    let windows_service_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "Windows service lifecycle")
        .expect("Windows service blocker");
    let windows_service_detail = windows_service_blocker["detail"].as_str().unwrap();
    assert!(windows_service_detail.contains("install/start/stop/status/uninstall/recovery"));
    assert!(windows_service_detail.contains("release Windows runner"));

    let macos_installer_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "macOS installer lifecycle")
        .expect("macOS installer blocker");
    let macos_installer_detail = macos_installer_blocker["detail"].as_str().unwrap();
    assert!(macos_installer_detail.contains("signed pkg/dmg"));
    assert!(macos_installer_detail.contains("install/upgrade/uninstall/rollback"));
    assert!(macos_installer_detail.contains("Gatekeeper validation"));

    let field_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "field extraction quality")
        .expect("field quality blocker");
    let field_detail = field_blocker["detail"].as_str().unwrap();
    assert!(field_detail.contains("min-samples 1000"));
    assert!(field_detail.contains("precision/recall/F1 >= 0.93"));

    let dedupe_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "dedupe quality")
        .expect("dedupe quality blocker");
    let dedupe_detail = dedupe_blocker["detail"].as_str().unwrap();
    assert!(dedupe_detail.contains("min-pairs 1000"));
    assert!(dedupe_detail.contains("min-positive-pairs 100"));
    assert!(dedupe_detail.contains("precision/recall/F1 >= 0.90"));

    let vector_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "vector quality")
        .expect("vector quality blocker");
    let vector_detail = vector_blocker["detail"].as_str().unwrap();
    assert!(vector_detail.contains("min-samples 1000"));
    assert!(vector_detail.contains("recall@k >= 0.90"));
    assert!(vector_detail.contains("MRR >= 0.85"));
    assert!(vector_detail.contains("NDCG@k >= 0.90"));

    let ocr_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "OCR throughput")
        .expect("OCR throughput blocker");
    let ocr_detail = ocr_blocker["detail"].as_str().unwrap();
    assert!(ocr_detail.contains("min-pages 500"));
    assert!(ocr_detail.contains("OCR p95 <= 1000ms"));
    assert!(ocr_detail.contains("pages_per_second >= 1"));

    let ocr_license_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "OCR engine license/distribution")
        .expect("OCR license blocker");
    let ocr_license_detail = ocr_license_blocker["detail"].as_str().unwrap();
    assert!(ocr_license_detail.contains("reviewed OCR runtime manifest"));
    assert!(ocr_license_detail.contains("engine distribution license"));
    assert!(ocr_license_detail.contains("language-pack distribution license"));
    assert!(ocr_license_detail.contains("offline packaging evidence"));

    let model_license_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "embedding model license/distribution")
        .expect("embedding model license blocker");
    let model_license_detail = model_license_blocker["detail"].as_str().unwrap();
    assert!(model_license_detail.contains("reviewed licensed embedding model"));
    assert!(model_license_detail.contains("model manifest"));
    assert!(model_license_detail.contains("offline distribution"));
    assert!(model_license_detail.contains("license review"));

    let platform_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "cross-platform release validation")
        .expect("cross-platform release validation blocker");
    let platform_detail = platform_blocker["detail"].as_str().unwrap();
    assert!(platform_detail.contains("Windows and macOS release platforms"));
    assert!(platform_detail.contains("fresh release artifacts"));
    assert!(platform_detail.contains("install/upgrade/uninstall"));
    assert!(platform_detail.contains("service lifecycle"));

    assert!(stderr.contains("release readiness blocked"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&data_dir)));
}

fn temp_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("resume-ir-{label}-{unique}"))
}

fn path_str(path: &Path) -> &str {
    path.to_str().expect("test paths are utf-8")
}
