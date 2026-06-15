use std::fs;
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
    assert!(stdout.contains("min-documents 8000"));
    assert!(stdout.contains("500 query samples"));
    assert!(stdout.contains("observed P50/P95/P99 metrics"));
    assert!(stdout.contains("follow-up performance-optimization goal"));
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
    assert!(stdout.contains("observed OCR page latency P50/P95/P99 metrics"));
    assert!(stdout.contains("observed pages_per_second"));
    assert!(stdout.contains("follow-up performance-optimization goal"));
    assert!(stdout.contains("OCR runtime manifest/dependency evidence: blocked"));
    assert!(stdout.contains("reviewed OCR runtime manifest"));
    assert!(stdout.contains("Tesseract/tessdata"));
    assert!(stdout.contains("Apache-2.0"));
    assert!(stdout.contains("Poppler/pdftoppm"));
    assert!(stdout.contains("not bundled by default"));
    assert!(stdout.contains("dependency detection"));
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
    assert!(stdout.contains("redacted diagnostics evidence: blocked"));
    assert!(stdout.contains("export-diagnostics --redact"));
    assert!(stdout.contains("diagnostics.v1"));
    assert!(stdout.contains("local aggregate diagnostics"));
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
    assert_eq!(blockers.len(), 15);
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
    assert!(labels.contains(&"OCR runtime manifest/dependency evidence"));
    assert!(labels.contains(&"embedding model license/distribution"));
    assert!(labels.contains(&"cross-platform release validation"));
    assert!(labels.contains(&"redacted diagnostics evidence"));
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
    assert!(benchmark_detail.contains("min-documents 8000"));
    assert!(benchmark_detail.contains("500 query samples"));
    assert!(benchmark_detail.contains("observed P50/P95/P99 metrics"));
    assert!(benchmark_detail.contains("follow-up performance-optimization goal"));
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
    assert!(ocr_detail.contains("observed OCR page latency P50/P95/P99 metrics"));
    assert!(ocr_detail.contains("observed pages_per_second"));
    assert!(ocr_detail.contains("follow-up performance-optimization goal"));

    let ocr_license_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "OCR runtime manifest/dependency evidence")
        .expect("OCR license blocker");
    let ocr_license_detail = ocr_license_blocker["detail"].as_str().unwrap();
    assert!(ocr_license_detail.contains("reviewed OCR runtime manifest"));
    assert!(ocr_license_detail.contains("Tesseract/tessdata"));
    assert!(ocr_license_detail.contains("Apache-2.0"));
    assert!(ocr_license_detail.contains("Poppler/pdftoppm"));
    assert!(ocr_license_detail.contains("not bundled by default"));
    assert!(ocr_license_detail.contains("dependency detection"));

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

    let diagnostics_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "redacted diagnostics evidence")
        .expect("redacted diagnostics blocker");
    let diagnostics_detail = diagnostics_blocker["detail"].as_str().unwrap();
    assert!(diagnostics_detail.contains("export-diagnostics --redact"));
    assert!(diagnostics_detail.contains("diagnostics.v1"));
    assert!(diagnostics_detail.contains("local aggregate diagnostics"));

    assert!(stderr.contains("release readiness blocked"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&data_dir)));
}

#[test]
fn release_readiness_json_reports_goal_gap_matrix_without_claiming_complete_product() {
    let data_dir = temp_path("release-readiness-goal-gap-private-data");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
        ])
        .output()
        .expect("run release readiness goal gap matrix");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let report: serde_json::Value =
        serde_json::from_str(&stdout).expect("release readiness json report");

    let matrix = &report["goal_gap_matrix"];
    assert_eq!(matrix["schema_version"], "resume-ir.goal-gap-matrix.v1");
    assert_eq!(matrix["complete_product"], false);
    assert_eq!(matrix["current_stage"], "baseline_not_complete");
    assert_eq!(
        matrix["completion_statement"],
        "complete product is not complete while any row is blocked or not_complete"
    );

    let rows = matrix["rows"].as_array().expect("goal matrix rows");
    assert_eq!(rows.len(), 7);
    let ids = rows
        .iter()
        .map(|row| row["id"].as_str().expect("row id"))
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            "P0_foundation",
            "P1_text_import_fulltext",
            "P2_fields_dedupe",
            "P3_semantic_vector",
            "P4_ocr",
            "P5_cross_platform_release",
            "P6_performance_stability",
        ]
    );

    let p0 = rows
        .iter()
        .find(|row| row["id"] == "P0_foundation")
        .expect("P0 row");
    assert_eq!(p0["implementation_status"], "production_complete");
    assert_eq!(p0["release_status"], "covered_by_local_ci");
    assert!(p0["evidence"]
        .as_array()
        .expect("P0 evidence")
        .iter()
        .any(|item| item == "daemon/CLI/metadata/IPC tests"));

    let p2 = rows
        .iter()
        .find(|row| row["id"] == "P2_fields_dedupe")
        .expect("P2 row");
    assert_eq!(p2["implementation_status"], "production_complete");
    assert_eq!(p2["release_status"], "blocked");
    assert!(p2["blocked_by"]
        .as_array()
        .expect("P2 blockers")
        .iter()
        .any(|item| item == "private business labeled field/dedupe quality reports"));

    let p5 = rows
        .iter()
        .find(|row| row["id"] == "P5_cross_platform_release")
        .expect("P5 row");
    assert_eq!(p5["implementation_status"], "blocked");
    assert_eq!(p5["release_status"], "blocked");
    assert!(p5["blocked_by"]
        .as_array()
        .expect("P5 blockers")
        .iter()
        .any(|item| item == "real signing/notarization credentials"));

    let p6 = rows
        .iter()
        .find(|row| row["id"] == "P6_performance_stability")
        .expect("P6 row");
    assert_eq!(p6["implementation_status"], "not_complete");
    assert_eq!(p6["release_status"], "blocked");
    assert!(p6["blocked_by"]
        .as_array()
        .expect("P6 blockers")
        .iter()
        .any(|item| item == "full current-stage local baseline evidence"));

    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stdout.contains("PRIVATE"));
}

#[test]
fn release_readiness_json_accepts_redacted_diagnostics_report_without_path_leaks() {
    let data_dir = temp_path("release-readiness-diagnostics-private-data");
    let evidence_dir = temp_path("release-readiness-diagnostics-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let diagnostics_report = evidence_dir.join("redacted-diagnostics.json");
    fs::write(&diagnostics_report, redacted_diagnostics_report()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--diagnostics-report",
            path_str(&diagnostics_report),
        ])
        .output()
        .expect("run release readiness with redacted diagnostics report");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let report: serde_json::Value =
        serde_json::from_str(&stdout).expect("release readiness diagnostics json report");
    let provided = report["provided_evidence"]
        .as_array()
        .expect("provided evidence array");
    let diagnostics = provided
        .iter()
        .find(|evidence| evidence["label"] == "redacted diagnostics evidence")
        .expect("redacted diagnostics evidence");
    assert_eq!(diagnostics["status"], "provided");
    assert_eq!(diagnostics["privacy_boundary"], "redacted_local_aggregate");
    assert!(diagnostics["detail"]
        .as_str()
        .unwrap()
        .contains("diagnostics.v1"));

    let blockers = report["blockers"].as_array().expect("blockers array");
    let blocker_labels = blockers
        .iter()
        .map(|blocker| blocker["label"].as_str().expect("blocker label"))
        .collect::<Vec<_>>();
    assert!(!blocker_labels.contains(&"redacted diagnostics evidence"));
    assert!(blocker_labels.contains(&"signing certificates"));
    assert!(blocker_labels.contains(&"private real-corpus performance evidence"));
    assert!(stderr.contains("release readiness blocked"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stdout.contains("PRIVATE"));
    assert!(!stdout.contains("raw resume"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_json_accepts_blocked_release_automation_evidence_without_clearing_blockers() {
    let data_dir = temp_path("release-readiness-release-evidence-private-data");
    let evidence_dir = temp_path("release-readiness-release-evidence-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let signing_evidence = evidence_dir.join("signing-evidence.json");
    let notarization_evidence = evidence_dir.join("notarization-evidence.json");
    let macos_installer_evidence = evidence_dir.join("macos-installer-evidence.json");
    let windows_installer_evidence = evidence_dir.join("windows-installer-evidence.json");
    let windows_service_evidence = evidence_dir.join("windows-service-evidence.json");
    fs::write(&signing_evidence, blocked_signing_evidence()).unwrap();
    fs::write(&notarization_evidence, blocked_notarization_evidence()).unwrap();
    fs::write(
        &macos_installer_evidence,
        blocked_macos_installer_evidence(),
    )
    .unwrap();
    fs::write(
        &windows_installer_evidence,
        blocked_windows_installer_evidence(),
    )
    .unwrap();
    fs::write(
        &windows_service_evidence,
        blocked_windows_service_evidence(),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--signing-evidence",
            path_str(&signing_evidence),
            "--notarization-evidence",
            path_str(&notarization_evidence),
            "--macos-installer-evidence",
            path_str(&macos_installer_evidence),
            "--windows-installer-evidence",
            path_str(&windows_installer_evidence),
            "--windows-service-evidence",
            path_str(&windows_service_evidence),
        ])
        .output()
        .expect("run release readiness with blocked release automation evidence");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let report: serde_json::Value =
        serde_json::from_str(&stdout).expect("release readiness automation evidence json report");
    let provided = report["provided_evidence"]
        .as_array()
        .expect("provided evidence array");
    let provided_labels = provided
        .iter()
        .map(|evidence| evidence["label"].as_str().expect("provided label"))
        .collect::<Vec<_>>();
    assert!(provided_labels.contains(&"signing automation evidence"));
    assert!(provided_labels.contains(&"notarization automation evidence"));
    assert!(provided_labels.contains(&"macOS installer automation evidence"));
    assert!(provided_labels.contains(&"Windows installer automation evidence"));
    assert!(provided_labels.contains(&"Windows service automation evidence"));
    for evidence in provided {
        assert_eq!(evidence["status"], "provided");
        assert_eq!(
            evidence["privacy_boundary"],
            "blocked_release_evidence_manifest"
        );
        assert!(evidence["detail"]
            .as_str()
            .expect("provided detail")
            .contains("blocked dry-run evidence"));
    }

    let blockers = report["blockers"].as_array().expect("blockers array");
    let blocker_labels = blockers
        .iter()
        .map(|blocker| blocker["label"].as_str().expect("blocker label"))
        .collect::<Vec<_>>();
    assert!(blocker_labels.contains(&"signing certificates"));
    assert!(blocker_labels.contains(&"macOS notarization"));
    assert!(blocker_labels.contains(&"macOS installer lifecycle"));
    assert!(blocker_labels.contains(&"Windows installer lifecycle"));
    assert!(blocker_labels.contains(&"Windows service lifecycle"));
    assert!(blocker_labels.contains(&"cross-platform release validation"));
    assert!(blocker_labels.contains(&"redacted diagnostics evidence"));
    assert!(!blocker_labels.contains(&"signing automation evidence"));
    assert!(!blocker_labels.contains(&"notarization automation evidence"));
    assert!(stderr.contains("release readiness blocked"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stdout.contains("PRIVATE"));
    assert!(!stdout.contains("resume-ir-v0.0.0"));
    assert!(!stderr.contains("resume-ir-v0.0.0"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_json_accepts_windows_service_lifecycle_plan_without_clearing_blockers() {
    let data_dir = temp_path("release-readiness-service-lifecycle-plan-private-data");
    let evidence_dir = temp_path("release-readiness-service-lifecycle-plan-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let lifecycle_plan = evidence_dir.join("windows-service-lifecycle-dry-run.json");
    fs::write(&lifecycle_plan, blocked_windows_service_lifecycle_plan()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--windows-service-lifecycle-plan",
            path_str(&lifecycle_plan),
        ])
        .output()
        .expect("run release readiness with Windows service lifecycle plan evidence");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let report: serde_json::Value = serde_json::from_str(&stdout)
        .expect("release readiness Windows service lifecycle plan json report");
    let provided = report["provided_evidence"]
        .as_array()
        .expect("provided evidence array");
    let service_lifecycle_plan = provided
        .iter()
        .find(|evidence| evidence["label"] == "Windows service lifecycle plan evidence")
        .expect("Windows service lifecycle plan evidence");
    assert_eq!(service_lifecycle_plan["status"], "provided");
    assert_eq!(
        service_lifecycle_plan["privacy_boundary"],
        "blocked_release_evidence_manifest"
    );
    assert!(service_lifecycle_plan["detail"]
        .as_str()
        .expect("provided detail")
        .contains("release.windows_service_lifecycle_plan.v1"));

    let blockers = report["blockers"].as_array().expect("blockers array");
    let blocker_labels = blockers
        .iter()
        .map(|blocker| blocker["label"].as_str().expect("blocker label"))
        .collect::<Vec<_>>();
    assert!(blocker_labels.contains(&"Windows service lifecycle"));
    assert!(blocker_labels.contains(&"cross-platform release validation"));
    assert!(blocker_labels.contains(&"signing certificates"));
    assert!(!blocker_labels.contains(&"Windows service lifecycle plan evidence"));
    assert!(stderr.contains("release readiness blocked"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stdout.contains("PRIVATE"));
    assert!(!stdout.contains("resume-ir-v0.0.0"));
    assert!(!stderr.contains("resume-ir-v0.0.0"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_json_accepts_installer_lifecycle_plans_without_clearing_blockers() {
    let data_dir = temp_path("release-readiness-installer-lifecycle-plan-private-data");
    let evidence_dir = temp_path("release-readiness-installer-lifecycle-plan-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let macos_lifecycle_plan = evidence_dir.join("macos-installer-lifecycle-dry-run.json");
    let windows_lifecycle_plan = evidence_dir.join("windows-installer-lifecycle-dry-run.json");
    fs::write(
        &macos_lifecycle_plan,
        blocked_macos_installer_lifecycle_plan(),
    )
    .unwrap();
    fs::write(
        &windows_lifecycle_plan,
        blocked_windows_installer_lifecycle_plan(),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--macos-installer-lifecycle-plan",
            path_str(&macos_lifecycle_plan),
            "--windows-installer-lifecycle-plan",
            path_str(&windows_lifecycle_plan),
        ])
        .output()
        .expect("run release readiness with installer lifecycle plan evidence");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let report: serde_json::Value = serde_json::from_str(&stdout)
        .expect("release readiness installer lifecycle plan json report");
    let provided = report["provided_evidence"]
        .as_array()
        .expect("provided evidence array");
    let provided_labels = provided
        .iter()
        .map(|evidence| evidence["label"].as_str().expect("provided label"))
        .collect::<Vec<_>>();
    assert!(provided_labels.contains(&"macOS installer lifecycle plan evidence"));
    assert!(provided_labels.contains(&"Windows installer lifecycle plan evidence"));
    for label in [
        "macOS installer lifecycle plan evidence",
        "Windows installer lifecycle plan evidence",
    ] {
        let evidence = provided
            .iter()
            .find(|evidence| evidence["label"] == label)
            .expect("installer lifecycle plan evidence");
        assert_eq!(evidence["status"], "provided");
        assert_eq!(
            evidence["privacy_boundary"],
            "blocked_release_evidence_manifest"
        );
        assert!(evidence["detail"]
            .as_str()
            .expect("provided detail")
            .contains("installer_lifecycle_plan.v1"));
    }

    let blockers = report["blockers"].as_array().expect("blockers array");
    let blocker_labels = blockers
        .iter()
        .map(|blocker| blocker["label"].as_str().expect("blocker label"))
        .collect::<Vec<_>>();
    assert!(blocker_labels.contains(&"macOS installer lifecycle"));
    assert!(blocker_labels.contains(&"Windows installer lifecycle"));
    assert!(blocker_labels.contains(&"cross-platform release validation"));
    assert!(blocker_labels.contains(&"signing certificates"));
    assert!(!blocker_labels.contains(&"macOS installer lifecycle plan evidence"));
    assert!(!blocker_labels.contains(&"Windows installer lifecycle plan evidence"));
    assert!(stderr.contains("release readiness blocked"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stdout.contains("PRIVATE"));
    assert!(!stdout.contains("resume-ir-v0.0.0"));
    assert!(!stderr.contains("resume-ir-v0.0.0"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_json_accepts_release_artifact_and_sbom_evidence_without_clearing_blockers() {
    let data_dir = temp_path("release-readiness-release-manifest-private-data");
    let evidence_dir = temp_path("release-readiness-release-manifest-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let release_artifacts = evidence_dir.join("release-artifacts.json");
    let release_sbom = evidence_dir.join("release-sbom.json");
    fs::write(&release_artifacts, release_artifacts_manifest()).unwrap();
    fs::write(&release_sbom, release_sbom_manifest()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--release-artifact-manifest",
            path_str(&release_artifacts),
            "--release-sbom",
            path_str(&release_sbom),
        ])
        .output()
        .expect("run release readiness with release artifact and SBOM evidence");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let report: serde_json::Value = serde_json::from_str(&stdout)
        .expect("release readiness artifact/SBOM evidence json report");
    let provided = report["provided_evidence"]
        .as_array()
        .expect("provided evidence array");
    let provided_labels = provided
        .iter()
        .map(|evidence| evidence["label"].as_str().expect("provided label"))
        .collect::<Vec<_>>();
    assert!(provided_labels.contains(&"release artifact manifest evidence"));
    assert!(provided_labels.contains(&"release SBOM evidence"));
    for evidence in provided {
        assert_eq!(evidence["status"], "provided");
        assert_eq!(
            evidence["privacy_boundary"],
            "blocked_release_evidence_manifest"
        );
        assert!(evidence["detail"]
            .as_str()
            .expect("provided detail")
            .contains("dry-run"));
    }

    let blockers = report["blockers"].as_array().expect("blockers array");
    let blocker_labels = blockers
        .iter()
        .map(|blocker| blocker["label"].as_str().expect("blocker label"))
        .collect::<Vec<_>>();
    assert!(blocker_labels.contains(&"signing certificates"));
    assert!(blocker_labels.contains(&"macOS notarization"));
    assert!(blocker_labels.contains(&"macOS installer lifecycle"));
    assert!(blocker_labels.contains(&"Windows installer lifecycle"));
    assert!(blocker_labels.contains(&"Windows service lifecycle"));
    assert!(blocker_labels.contains(&"cross-platform release validation"));
    assert!(blocker_labels.contains(&"redacted diagnostics evidence"));
    assert!(!blocker_labels.contains(&"release artifact manifest evidence"));
    assert!(!blocker_labels.contains(&"release SBOM evidence"));
    assert!(stderr.contains("release readiness blocked"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stdout.contains("PRIVATE"));
    assert!(!stdout.contains("resume-ir-v0.0.0"));
    assert!(!stderr.contains("resume-ir-v0.0.0"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_json_accepts_platform_package_manifest_evidence_without_clearing_blockers() {
    let data_dir = temp_path("release-readiness-package-manifest-private-data");
    let evidence_dir = temp_path("release-readiness-package-manifest-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let macos_package = evidence_dir.join("macos-package.json");
    let windows_package = evidence_dir.join("windows-package.json");
    fs::write(&macos_package, macos_package_manifest()).unwrap();
    fs::write(&windows_package, windows_package_manifest()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--macos-package-manifest",
            path_str(&macos_package),
            "--windows-package-manifest",
            path_str(&windows_package),
        ])
        .output()
        .expect("run release readiness with platform package manifest evidence");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let report: serde_json::Value = serde_json::from_str(&stdout)
        .expect("release readiness platform package evidence json report");
    let provided = report["provided_evidence"]
        .as_array()
        .expect("provided evidence array");
    let provided_labels = provided
        .iter()
        .map(|evidence| evidence["label"].as_str().expect("provided label"))
        .collect::<Vec<_>>();
    assert!(provided_labels.contains(&"macOS package manifest evidence"));
    assert!(provided_labels.contains(&"Windows package manifest evidence"));
    for evidence in provided {
        assert_eq!(evidence["status"], "provided");
        assert_eq!(
            evidence["privacy_boundary"],
            "blocked_release_evidence_manifest"
        );
        assert!(evidence["detail"]
            .as_str()
            .expect("provided detail")
            .contains("unsigned dry-run"));
    }

    let blockers = report["blockers"].as_array().expect("blockers array");
    let blocker_labels = blockers
        .iter()
        .map(|blocker| blocker["label"].as_str().expect("blocker label"))
        .collect::<Vec<_>>();
    assert!(blocker_labels.contains(&"signing certificates"));
    assert!(blocker_labels.contains(&"macOS notarization"));
    assert!(blocker_labels.contains(&"macOS installer lifecycle"));
    assert!(blocker_labels.contains(&"Windows installer lifecycle"));
    assert!(blocker_labels.contains(&"Windows service lifecycle"));
    assert!(blocker_labels.contains(&"cross-platform release validation"));
    assert!(!blocker_labels.contains(&"macOS package manifest evidence"));
    assert!(!blocker_labels.contains(&"Windows package manifest evidence"));
    assert!(stderr.contains("release readiness blocked"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stdout.contains("PRIVATE"));
    assert!(!stdout.contains("resume-ir-v0.0.0"));
    assert!(!stderr.contains("resume-ir-v0.0.0"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_json_accepts_local_evidence_reports_but_keeps_external_blockers() {
    let data_dir = temp_path("release-readiness-evidence-private-data");
    let evidence_dir = temp_path("release-readiness-evidence-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let benchmark_report = evidence_dir.join("private-benchmark.json");
    let field_report = evidence_dir.join("private-field-quality.json");
    let dedupe_report = evidence_dir.join("private-dedupe-quality.json");
    let vector_report = evidence_dir.join("private-vector-quality.json");
    let ocr_report = evidence_dir.join("private-ocr-throughput.json");
    let model_artifact = evidence_dir.join("reviewed-model-artifact.bin");
    let model_manifest = evidence_dir.join("reviewed-model-manifest.json");
    let ocr_engine_artifact = evidence_dir.join("reviewed-ocr-engine.bin");
    let ocr_renderer_artifact = evidence_dir.join("reviewed-ocr-renderer.bin");
    let ocr_manifest = evidence_dir.join("reviewed-ocr-manifest.json");
    fs::write(&benchmark_report, private_real_benchmark_report()).unwrap();
    fs::write(&field_report, private_business_field_quality_report()).unwrap();
    fs::write(&dedupe_report, private_business_dedupe_quality_report()).unwrap();
    fs::write(&vector_report, private_business_vector_quality_report()).unwrap();
    fs::write(&ocr_report, private_real_ocr_throughput_report()).unwrap();
    write_reviewed_model_manifest(&model_artifact, &model_manifest);
    write_reviewed_ocr_manifest(&ocr_engine_artifact, &ocr_renderer_artifact, &ocr_manifest);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--benchmark-report",
            path_str(&benchmark_report),
            "--field-quality-report",
            path_str(&field_report),
            "--dedupe-quality-report",
            path_str(&dedupe_report),
            "--vector-quality-report",
            path_str(&vector_report),
            "--ocr-throughput-report",
            path_str(&ocr_report),
            "--model-manifest",
            path_str(&model_manifest),
            "--ocr-runtime-manifest",
            path_str(&ocr_manifest),
        ])
        .output()
        .expect("run release readiness with local evidence reports");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let report: serde_json::Value =
        serde_json::from_str(&stdout).expect("release readiness evidence json report");
    assert_eq!(report["schema_version"], "release-readiness.v1");
    assert_eq!(report["stable_release"], "blocked");
    let provided = report["provided_evidence"]
        .as_array()
        .expect("provided evidence array");
    let provided_labels = provided
        .iter()
        .map(|evidence| evidence["label"].as_str().expect("provided label"))
        .collect::<Vec<_>>();
    assert!(provided_labels.contains(&"private real-corpus performance evidence"));
    assert!(provided_labels.contains(&"field extraction quality"));
    assert!(provided_labels.contains(&"dedupe quality"));
    assert!(provided_labels.contains(&"vector quality"));
    assert!(provided_labels.contains(&"OCR throughput"));
    assert!(provided_labels.contains(&"embedding model license/distribution"));
    assert!(provided_labels.contains(&"OCR runtime manifest/dependency evidence"));
    for evidence in provided {
        assert_eq!(evidence["status"], "provided");
        let label = evidence["label"].as_str().expect("provided label");
        let expected_boundary = match label {
            "embedding model license/distribution" | "OCR runtime manifest/dependency evidence" => {
                "reviewed_local_manifest"
            }
            _ => "redacted_local_aggregate",
        };
        assert_eq!(evidence["privacy_boundary"], expected_boundary);
    }

    let blockers = report["blockers"].as_array().expect("blockers array");
    let blocker_labels = blockers
        .iter()
        .map(|blocker| blocker["label"].as_str().expect("blocker label"))
        .collect::<Vec<_>>();
    assert!(!blocker_labels.contains(&"private real-corpus performance evidence"));
    assert!(!blocker_labels.contains(&"field extraction quality"));
    assert!(!blocker_labels.contains(&"dedupe quality"));
    assert!(!blocker_labels.contains(&"vector quality"));
    assert!(!blocker_labels.contains(&"OCR throughput"));
    assert!(!blocker_labels.contains(&"embedding model license/distribution"));
    assert!(!blocker_labels.contains(&"OCR runtime manifest/dependency evidence"));
    assert!(blocker_labels.contains(&"redacted diagnostics evidence"));
    assert!(blocker_labels.contains(&"signing certificates"));
    assert!(blocker_labels.contains(&"macOS notarization"));
    assert!(blocker_labels.contains(&"cross-platform release validation"));
    assert!(stderr.contains("release readiness blocked"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stdout.contains(path_str(&model_artifact)));
    assert!(!stdout.contains(path_str(&ocr_engine_artifact)));
    assert!(!stdout.contains("SYNTHETIC REVIEWED"));
    assert!(!stdout.contains("PRIVATE"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_json_accepts_current_stage_evidence_without_clearing_blockers() {
    let data_dir = temp_path("release-readiness-current-stage-private-data");
    let evidence_dir = temp_path("release-readiness-current-stage-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let current_stage_evidence = evidence_dir.join("current-stage-validation-evidence.json");
    fs::write(&current_stage_evidence, current_stage_evidence_manifest()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--current-stage-evidence",
            path_str(&current_stage_evidence),
        ])
        .output()
        .expect("run release readiness with current-stage evidence manifest");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let report: serde_json::Value =
        serde_json::from_str(&stdout).expect("release readiness current-stage json report");
    let provided = report["provided_evidence"]
        .as_array()
        .expect("provided evidence array");
    let current_stage = provided
        .iter()
        .find(|evidence| evidence["label"] == "current-stage validation evidence manifest")
        .expect("current-stage evidence manifest");
    assert_eq!(current_stage["status"], "provided");
    assert_eq!(
        current_stage["privacy_boundary"],
        "local_only_redacted_evidence_manifest"
    );
    assert!(current_stage["detail"]
        .as_str()
        .unwrap()
        .contains("current-stage validation evidence manifest"));

    let blockers = report["blockers"].as_array().expect("blockers array");
    let blocker_labels = blockers
        .iter()
        .map(|blocker| blocker["label"].as_str().expect("blocker label"))
        .collect::<Vec<_>>();
    assert!(blocker_labels.contains(&"private real-corpus performance evidence"));
    assert!(blocker_labels.contains(&"embedding model license/distribution"));
    assert!(blocker_labels.contains(&"OCR runtime manifest/dependency evidence"));
    assert!(blocker_labels.contains(&"redacted diagnostics evidence"));
    assert!(blocker_labels.contains(&"signing certificates"));
    assert!(blocker_labels.contains(&"macOS notarization"));
    assert!(blocker_labels.contains(&"cross-platform release validation"));
    assert!(!blocker_labels.contains(&"current-stage validation evidence manifest"));
    assert!(stderr.contains("release readiness blocked"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stdout.contains("PRIVATE"));
    assert!(!stdout.contains("private fake query"));
    assert!(!stdout.contains("/Users/"));
    assert!(!stderr.contains("/Users/"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_rejects_current_stage_evidence_missing_local_flow_output_without_path_leaks() {
    let data_dir = temp_path("release-readiness-current-stage-missing-output-private-data");
    let evidence_dir = temp_path("release-readiness-current-stage-missing-output-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let current_stage_evidence = evidence_dir.join("current-stage-validation-evidence.json");
    fs::write(
        &current_stage_evidence,
        current_stage_evidence_manifest_missing_dataset_output(),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--current-stage-evidence",
            path_str(&current_stage_evidence),
        ])
        .output()
        .expect("reject current-stage evidence missing local flow output");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("release readiness evidence failed validation"));
    assert!(stderr.contains("current-stage validation evidence manifest"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&current_stage_evidence)));
    assert!(!stderr.contains("PRIVATE"));
    assert!(!stderr.contains("/Users/"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_rejects_current_stage_evidence_below_local_baseline_floor_without_path_leaks()
{
    let data_dir = temp_path("release-readiness-current-stage-floor-private-data");
    let evidence_dir = temp_path("release-readiness-current-stage-floor-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let current_stage_evidence = evidence_dir.join("current-stage-validation-evidence.json");
    fs::write(
        &current_stage_evidence,
        current_stage_evidence_manifest()
            .replace("\"max_files\":10000", "\"max_files\":7999")
            .replace("\"max_queries\":500", "\"max_queries\":499"),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--current-stage-evidence",
            path_str(&current_stage_evidence),
        ])
        .output()
        .expect("reject undersized current-stage evidence");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("release readiness evidence failed validation"));
    assert!(stderr.contains("current-stage validation evidence manifest"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&current_stage_evidence)));
    assert!(!stderr.contains("PRIVATE"));
    assert!(!stderr.contains("/Users/"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_rejects_current_stage_evidence_with_mismatched_dataset_digest_without_path_leaks(
) {
    let data_dir = temp_path("release-readiness-current-stage-digest-private-data");
    let evidence_dir = temp_path("release-readiness-current-stage-digest-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let current_stage_evidence = evidence_dir.join("current-stage-validation-evidence.json");
    fs::write(
        &current_stage_evidence,
        current_stage_evidence_manifest().replace(
            "\"file\":\"dataset-manifest.local.json\",\"sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"",
            "\"file\":\"dataset-manifest.local.json\",\"sha256\":\"9999999999999999999999999999999999999999999999999999999999999999\"",
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--current-stage-evidence",
            path_str(&current_stage_evidence),
        ])
        .output()
        .expect("reject digest-mismatched current-stage evidence");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("release readiness evidence failed validation"));
    assert!(stderr.contains("current-stage validation evidence manifest"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&current_stage_evidence)));
    assert!(!stderr.contains("PRIVATE"));
    assert!(!stderr.contains("/Users/"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_rejects_current_stage_evidence_missing_runtime_manifest_outputs_without_path_leaks(
) {
    let data_dir =
        temp_path("release-readiness-current-stage-runtime-manifest-missing-private-data");
    let evidence_dir =
        temp_path("release-readiness-current-stage-runtime-manifest-missing-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let current_stage_evidence = evidence_dir.join("current-stage-validation-evidence.json");
    fs::write(
        &current_stage_evidence,
        current_stage_evidence_manifest_missing_runtime_manifest_outputs(),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--current-stage-evidence",
            path_str(&current_stage_evidence),
        ])
        .output()
        .expect("reject current-stage evidence missing runtime manifest outputs");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("release readiness evidence failed validation"));
    assert!(stderr.contains("current-stage validation evidence manifest"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&current_stage_evidence)));
    assert!(!stderr.contains("PRIVATE"));
    assert!(!stderr.contains("/Users/"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_rejects_current_stage_evidence_with_mismatched_runtime_manifest_digests_without_path_leaks(
) {
    let data_dir =
        temp_path("release-readiness-current-stage-runtime-manifest-digest-private-data");
    let evidence_dir =
        temp_path("release-readiness-current-stage-runtime-manifest-digest-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let current_stage_evidence = evidence_dir.join("current-stage-validation-evidence.json");
    fs::write(
        &current_stage_evidence,
        current_stage_evidence_manifest()
            .replace(
                "\"file\":\"model-manifest.local.json\",\"sha256\":\"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\"",
                "\"file\":\"model-manifest.local.json\",\"sha256\":\"9999999999999999999999999999999999999999999999999999999999999999\"",
            )
            .replace(
                "\"file\":\"ocr-runtime-manifest.local.json\",\"sha256\":\"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\"",
                "\"file\":\"ocr-runtime-manifest.local.json\",\"sha256\":\"8888888888888888888888888888888888888888888888888888888888888888\"",
            ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--current-stage-evidence",
            path_str(&current_stage_evidence),
        ])
        .output()
        .expect("reject digest-mismatched runtime manifest evidence");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("release readiness evidence failed validation"));
    assert!(stderr.contains("current-stage validation evidence manifest"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&current_stage_evidence)));
    assert!(!stderr.contains("PRIVATE"));
    assert!(!stderr.contains("/Users/"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_rejects_current_stage_evidence_missing_preflight_probe_status_without_path_leaks(
) {
    let data_dir =
        temp_path("release-readiness-current-stage-preflight-probe-missing-private-data");
    let evidence_dir =
        temp_path("release-readiness-current-stage-preflight-probe-missing-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let current_stage_evidence = evidence_dir.join("current-stage-validation-evidence.json");
    fs::write(
        &current_stage_evidence,
        current_stage_evidence_manifest_missing_preflight_probes(),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--current-stage-evidence",
            path_str(&current_stage_evidence),
        ])
        .output()
        .expect("reject current-stage evidence missing preflight probe status");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("release readiness evidence failed validation"));
    assert!(stderr.contains("current-stage validation evidence manifest"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&current_stage_evidence)));
    assert!(!stderr.contains("PRIVATE"));
    assert!(!stderr.contains("/Users/"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_rejects_current_stage_evidence_with_failed_preflight_probe_without_path_leaks()
{
    let data_dir = temp_path("release-readiness-current-stage-preflight-probe-failed-private-data");
    let evidence_dir =
        temp_path("release-readiness-current-stage-preflight-probe-failed-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let current_stage_evidence = evidence_dir.join("current-stage-validation-evidence.json");
    fs::write(
        &current_stage_evidence,
        current_stage_evidence_manifest().replace(
            "\"ocr_runtime_probe\":\"passed\"",
            "\"ocr_runtime_probe\":\"failed\"",
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--current-stage-evidence",
            path_str(&current_stage_evidence),
        ])
        .output()
        .expect("reject current-stage evidence with failed preflight probe");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("release readiness evidence failed validation"));
    assert!(stderr.contains("current-stage validation evidence manifest"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&current_stage_evidence)));
    assert!(!stderr.contains("PRIVATE"));
    assert!(!stderr.contains("/Users/"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_rejects_current_stage_evidence_with_duplicate_step_without_path_leaks() {
    let data_dir = temp_path("release-readiness-current-stage-duplicate-step-private-data");
    let evidence_dir = temp_path("release-readiness-current-stage-duplicate-step-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let current_stage_evidence = evidence_dir.join("current-stage-validation-evidence.json");
    fs::write(
        &current_stage_evidence,
        current_stage_evidence_manifest().replace(
            "{\"id\":\"dataset_manifest\",\"status\":\"success\"}",
            "{\"id\":\"dataset_manifest\",\"status\":\"failed\"},{\"id\":\"dataset_manifest\",\"status\":\"success\"}",
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--current-stage-evidence",
            path_str(&current_stage_evidence),
        ])
        .output()
        .expect("reject duplicate current-stage evidence step");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("release readiness evidence failed validation"));
    assert!(stderr.contains("current-stage validation evidence manifest"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&current_stage_evidence)));
    assert!(!stderr.contains("PRIVATE"));
    assert!(!stderr.contains("/Users/"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_rejects_current_stage_evidence_with_unknown_step_without_path_leaks() {
    let data_dir = temp_path("release-readiness-current-stage-unknown-step-private-data");
    let evidence_dir = temp_path("release-readiness-current-stage-unknown-step-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let current_stage_evidence = evidence_dir.join("current-stage-validation-evidence.json");
    fs::write(
        &current_stage_evidence,
        current_stage_evidence_manifest().replace(
            "\"steps\":[",
            "\"steps\":[{\"id\":\"unexpected_private_upload\",\"status\":\"success\"},",
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--current-stage-evidence",
            path_str(&current_stage_evidence),
        ])
        .output()
        .expect("reject unknown current-stage evidence step");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("release readiness evidence failed validation"));
    assert!(stderr.contains("current-stage validation evidence manifest"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&current_stage_evidence)));
    assert!(!stderr.contains("PRIVATE"));
    assert!(!stderr.contains("/Users/"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_rejects_current_stage_evidence_with_unknown_redacted_output_without_path_leaks(
) {
    let data_dir = temp_path("release-readiness-current-stage-unknown-output-private-data");
    let evidence_dir = temp_path("release-readiness-current-stage-unknown-output-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let current_stage_evidence = evidence_dir.join("current-stage-validation-evidence.json");
    fs::write(
        &current_stage_evidence,
        current_stage_evidence_manifest().replace(
            "\"redacted_outputs\":[",
            "\"redacted_outputs\":[{\"file\":\"unexpected-private-report.txt\",\"sha256\":\"9999999999999999999999999999999999999999999999999999999999999999\"},",
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--current-stage-evidence",
            path_str(&current_stage_evidence),
        ])
        .output()
        .expect("reject unknown current-stage redacted output");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("release readiness evidence failed validation"));
    assert!(stderr.contains("current-stage validation evidence manifest"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&current_stage_evidence)));
    assert!(!stderr.contains("PRIVATE"));
    assert!(!stderr.contains("/Users/"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_rejects_current_stage_evidence_with_private_marker_without_path_leaks() {
    let data_dir = temp_path("release-readiness-current-stage-marker-private-data");
    let evidence_dir = temp_path("release-readiness-current-stage-marker-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let current_stage_evidence = evidence_dir.join("current-stage-validation-evidence.json");
    fs::write(
        &current_stage_evidence,
        current_stage_evidence_manifest().replace(
            "\"dataset_manifest_sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"",
            "\"dataset_manifest_sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"private_path\":\"/Users/frank/private-resumes\"",
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--current-stage-evidence",
            path_str(&current_stage_evidence),
        ])
        .output()
        .expect("reject current-stage evidence with private marker");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("release readiness evidence failed validation"));
    assert!(stderr.contains("current-stage validation evidence manifest"));
    assert!(stderr.contains("private marker is present"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&current_stage_evidence)));
    assert!(!stderr.contains("/Users/frank"));
    assert!(!stderr.contains("private-resumes"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_rejects_unreviewed_model_manifest_without_path_leaks() {
    let data_dir = temp_path("release-readiness-unreviewed-model-private-data");
    let evidence_dir = temp_path("release-readiness-unreviewed-model-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let model_artifact = evidence_dir.join("unreviewed-model-artifact.bin");
    let model_manifest = evidence_dir.join("unreviewed-model-manifest.json");
    write_unreviewed_model_manifest(&model_artifact, &model_manifest);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--model-manifest",
            path_str(&model_manifest),
        ])
        .output()
        .expect("run release readiness with unreviewed model manifest");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("model manifest blocked: license has not been reviewed"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&model_artifact)));
    assert!(!stderr.contains(path_str(&model_manifest)));
    assert!(!stderr.contains("SYNTHETIC UNREVIEWED MODEL ARTIFACT"));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_rejects_benchmark_evidence_below_local_document_floor_without_path_leaks() {
    let data_dir = temp_path("release-readiness-benchmark-floor-private-data");
    let evidence_dir = temp_path("release-readiness-benchmark-floor-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let benchmark_report = evidence_dir.join("private-benchmark-below-floor.json");
    fs::write(
        &benchmark_report,
        private_real_benchmark_report()
            .replace("\"document_count\":8720,", "\"document_count\":7999,")
            .replace(
                "\"searchable_document_count\":8720,",
                "\"searchable_document_count\":7999,",
            )
            .replace(
                "\"vector_indexed_document_count\":8720,",
                "\"vector_indexed_document_count\":7999,",
            ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--benchmark-report",
            path_str(&benchmark_report),
        ])
        .output()
        .expect("reject benchmark evidence below local document floor");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.is_empty());
    assert!(stderr.contains("release readiness evidence failed validation"));
    assert!(stderr.contains("private real-corpus performance evidence"));
    assert!(stderr.contains("document count below gate minimum"));
    assert!(!stderr.contains(path_str(&benchmark_report)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&benchmark_report)));
    assert!(!stdout.contains(path_str(&evidence_dir)));
    assert!(!stdout.contains(path_str(&data_dir)));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
}

#[test]
fn release_readiness_rejects_benchmark_evidence_below_hot_index_coverage_floor_without_path_leaks()
{
    let data_dir = temp_path("release-readiness-benchmark-coverage-private-data");
    let evidence_dir = temp_path("release-readiness-benchmark-coverage-private-reports");
    fs::create_dir_all(&evidence_dir).unwrap();
    let benchmark_report = evidence_dir.join("private-benchmark-below-coverage.json");
    fs::write(
        &benchmark_report,
        private_real_benchmark_report().replace(
            "\"searchable_document_count\":8720,",
            "\"searchable_document_count\":146,",
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
            "--benchmark-report",
            path_str(&benchmark_report),
        ])
        .output()
        .expect("reject benchmark evidence below hot-index coverage floor");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.is_empty());
    assert!(stderr.contains("release readiness evidence failed validation"));
    assert!(stderr.contains("private real-corpus performance evidence"));
    assert!(stderr
        .contains("private real-corpus benchmark requires hot-index document coverage evidence"));
    assert!(!stderr.contains(path_str(&benchmark_report)));
    assert!(!stderr.contains(path_str(&evidence_dir)));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&benchmark_report)));
    assert!(!stdout.contains(path_str(&evidence_dir)));
    assert!(!stdout.contains(path_str(&data_dir)));

    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_dir_all(&evidence_dir);
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

fn json_path(path: &Path) -> String {
    path_str(path).replace('\\', "\\\\").replace('"', "\\\"")
}

fn write_reviewed_model_manifest(model_artifact: &Path, model_manifest: &Path) {
    fs::write(model_artifact, b"SYNTHETIC REVIEWED MODEL ARTIFACT\n").unwrap();
    fs::write(
        model_manifest,
        format!(
            r#"{{
  "schema_version": "resume-ir.model-manifest.v1",
  "model_pack_id": "fixture-pack-reviewed",
  "models": [
    {{
      "id": "fixture-reviewed-embedding-model",
      "type": "embedding",
      "dim": 4,
      "format": "onnx",
      "artifact": {{
        "path": "{}",
        "sha256": "57aac1132f550796663cdadce2ae702cb0bbf96b8620bc12f385d7b8aae0e492"
      }},
      "license": {{
        "id": "Apache-2.0",
        "reviewed": true
      }}
    }}
  ]
}}"#,
            json_path(model_artifact)
        ),
    )
    .unwrap();
}

fn write_unreviewed_model_manifest(model_artifact: &Path, model_manifest: &Path) {
    fs::write(model_artifact, b"SYNTHETIC UNREVIEWED MODEL ARTIFACT\n").unwrap();
    fs::write(
        model_manifest,
        format!(
            r#"{{
  "schema_version": "resume-ir.model-manifest.v1",
  "model_pack_id": "fixture-pack-unreviewed",
  "models": [
    {{
      "id": "fixture-unreviewed-embedding-model",
      "type": "embedding",
      "dim": 4,
      "format": "onnx",
      "artifact": {{
        "path": "{}",
        "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
      }},
      "license": {{
        "id": "Proprietary",
        "reviewed": false
      }}
    }}
  ]
}}"#,
            json_path(model_artifact)
        ),
    )
    .unwrap();
}

fn write_reviewed_ocr_manifest(ocr_engine: &Path, ocr_renderer: &Path, ocr_manifest: &Path) {
    fs::write(ocr_engine, b"SYNTHETIC TESSERACT RUNTIME\n").unwrap();
    fs::write(ocr_renderer, b"SYNTHETIC PDFTOPPM RUNTIME\n").unwrap();
    fs::write(
        ocr_manifest,
        format!(
            r#"{{
  "schema_version": "resume-ir.ocr-runtime-manifest.v1",
  "runtime_pack_id": "fixture-ocr-pack-reviewed",
  "components": [
    {{
      "id": "fixture-tesseract",
      "kind": "ocr-engine",
      "engine": "tesseract",
      "version": "5.5.1",
      "artifact": {{
        "path": "{}",
        "sha256": "f4c4eb4c45e595f803f076791dd942e6fd8bb93076207f8830ed6b8694f11e4a"
      }},
      "license": {{
        "id": "Apache-2.0",
        "reviewed": true
      }}
    }},
    {{
      "id": "fixture-pdftoppm",
      "kind": "pdf-renderer",
      "engine": "poppler-pdftoppm",
      "version": "25.12.0",
      "artifact": {{
        "path": "{}",
        "sha256": "571699d70504c3e505293c25953a85c38bdc8c13681aed7f7e3c4ce77fc8245f"
      }},
      "license": {{
        "id": "GPL-2.0-or-later",
        "reviewed": true
      }}
    }}
  ],
  "languages": [
    {{
      "id": "eng",
      "artifact": {{
        "path": "{}",
        "sha256": "f4c4eb4c45e595f803f076791dd942e6fd8bb93076207f8830ed6b8694f11e4a"
      }},
      "license": {{
        "id": "Apache-2.0",
        "reviewed": true
      }}
    }}
  ]
}}"#,
            json_path(ocr_engine),
            json_path(ocr_renderer),
            json_path(ocr_engine)
        ),
    )
    .unwrap();
}

fn redacted_diagnostics_report() -> String {
    concat!(
        "{",
        "\"schema_version\":\"diagnostics.v1\",",
        "\"redacted\":true,",
        "\"raw_paths\":\"<redacted>\",",
        "\"raw_queries\":\"<redacted>\",",
        "\"raw_resume_text\":\"<redacted>\",",
        "\"metadata\":{\"indexed_documents\":8720,\"searchable_documents\":8720,\"ocr_queue_depth\":0},",
        "\"search_index_state\":\"available\",",
        "\"vector_index_state\":\"available\",",
        "\"query_latency\":{\"sample_count\":500,\"p50_ms\":120,\"p95_ms\":2400,\"p99_ms\":4000,\"raw_queries\":\"<redacted>\"},",
        "\"resource_telemetry\":{\"paths\":\"<redacted>\"},",
        "\"ocr_runtime\":{\"paths\":\"<redacted>\",\"pdftoppm\":\"available\",\"tesseract\":\"available\"},",
        "\"diagnostic_scope\":{",
        "\"metadata\":\"aggregate_counts\",",
        "\"search_index\":\"state_and_snapshot_health\",",
        "\"vector_index\":\"state_backend_and_counts\",",
        "\"query_latency\":\"aggregate_observations\",",
        "\"runtime_dependencies\":\"presence_only\",",
        "\"fault_simulations\":\"available_cases_only\"",
        "},",
        "\"evidence_level\":\"local_aggregate_only\",",
        "\"scope\":\"redacted local aggregate diagnostics; no raw resume text, paths, queries, tokens, or index segment contents included\"",
        "}"
    )
    .to_string()
}

fn current_stage_evidence_manifest() -> String {
    concat!(
        "{",
        "\"schema_version\":\"resume-ir.current-stage-validation-evidence.v1\",",
        "\"privacy_boundary\":\"local_only_redacted_evidence_manifest\",",
        "\"current_stage_target\":\"reproducible_local_10k_baseline\",",
        "\"performance_optimization_deferred\":true,",
        "\"release_readiness_exit\":1,",
        "\"stable_release_expected_blocked\":true,",
        "\"input_digests\":{",
        "\"dataset_manifest_sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",",
        "\"query_set_sha256\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",",
        "\"model_manifest_sha256\":\"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\",",
        "\"ocr_runtime_manifest_sha256\":\"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\"",
        "},",
        "\"parameters\":{",
        "\"max_files\":10000,",
        "\"max_queries\":500,",
        "\"top_k\":10,",
        "\"embedding_dimension\":384,",
        "\"ocr_worker_ticks\":10000,",
        "\"embedding_worker_ticks\":10000",
        "},",
        "\"preflight_probes\":{",
        "\"ocr_runtime_probe\":\"passed\",",
        "\"embedding_protocol\":\"passed\"",
        "},",
        "\"steps\":[",
        "{\"id\":\"ocr_preflight\",\"status\":\"success\"},",
        "{\"id\":\"ocr_manifest_draft\",\"status\":\"success\"},",
        "{\"id\":\"ocr_manifest_validate\",\"status\":\"success\"},",
        "{\"id\":\"model_manifest_draft\",\"status\":\"success\"},",
        "{\"id\":\"model_manifest_validate\",\"status\":\"success\"},",
        "{\"id\":\"model_preflight\",\"status\":\"success\"},",
        "{\"id\":\"dataset_manifest\",\"status\":\"success\"},",
        "{\"id\":\"import_private_corpus\",\"status\":\"success\"},",
        "{\"id\":\"ocr_worker_bounded_loop\",\"status\":\"success\"},",
        "{\"id\":\"embedding_worker_bounded_loop\",\"status\":\"success\"},",
        "{\"id\":\"corpus_summary\",\"status\":\"success\"},",
        "{\"id\":\"query_set_draft\",\"status\":\"success\"},",
        "{\"id\":\"private_query_baseline\",\"status\":\"success\"},",
        "{\"id\":\"baseline_shape_gate\",\"status\":\"success\"},",
        "{\"id\":\"private_ocr_throughput_baseline\",\"status\":\"success\"},",
        "{\"id\":\"ocr_throughput_baseline_gate\",\"status\":\"success\"},",
        "{\"id\":\"redacted_diagnostics\",\"status\":\"success\"},",
        "{\"id\":\"release_readiness_intake\",\"status\":\"expected_blocked\",\"exit_code\":1}",
        "],",
        "\"redacted_outputs\":[",
        "{\"file\":\"dataset-manifest.local.json\",\"sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"},",
        "{\"file\":\"dataset-manifest.stdout.txt\",\"sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\"},",
        "{\"file\":\"ocr-runtime-manifest.local.json\",\"sha256\":\"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\"},",
        "{\"file\":\"ocr-preflight.json\",\"sha256\":\"1212121212121212121212121212121212121212121212121212121212121212\"},",
        "{\"file\":\"ocr-draft-manifest.stdout.txt\",\"sha256\":\"1313131313131313131313131313131313131313131313131313131313131313\"},",
        "{\"file\":\"ocr-validate-manifest.stdout.txt\",\"sha256\":\"1414141414141414141414141414141414141414141414141414141414141414\"},",
        "{\"file\":\"model-manifest.local.json\",\"sha256\":\"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\"},",
        "{\"file\":\"model-draft-manifest.stdout.txt\",\"sha256\":\"1515151515151515151515151515151515151515151515151515151515151515\"},",
        "{\"file\":\"model-validate-manifest.stdout.txt\",\"sha256\":\"1616161616161616161616161616161616161616161616161616161616161616\"},",
        "{\"file\":\"model-preflight.json\",\"sha256\":\"1717171717171717171717171717171717171717171717171717171717171717\"},",
        "{\"file\":\"import.stdout.txt\",\"sha256\":\"1818181818181818181818181818181818181818181818181818181818181818\"},",
        "{\"file\":\"ocr-worker.stdout.txt\",\"sha256\":\"1919191919191919191919191919191919191919191919191919191919191919\"},",
        "{\"file\":\"embedding-worker.stdout.txt\",\"sha256\":\"2020202020202020202020202020202020202020202020202020202020202020\"},",
        "{\"file\":\"benchmark-corpus-summary.local.json\",\"sha256\":\"2121212121212121212121212121212121212121212121212121212121212121\"},",
        "{\"file\":\"private-query-set.local.jsonl\",\"sha256\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"},",
        "{\"file\":\"query-set-draft.stdout.txt\",\"sha256\":\"2323232323232323232323232323232323232323232323232323232323232323\"},",
        "{\"file\":\"private-benchmark-local.json\",\"sha256\":\"2424242424242424242424242424242424242424242424242424242424242424\"},",
        "{\"file\":\"private-benchmark-gate.stdout.txt\",\"sha256\":\"2525252525252525252525252525252525252525252525252525252525252525\"},",
        "{\"file\":\"private-ocr-throughput.json\",\"sha256\":\"2626262626262626262626262626262626262626262626262626262626262626\"},",
        "{\"file\":\"ocr-throughput-gate.stdout.txt\",\"sha256\":\"2727272727272727272727272727272727272727272727272727272727272727\"},",
        "{\"file\":\"redacted-diagnostics.json\",\"sha256\":\"2828282828282828282828282828282828282828282828282828282828282828\"},",
        "{\"file\":\"release-readiness.json\",\"sha256\":\"2929292929292929292929292929292929292929292929292929292929292929\"},",
        "{\"file\":\"release-readiness.stderr.txt\",\"sha256\":\"3030303030303030303030303030303030303030303030303030303030303030\"}",
        "],",
        "\"privacy_sentinels\":{",
        "\"local_paths_included\":false,",
        "\"raw_resume_text_included\":false,",
        "\"raw_query_text_included\":false,",
        "\"model_bytes_included\":false,",
        "\"runtime_binaries_included\":false,",
        "\"report_bodies_included\":false",
        "},",
        "\"must_not_upload\":[",
        "\"raw resumes\",",
        "\"query set\",",
        "\"local manifests\",",
        "\"benchmark reports\",",
        "\"diagnostics\",",
        "\"indexes\",",
        "\"SQLite databases\",",
        "\"model caches\",",
        "\"runtime binaries\"",
        "]",
        "}"
    )
    .to_string()
}

fn current_stage_evidence_manifest_missing_preflight_probes() -> String {
    current_stage_evidence_manifest()
        .replace(
            "\"preflight_probes\":{\"ocr_runtime_probe\":\"passed\",\"embedding_protocol\":\"passed\"},",
            "",
        )
}

fn current_stage_evidence_manifest_missing_runtime_manifest_outputs() -> String {
    current_stage_evidence_manifest()
        .replace(
            "{\"file\":\"ocr-runtime-manifest.local.json\",\"sha256\":\"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\"},",
            "",
        )
        .replace(
            "{\"file\":\"model-manifest.local.json\",\"sha256\":\"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\"},",
            "",
        )
}

fn current_stage_evidence_manifest_missing_dataset_output() -> String {
    concat!(
        "{",
        "\"schema_version\":\"resume-ir.current-stage-validation-evidence.v1\",",
        "\"privacy_boundary\":\"local_only_redacted_evidence_manifest\",",
        "\"current_stage_target\":\"reproducible_local_10k_baseline\",",
        "\"performance_optimization_deferred\":true,",
        "\"release_readiness_exit\":1,",
        "\"stable_release_expected_blocked\":true,",
        "\"input_digests\":{",
        "\"dataset_manifest_sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",",
        "\"query_set_sha256\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",",
        "\"model_manifest_sha256\":\"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\",",
        "\"ocr_runtime_manifest_sha256\":\"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\"",
        "},",
        "\"parameters\":{",
        "\"max_files\":10000,",
        "\"max_queries\":500,",
        "\"top_k\":10,",
        "\"embedding_dimension\":384,",
        "\"ocr_worker_ticks\":10000,",
        "\"embedding_worker_ticks\":10000",
        "},",
        "\"steps\":[",
        "{\"id\":\"ocr_preflight\",\"status\":\"success\"},",
        "{\"id\":\"ocr_manifest_draft\",\"status\":\"success\"},",
        "{\"id\":\"ocr_manifest_validate\",\"status\":\"success\"},",
        "{\"id\":\"model_manifest_draft\",\"status\":\"success\"},",
        "{\"id\":\"model_manifest_validate\",\"status\":\"success\"},",
        "{\"id\":\"model_preflight\",\"status\":\"success\"},",
        "{\"id\":\"dataset_manifest\",\"status\":\"success\"},",
        "{\"id\":\"import_private_corpus\",\"status\":\"success\"},",
        "{\"id\":\"ocr_worker_bounded_loop\",\"status\":\"success\"},",
        "{\"id\":\"embedding_worker_bounded_loop\",\"status\":\"success\"},",
        "{\"id\":\"corpus_summary\",\"status\":\"success\"},",
        "{\"id\":\"query_set_draft\",\"status\":\"success\"},",
        "{\"id\":\"private_query_baseline\",\"status\":\"success\"},",
        "{\"id\":\"baseline_shape_gate\",\"status\":\"success\"},",
        "{\"id\":\"redacted_diagnostics\",\"status\":\"success\"},",
        "{\"id\":\"release_readiness_intake\",\"status\":\"expected_blocked\",\"exit_code\":1}",
        "],",
        "\"redacted_outputs\":[",
        "{\"file\":\"dataset-manifest.stdout.txt\",\"sha256\":\"1010101010101010101010101010101010101010101010101010101010101010\"},",
        "{\"file\":\"ocr-preflight.json\",\"sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\"},",
        "{\"file\":\"model-preflight.json\",\"sha256\":\"1212121212121212121212121212121212121212121212121212121212121212\"},",
        "{\"file\":\"benchmark-corpus-summary.local.json\",\"sha256\":\"1313131313131313131313131313131313131313131313131313131313131313\"},",
        "{\"file\":\"private-query-set.local.jsonl\",\"sha256\":\"1414141414141414141414141414141414141414141414141414141414141414\"},",
        "{\"file\":\"query-set-draft.stdout.txt\",\"sha256\":\"1515151515151515151515151515151515151515151515151515151515151515\"},",
        "{\"file\":\"private-benchmark-local.json\",\"sha256\":\"1616161616161616161616161616161616161616161616161616161616161616\"},",
        "{\"file\":\"private-benchmark-gate.stdout.txt\",\"sha256\":\"1717171717171717171717171717171717171717171717171717171717171717\"},",
        "{\"file\":\"redacted-diagnostics.json\",\"sha256\":\"1818181818181818181818181818181818181818181818181818181818181818\"},",
        "{\"file\":\"release-readiness.json\",\"sha256\":\"1919191919191919191919191919191919191919191919191919191919191919\"},",
        "{\"file\":\"release-readiness.stderr.txt\",\"sha256\":\"2020202020202020202020202020202020202020202020202020202020202020\"}",
        "],",
        "\"privacy_sentinels\":{",
        "\"local_paths_included\":false,",
        "\"raw_resume_text_included\":false,",
        "\"raw_query_text_included\":false,",
        "\"model_bytes_included\":false,",
        "\"runtime_binaries_included\":false,",
        "\"report_bodies_included\":false",
        "},",
        "\"must_not_upload\":[",
        "\"raw resumes\",",
        "\"query set\",",
        "\"local manifests\",",
        "\"benchmark reports\",",
        "\"diagnostics\",",
        "\"indexes\",",
        "\"SQLite databases\",",
        "\"model caches\",",
        "\"runtime binaries\"",
        "]",
        "}"
    )
    .to_string()
}

fn blocked_signing_evidence() -> String {
    concat!(
        "{",
        "\"schema_version\":\"release.signing_evidence.v1\",",
        "\"version\":\"v0.0.0\",",
        "\"signing_status\":\"blocked\",",
        "\"evidence_boundary\":\"dry_run_no_signing_material\",",
        "\"artifact_manifest_sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",",
        "\"required_evidence\":[\"certificate_chain\",\"private_key_custody\",\"signature_verification_evidence\"],",
        "\"blocked_release_steps\":[\"production_signing_certificates\",\"certificate_chain_review\",\"private_key_custody\",\"artifact_signature_verification\"],",
        "\"prohibited_public_material\":[\"signing_private_key\",\"certificate_password\",\"signing_token\",\"local_paths\",\"raw_resume_data\"]",
        "}"
    )
    .to_string()
}

fn blocked_notarization_evidence() -> String {
    concat!(
        "{",
        "\"schema_version\":\"release.notarization_evidence.v1\",",
        "\"version\":\"v0.0.0\",",
        "\"notarization_status\":\"blocked\",",
        "\"evidence_boundary\":\"dry_run_no_notarization_credentials\",",
        "\"macos_package_manifest_sha256\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",",
        "\"required_evidence\":[\"apple_developer_id_certificate\",\"notarytool_submission\",\"notarization_ticket\",\"stapled_ticket\",\"gatekeeper_validation\"],",
        "\"blocked_release_steps\":[\"apple_developer_id_certificate\",\"notarytool_submission\",\"notarization_ticket_stapling\",\"spctl_gatekeeper_validation\"],",
        "\"prohibited_public_material\":[\"notary_credentials\",\"notary_password\",\"notary_api_secret\",\"local_paths\",\"raw_resume_data\"]",
        "}"
    )
    .to_string()
}

fn blocked_macos_installer_evidence() -> String {
    concat!(
        "{",
        "\"schema_version\":\"release.macos_installer_evidence.v1\",",
        "\"version\":\"v0.0.0\",",
        "\"installer_lifecycle_status\":\"blocked\",",
        "\"evidence_boundary\":\"dry_run_no_macos_installer_execution\",",
        "\"macos_package_manifest_sha256\":\"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\",",
        "\"admin_elevation\":\"required_not_observed\",",
        "\"installation_status\":\"not_installed\",",
        "\"rollback_validation_status\":\"blocked\",",
        "\"launch_agent_validation_status\":\"blocked\",",
        "\"planned_actions\":[{\"action\":\"install\",\"action_status\":\"blocked\"},{\"action\":\"upgrade\",\"action_status\":\"blocked\"},{\"action\":\"uninstall\",\"action_status\":\"blocked\"},{\"action\":\"rollback\",\"action_status\":\"blocked\"},{\"action\":\"launch-agent-start\",\"action_status\":\"blocked\"},{\"action\":\"launch-agent-stop\",\"action_status\":\"blocked\"}],",
        "\"required_evidence\":[\"administrator-elevated install transcript\",\"installer_lifecycle_validation\",\"upgrade_validation\",\"uninstall_validation\",\"rollback_validation\",\"launch_agent_start_validation\",\"launch_agent_stop_validation\"],",
        "\"blocked_release_steps\":[\"macos_pkg_install\",\"macos_pkg_upgrade\",\"macos_pkg_uninstall\",\"macos_pkg_rollback\",\"macos_launch_agent_start\",\"macos_launch_agent_stop\"],",
        "\"prohibited_public_material\":[\"installer_tokens\",\"administrator_passwords\",\"local_paths\",\"raw_installer_logs\",\"raw_resume_data\",\"diagnostic_packages\",\"model_artifact_caches\"]",
        "}"
    )
    .to_string()
}

fn blocked_windows_installer_evidence() -> String {
    concat!(
        "{",
        "\"schema_version\":\"release.windows_installer_evidence.v1\",",
        "\"version\":\"v0.0.0\",",
        "\"installer_lifecycle_status\":\"blocked\",",
        "\"evidence_boundary\":\"dry_run_no_windows_installer_execution\",",
        "\"windows_package_manifest_sha256\":\"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\",",
        "\"admin_elevation\":\"required_not_observed\",",
        "\"installation_status\":\"not_installed\",",
        "\"rollback_validation_status\":\"blocked\",",
        "\"planned_actions\":[{\"action\":\"install\",\"action_status\":\"blocked\"},{\"action\":\"upgrade\",\"action_status\":\"blocked\"},{\"action\":\"repair\",\"action_status\":\"blocked\"},{\"action\":\"uninstall\",\"action_status\":\"blocked\"},{\"action\":\"rollback\",\"action_status\":\"blocked\"}],",
        "\"required_evidence\":[\"administrator-elevated install transcript\",\"installer_lifecycle_validation\",\"upgrade_validation\",\"repair_validation\",\"uninstall_validation\",\"rollback_validation\"],",
        "\"blocked_release_steps\":[\"windows_msi_install\",\"windows_msi_upgrade\",\"windows_msi_repair\",\"windows_msi_uninstall\",\"windows_msi_rollback\"],",
        "\"prohibited_public_material\":[\"installer_tokens\",\"administrator_passwords\",\"local_paths\",\"raw_installer_logs\",\"raw_resume_data\",\"diagnostic_packages\",\"model_artifact_caches\"]",
        "}"
    )
    .to_string()
}

fn blocked_macos_installer_lifecycle_plan() -> String {
    concat!(
        "{",
        "\"schema_version\":\"release.macos_installer_lifecycle_plan.v1\",",
        "\"version\":\"v0.0.0\",",
        "\"execution_mode\":\"dry_run\",",
        "\"installer_lifecycle_status\":\"blocked\",",
        "\"evidence_boundary\":\"dry_run_no_macos_installer_execution\",",
        "\"macos_package_manifest_sha256\":\"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\",",
        "\"admin_elevation\":\"required_not_observed\",",
        "\"release_runner\":\"macos_required_not_observed\",",
        "\"installer_artifacts\":[",
        "{\"kind\":\"pkg\",\"file\":\"resume-ir-v0.0.0-macos.pkg\",\"artifact_sha256\":\"4444444444444444444444444444444444444444444444444444444444444444\",\"bytes\":404},",
        "{\"kind\":\"dmg\",\"file\":\"resume-ir-v0.0.0-macos.dmg\",\"artifact_sha256\":\"5555555555555555555555555555555555555555555555555555555555555555\",\"bytes\":505}",
        "],",
        "\"planned_actions\":[",
        "{\"action\":\"install\",\"command\":\"installer\",\"target_artifact\":\"resume-ir-v0.0.0-macos.pkg\",\"dry_run_intent\":\"validate administrator-elevated pkg install on release runner\",\"requires_approval\":true,\"action_status\":\"blocked\"},",
        "{\"action\":\"upgrade\",\"command\":\"installer\",\"target_artifact\":\"resume-ir-v0.0.0-macos.pkg\",\"dry_run_intent\":\"install prior version, upgrade, and verify binary replacement\",\"requires_approval\":true,\"action_status\":\"blocked\"},",
        "{\"action\":\"uninstall\",\"command\":\"pkgutil\",\"target_artifact\":\"resume-ir-v0.0.0-macos.pkg\",\"dry_run_intent\":\"forget package receipt and remove installed files while preserving user data\",\"requires_approval\":true,\"action_status\":\"blocked\"},",
        "{\"action\":\"rollback\",\"command\":\"installer\",\"target_artifact\":\"resume-ir-v0.0.0-macos.pkg\",\"dry_run_intent\":\"force installer failure and verify system state restoration\",\"requires_approval\":true,\"action_status\":\"blocked\"},",
        "{\"action\":\"launch-agent-start\",\"command\":\"launchctl\",\"target_artifact\":\"resume-ir-v0.0.0-macos.dmg\",\"dry_run_intent\":\"bootstrap LaunchAgent and verify daemon IPC health\",\"requires_approval\":true,\"action_status\":\"blocked\"},",
        "{\"action\":\"launch-agent-stop\",\"command\":\"launchctl\",\"target_artifact\":\"resume-ir-v0.0.0-macos.dmg\",\"dry_run_intent\":\"stop and bootout LaunchAgent and verify daemon shutdown\",\"requires_approval\":true,\"action_status\":\"blocked\"}",
        "],",
        "\"blocked_release_steps\":[\"macos_pkg_install\",\"macos_pkg_upgrade\",\"macos_pkg_uninstall\",\"macos_pkg_rollback\",\"macos_launch_agent_start\",\"macos_launch_agent_stop\"],",
        "\"prohibited_public_material\":[\"installer_tokens\",\"administrator_passwords\",\"local_paths\",\"raw_installer_logs\",\"raw_resume_data\",\"diagnostic_packages\",\"model_artifact_caches\"],",
        "\"notes\":\"Dry-run operator plan only. It does not execute installer lifecycle commands or clear release blockers; release-runner transcripts are required before stable release.\"",
        "}"
    )
    .to_string()
}

fn blocked_windows_installer_lifecycle_plan() -> String {
    concat!(
        "{",
        "\"schema_version\":\"release.windows_installer_lifecycle_plan.v1\",",
        "\"version\":\"v0.0.0\",",
        "\"execution_mode\":\"dry_run\",",
        "\"installer_lifecycle_status\":\"blocked\",",
        "\"evidence_boundary\":\"dry_run_no_windows_installer_execution\",",
        "\"windows_package_manifest_sha256\":\"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\",",
        "\"installer_engine\":\"msiexec.exe\",",
        "\"admin_elevation\":\"required_not_observed\",",
        "\"release_runner\":\"windows_required_not_observed\",",
        "\"installation_status\":\"not_installed\",",
        "\"rollback_validation_status\":\"blocked\",",
        "\"installer_artifacts\":[{\"kind\":\"msi\",\"file\":\"resume-ir-v0.0.0-windows.msi\",\"artifact_sha256\":\"6666666666666666666666666666666666666666666666666666666666666666\",\"bytes\":606}],",
        "\"planned_actions\":[",
        "{\"action\":\"install\",\"command\":\"msiexec.exe\",\"target_artifact\":\"resume-ir-v0.0.0-windows.msi\",\"dry_run_intent\":\"validate administrator-elevated MSI install on release runner\",\"requires_approval\":true,\"action_status\":\"blocked\"},",
        "{\"action\":\"upgrade\",\"command\":\"msiexec.exe\",\"target_artifact\":\"resume-ir-v0.0.0-windows.msi\",\"dry_run_intent\":\"install prior version, upgrade, and verify binary replacement\",\"requires_approval\":true,\"action_status\":\"blocked\"},",
        "{\"action\":\"repair\",\"command\":\"msiexec.exe\",\"target_artifact\":\"resume-ir-v0.0.0-windows.msi\",\"dry_run_intent\":\"run MSI repair and verify installed-file integrity\",\"requires_approval\":true,\"action_status\":\"blocked\"},",
        "{\"action\":\"uninstall\",\"command\":\"msiexec.exe\",\"target_artifact\":\"resume-ir-v0.0.0-windows.msi\",\"dry_run_intent\":\"uninstall MSI and verify user-data preservation\",\"requires_approval\":true,\"action_status\":\"blocked\"},",
        "{\"action\":\"rollback\",\"command\":\"msiexec.exe\",\"target_artifact\":\"resume-ir-v0.0.0-windows.msi\",\"dry_run_intent\":\"force MSI failure and verify rollback state restoration\",\"requires_approval\":true,\"action_status\":\"blocked\"}",
        "],",
        "\"blocked_release_steps\":[\"windows_msi_install\",\"windows_msi_upgrade\",\"windows_msi_repair\",\"windows_msi_uninstall\",\"windows_msi_rollback\"],",
        "\"prohibited_public_material\":[\"installer_tokens\",\"administrator_passwords\",\"local_paths\",\"raw_installer_logs\",\"raw_resume_data\",\"diagnostic_packages\",\"model_artifact_caches\"],",
        "\"notes\":\"Dry-run operator plan only. It does not execute installer lifecycle commands or clear release blockers; release-runner transcripts are required before stable release.\"",
        "}"
    )
    .to_string()
}

fn blocked_windows_service_evidence() -> String {
    concat!(
        "{",
        "\"schema_version\":\"release.windows_service_evidence.v1\",",
        "\"version\":\"v0.0.0\",",
        "\"service_lifecycle_status\":\"blocked\",",
        "\"evidence_boundary\":\"dry_run_no_windows_service_registration\",",
        "\"windows_package_manifest_sha256\":\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\",",
        "\"admin_elevation\":\"required_not_observed\",",
        "\"registration_status\":\"not_registered\",",
        "\"recovery_validation_status\":\"blocked\",",
        "\"planned_actions\":[{\"action\":\"install\",\"action_status\":\"blocked\"},{\"action\":\"start\",\"action_status\":\"blocked\"},{\"action\":\"status\",\"action_status\":\"blocked\"},{\"action\":\"stop\",\"action_status\":\"blocked\"},{\"action\":\"uninstall\",\"action_status\":\"blocked\"},{\"action\":\"recovery\",\"action_status\":\"blocked\"}],",
        "\"required_evidence\":[\"administrator-elevated install transcript\",\"service_install_validation\",\"service_start_validation\",\"service_status_validation\",\"service_stop_validation\",\"service_uninstall_validation\",\"service_recovery_validation\"],",
        "\"blocked_release_steps\":[\"windows_service_install\",\"windows_service_start\",\"windows_service_status\",\"windows_service_stop\",\"windows_service_uninstall\",\"windows_service_recovery\",\"windows_service_rollback\"],",
        "\"prohibited_public_material\":[\"service_tokens\",\"administrator_passwords\",\"local_paths\",\"raw_service_logs\",\"raw_resume_data\",\"diagnostic_packages\",\"model_artifact_caches\"]",
        "}"
    )
    .to_string()
}

fn blocked_windows_service_lifecycle_plan() -> String {
    concat!(
        "{",
        "\"schema_version\":\"release.windows_service_lifecycle_plan.v1\",",
        "\"version\":\"v0.0.0\",",
        "\"execution_mode\":\"dry_run\",",
        "\"service_lifecycle_status\":\"blocked\",",
        "\"evidence_boundary\":\"dry_run_no_windows_service_registration\",",
        "\"windows_package_manifest_sha256\":\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\",",
        "\"service_manager\":\"sc.exe\",",
        "\"admin_elevation\":\"required_not_observed\",",
        "\"release_runner\":\"windows_required_not_observed\",",
        "\"registration_status\":\"not_registered\",",
        "\"recovery_validation_status\":\"blocked\",",
        "\"rollback_validation_status\":\"blocked\",",
        "\"service_artifacts\":[{\"kind\":\"msi\",\"file\":\"resume-ir-v0.0.0-windows.msi\",\"artifact_sha256\":\"6666666666666666666666666666666666666666666666666666666666666666\",\"bytes\":606,\"service_validation_status\":\"not_executed\"}],",
        "\"planned_actions\":[",
        "{\"action\":\"install\",\"command\":\"sc.exe\",\"target_artifact\":\"resume-ir-v0.0.0-windows.msi\",\"dry_run_intent\":\"register Windows Service after administrator-elevated MSI install and verify binary binding\",\"requires_approval\":true,\"action_status\":\"blocked\"},",
        "{\"action\":\"start\",\"command\":\"sc.exe\",\"target_artifact\":\"resume-ir-v0.0.0-windows.msi\",\"dry_run_intent\":\"start service and verify daemon IPC health\",\"requires_approval\":true,\"action_status\":\"blocked\"},",
        "{\"action\":\"status\",\"command\":\"sc.exe\",\"target_artifact\":\"resume-ir-v0.0.0-windows.msi\",\"dry_run_intent\":\"query service status on release Windows runner\",\"requires_approval\":true,\"action_status\":\"blocked\"},",
        "{\"action\":\"stop\",\"command\":\"sc.exe\",\"target_artifact\":\"resume-ir-v0.0.0-windows.msi\",\"dry_run_intent\":\"stop service and verify daemon shutdown\",\"requires_approval\":true,\"action_status\":\"blocked\"},",
        "{\"action\":\"recovery\",\"command\":\"sc.exe\",\"target_artifact\":\"resume-ir-v0.0.0-windows.msi\",\"dry_run_intent\":\"configure and prove restart-after-kill recovery policy\",\"requires_approval\":true,\"action_status\":\"blocked\"},",
        "{\"action\":\"uninstall\",\"command\":\"sc.exe\",\"target_artifact\":\"resume-ir-v0.0.0-windows.msi\",\"dry_run_intent\":\"delete service registration while preserving user data\",\"requires_approval\":true,\"action_status\":\"blocked\"},",
        "{\"action\":\"rollback\",\"command\":\"sc.exe\",\"target_artifact\":\"resume-ir-v0.0.0-windows.msi\",\"dry_run_intent\":\"force service install/start failure and verify rollback state restoration\",\"requires_approval\":true,\"action_status\":\"blocked\"}",
        "],",
        "\"blocked_release_steps\":[\"windows_service_install\",\"windows_service_start\",\"windows_service_status\",\"windows_service_stop\",\"windows_service_recovery\",\"windows_service_uninstall\",\"windows_service_rollback\"],",
        "\"prohibited_public_material\":[\"service_tokens\",\"administrator_passwords\",\"local_paths\",\"raw_service_logs\",\"raw_resume_data\",\"diagnostic_packages\",\"model_artifact_caches\"],",
        "\"notes\":\"Dry-run operator plan only. It does not register, start, stop, query, recover, uninstall, or roll back a Windows service; release-runner transcripts are required before stable release.\"",
        "}"
    )
    .to_string()
}

fn release_artifacts_manifest() -> String {
    concat!(
        "{",
        "\"schema_version\":\"release.artifacts.v1\",",
        "\"version\":\"v0.0.0\",",
        "\"packaging_status\":\"blocked\",",
        "\"artifacts\":[",
        "{\"name\":\"resume-cli\",\"file\":\"resume-cli\",\"sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\",\"bytes\":101},",
        "{\"name\":\"resume-daemon\",\"file\":\"resume-daemon\",\"sha256\":\"2222222222222222222222222222222222222222222222222222222222222222\",\"bytes\":202},",
        "{\"name\":\"resume-benchmark\",\"file\":\"resume-benchmark\",\"sha256\":\"3333333333333333333333333333333333333333333333333333333333333333\",\"bytes\":303}",
        "],",
        "\"blocked_release_steps\":[\"packaging\",\"signing\",\"notarization\",\"github_release_upload\"],",
        "\"notes\":\"Dry-run manifest only; no installer, signature, notarization ticket, release upload, local data, or runtime data is included.\"",
        "}"
    )
    .to_string()
}

fn release_sbom_manifest() -> String {
    concat!(
        "{",
        "\"spdxVersion\":\"SPDX-2.3\",",
        "\"dataLicense\":\"CC0-1.0\",",
        "\"SPDXID\":\"SPDXRef-DOCUMENT\",",
        "\"name\":\"resume-ir-v0.0.0\",",
        "\"documentNamespace\":\"https://github.com/FrankQDWang/resume-ir/sbom/v0.0.0\",",
        "\"creationInfo\":{\"created\":\"2026-06-10T00:00:00Z\",\"creators\":[\"Tool: resume-ir-release-sbom\"]},",
        "\"packages\":[",
        "{\"SPDXID\":\"SPDXRef-Package-resume-cli\",\"name\":\"resume-cli\",\"versionInfo\":\"0.1.0\",\"filesAnalyzed\":false,\"licenseDeclared\":\"MIT\",\"externalRefs\":[{\"referenceCategory\":\"PACKAGE-MANAGER\",\"referenceType\":\"purl\",\"referenceLocator\":\"pkg:cargo/resume-cli@0.1.0\"}]},",
        "{\"SPDXID\":\"SPDXRef-Package-resume-daemon\",\"name\":\"resume-daemon\",\"versionInfo\":\"0.1.0\",\"filesAnalyzed\":false,\"licenseDeclared\":\"MIT\",\"externalRefs\":[{\"referenceCategory\":\"PACKAGE-MANAGER\",\"referenceType\":\"purl\",\"referenceLocator\":\"pkg:cargo/resume-daemon@0.1.0\"}]},",
        "{\"SPDXID\":\"SPDXRef-Package-benchmark-runner\",\"name\":\"benchmark-runner\",\"versionInfo\":\"0.1.0\",\"filesAnalyzed\":false,\"licenseDeclared\":\"MIT\",\"externalRefs\":[{\"referenceCategory\":\"PACKAGE-MANAGER\",\"referenceType\":\"purl\",\"referenceLocator\":\"pkg:cargo/benchmark-runner@0.1.0\"}]}",
        "],",
        "\"relationships\":[",
        "{\"spdxElementId\":\"SPDXRef-DOCUMENT\",\"relationshipType\":\"DESCRIBES\",\"relatedSpdxElement\":\"SPDXRef-Package-resume-cli\"},",
        "{\"spdxElementId\":\"SPDXRef-DOCUMENT\",\"relationshipType\":\"DESCRIBES\",\"relatedSpdxElement\":\"SPDXRef-Package-resume-daemon\"},",
        "{\"spdxElementId\":\"SPDXRef-DOCUMENT\",\"relationshipType\":\"DESCRIBES\",\"relatedSpdxElement\":\"SPDXRef-Package-benchmark-runner\"}",
        "]",
        "}"
    )
    .to_string()
}

fn macos_package_manifest() -> String {
    concat!(
        "{",
        "\"schema_version\":\"release.macos_package.v1\",",
        "\"version\":\"v0.0.0\",",
        "\"packaging_status\":\"unsigned_dry_run\",",
        "\"install_location\":\"/usr/local/bin\",",
        "\"signing_status\":\"unsigned\",",
        "\"notarization_status\":\"not_requested\",",
        "\"artifacts\":[",
        "{\"kind\":\"pkg\",\"file\":\"resume-ir-v0.0.0-macos.pkg\",\"sha256\":\"4444444444444444444444444444444444444444444444444444444444444444\",\"bytes\":404},",
        "{\"kind\":\"dmg\",\"file\":\"resume-ir-v0.0.0-macos.dmg\",\"sha256\":\"5555555555555555555555555555555555555555555555555555555555555555\",\"bytes\":505}",
        "],",
        "\"blocked_release_steps\":[\"signing\",\"notarization\",\"github_release_upload\",\"installer_lifecycle_validation\",\"windows_msi\"],",
        "\"notes\":\"Unsigned local macOS package dry run only; no signing, notarization, installer lifecycle validation, GitHub Release upload, local data, or runtime data is included.\"",
        "}"
    )
    .to_string()
}

fn windows_package_manifest() -> String {
    concat!(
        "{",
        "\"schema_version\":\"release.windows_package.v1\",",
        "\"version\":\"v0.0.0\",",
        "\"packaging_status\":\"unsigned_dry_run\",",
        "\"installer_kind\":\"msi\",",
        "\"install_location\":\"ProgramFilesFolder/resume-ir\",",
        "\"signing_status\":\"unsigned\",",
        "\"artifacts\":[",
        "{\"kind\":\"msi\",\"file\":\"resume-ir-v0.0.0-windows.msi\",\"sha256\":\"6666666666666666666666666666666666666666666666666666666666666666\",\"bytes\":606}",
        "],",
        "\"blocked_release_steps\":[\"signing\",\"github_release_upload\",\"installer_lifecycle_validation\",\"service_install_validation\",\"macos_notarization\"],",
        "\"notes\":\"Unsigned Windows MSI dry run only; no signing, service lifecycle validation, installer lifecycle validation, GitHub Release upload, local data, or runtime data is included.\"",
        "}"
    )
    .to_string()
}

fn private_real_benchmark_report() -> String {
    concat!(
        "{",
        "\"schema_version\":\"benchmark.v1\",",
        "\"run_id\":\"bench_test\",",
        "\"platform\":\"test/test\",",
        "\"dataset_kind\":\"private-real-corpus\",",
        "\"document_count\":8720,",
        "\"searchable_document_count\":8720,",
        "\"vector_indexed_document_count\":8720,",
        "\"query_count\":500,",
        "\"top_k\":10,",
        "\"build_ms\":1.0,",
        "\"query_total_ms\":600000.0,",
        "\"qps\":0.833333,",
        "\"index_size_bytes\":1000,",
        "\"query_latency_ms\":{\"samples\":500,\"min\":10.0,\"mean\":900.0,\"p50\":850.0,\"p95\":2500.0,\"p99\":4000.0,\"max\":5000.0},",
        "\"zero_result_queries\":0,",
        "\"total_hits\":100,",
        "\"million_scale_verified\":false,",
        "\"percentile_confidence\":\"sampled\",",
        "\"target_claim\":\"benchmark_baseline_observed\",",
        "\"scope\":\"private local real-corpus query benchmark; aggregate redacted report only\",",
        "\"corpus_origin\":\"private_local\",",
        "\"privacy_boundary\":\"redacted_local_aggregate\",",
        "\"query_protocol\":\"resume-ir-query-v1\",",
        "\"query_mode\":\"hybrid\",",
        "\"retrieval_layers\":\"fulltext+field+vector+rrf\",",
        "\"hot_index\":true,",
        "\"hot_path_ocr\":false,",
        "\"hot_path_parsing\":false,",
        "\"hot_path_heavy_model_inference\":false,",
        "\"contains_raw_resume_text\":false,",
        "\"contains_resume_paths\":false,",
        "\"contains_queries\":false,",
        "\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",",
        "\"query_set_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\",",
        "\"model_manifest_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\",",
        "\"corpus_summary_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\"",
        "}"
    )
    .to_string()
}

fn private_business_field_quality_report() -> String {
    let metric = "{\"true_positive\":100,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0}";
    format!(
        concat!(
            "{{",
            "\"schema_version\":\"field-quality.v1\",",
            "\"run_id\":\"fieldq_test\",",
            "\"platform\":\"test/test\",",
            "\"dataset_kind\":\"private-business-labeled\",",
            "\"sample_count\":1000,",
            "\"expected_mentions\":1500,",
            "\"predicted_mentions\":1500,",
            "\"overall\":{},",
            "\"fields\":{{",
            "\"name\":{},\"email\":{},\"phone\":{},\"wechat\":{},\"school\":{},\"school_tier\":{},",
            "\"degree\":{},\"major\":{},\"company\":{},\"title\":{},\"location\":{},",
            "\"skill\":{},\"certificate\":{},\"date_range\":{},\"years_experience\":{}",
            "}},",
            "\"target_claim\":\"field_quality_target_met\",",
            "\"corpus_origin\":\"private_local\",",
            "\"privacy_boundary\":\"redacted_local_aggregate\",",
            "\"contains_raw_resume_text\":false,",
            "\"contains_resume_paths\":false,",
            "\"contains_field_values\":false,",
            "\"contains_sample_ids\":false,",
            "\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",",
            "\"annotation_manifest_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\",",
            "\"field_taxonomy\":\"resume-ir.fields.v1\",",
            "\"scope\":\"private business field-quality benchmark; aggregate redacted report only\"",
            "}}"
        ),
        metric,
        metric,
        metric,
        metric,
        metric,
        metric,
        metric,
        metric,
        metric,
        metric,
        metric,
        metric,
        metric,
        metric,
        metric,
        metric,
    )
}

fn private_business_dedupe_quality_report() -> String {
    concat!(
        "{",
        "\"schema_version\":\"dedupe-quality.v1\",",
        "\"run_id\":\"dedupeq_test\",",
        "\"platform\":\"test/test\",",
        "\"dataset_kind\":\"private-business-labeled\",",
        "\"pair_count\":1000,",
        "\"positive_pair_count\":100,",
        "\"predicted_duplicate_pairs\":100,",
        "\"true_positive\":100,",
        "\"false_positive\":0,",
        "\"false_negative\":0,",
        "\"true_negative\":900,",
        "\"precision\":1.0,",
        "\"recall\":1.0,",
        "\"f1\":1.0,",
        "\"target_claim\":\"dedupe_quality_target_met\",",
        "\"corpus_origin\":\"private_local\",",
        "\"privacy_boundary\":\"redacted_local_aggregate\",",
        "\"contains_raw_resume_text\":false,",
        "\"contains_resume_paths\":false,",
        "\"contains_profile_values\":false,",
        "\"contains_sample_ids\":false,",
        "\"contains_document_ids\":false,",
        "\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",",
        "\"annotation_manifest_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\",",
        "\"dedupe_taxonomy\":\"resume-ir.dedupe.v1\",",
        "\"scope\":\"private business dedupe-quality benchmark; aggregate redacted report only\"",
        "}"
    )
    .to_string()
}

fn private_business_vector_quality_report() -> String {
    concat!(
        "{",
        "\"schema_version\":\"vector-quality.v1\",",
        "\"run_id\":\"vectorq_test\",",
        "\"platform\":\"test/test\",",
        "\"dataset_kind\":\"private-business-labeled\",",
        "\"sample_count\":1000,",
        "\"candidate_count\":10000,",
        "\"top_k\":10,",
        "\"recall_at_k\":0.95,",
        "\"mrr\":0.91,",
        "\"ndcg_at_k\":0.93,",
        "\"zero_recall_queries\":0,",
        "\"target_claim\":\"vector_quality_target_met\",",
        "\"corpus_origin\":\"private_local\",",
        "\"privacy_boundary\":\"redacted_local_aggregate\",",
        "\"contains_raw_queries\":false,",
        "\"contains_candidate_text\":false,",
        "\"contains_resume_paths\":false,",
        "\"contains_sample_ids\":false,",
        "\"contains_candidate_ids\":false,",
        "\"contains_vectors\":false,",
        "\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",",
        "\"annotation_manifest_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\",",
        "\"model_manifest_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\",",
        "\"vector_taxonomy\":\"resume-ir.vector-quality.v1\",",
        "\"scope\":\"private business vector-quality benchmark; aggregate redacted report only\"",
        "}"
    )
    .to_string()
}

fn private_real_ocr_throughput_report() -> String {
    concat!(
        "{",
        "\"schema_version\":\"ocr-throughput.v1\",",
        "\"run_id\":\"ocr_release_test\",",
        "\"platform\":\"test/test\",",
        "\"dataset_kind\":\"private-real-corpus\",",
        "\"page_count\":500,",
        "\"document_count\":250,",
        "\"scanned_document_count\":250,",
        "\"failed_document_count\":0,",
        "\"render_failure_count\":0,",
        "\"ocr_failure_count\":0,",
        "\"run_budget_exhausted\":false,",
        "\"engine_kind\":\"tesseract\",",
        "\"total_ms\":1250000.0,",
        "\"page_latency_ms\":{\"samples\":500,\"p50\":2200.0,\"p95\":4200.0,\"p99\":6100.0},",
        "\"pages_per_second\":0.4,",
        "\"target_claim\":\"ocr_throughput_baseline_observed\",",
        "\"corpus_origin\":\"private_local\",",
        "\"privacy_boundary\":\"redacted_local_aggregate\",",
        "\"contains_raw_ocr_text\":false,",
        "\"contains_page_images\":false,",
        "\"contains_resume_paths\":false,",
        "\"contains_document_ids\":false,",
        "\"contains_page_ids\":false,",
        "\"contains_command_paths\":false,",
        "\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",",
        "\"ocr_runtime_manifest_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\",",
        "\"renderer_manifest_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\",",
        "\"language_pack_manifest_sha256\":\"2222222222222222222222222222222222222222222222222222222222222222\",",
        "\"scope\":\"private real-corpus OCR throughput benchmark; aggregate redacted report only\"",
        "}"
    )
    .to_string()
}
