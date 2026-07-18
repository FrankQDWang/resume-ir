//! Bounded product-gap projection for the release-readiness report.

pub(crate) fn release_readiness_goal_gap_matrix_json() -> serde_json::Value {
    serde_json::json!({
        "schema_version": "resume-ir.goal-gap-matrix.v2",
        "complete_product": false,
        "current_stage": "tauri_desktop_product_incomplete_release_blocked",
        "stable_release": "blocked",
        "completion_statement": "core local import/search closure is verified; ordinary-user Tauri desktop installers are incomplete and stable release remains blocked by implementation, evidence, credentials, and platform transcripts",
        "rows": [
            {
                "id": "P0_foundation",
                "label": "Rust workspace, daemon, CLI, metadata, task queue, IPC, diagnostics skeleton, CI",
                "implementation_status": "production_complete",
                "release_status": "covered_by_local_ci",
                "evidence": [
                    "daemon/CLI/metadata/IPC tests",
                    "kill/restart recovery tests",
                    "PR rust workspace checks"
                ],
                "blocked_by": []
            },
            {
                "id": "P1_text_import_fulltext",
                "label": "file scan, docx/PDF text layer parsing, normalization, full-text index, snippets",
                "implementation_status": "production_complete",
                "release_status": "covered_by_local_ci",
                "evidence": [
                    "parser fixture tests",
                    "import/search closed-loop checks",
                    "persistent full-text index recovery tests"
                ],
                "blocked_by": []
            },
            {
                "id": "P2_fields_dedupe",
                "label": "field extraction, confidence/evidence, filters, soft dedupe, multi-version folding",
                "implementation_status": "production_complete",
                "release_status": "blocked",
                "evidence": [
                    "extractor/filter tests",
                    "candidate folding tests",
                    "field persistence tests"
                ],
                "blocked_by": [
                    "private business labeled field/dedupe quality reports",
                    "field F1 production threshold evidence",
                    "dedupe precision/recall/F1 production threshold evidence"
                ]
            },
            {
                "id": "P3_semantic_vector",
                "label": "local embedding protocol, vector persistence, semantic/hybrid search, RRF",
                "implementation_status": "production_complete",
                "release_status": "blocked",
                "evidence": [
                    "local embedding protocol tests",
                    "persistent vector snapshot tests",
                    "hybrid search tests"
                ],
                "blocked_by": [
                    "final reviewed embedding model distribution decision",
                    "private business vector quality report",
                    "release model manifest evidence"
                ]
            },
            {
                "id": "P4_ocr",
                "label": "scanned PDF detection, OCR worker, cache, pause/resume, retry, OCR result indexing",
                "implementation_status": "production_complete",
                "release_status": "blocked",
                "evidence": [
                    "OCR worker tests",
                    "OCR manifest/preflight tests",
                    "current-stage smoke local runtime probe"
                ],
                "blocked_by": [
                    "stable-release OCR throughput evidence deferred to performance optimization goal",
                    "stable-release hot-index coverage evidence deferred to performance optimization goal",
                    "representative OCR backlog drain evidence deferred to performance optimization goal"
                ]
            },
            {
                "id": "P5_cross_platform_release",
                "label": "ordinary-user macOS and Windows Tauri v2 desktop installers",
                "implementation_status": "incomplete",
                "release_status": "blocked",
                "evidence": [
                    "legacy package automation is tracked separately and is not product installer proof",
                    "self-contained macOS arm64 DMG with exact daemon, embedding, model, OCR, and renderer closure",
                    "unsigned macOS /Applications install, first-run, data-preserving uninstall, reinstall, and LaunchServices discovery",
                    "unsigned macOS real-version upgrade and injected post-swap rollback with user-state preservation",
                    "partial Windows target-triple runtime composition contracts"
                ],
                "blocked_by": [
                    "Windows per-user NSIS self-contained runtime closure",
                    "Developer ID signed and notarized macOS clean-host upgrade, rollback, recovery, and Gatekeeper evidence",
                    "Windows per-user Tauri NSIS clean-H0 install, upgrade, uninstall, and rollback evidence",
                    "real signing/notarization credentials",
                    "signed updater artifacts served over HTTPS with mandatory signature verification",
                    "GitHub Release approval"
                ],
                "surfaces": {
                    "legacy_cli_package_automation": {
                        "implementation_status": "production_complete",
                        "product_scope": "legacy CLI/daemon package and lifecycle automation only",
                        "counts_as_tauri_desktop_installer": false,
                        "evidence": [
                            "unsigned dry-run package manifests",
                            "installer lifecycle dry-run plans",
                            "Windows Service lifecycle dry-run plan",
                            "signing/notarization fail-closed dry-run gates",
                            "hosted Rust workspace build/test workflows"
                        ]
                    },
                    "tauri_v2_desktop_installers": {
                        "implementation_status": "incomplete",
                        "product_scope": "ordinary-user macOS app/DMG and Windows per-user NSIS",
                        "counts_as_tauri_desktop_installer": true,
                        "evidence": [
                            "self-contained macOS arm64 DMG and exact daemon/model/OCR/PDF runtime composition",
                            "unsigned macOS /Applications install, first-run, data-preserving uninstall, reinstall, and LaunchServices discovery",
                            "unsigned macOS real-version upgrade and injected post-swap rollback with user-state preservation",
                            "Windows process-tree containment contract",
                            "Windows static-CRT embedding source contract",
                            "Windows static Tesseract OCR source contract",
                            "Windows static PDFium renderer source contract",
                            "partial target-triple runtime composition contracts"
                        ],
                        "missing": [
                            "macOS signed and notarized Tauri app/DMG",
                            "Windows per-user Tauri NSIS setup.exe",
                            "reviewed bundled daemon, embedding, model, OCR, and renderer closure on Windows",
                            "Developer ID signed and notarized clean-host real-version upgrade, rollback, recovery, and Gatekeeper evidence on macOS",
                            "signed clean-H0 real-version upgrade, rollback, and recovery evidence on Windows",
                            "signed HTTPS updater end-to-end evidence"
                        ]
                    }
                }
            },
            {
                "id": "P6_performance_stability",
                "label": "performance baseline, regression gates, fault injection, diagnostics, 100k/1M validation",
                "implementation_status": "deferred_to_performance_optimization_goal",
                "release_status": "blocked",
                "evidence": [
                    "benchmark runner tests",
                    "fault simulation tests",
                    "diagnostics redaction tests",
                    "current-stage smoke handoff"
                ],
                "blocked_by": [
                    "500-query/full hot-index baseline deferred to performance optimization goal",
                    "private labeled quality datasets",
                    "real hardware/platform fault drill transcripts",
                    "external 100k/1M real-corpus validation deferred to performance goal"
                ]
            }
        ]
    })
}
