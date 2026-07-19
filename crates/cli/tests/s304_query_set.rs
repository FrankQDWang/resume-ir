mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use benchmark_runner::{
    run_private_query_benchmark, PrivateQueryBenchmarkCommand, PrivateQueryBenchmarkConfig,
    PrivateQueryCorpusSummary, PrivateQueryManifestDigests,
};
use core_domain::{QuerySetSampleShape, QuerySetSourceKind, QUERY_SET_BUCKETS};
use meta_store::EntityType;
use privacy::ContactHasher;
use sha2::{Digest, Sha256};
use support::{assert_import_succeeded, import_existing_root};

#[test]
fn benchmark_query_set_rejects_removed_draft_source_path() {
    let data_dir = temp_dir("query-set-removed-draft-data");
    let out_dir = temp_dir("query-set-removed-draft-private-out");
    let query_set = out_dir.join("private-query-set.local.jsonl");
    seed_searchable_document_with_mentions(
        &data_dir,
        "removed-draft-private-resume.pdf",
        &[
            mention(
                EntityType::Title,
                "Search Infrastructure Engineer",
                "search_infrastructure_engineer",
                0.96,
            ),
            mention(EntityType::Skill, "Rust", "rust", 0.97),
            mention(EntityType::Skill, "Tantivy", "tantivy", 0.94),
            mention(EntityType::Location, "Singapore", "singapore", 0.93),
        ],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "draft",
            "--out",
            path_str(&query_set),
        ])
        .output()
        .expect("reject removed local-field query set draft source path");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("usage: resume-cli benchmark-query-set"));
    assert!(!stderr.contains("benchmark-query-set draft"));
    assert!(!stderr.contains("--allow-keyword-fallback"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&out_dir)));
    assert!(!query_set.exists());

    remove_dir(&data_dir);
    remove_dir(&out_dir);
}

#[test]
fn benchmark_query_set_preflight_agent_replay_creates_output_parent() {
    let data_dir = temp_dir("query-set-trace-preflight-parent-data");
    let out_dir = temp_dir("query-set-trace-preflight-parent-out");
    let trace_root = temp_dir("query-set-trace-preflight-parent-artifacts");
    let preflight = out_dir
        .join("nested")
        .join("query-set-trace-preflight.local.json");

    write_trace_log(
        &trace_root,
        "run-trace-preflight-parent",
        &["[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | rust backend"],
    );
    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "trace-preflight-parent-private-resume.pdf",
        &[],
        "rust backend",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "trace-preflight-parent-private-resume.pdf",
            "rust backend",
        )],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "preflight-agent-replay",
            "--out",
            path_str(&preflight),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "1",
        ])
        .output()
        .expect("write trace preflight into a fresh output parent");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("query set trace preflight: written"));
    for forbidden in [path_str(&data_dir), path_str(&trace_root), "rust backend"] {
        assert!(!stdout.contains(forbidden), "stdout leaked {forbidden}");
    }
    let preflight_text = fs::read_to_string(&preflight).unwrap();
    let preflight_json: serde_json::Value = serde_json::from_str(&preflight_text).unwrap();
    assert_eq!(
        preflight_json["schema_version"].as_str(),
        Some("resume-ir.query-set-trace-preflight.v1")
    );
    assert_eq!(
        preflight_json["contains_raw_query_text"].as_bool(),
        Some(false)
    );
    assert!(!preflight_text.contains(path_str(&trace_root)));
    assert!(!preflight_text.contains("rust backend"));
    #[cfg(unix)]
    {
        assert_eq!(file_mode(preflight.parent().unwrap()), 0o700);
        assert_eq!(file_mode(&preflight), 0o600);
    }

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_preflight_agent_replay_uses_query_artifact_root_env() {
    let data_dir = temp_dir("query-set-trace-preflight-env-data");
    let out_dir = temp_dir("query-set-trace-preflight-env-out");
    let trace_root = temp_dir("query-set-trace-preflight-env-artifacts");
    let preflight = out_dir.join("query-set-trace-preflight.local.json");

    write_trace_log(
        &trace_root,
        "run-trace-preflight-env",
        &["[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | rust backend"],
    );
    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "trace-preflight-env-private-resume.pdf",
        &[],
        "rust backend",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "trace-preflight-env-private-resume.pdf",
            "rust backend",
        )],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .env("RESUME_IR_QUERY_ARTIFACT_ROOT", path_str(&trace_root))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "preflight-agent-replay",
            "--out",
            path_str(&preflight),
            "--max-queries",
            "1",
        ])
        .output()
        .expect("preflight trace root from query artifact env");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("query set trace preflight: written"));
    for forbidden in [path_str(&data_dir), path_str(&trace_root), "rust backend"] {
        assert!(!stdout.contains(forbidden), "stdout leaked {forbidden}");
    }
    let preflight_text = fs::read_to_string(&preflight).unwrap();
    assert!(!preflight_text.contains(path_str(&trace_root)));
    assert!(!preflight_text.contains("rust backend"));

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_preflight_agent_replay_uses_local_evidence_dir_env_for_default_out() {
    let data_dir = temp_dir("query-set-trace-preflight-default-data");
    let evidence_dir = temp_dir("query-set-trace-preflight-default-evidence");
    let trace_root = temp_dir("query-set-trace-preflight-default-artifacts");
    let preflight = evidence_dir.join("query-set-trace-preflight.local.json");

    write_trace_log(
        &trace_root,
        "run-trace-preflight-default",
        &["[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | rust backend"],
    );
    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "trace-preflight-default-private-resume.pdf",
        &[],
        "rust backend",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "trace-preflight-default-private-resume.pdf",
            "rust backend",
        )],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .env("RESUME_IR_QUERY_ARTIFACT_ROOT", path_str(&trace_root))
        .env("RESUME_IR_LOCAL_EVIDENCE_DIR", path_str(&evidence_dir))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "preflight-agent-replay",
            "--max-queries",
            "1",
        ])
        .output()
        .expect("preflight writes default output under local evidence dir");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("query set trace preflight: written"));
    for forbidden in [
        path_str(&data_dir),
        path_str(&trace_root),
        path_str(&evidence_dir),
        "rust backend",
    ] {
        assert!(!stdout.contains(forbidden), "stdout leaked {forbidden}");
    }
    let preflight_text = fs::read_to_string(&preflight).unwrap();
    assert!(!preflight_text.contains(path_str(&trace_root)));
    assert!(!preflight_text.contains(path_str(&evidence_dir)));
    assert!(!preflight_text.contains("rust backend"));

    remove_dir(&data_dir);
    remove_dir(&evidence_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_preflight_agent_replay_rejects_default_output_inside_git_worktree() {
    let data_dir = temp_dir("query-set-trace-preflight-git-default-data");
    let git_root = temp_dir("query-set-trace-preflight-git-default-repo");
    let trace_root = temp_dir("query-set-trace-preflight-git-default-artifacts");
    fs::create_dir_all(git_root.join(".git")).unwrap();
    let preflight = git_root.join("query-set-trace-preflight.local.json");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .env("RESUME_IR_QUERY_ARTIFACT_ROOT", path_str(&trace_root))
        .env("RESUME_IR_LOCAL_EVIDENCE_DIR", path_str(&git_root))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "preflight-agent-replay",
            "--max-queries",
            "1",
        ])
        .output()
        .expect("reject default preflight output inside a git worktree");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(
        "query set blocked: local query artifacts must not be written inside a git worktree"
    ));
    for forbidden in [
        path_str(&data_dir),
        path_str(&git_root),
        path_str(&trace_root),
    ] {
        assert!(!stderr.contains(forbidden), "stderr leaked {forbidden}");
    }
    assert!(!preflight.exists());

    remove_dir(&data_dir);
    remove_dir(&git_root);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_preflight_agent_replay_reports_trace_counts_without_local_search_index() {
    let data_dir = temp_dir("query-set-trace-preflight-index-required-data");
    let out_dir = temp_dir("query-set-trace-preflight-index-required-out");
    let trace_root = temp_dir("query-set-trace-preflight-index-required-artifacts");
    let preflight = out_dir.join("query-set-trace-preflight.local.json");

    write_trace_log(
        &trace_root,
        "run-trace-preflight-index-required",
        &["[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | rust backend"],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "preflight-agent-replay",
            "--out",
            path_str(&preflight),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "1",
        ])
        .output()
        .expect("preflight trace query workload without local search index");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("query set trace preflight: written"));
    let preflight_text = fs::read_to_string(&preflight).unwrap();
    let value: serde_json::Value = serde_json::from_str(&preflight_text).unwrap();
    assert_eq!(value["query_index_available"].as_bool(), Some(false));
    assert_eq!(value["trace_logs"].as_u64(), Some(1));
    assert_eq!(value["trace_lines"].as_u64(), Some(1));
    assert_eq!(value["source_search_lines"].as_u64(), Some(1));
    assert_eq!(value["extracted_queries"].as_u64(), Some(1));
    assert_eq!(value["candidate_queries_sampled"].as_u64(), Some(1));
    assert_eq!(value["candidate_bucket_counts"]["and_2"].as_u64(), Some(1));
    assert_eq!(value["zero_hit_queries_dropped"].as_u64(), Some(0));
    assert_eq!(value["corpus_valid_queries"].as_u64(), Some(0));
    assert_eq!(
        value["corpus_valid_bucket_counts"]["and_2"].as_u64(),
        Some(0)
    );
    for forbidden in [path_str(&data_dir), path_str(&trace_root), "rust backend"] {
        assert!(!stdout.contains(forbidden), "stdout leaked {forbidden}");
        assert!(
            !preflight_text.contains(forbidden),
            "preflight artifact leaked {forbidden}"
        );
    }
    assert!(
        fs::read_dir(&data_dir).unwrap().next().is_none(),
        "read-only preflight must not create or migrate metadata"
    );

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_freeze_agent_replay_creates_private_output_parent_owner_only() {
    let data_dir = temp_dir("query-set-trace-freeze-parent-data");
    let out_dir = temp_dir("query-set-trace-freeze-parent-out");
    let trace_root = temp_dir("query-set-trace-freeze-parent-artifacts");
    let artifact_dir = out_dir.join("nested");
    let query_set = artifact_dir.join("private-query-set.local.jsonl");
    let summary = query_set_summary_path(&query_set);

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "alpha-private-resume.pdf",
        &[],
        "rust backend retrieval ranking",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "alpha-private-resume.pdf",
            "rust backend retrieval ranking",
        )],
    );
    write_trace_log(
        &trace_root,
        "run-trace-freeze-parent",
        &["[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | rust backend"],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--out",
            path_str(&query_set),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "1",
            "--min-queries",
            "1",
        ])
        .output()
        .expect("write frozen query set into private output parent");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    #[cfg(unix)]
    {
        assert_eq!(file_mode(&artifact_dir), 0o700);
        assert_eq!(file_mode(&query_set), 0o600);
        assert_eq!(file_mode(&summary), 0o600);
    }

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_freeze_agent_replay_rejects_output_inside_git_worktree() {
    let data_dir = temp_dir("query-set-trace-freeze-git-out-data");
    let git_root = temp_dir("query-set-trace-freeze-git-out-repo");
    let trace_root = temp_dir("query-set-trace-freeze-git-out-artifacts");
    fs::create_dir_all(git_root.join(".git")).unwrap();
    let query_set = git_root.join("private-query-set.local.jsonl");
    let summary = query_set_summary_path(&query_set);

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "trace-freeze-git-out-private-resume.pdf",
        &[],
        "rust backend",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "trace-freeze-git-out-private-resume.pdf",
            "rust backend",
        )],
    );
    write_trace_log(
        &trace_root,
        "run-trace-freeze-git-out",
        &["[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | rust backend"],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--out",
            path_str(&query_set),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "1",
            "--min-queries",
            "1",
        ])
        .output()
        .expect("reject query-set output inside a git worktree");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(
        "query set blocked: local query artifacts must not be written inside a git worktree"
    ));
    for forbidden in [
        path_str(&data_dir),
        path_str(&git_root),
        path_str(&trace_root),
        "rust backend",
    ] {
        assert!(!stderr.contains(forbidden), "stderr leaked {forbidden}");
    }
    assert!(!query_set.exists());
    assert!(!summary.exists());

    remove_dir(&data_dir);
    remove_dir(&git_root);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_freeze_agent_replay_uses_query_artifact_root_env() {
    let data_dir = temp_dir("query-set-trace-freeze-env-data");
    let out_dir = temp_dir("query-set-trace-freeze-env-out");
    let trace_root = temp_dir("query-set-trace-freeze-env-artifacts");
    let query_set = out_dir.join("private-query-set.local.jsonl");
    let summary = query_set_summary_path(&query_set);

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "alpha-private-resume.pdf",
        &[],
        "rust backend retrieval ranking",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "alpha-private-resume.pdf",
            "rust backend retrieval ranking",
        )],
    );
    write_trace_log(
        &trace_root,
        "run-trace-freeze-env",
        &["[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | rust backend"],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .env("RESUME_IR_QUERY_ARTIFACT_ROOT", path_str(&trace_root))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--out",
            path_str(&query_set),
            "--max-queries",
            "1",
            "--min-queries",
            "1",
        ])
        .output()
        .expect("freeze trace root from query artifact env");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("query set: frozen"));
    for forbidden in [path_str(&data_dir), path_str(&trace_root), "rust backend"] {
        assert!(!stdout.contains(forbidden), "stdout leaked {forbidden}");
    }
    assert!(query_set.exists());
    let summary_text = fs::read_to_string(&summary).unwrap();
    assert!(!summary_text.contains(path_str(&trace_root)));
    assert!(!summary_text.contains("rust backend"));

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_freeze_agent_replay_uses_local_evidence_dir_env_for_default_out() {
    let data_dir = temp_dir("query-set-trace-freeze-default-data");
    let evidence_dir = temp_dir("query-set-trace-freeze-default-evidence");
    let trace_root = temp_dir("query-set-trace-freeze-default-artifacts");
    let query_set = evidence_dir.join("private-query-set.local.jsonl");
    let summary = query_set_summary_path(&query_set);

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "alpha-private-resume.pdf",
        &[],
        "rust backend retrieval ranking",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "alpha-private-resume.pdf",
            "rust backend retrieval ranking",
        )],
    );
    write_trace_log(
        &trace_root,
        "run-trace-freeze-default",
        &["[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | rust backend"],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .env("RESUME_IR_QUERY_ARTIFACT_ROOT", path_str(&trace_root))
        .env("RESUME_IR_LOCAL_EVIDENCE_DIR", path_str(&evidence_dir))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--max-queries",
            "1",
            "--min-queries",
            "1",
        ])
        .output()
        .expect("freeze writes default output under local evidence dir");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("query set: frozen"));
    for forbidden in [
        path_str(&data_dir),
        path_str(&trace_root),
        path_str(&evidence_dir),
        "rust backend",
    ] {
        assert!(!stdout.contains(forbidden), "stdout leaked {forbidden}");
    }
    assert!(query_set.exists());
    assert!(summary.exists());
    let summary_text = fs::read_to_string(&summary).unwrap();
    assert!(!summary_text.contains(path_str(&trace_root)));
    assert!(!summary_text.contains(path_str(&evidence_dir)));
    assert!(!summary_text.contains("rust backend"));
    #[cfg(unix)]
    {
        assert_eq!(file_mode(&evidence_dir), 0o700);
        assert_eq!(file_mode(&query_set), 0o600);
        assert_eq!(file_mode(&summary), 0o600);
    }

    remove_dir(&data_dir);
    remove_dir(&evidence_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_freeze_agent_replay_keeps_only_corpus_valid_source_search_queries() {
    let data_dir = temp_dir("query-set-trace-root-data");
    let out_dir = temp_dir("query-set-trace-root-out");
    let trace_root = temp_dir("query-set-trace-root-artifacts");
    let query_set = out_dir.join("private-query-set.local.jsonl");
    let summary = query_set_summary_path(&query_set);

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "alpha-private-resume.pdf",
        &[],
        "rust backend shanghai retrieval ranking",
    );
    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "beta-private-resume.pdf",
        &[],
        "java architect beijing platform search",
    );
    seed_fulltext_index(
        &data_dir,
        vec![
            index_document(
                "alpha-private-resume.pdf",
                "rust backend shanghai retrieval ranking",
            ),
            index_document(
                "beta-private-resume.pdf",
                "java architect beijing platform search",
            ),
        ],
    );
    write_trace_log(
        &trace_root,
        "run-trace-valid",
        &[
            "[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | ｒｕｓｔ shanghai rust",
            "[2026-06-05T12:09:21+08:00] | tool_called | round=1 | tool=browser_open | ignore me",
            "[2026-06-05T12:09:22+08:00] | tool_called | round=1 | tool=source_search | java beijing",
            "[2026-06-05T12:09:23+08:00] | tool_called | round=1 | tool=source_search | unicorn nohit",
            "[2026-06-05T12:09:24+08:00] | source_plan_created | Runtime source plan created.",
        ],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--out",
            path_str(&query_set),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "2",
            "--min-queries",
            "2",
        ])
        .output()
        .expect("freeze agent replay query set");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("query set: frozen"));
    assert!(stdout.contains("schema: resume-ir.query-set.jsonl.v2"));
    assert!(stdout.contains("query set summary: written"));
    assert!(stdout.contains("query source: trace_source_search_v1"));
    assert!(stdout.contains("candidate queries sampled: "));
    assert!(stdout.contains("zero-hit queries dropped: "));
    assert!(!stdout.contains("query fallback"));
    assert!(stdout.contains("hmac split: true"));
    assert!(stdout.contains("queries: <redacted>"));
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        path_str(&trace_root),
        "ｒｕｓｔ shanghai rust",
        "rust shanghai",
        "java beijing",
        "unicorn nohit",
    ] {
        assert!(!stdout.contains(forbidden), "stdout leaked {forbidden}");
    }

    let query_set_text = fs::read_to_string(&query_set).unwrap();
    let raw_query_set_sha256 = file_sha256_hex(&query_set);
    let lines = query_set_text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    let values = lines
        .iter()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    let queries = values
        .iter()
        .map(|value| value["query"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(queries, vec!["rust shanghai", "java beijing"]);
    for value in &values {
        assert_eq!(
            value["schema_version"].as_str(),
            Some("resume-ir.query-set.jsonl.v2")
        );
        assert_eq!(
            value["source_kind"].as_str(),
            Some("trace_source_search_v1")
        );
        assert_eq!(value["bucket"].as_str(), Some("field_filter"));
        assert_eq!(value["query_shape"]["term_count"].as_u64(), Some(2));
        assert_eq!(value["query_shape"]["has_location"].as_bool(), Some(true));
        assert_eq!(value["query_shape"]["has_boolean"].as_bool(), Some(false));
        assert_eq!(value["query_shape"]["has_phrase"].as_bool(), Some(false));
    }
    assert!(!query_set_text.contains("unicorn nohit"));

    let summary_text = fs::read_to_string(&summary).unwrap();
    let summary_value: serde_json::Value = serde_json::from_str(&summary_text).unwrap();
    assert_eq!(
        summary_value["schema_version"].as_str(),
        Some("resume-ir.query-set-summary.v2")
    );
    assert_eq!(
        summary_value["privacy_boundary"].as_str(),
        Some("redacted_local_aggregate")
    );
    assert_eq!(
        summary_value["query_source"].as_str(),
        Some("trace_source_search_v1")
    );
    assert_eq!(summary_value["query_count"].as_u64(), Some(2));
    assert_eq!(
        summary_value["bucket_counts"]["field_filter"].as_u64(),
        Some(2)
    );
    assert_eq!(summary_value["bucket_counts"]["and_2"].as_u64(), Some(0));
    assert_eq!(
        summary_value["bucket_counts"]["single_term"].as_u64(),
        Some(0)
    );
    assert_eq!(summary_value["candidate_queries_sampled"].as_u64(), Some(2));
    assert_eq!(summary_value["zero_hit_queries_dropped"].as_u64(), Some(0));
    assert!(summary_value.get("query_fallback").is_none());
    assert_eq!(summary_value["hmac_split"].as_bool(), Some(true));
    let query_set_sha256 = summary_value["query_set_sha256"].as_str().unwrap();
    let tune_sha256 = summary_value["tune_sha256"].as_str().unwrap();
    let holdout_sha256 = summary_value["holdout_sha256"].as_str().unwrap();
    assert_eq!(query_set_sha256.len(), 64);
    assert_eq!(tune_sha256.len(), 64);
    assert_eq!(holdout_sha256.len(), 64);
    assert!(query_set_sha256
        .chars()
        .all(|character| character.is_ascii_hexdigit()));
    assert!(tune_sha256
        .chars()
        .all(|character| character.is_ascii_hexdigit()));
    assert!(holdout_sha256
        .chars()
        .all(|character| character.is_ascii_hexdigit()));
    assert_ne!(query_set_sha256, raw_query_set_sha256);
    let hasher = ContactHasher::load_or_create(&data_dir).unwrap();
    let (expected_tune_queries, expected_holdout_queries) =
        expected_query_set_split(&hasher, &queries);
    assert_eq!(
        query_set_sha256,
        hasher
            .hmac_hex(
                "resume-ir:query-set-summary:v2:all",
                &expected_query_set_hmac_payload(&queries),
            )
            .unwrap()
    );
    assert_eq!(
        tune_sha256,
        hasher
            .hmac_hex(
                "resume-ir:query-set-summary:v2:tune",
                &expected_query_set_hmac_payload(&expected_tune_queries),
            )
            .unwrap()
    );
    assert_eq!(
        holdout_sha256,
        hasher
            .hmac_hex(
                "resume-ir:query-set-summary:v2:holdout",
                &expected_query_set_hmac_payload(&expected_holdout_queries),
            )
            .unwrap()
    );
    assert_ne!(
        query_set_sha256,
        hasher
            .hmac_hex(
                "resume-ir:query-set-summary:v1:all",
                &expected_query_set_hmac_payload(&queries),
            )
            .unwrap()
    );
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        path_str(&trace_root),
        "rust shanghai",
        "java beijing",
        "rust backend",
        "java architect",
        "unicorn nohit",
    ] {
        assert!(
            !summary_text.contains(forbidden),
            "summary leaked {forbidden}"
        );
    }
    #[cfg(unix)]
    {
        assert_eq!(file_mode(&query_set), 0o600);
        assert_eq!(file_mode(&summary), 0o600);
    }

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_freeze_agent_replay_output_runs_private_query_runner() {
    let data_dir = temp_dir("query-set-trace-runner-handoff-data");
    let out_dir = temp_dir("query-set-trace-runner-handoff-out");
    let trace_root = temp_dir("query-set-trace-runner-handoff-artifacts");
    let query_set = out_dir.join("private-query-set.local.jsonl");
    let summary = query_set_summary_path(&query_set);

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "handoff-alpha-private-resume.pdf",
        &[],
        "rust backend shanghai retrieval ranking",
    );
    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "handoff-beta-private-resume.pdf",
        &[],
        "java architect beijing platform search",
    );
    seed_fulltext_index(
        &data_dir,
        vec![
            index_document(
                "handoff-alpha-private-resume.pdf",
                "rust backend shanghai retrieval ranking",
            ),
            index_document(
                "handoff-beta-private-resume.pdf",
                "java architect beijing platform search",
            ),
        ],
    );
    write_trace_log(
        &trace_root,
        "run-trace-runner-handoff",
        &[
            "[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | rust shanghai",
            "[2026-06-05T12:09:21+08:00] | tool_called | round=1 | tool=source_search | java beijing",
        ],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--out",
            path_str(&query_set),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "2",
            "--min-queries",
            "2",
        ])
        .output()
        .expect("freeze query set for private-query runner handoff");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let summary_text = fs::read_to_string(&summary).unwrap();
    let summary_value: serde_json::Value = serde_json::from_str(&summary_text).unwrap();
    let query_set_sha256 = summary_value["query_set_sha256"].as_str().unwrap();
    let command = query_handoff_fixture_script("query-set-trace-runner-handoff-command");
    let corpus_summary =
        PrivateQueryCorpusSummary::from_redacted_json_bytes(private_query_corpus_summary_json(2))
            .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(2)
    .unwrap()
    .with_request_sample_count(2)
    .unwrap()
    .with_top_k(5)
    .unwrap()
    .with_timeout_ms(5_000)
    .unwrap()
    .with_synthetic_smoke_evidence();

    let report = run_private_query_benchmark(config).unwrap();
    let report_json = report.to_redacted_json();
    let report_value: serde_json::Value = serde_json::from_str(&report_json).unwrap();

    assert_eq!(report_value["dataset_kind"], "synthetic-smoke");
    assert_eq!(report_value["target_claim"], "not_evaluated");
    assert_eq!(report_value["query_runner"], "resident-batch-command");
    assert_eq!(report_value["spawn_per_query"], false);
    assert_eq!(report_value["query_protocol"], "resume-ir-query-v2");
    assert_eq!(report_value["query_source"], "trace_source_search_v1");
    assert_eq!(report_value["query_set_sha256"], query_set_sha256);
    assert_eq!(report_value["query_count"], 2);
    assert_eq!(report_value["request_sample_count"], 2);
    assert_eq!(report_value["bucket_counts"]["field_filter"], 2);
    assert_eq!(report_value["samples_per_bucket"]["field_filter"], 2);
    assert_eq!(report_value["query_embedding_command_invocations"], 2);
    assert_eq!(report_value["zero_result_queries"], 0);
    assert_eq!(report_value["private_scale_gate"], serde_json::Value::Null);
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        path_str(&trace_root),
        path_str(&command),
        "rust shanghai",
        "java beijing",
        "handoff-alpha-private-resume",
        "handoff-beta-private-resume",
    ] {
        assert!(
            !report_json.contains(forbidden),
            "private-query report leaked {forbidden}"
        );
    }

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
    remove_dir(command.parent().unwrap());
}

#[test]
fn benchmark_query_set_freeze_agent_replay_uses_immediate_source_search_keyword_segment() {
    let data_dir = temp_dir("query-set-trace-summary-segment-data");
    let out_dir = temp_dir("query-set-trace-summary-segment-out");
    let trace_root = temp_dir("query-set-trace-summary-segment-artifacts");
    let query_set = out_dir.join("private-query-set.local.jsonl");

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "alpha-private-resume.pdf",
        &[],
        "rust backend retrieval ranking",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "alpha-private-resume.pdf",
            "rust backend retrieval ranking",
        )],
    );
    write_trace_log(
        &trace_root,
        "run-trace-summary-with-metadata",
        &[
            "[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | rust backend | status=ok",
        ],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--out",
            path_str(&query_set),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "1",
            "--min-queries",
            "1",
        ])
        .output()
        .expect("freeze agent replay query set from immediate summary");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let query_set_contents = fs::read_to_string(&query_set).expect("query set should be written");
    let sample: serde_json::Value =
        serde_json::from_str(query_set_contents.lines().next().unwrap()).unwrap();
    assert_eq!(sample["query"], "rust backend");
    assert_eq!(sample["source_kind"], "trace_source_search_v1");
    assert!(!query_set_contents.contains("status=ok"));

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_freeze_agent_replay_keeps_safe_slash_skill_queries() {
    let data_dir = temp_dir("query-set-trace-root-safe-slash-data");
    let out_dir = temp_dir("query-set-trace-root-safe-slash-out");
    let trace_root = temp_dir("query-set-trace-root-safe-slash-artifacts");
    let query_set = out_dir.join("private-query-set.local.jsonl");

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "slash-skill-private-resume.pdf",
        &[],
        "ai ml shanghai backend retrieval ranking",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "slash-skill-private-resume.pdf",
            "ai ml shanghai backend retrieval ranking",
        )],
    );
    write_trace_log(
        &trace_root,
        "run-trace-safe-slash",
        &[
            "[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | ai/ml shanghai",
            "[2026-06-05T12:09:21+08:00] | tool_called | round=1 | tool=source_search | /Users/private resume",
        ],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--out",
            path_str(&query_set),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "1",
            "--min-queries",
            "1",
        ])
        .output()
        .expect("freeze safe slash agent replay query set");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("query source: trace_source_search_v1"));
    assert!(stdout.contains("queries: <redacted>"));
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        path_str(&trace_root),
        "ai/ml shanghai",
        "/Users/private",
    ] {
        assert!(!stdout.contains(forbidden), "stdout leaked {forbidden}");
    }

    let query_set_text = fs::read_to_string(&query_set).unwrap();
    let lines = query_set_text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    let value: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(value["query"].as_str(), Some("ai/ml shanghai"));
    assert!(!query_set_text.contains("/Users/private"));

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_freeze_agent_replay_counts_quoted_phrase_as_one_term() {
    let data_dir = temp_dir("query-set-trace-root-quoted-term-data");
    let out_dir = temp_dir("query-set-trace-root-quoted-term-out");
    let trace_root = temp_dir("query-set-trace-root-quoted-term-artifacts");
    let query_set = out_dir.join("private-query-set.local.jsonl");
    let query = "\"machine learning\" backend search ranking vector rust python java go shanghai beijing data platform index retrieval scala";

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "quoted-term-private-resume.pdf",
        &[],
        "machine learning backend search ranking vector rust python java go shanghai beijing data platform index retrieval scala",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "quoted-term-private-resume.pdf",
            "machine learning backend search ranking vector rust python java go shanghai beijing data platform index retrieval scala",
        )],
    );
    write_trace_log(
        &trace_root,
        "run-trace-quoted-term",
        &[&source_search_trace_line(query)],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--out",
            path_str(&query_set),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "1",
            "--min-queries",
            "1",
        ])
        .output()
        .expect("freeze quoted phrase agent replay query set");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("query source: trace_source_search_v1"));
    assert!(stdout.contains("queries: <redacted>"));
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        path_str(&trace_root),
        "machine learning",
        "quoted-term-private-resume",
    ] {
        assert!(!stdout.contains(forbidden), "stdout leaked {forbidden}");
    }

    let query_set_text = fs::read_to_string(&query_set).unwrap();
    let lines = query_set_text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    let value: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(value["query"].as_str(), Some(query));
    assert_eq!(value["query_shape"]["term_count"].as_u64(), Some(16));
    assert_eq!(value["query_shape"]["has_phrase"].as_bool(), Some(true));
    assert_eq!(value["bucket"].as_str(), Some("semantic"));

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_freeze_agent_replay_keeps_lowercase_connector_in_plain_bucket() {
    let data_dir = temp_dir("query-set-trace-root-lowercase-connector-data");
    let out_dir = temp_dir("query-set-trace-root-lowercase-connector-out");
    let trace_root = temp_dir("query-set-trace-root-lowercase-connector-artifacts");
    let query_set = out_dir.join("private-query-set.local.jsonl");
    let query = "research and development backend";

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "lowercase-connector-private-resume.pdf",
        &[],
        "research and development backend search platform",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "lowercase-connector-private-resume.pdf",
            "research and development backend search platform",
        )],
    );
    write_trace_log(
        &trace_root,
        "run-trace-lowercase-connector",
        &[&source_search_trace_line(query)],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--out",
            path_str(&query_set),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "1",
            "--min-queries",
            "1",
        ])
        .output()
        .expect("freeze lowercase connector agent replay query set");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("query source: trace_source_search_v1"));
    assert!(stdout.contains("queries: <redacted>"));
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        path_str(&trace_root),
        "research and development",
        "lowercase-connector-private-resume",
    ] {
        assert!(!stdout.contains(forbidden), "stdout leaked {forbidden}");
    }

    let query_set_text = fs::read_to_string(&query_set).unwrap();
    let lines = query_set_text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    let value: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(value["query"].as_str(), Some(query));
    assert_eq!(value["query_shape"]["term_count"].as_u64(), Some(4));
    assert_eq!(value["query_shape"]["has_boolean"].as_bool(), Some(false));
    assert_eq!(value["bucket"].as_str(), Some("and_3_5"));

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_freeze_agent_replay_applies_nfkc_before_static_selection() {
    let data_dir = temp_dir("query-set-trace-root-nfkc-data");
    let out_dir = temp_dir("query-set-trace-root-nfkc-out");
    let trace_root = temp_dir("query-set-trace-root-nfkc-artifacts");
    let query_set = out_dir.join("private-query-set.local.jsonl");

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "nfkc-private-resume.pdf",
        &[],
        "rust shanghai backend search platform",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "nfkc-private-resume.pdf",
            "rust shanghai backend search platform",
        )],
    );
    write_trace_log(
        &trace_root,
        "run-trace-nfkc",
        &["[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | ｒｕｓｔ shanghai"],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--out",
            path_str(&query_set),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "1",
            "--min-queries",
            "1",
        ])
        .output()
        .expect("freeze NFKC-normalized agent replay query set");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("query source: trace_source_search_v1"));
    assert!(stdout.contains("queries: <redacted>"));
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        path_str(&trace_root),
        "ｒｕｓｔ",
        "rust shanghai",
        "nfkc-private-resume",
    ] {
        assert!(!stdout.contains(forbidden), "stdout leaked {forbidden}");
    }

    let query_set_text = fs::read_to_string(&query_set).unwrap();
    let lines = query_set_text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    let value: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(value["query"].as_str(), Some("rust shanghai"));
    assert_eq!(value["query_shape"]["term_count"].as_u64(), Some(2));
    assert_eq!(value["query_shape"]["has_location"].as_bool(), Some(true));
    assert_eq!(value["bucket"].as_str(), Some("field_filter"));
    assert!(!query_set_text.contains("ｒｕｓｔ"));

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_freeze_agent_replay_ignores_structured_payload_envelopes() {
    let data_dir = temp_dir("query-set-trace-root-structured-data");
    let out_dir = temp_dir("query-set-trace-root-structured-out");
    let trace_root = temp_dir("query-set-trace-root-structured-artifacts");
    let query_set = out_dir.join("private-query-set.local.jsonl");

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "structured-private-resume.pdf",
        &[],
        "rust backend shanghai retrieval ranking",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "structured-private-resume.pdf",
            "rust backend shanghai retrieval ranking",
        )],
    );
    write_trace_log(
        &trace_root,
        "run-trace-structured",
        &[
            r#"[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | call=call_123 | args={"query":"rust shanghai","limit":20}"#,
        ],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--out",
            path_str(&query_set),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "1",
            "--min-queries",
            "1",
        ])
        .output()
        .expect("reject structured payload envelope as query keyword source");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(
        "query set blocked: not enough corpus-valid trace queries for the current indexed corpus"
    ));
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        path_str(&trace_root),
        "rust shanghai",
        "args=",
        "call_123",
    ] {
        assert!(!stderr.contains(forbidden), "stderr leaked {forbidden}");
    }
    assert!(!query_set.exists());

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_freeze_agent_replay_rejects_metadata_in_keyword_segment() {
    let data_dir = temp_dir("query-set-trace-root-post-tool-metadata-data");
    let out_dir = temp_dir("query-set-trace-root-post-tool-metadata-out");
    let trace_root = temp_dir("query-set-trace-root-post-tool-metadata-artifacts");
    let query_set = out_dir.join("private-query-set.local.jsonl");

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "post-tool-metadata-private-resume.pdf",
        &[],
        "rust backend retrieval ranking",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "post-tool-metadata-private-resume.pdf",
            "rust backend retrieval ranking",
        )],
    );
    write_trace_log(
        &trace_root,
        "run-trace-post-tool-metadata",
        &[
            "[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | status=ok | rust backend",
        ],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--out",
            path_str(&query_set),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "1",
            "--min-queries",
            "1",
        ])
        .output()
        .expect("reject metadata segment as static replay query source");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(
        "query set blocked: not enough corpus-valid trace queries for the current indexed corpus"
    ));
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        path_str(&trace_root),
        "rust backend",
        "status=ok",
        "post-tool-metadata-private-resume",
    ] {
        assert!(!stderr.contains(forbidden), "stderr leaked {forbidden}");
    }
    assert!(!query_set.exists());

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_preflight_agent_replay_reports_redacted_corpus_valid_shape() {
    let data_dir = temp_dir("query-set-trace-preflight-data");
    let out_dir = temp_dir("query-set-trace-preflight-out");
    let trace_root = temp_dir("query-set-trace-preflight-artifacts");
    let preflight = out_dir.join("query-set-trace-preflight.local.json");
    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "trace-preflight-private-resume.pdf",
        &[],
        "rust backend semantic systems senior java",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "trace-preflight-private-resume.pdf",
            "rust backend semantic systems senior java",
        )],
    );

    write_trace_log(
        &trace_root,
        "run-trace-preflight",
        &[
            "[2026-06-05T12:09:19+08:00] | tool_called | round=1 | tool=browser_open | ignore me",
            r#"[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | call=call_123 | args={"query":"rust backend","limit":20}"#,
            "[2026-06-05T12:09:21+08:00] | tool_called | round=1 | tool=source_search | /Users/private resume",
            "[2026-06-05T12:09:22+08:00] | tool_called | round=1 | tool=source_search | rust backend",
            "[2026-06-05T12:09:23+08:00] | tool_called | round=1 | tool=source_search | rust backend",
            "[2026-06-05T12:09:24+08:00] | tool_called | round=1 | tool=source_search | missingtoken absenttoken",
            "[2026-06-05T12:09:25+08:00] | tool_called | round=1 | tool=source_search | \"semantic systems\"",
            "[2026-06-05T12:09:26+08:00] | tool_called | round=1 | tool=source_search | senior AND java",
        ],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "preflight-agent-replay",
            "--out",
            path_str(&preflight),
            "--trace-root",
            path_str(&trace_root),
        ])
        .output()
        .expect("preflight agent replay query set");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("query set trace preflight: written"));
    assert!(stdout.contains("schema: resume-ir.query-set-trace-preflight.v1"));
    assert!(stdout.contains("privacy boundary: redacted_local_aggregate"));
    assert!(stdout.contains("queries: <redacted>"));
    let preflight_text = fs::read_to_string(&preflight).unwrap();
    let value: serde_json::Value = serde_json::from_str(&preflight_text).unwrap();
    assert_eq!(
        value["schema_version"].as_str(),
        Some("resume-ir.query-set-trace-preflight.v1")
    );
    assert_eq!(
        value["privacy_boundary"].as_str(),
        Some("redacted_local_aggregate")
    );
    assert_eq!(
        value["query_source"].as_str(),
        Some("trace_source_search_v1")
    );
    assert_eq!(value["target_query_count"].as_u64(), Some(500));
    assert_eq!(value["query_index_available"].as_bool(), Some(true));
    assert_eq!(value["document_count"].as_u64(), Some(1));
    assert_eq!(value["searchable_document_count"].as_u64(), Some(1));
    assert_eq!(value["vector_indexed_document_count"].as_u64(), Some(0));
    assert_eq!(value["d10k_min_document_count"].as_u64(), Some(10_000));
    assert_eq!(
        value["d10k_min_searchable_document_count"].as_u64(),
        Some(8_000)
    );
    assert_eq!(
        value["d10k_min_vector_indexed_document_count"].as_u64(),
        Some(8_000)
    );
    assert_eq!(value["d10k_corpus_ready"].as_bool(), Some(false));
    assert_eq!(
        value["d10k_corpus_deficits"]["document_count"].as_u64(),
        Some(9_999)
    );
    assert_eq!(
        value["d10k_corpus_deficits"]["searchable_document_count"].as_u64(),
        Some(7_999)
    );
    assert_eq!(
        value["d10k_corpus_deficits"]["vector_indexed_document_count"].as_u64(),
        Some(8_000)
    );
    assert_eq!(value["trace_logs"].as_u64(), Some(1));
    assert_eq!(value["trace_lines"].as_u64(), Some(8));
    assert_eq!(value["source_search_lines"].as_u64(), Some(7));
    assert_eq!(value["extracted_queries"].as_u64(), Some(6));
    assert_eq!(value["normalization_rejected"].as_u64(), Some(1));
    assert_eq!(value["duplicate_queries_dropped"].as_u64(), Some(1));
    assert_eq!(value["candidate_queries_sampled"].as_u64(), Some(4));
    assert_eq!(value["candidate_bucket_counts"]["and_2"].as_u64(), Some(2));
    assert_eq!(
        value["candidate_bucket_counts"]["semantic"].as_u64(),
        Some(1)
    );
    assert_eq!(value["candidate_bucket_counts"]["hybrid"].as_u64(), Some(1));
    assert_eq!(
        value["candidate_bucket_deficits"]["and_2"].as_u64(),
        Some(73)
    );
    assert_eq!(
        value["candidate_bucket_deficits"]["semantic"].as_u64(),
        Some(24)
    );
    assert_eq!(value["zero_hit_queries_dropped"].as_u64(), Some(1));
    assert_eq!(value["corpus_valid_queries"].as_u64(), Some(3));
    assert_eq!(
        value["corpus_valid_bucket_counts"]["and_2"].as_u64(),
        Some(1)
    );
    assert_eq!(
        value["corpus_valid_bucket_counts"]["field_filter"].as_u64(),
        Some(0)
    );
    assert_eq!(
        value["corpus_valid_bucket_counts"]["semantic"].as_u64(),
        Some(1)
    );
    assert_eq!(
        value["corpus_valid_bucket_counts"]["hybrid"].as_u64(),
        Some(1)
    );
    assert_eq!(
        value["required_bucket_counts"]["single_term"].as_u64(),
        Some(50)
    );
    assert_eq!(
        value["corpus_valid_bucket_deficits"]["single_term"].as_u64(),
        Some(50)
    );
    assert_eq!(
        value["corpus_valid_bucket_deficits"]["and_3_5"].as_u64(),
        Some(150)
    );
    assert_eq!(
        value["corpus_valid_bucket_deficits"]["field_filter"].as_u64(),
        Some(75)
    );
    assert_eq!(value["contains_raw_query_text"].as_bool(), Some(false));
    assert_eq!(value["contains_raw_resume_text"].as_bool(), Some(false));
    assert_eq!(value["contains_candidate_results"].as_bool(), Some(false));
    assert_eq!(value["contains_local_paths"].as_bool(), Some(false));
    for forbidden in [
        path_str(&data_dir),
        path_str(&trace_root),
        "rust backend",
        "missingtoken absenttoken",
        "semantic systems",
        "senior AND java",
        "/Users/private",
        "call_123",
        "trace-preflight-private-resume",
    ] {
        assert!(!stdout.contains(forbidden), "stdout leaked {forbidden}");
        assert!(
            !preflight_text.contains(forbidden),
            "preflight artifact leaked {forbidden}"
        );
    }
    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_preflight_agent_replay_rejects_key_value_metadata_segment() {
    let data_dir = temp_dir("query-set-trace-preflight-key-value-data");
    let out_dir = temp_dir("query-set-trace-preflight-key-value-out");
    let trace_root = temp_dir("query-set-trace-preflight-key-value-artifacts");
    let preflight = out_dir.join("query-set-trace-preflight.local.json");

    write_trace_log(
        &trace_root,
        "run-trace-preflight-key-value",
        &[
            "[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | query=rust backend",
        ],
    );
    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "trace-preflight-key-value-private-resume.pdf",
        &[],
        "rust backend",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "trace-preflight-key-value-private-resume.pdf",
            "rust backend",
        )],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "preflight-agent-replay",
            "--out",
            path_str(&preflight),
            "--trace-root",
            path_str(&trace_root),
        ])
        .output()
        .expect("preflight agent replay query set");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("query set trace preflight: written"));
    let preflight_text = fs::read_to_string(&preflight).unwrap();
    let value: serde_json::Value = serde_json::from_str(&preflight_text).unwrap();
    assert_eq!(value["source_search_lines"].as_u64(), Some(1));
    assert_eq!(value["extracted_queries"].as_u64(), Some(0));
    assert_eq!(value["candidate_queries_sampled"].as_u64(), Some(0));
    assert_eq!(
        value["corpus_valid_bucket_counts"]["and_2"].as_u64(),
        Some(0)
    );
    for forbidden in [path_str(&data_dir), path_str(&trace_root), "rust backend"] {
        assert!(!stdout.contains(forbidden), "stdout leaked {forbidden}");
        assert!(
            !preflight_text.contains(forbidden),
            "preflight artifact leaked {forbidden}"
        );
    }

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_preflight_agent_replay_rejects_oversized_trace_line() {
    let data_dir = temp_dir("query-set-trace-preflight-oversized-data");
    let out_dir = temp_dir("query-set-trace-preflight-oversized-out");
    let trace_root = temp_dir("query-set-trace-preflight-oversized-artifacts");
    let preflight = out_dir.join("query-set-trace-preflight.local.json");
    let oversized_query = "rust ".repeat(20_000);
    let oversized_line =
        format!("[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | {oversized_query}");

    write_trace_log_strings(
        &trace_root,
        "run-trace-preflight-oversized",
        &[oversized_line],
    );
    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "trace-preflight-oversized-private-resume.pdf",
        &[],
        "rust backend",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "trace-preflight-oversized-private-resume.pdf",
            "rust backend",
        )],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "preflight-agent-replay",
            "--out",
            path_str(&preflight),
            "--trace-root",
            path_str(&trace_root),
        ])
        .output()
        .expect("reject oversized preflight trace line");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("query set blocked: trace log line is too large"));
    for forbidden in [path_str(&data_dir), path_str(&trace_root), "rust rust"] {
        assert!(!stderr.contains(forbidden), "stderr leaked {forbidden}");
    }
    assert!(!preflight.exists());

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_freeze_agent_replay_rejects_zero_hit_full_freeze_bucket_fill() {
    let data_dir = temp_dir("query-set-trace-root-zero-hit-bucket-data");
    let out_dir = temp_dir("query-set-trace-root-zero-hit-bucket-out");
    let trace_root = temp_dir("query-set-trace-root-zero-hit-bucket-artifacts");
    let query_set = out_dir.join("private-query-set.local.jsonl");

    let mut lines = Vec::new();
    let mut hit_query_text = String::new();
    for index in 0..50 {
        let query = full_freeze_query("single", index, 1);
        hit_query_text.push_str(&query);
        hit_query_text.push(' ');
        lines.push(source_search_trace_line(&query));
    }
    for index in 0..75 {
        let query = full_freeze_query("andtwo", index, 2);
        hit_query_text.push_str(&query);
        hit_query_text.push(' ');
        lines.push(source_search_trace_line(&query));
    }
    for index in 0..125 {
        let query = full_freeze_query("andthreefive", index, 3);
        hit_query_text.push_str(&query);
        hit_query_text.push(' ');
        lines.push(source_search_trace_line(&query));
    }
    for index in 0..50 {
        let query = full_freeze_query("andsix", index, 6);
        hit_query_text.push_str(&query);
        hit_query_text.push(' ');
        lines.push(source_search_trace_line(&query));
    }
    for index in 0..75 {
        let id = alpha_id(index);
        let query = format!("field{id} shanghai backend{id}");
        hit_query_text.push_str(&query);
        hit_query_text.push(' ');
        lines.push(source_search_trace_line(&query));
    }
    for index in 0..75 {
        let id = alpha_id(index);
        let query = format!("hybrid{id} AND backend{id}");
        hit_query_text.push_str(&query);
        hit_query_text.push(' ');
        lines.push(source_search_trace_line(&query));
    }
    for index in 0..25 {
        let id = alpha_id(index);
        let query = format!("\"semantic{id} systems{id}\"");
        hit_query_text.push_str(&query.replace('"', " "));
        hit_query_text.push(' ');
        lines.push(source_search_trace_line(&query));
    }
    for index in 0..25 {
        let id = alpha_id(index);
        lines.push(source_search_trace_line(&format!("cold{id} missing{id}")));
    }

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "zero-hit-bucket-private-resume.pdf",
        &[],
        &hit_query_text,
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "zero-hit-bucket-private-resume.pdf",
            &hit_query_text,
        )],
    );
    write_trace_log_strings(&trace_root, "run-trace-zero-hit-bucket", &lines);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--out",
            path_str(&query_set),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "499",
            "--min-queries",
            "499",
        ])
        .output()
        .expect("reject full agent replay query set filled by zero-hit queries");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(
        "query set blocked: not enough corpus-valid trace queries for the current indexed corpus"
    ));
    assert!(stderr.contains("zero_hit_queries_dropped=25"));
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        path_str(&trace_root),
        "zero-hit-bucket-private-resume",
        "cold",
        "missing",
    ] {
        assert!(!stderr.contains(forbidden), "stderr leaked {forbidden}");
    }
    assert!(!query_set.exists());

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_freeze_agent_replay_writes_static_query_bucket_metadata() {
    let data_dir = temp_dir("query-set-trace-root-smoke-buckets-data");
    let out_dir = temp_dir("query-set-trace-root-smoke-buckets-out");
    let trace_root = temp_dir("query-set-trace-root-smoke-buckets-artifacts");
    let query_set = out_dir.join("private-query-set.local.jsonl");
    let summary = query_set_summary_path(&query_set);

    let queries = [
        full_freeze_query("single", 0, 1),
        full_freeze_query("andtwo", 0, 2),
        full_freeze_query("andthree", 0, 3),
        full_freeze_query("andsix", 0, 6),
        "fieldaaaa shanghai".to_string(),
        "hybridaaaa AND backendaaaa".to_string(),
        "\"semantic aaaa\"".to_string(),
    ];
    let hit_query_text = queries
        .iter()
        .map(|query| query.replace('"', " "))
        .collect::<Vec<_>>()
        .join(" ");
    let lines = queries
        .iter()
        .map(|query| source_search_trace_line(query))
        .collect::<Vec<_>>();

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "smoke-bucketed-private-resume.pdf",
        &[],
        &hit_query_text,
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "smoke-bucketed-private-resume.pdf",
            &hit_query_text,
        )],
    );
    write_trace_log_strings(&trace_root, "run-trace-smoke-buckets", &lines);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--out",
            path_str(&query_set),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "7",
            "--min-queries",
            "7",
        ])
        .output()
        .expect("freeze smoke agent replay query set with bucket metadata");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("queries: 7"));
    assert!(stdout.contains("candidate queries sampled: 7"));
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        path_str(&trace_root),
        "smoke-bucketed-private-resume",
        "fieldaaaa",
        "semantic",
    ] {
        assert!(!stdout.contains(forbidden), "stdout leaked {forbidden}");
    }

    let query_set_text = fs::read_to_string(&query_set).unwrap();
    let lines = query_set_text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 7);
    let mut bucket_counts = std::collections::BTreeMap::new();
    for line in lines {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        *bucket_counts
            .entry(value["bucket"].as_str().unwrap().to_string())
            .or_insert(0_u64) += 1;
    }
    assert_eq!(bucket_counts["single_term"], 1);
    assert_eq!(bucket_counts["and_2"], 1);
    assert_eq!(bucket_counts["and_3_5"], 1);
    assert_eq!(bucket_counts["and_6_16"], 1);
    assert_eq!(bucket_counts["field_filter"], 1);
    assert_eq!(bucket_counts["hybrid"], 1);
    assert_eq!(bucket_counts["semantic"], 1);

    let summary_text = fs::read_to_string(&summary).unwrap();
    let summary_value: serde_json::Value = serde_json::from_str(&summary_text).unwrap();
    assert_eq!(summary_value["query_count"].as_u64(), Some(7));
    assert_eq!(summary_value["candidate_queries_sampled"].as_u64(), Some(7));
    for bucket in QUERY_SET_BUCKETS {
        let tune_count = summary_value["tune_bucket_counts"][bucket].as_u64();
        let holdout_count = summary_value["holdout_bucket_counts"][bucket].as_u64();
        assert_eq!(
            tune_count
                .zip(holdout_count)
                .map(|(tune, holdout)| tune + holdout),
            Some(1)
        );
    }
    assert_eq!(summary_value["zero_hit_queries_dropped"].as_u64(), Some(0));

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_preflight_agent_replay_reports_full_freeze_bucket_deficits() {
    let data_dir = temp_dir("query-set-trace-root-bucket-deficit-data");
    let out_dir = temp_dir("query-set-trace-root-bucket-deficit-out");
    let trace_root = temp_dir("query-set-trace-root-bucket-deficit-artifacts");
    let preflight = out_dir.join("query-set-trace-preflight.local.json");

    let mut lines = Vec::new();
    let mut hit_query_text = String::new();
    for index in 0..50 {
        let query = full_freeze_query("single", index, 1);
        hit_query_text.push_str(&query);
        hit_query_text.push(' ');
        lines.push(source_search_trace_line(&query));
    }
    for index in 0..75 {
        let query = full_freeze_query("andtwo", index, 2);
        hit_query_text.push_str(&query);
        hit_query_text.push(' ');
        lines.push(source_search_trace_line(&query));
    }

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "bucket-deficit-private-resume.pdf",
        &[],
        &hit_query_text,
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "bucket-deficit-private-resume.pdf",
            &hit_query_text,
        )],
    );
    write_trace_log_strings(&trace_root, "run-trace-bucket-deficit", &lines);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "preflight-agent-replay",
            "--out",
            path_str(&preflight),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "500",
        ])
        .output()
        .expect("preflight full agent replay query set bucket deficits");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("query set trace preflight: written"));
    let preflight_text = fs::read_to_string(&preflight).unwrap();
    let value: serde_json::Value = serde_json::from_str(&preflight_text).unwrap();
    assert_eq!(value["d10k_corpus_ready"].as_bool(), Some(false));
    assert_eq!(
        value["candidate_bucket_deficits"]["and_3_5"].as_u64(),
        Some(150)
    );
    assert_eq!(
        value["candidate_bucket_deficits"]["field_filter"].as_u64(),
        Some(75)
    );
    assert_eq!(
        value["candidate_bucket_deficits"]["hybrid"].as_u64(),
        Some(75)
    );
    assert_eq!(
        value["candidate_bucket_deficits"]["semantic"].as_u64(),
        Some(25)
    );
    assert_eq!(
        value["corpus_valid_bucket_deficits"]["and_3_5"].as_u64(),
        Some(150)
    );
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        path_str(&trace_root),
        "bucket-deficit-private-resume",
        "singleaaaa",
        "andtwoaaaa",
    ] {
        assert!(!stdout.contains(forbidden), "stdout leaked {forbidden}");
        assert!(
            !preflight_text.contains(forbidden),
            "preflight leaked {forbidden}"
        );
    }
    assert!(preflight.exists());

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_freeze_agent_replay_defaults_min_queries_to_max_queries() {
    let data_dir = temp_dir("query-set-trace-root-default-min-data");
    let out_dir = temp_dir("query-set-trace-root-default-min-out");
    let trace_root = temp_dir("query-set-trace-root-default-min-artifacts");
    let query_set = out_dir.join("private-query-set.local.jsonl");

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "alpha-private-resume.pdf",
        &[],
        "rust backend shanghai retrieval ranking",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "alpha-private-resume.pdf",
            "rust backend shanghai retrieval ranking",
        )],
    );
    write_trace_log(
        &trace_root,
        "run-trace-default-min",
        &[
            "[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | rust backend",
            "[2026-06-05T12:09:21+08:00] | tool_called | round=1 | tool=source_search | unicorn nohit",
        ],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--out",
            path_str(&query_set),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "2",
        ])
        .output()
        .expect("reject agent replay query set below implicit max-sized minimum");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(
        "query set blocked: not enough corpus-valid trace queries for the current indexed corpus"
    ));
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        path_str(&trace_root),
        "rust backend",
        "unicorn nohit",
    ] {
        assert!(!stderr.contains(forbidden), "stderr leaked {forbidden}");
    }
    assert!(!query_set.exists());

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_freeze_agent_replay_rejects_d10k_max_with_lower_min_queries() {
    let data_dir = temp_dir("query-set-trace-root-d10k-lowered-min-data");
    let out_dir = temp_dir("query-set-trace-root-d10k-lowered-min-out");
    let trace_root = temp_dir("query-set-trace-root-d10k-lowered-min-artifacts");
    let query_set = out_dir.join("private-query-set.local.jsonl");

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "alpha-private-resume.pdf",
        &[],
        "rust backend shanghai retrieval ranking",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "alpha-private-resume.pdf",
            "rust backend shanghai retrieval ranking",
        )],
    );
    write_trace_log(
        &trace_root,
        "run-trace-d10k-lowered-min",
        &["[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | rust backend"],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--out",
            path_str(&query_set),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "500",
            "--min-queries",
            "1",
        ])
        .output()
        .expect("reject downgraded D10K agent replay query set");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("query set blocked: D10K agent replay freeze requires 500 queries"));
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        path_str(&trace_root),
        "rust backend",
    ] {
        assert!(!stderr.contains(forbidden), "stderr leaked {forbidden}");
    }
    assert!(!query_set.exists());

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_freeze_agent_replay_rejects_d10k_freeze_on_non_d10k_corpus() {
    let data_dir = temp_dir("query-set-trace-root-d10k-corpus-data");
    let out_dir = temp_dir("query-set-trace-root-d10k-corpus-out");
    let trace_root = temp_dir("query-set-trace-root-d10k-corpus-artifacts");
    let query_set = out_dir.join("private-query-set.local.jsonl");
    let summary = query_set_summary_path(&query_set);

    let queries = full_freeze_queries();
    let hit_query_text = queries
        .iter()
        .map(|query| query.replace('"', " "))
        .collect::<Vec<_>>()
        .join(" ");
    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "d10k-corpus-private-resume.pdf",
        &[],
        &hit_query_text,
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "d10k-corpus-private-resume.pdf",
            &hit_query_text,
        )],
    );
    let lines = queries
        .iter()
        .map(|query| source_search_trace_line(query))
        .collect::<Vec<_>>();
    write_trace_log_strings(&trace_root, "run-trace-d10k-corpus", &lines);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--out",
            path_str(&query_set),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "500",
            "--min-queries",
            "500",
        ])
        .output()
        .expect("reject D10K agent replay query set on a non-D10K corpus");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(
        "query set blocked: D10K agent replay freeze requires a D10K-shaped indexed corpus"
    ));
    assert!(stderr.contains("document_count=9999"));
    assert!(stderr.contains("searchable_document_count=7999"));
    assert!(stderr.contains("vector_indexed_document_count=8000"));
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        path_str(&trace_root),
        "d10k-corpus-private-resume",
        "singleaaaa",
        "shanghai",
        "semantic",
    ] {
        assert!(!stderr.contains(forbidden), "stderr leaked {forbidden}");
    }
    assert!(!query_set.exists());
    assert!(!summary.exists());

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_freeze_agent_replay_rejects_insufficient_corpus_valid_queries() {
    let data_dir = temp_dir("query-set-trace-root-insufficient-data");
    let out_dir = temp_dir("query-set-trace-root-insufficient-out");
    let trace_root = temp_dir("query-set-trace-root-insufficient-artifacts");
    let query_set = out_dir.join("private-query-set.local.jsonl");

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "alpha-private-resume.pdf",
        &[],
        "rust backend shanghai retrieval ranking",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "alpha-private-resume.pdf",
            "rust backend shanghai retrieval ranking",
        )],
    );
    write_trace_log(
        &trace_root,
        "run-trace-insufficient",
        &["[2026-06-05T12:09:23+08:00] | tool_called | round=1 | tool=source_search | unicorn nohit"],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--out",
            path_str(&query_set),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "1",
            "--min-queries",
            "1",
        ])
        .output()
        .expect("reject insufficient trace-backed query set");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(
        "query set blocked: not enough corpus-valid trace queries for the current indexed corpus"
    ));
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        path_str(&trace_root),
        "unicorn nohit",
    ] {
        assert!(!stderr.contains(forbidden), "stderr leaked {forbidden}");
    }
    assert!(!query_set.exists());

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

#[test]
fn benchmark_query_set_freeze_agent_replay_reports_redacted_trace_selection_counts() {
    let data_dir = temp_dir("query-set-trace-root-selection-counts-data");
    let out_dir = temp_dir("query-set-trace-root-selection-counts-out");
    let trace_root = temp_dir("query-set-trace-root-selection-counts-artifacts");
    let query_set = out_dir.join("private-query-set.local.jsonl");

    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "selection-counts-private-resume.pdf",
        &[],
        "rust backend shanghai retrieval ranking",
    );
    seed_fulltext_index(
        &data_dir,
        vec![index_document(
            "selection-counts-private-resume.pdf",
            "rust backend shanghai retrieval ranking",
        )],
    );
    write_trace_log(
        &trace_root,
        "run-trace-selection-counts",
        &[
            "[2026-06-05T12:09:19+08:00] | tool_called | round=1 | tool=browser_open | ignore me",
            r#"[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | call=call_123 | args={"query":"rust shanghai","limit":20}"#,
            "[2026-06-05T12:09:21+08:00] | tool_called | round=1 | tool=source_search | /Users/private resume",
            "[2026-06-05T12:09:22+08:00] | tool_called | round=1 | tool=source_search | rust backend",
            "[2026-06-05T12:09:23+08:00] | tool_called | round=1 | tool=source_search | rust backend",
            "[2026-06-05T12:09:24+08:00] | tool_called | round=1 | tool=source_search | unicorn nohit",
        ],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "freeze-agent-replay",
            "--out",
            path_str(&query_set),
            "--trace-root",
            path_str(&trace_root),
            "--max-queries",
            "2",
            "--min-queries",
            "2",
        ])
        .output()
        .expect("reject insufficient trace-backed query set with redacted counts");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(
        "query set blocked: not enough corpus-valid trace queries for the current indexed corpus"
    ));
    assert!(stderr.contains("trace selection counts:"));
    assert!(stderr.contains("trace_logs=1"));
    assert!(stderr.contains("trace_lines=6"));
    assert!(stderr.contains("source_search_lines=5"));
    assert!(stderr.contains("extracted_queries=4"));
    assert!(stderr.contains("normalization_rejected=1"));
    assert!(stderr.contains("duplicate_queries_dropped=1"));
    assert!(stderr.contains("candidate_queries_sampled=2"));
    assert!(stderr.contains("zero_hit_queries_dropped=1"));
    assert!(stderr.contains("selected_queries=1"));
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        path_str(&trace_root),
        "selection-counts-private-resume",
        "rust backend",
        "unicorn nohit",
        "/Users/private",
        "args=",
        "call_123",
    ] {
        assert!(!stderr.contains(forbidden), "stderr leaked {forbidden}");
    }
    assert!(!query_set.exists());

    remove_dir(&data_dir);
    remove_dir(&out_dir);
    remove_dir(&trace_root);
}

fn seed_searchable_document_with_mentions(
    data_dir: &Path,
    file_name: &str,
    mentions: &[SeedMention],
) {
    seed_searchable_document_with_mentions_and_text(
        data_dir,
        file_name,
        mentions,
        &format!("synthetic text for {file_name}"),
    );
}

fn seed_searchable_document_with_mentions_and_text(
    data_dir: &Path,
    file_name: &str,
    mentions: &[SeedMention],
    text: &str,
) {
    let source_root = s304_source_root(data_dir);
    fs::create_dir_all(&source_root).unwrap();
    let mut content = String::from("SUMMARY\nSynthetic Query Candidate\n");
    for mention in mentions {
        content.push_str(seed_mention_label(&mention.entity_type));
        content.push_str(": ");
        content.push_str(mention.raw_value);
        content.push('\n');
    }
    content.push_str("EXPERIENCE\n");
    content.push_str("Built ");
    content.push_str(text);
    content.push_str(" systems");
    content.push_str("\nSKILLS\n");
    content.push_str(text);
    fs::write(s304_source_path(data_dir, file_name), content).unwrap();
}

fn mention(
    entity_type: EntityType,
    raw_value: &'static str,
    normalized_value: &'static str,
    confidence: f32,
) -> SeedMention {
    SeedMention {
        entity_type,
        raw_value,
        _normalized_value: normalized_value,
        _confidence: confidence,
    }
}

struct SeedMention {
    entity_type: EntityType,
    raw_value: &'static str,
    _normalized_value: &'static str,
    _confidence: f32,
}

#[derive(Clone)]
struct SeedIndexDocument {
    file_name: String,
    text: String,
}

fn seed_fulltext_index(data_dir: &Path, documents: Vec<SeedIndexDocument>) {
    for document in documents {
        let path = s304_source_path(data_dir, &document.file_name);
        let mut content = fs::read_to_string(&path).unwrap();
        content.push_str("\nSEARCH\n");
        content.push_str(&document.text);
        fs::write(path, content).unwrap();
    }
    let output = import_existing_root(data_dir, &s304_source_root(data_dir));
    assert_import_succeeded(&output);
}

fn index_document(file_name: &str, text: &str) -> SeedIndexDocument {
    SeedIndexDocument {
        file_name: file_name.to_string(),
        text: text.to_string(),
    }
}

fn s304_source_root(data_dir: &Path) -> PathBuf {
    data_dir.join("s304-source")
}

fn s304_source_path(data_dir: &Path, file_name: &str) -> PathBuf {
    let source_name = Path::new(file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(file_name);
    s304_source_root(data_dir).join(format!("{source_name}.txt"))
}

fn seed_mention_label(entity_type: &EntityType) -> &'static str {
    match entity_type {
        EntityType::Name => "Name",
        EntityType::Email => "Email",
        EntityType::Phone => "Phone",
        EntityType::WeChat => "WeChat",
        EntityType::School => "School",
        EntityType::SchoolTier => "School Tier",
        EntityType::Degree => "Degree",
        EntityType::Major => "Major",
        EntityType::Company => "Company",
        EntityType::Title => "Title",
        EntityType::Education => "Education",
        EntityType::Skills | EntityType::Skill => "Skills",
        EntityType::Certificate => "Certificate",
        EntityType::Date => "Date",
        EntityType::DateRange => "Date Range",
        EntityType::YearsExperience => "Years Experience",
        EntityType::Location => "Location",
        EntityType::Other(_) => "Other",
    }
}

fn write_trace_log(trace_root: &Path, run_id: &str, lines: &[&str]) {
    let runtime_dir = trace_root.join(run_id).join("runtime");
    fs::create_dir_all(&runtime_dir).unwrap();
    let content = if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    };
    fs::write(runtime_dir.join("trace.log"), content).unwrap();
}

fn write_trace_log_strings(trace_root: &Path, run_id: &str, lines: &[String]) {
    let runtime_dir = trace_root.join(run_id).join("runtime");
    fs::create_dir_all(&runtime_dir).unwrap();
    let content = if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    };
    fs::write(runtime_dir.join("trace.log"), content).unwrap();
}

fn source_search_trace_line(query: &str) -> String {
    format!("[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | {query}")
}

fn expected_query_set_split(
    hasher: &ContactHasher,
    queries: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut split_sides = Vec::new();
    let mut assignment_digests = Vec::new();
    for query in queries {
        let digest = hasher
            .hmac_hex("resume-ir:query-set-summary:v2:assign", query.as_bytes())
            .unwrap();
        let bucket = u8::from_str_radix(&digest[..2], 16).unwrap();
        let tune = bucket >= 0x33;
        split_sides.push(tune);
        assignment_digests.push(digest);
    }
    for bucket in QUERY_SET_BUCKETS {
        let mut indexes = queries
            .iter()
            .enumerate()
            .filter_map(|(index, query)| {
                (QuerySetSampleShape::from_query(query).bucket() == bucket).then_some(index)
            })
            .collect::<Vec<_>>();
        if indexes.len() <= 1 {
            continue;
        }
        let has_tune = indexes.iter().any(|index| split_sides[*index]);
        let has_holdout = indexes.iter().any(|index| !split_sides[*index]);
        if has_tune && has_holdout {
            continue;
        }
        indexes.sort_by(|left, right| {
            assignment_digests[*left]
                .cmp(&assignment_digests[*right])
                .then_with(|| left.cmp(right))
        });
        if let Some(index) = indexes.first().copied() {
            split_sides[index] = !split_sides[index];
        }
    }
    if queries.len() > 1 && split_sides.iter().all(|side| *side) {
        if let Some(side) = split_sides.last_mut() {
            *side = false;
        }
    }
    if queries.len() > 1 && split_sides.iter().all(|side| !*side) {
        if let Some(side) = split_sides.last_mut() {
            *side = true;
        }
    }
    let tune_queries = queries
        .iter()
        .zip(split_sides.iter())
        .filter(|&(_, tune)| *tune)
        .map(|(query, _)| query.clone())
        .collect::<Vec<_>>();
    let holdout_queries = queries
        .iter()
        .zip(split_sides.iter())
        .filter(|&(_, tune)| !*tune)
        .map(|(query, _)| query.clone())
        .collect::<Vec<_>>();
    (tune_queries, holdout_queries)
}

fn expected_query_set_hmac_payload<T: AsRef<str>>(queries: &[T]) -> Vec<u8> {
    let mut payload = Vec::new();
    update_expected_query_set_payload_string(&mut payload, "resume-ir.query-set.jsonl.v2");
    update_expected_query_set_payload_string(&mut payload, "resume-ir.query-set-summary.v2");
    update_expected_query_set_payload_string(
        &mut payload,
        QuerySetSourceKind::TraceSourceSearchV1.as_str(),
    );
    payload.extend((queries.len() as u64).to_le_bytes());
    for query in queries {
        let query = query.as_ref();
        let shape = QuerySetSampleShape::from_query(query);
        update_expected_query_set_payload_string(&mut payload, query);
        update_expected_query_set_payload_string(&mut payload, shape.bucket());
        payload.extend((shape.term_count() as u64).to_le_bytes());
        payload.push(u8::from(shape.has_boolean()));
        payload.push(u8::from(shape.has_location()));
        payload.push(u8::from(shape.has_years()));
        payload.push(u8::from(shape.has_degree()));
        payload.push(u8::from(shape.has_skill()));
        payload.push(u8::from(shape.has_phrase()));
    }
    payload
}

fn update_expected_query_set_payload_string(payload: &mut Vec<u8>, value: &str) {
    payload.extend((value.len() as u64).to_le_bytes());
    payload.extend(value.as_bytes());
}

fn full_freeze_query(prefix: &str, index: usize, term_count: usize) -> String {
    (0..term_count)
        .map(|term_index| format!("{prefix}{}{}", alpha_id(index), alpha_id(term_index)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn full_freeze_queries() -> Vec<String> {
    let mut queries = Vec::new();
    queries.extend((0..50).map(|index| full_freeze_query("single", index, 1)));
    queries.extend((0..75).map(|index| full_freeze_query("andtwo", index, 2)));
    queries.extend((0..150).map(|index| full_freeze_query("andthree", index, 3)));
    queries.extend((0..50).map(|index| full_freeze_query("andsix", index, 6)));
    queries.extend((0..75).map(|index| format!("field{} shanghai", alpha_id(index))));
    queries.extend(
        (0..75).map(|index| format!("hybrid{} AND backend{}", alpha_id(index), alpha_id(index))),
    );
    queries.extend((0..25).map(|index| format!("\"semantic {}\"", alpha_id(index))));
    queries
}

fn alpha_id(mut value: usize) -> String {
    let mut id = String::new();
    for _ in 0..4 {
        id.push(char::from(b'a' + (value % 26) as u8));
        value /= 26;
    }
    id
}

fn query_set_summary_path(query_set: &Path) -> PathBuf {
    let file_name = query_set.file_name().unwrap().to_str().unwrap();
    let base_name = file_name.strip_suffix(".local.jsonl").unwrap_or(file_name);
    query_set.with_file_name(format!("{base_name}.summary.json"))
}

fn private_query_corpus_summary_json(document_count: usize) -> Vec<u8> {
    format!(
        concat!(
            "{{",
            "\"schema_version\":\"benchmark-corpus-summary.v1\",",
            "\"privacy_boundary\":\"redacted_local_aggregate\",",
            "\"document_count\":{},",
            "\"searchable_document_count\":{},",
            "\"vector_indexed_document_count\":{},",
            "\"active_vector_document_count\":{},",
            "\"vector_count\":{},",
            "\"vector_deleted_count\":0,",
            "\"vector_index_state\":\"available\",",
            "\"vector_search_backend\":\"hnsw_ann\",",
            "\"hot_index_fully_covered\":true,",
            "\"contains_raw_resume_text\":false,",
            "\"contains_resume_paths\":false,",
            "\"contains_queries\":false,",
            "\"contains_sample_ids\":false",
            "}}"
        ),
        document_count, document_count, document_count, document_count, document_count
    )
    .into_bytes()
}

fn query_handoff_fixture_script(label: &str) -> PathBuf {
    let path = temp_dir(label).join(query_handoff_fixture_file_name());
    fs::write(&path, query_handoff_fixture_script_body()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).unwrap();
    }
    path
}

#[cfg(unix)]
fn query_handoff_fixture_file_name() -> &'static str {
    "query-handoff-fixture.sh"
}

#[cfg(windows)]
fn query_handoff_fixture_file_name() -> &'static str {
    "query-handoff-fixture.cmd"
}

#[cfg(unix)]
fn query_handoff_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "test -n \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\" || exit 42\n",
        "request_index=1\n",
        "while IFS= read -r line; do\n",
        "  request_id=\"private-query-request-$request_index\"\n",
        "  printf 'resume-ir-query-v2\\nrequest_id=%s\\nmode=hybrid\\nlayers=fulltext+field+vector+rrf\\ntop_k=%s\\nquery_embedding_runtime=local-command\\nquery_embedding_invocations=1\\nstage_query_parse_ms=1.0\\nstage_prefilter_ms=2.0\\nstage_bm25_ms=3.0\\nstage_ann_ms=4.0\\nstage_fusion_ms=5.0\\nstage_bulk_hydrate_ms=6.0\\nstage_snippet_ms=7.0\\nrss_delta_mb=0.0\\nelapsed_ms=8.0\\nhits=%s\\nresume-ir-query-end\\n' \"$request_id\" \"$RESUME_IR_QUERY_TOP_K\" \"$RESUME_IR_QUERY_TOP_K\"\n",
        "  request_index=$((request_index + 1))\n",
        "done < \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\"\n",
    )
}

#[cfg(windows)]
fn query_handoff_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "setlocal enabledelayedexpansion\r\n",
        "if \"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\"==\"\" exit /b 42\r\n",
        "set /a request_index=1\r\n",
        "for /f \"usebackq delims=\" %%L in (\"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\") do (\r\n",
        "  echo resume-ir-query-v2\r\n",
        "  echo request_id=private-query-request-!request_index!\r\n",
        "  echo mode=hybrid\r\n",
        "  echo layers=fulltext+field+vector+rrf\r\n",
        "  echo top_k=%RESUME_IR_QUERY_TOP_K%\r\n",
        "  echo query_embedding_runtime=local-command\r\n",
        "  echo query_embedding_invocations=1\r\n",
        "  echo stage_query_parse_ms=1.0\r\n",
        "  echo stage_prefilter_ms=2.0\r\n",
        "  echo stage_bm25_ms=3.0\r\n",
        "  echo stage_ann_ms=4.0\r\n",
        "  echo stage_fusion_ms=5.0\r\n",
        "  echo stage_bulk_hydrate_ms=6.0\r\n",
        "  echo stage_snippet_ms=7.0\r\n",
        "  echo rss_delta_mb=0.0\r\n",
        "  echo elapsed_ms=8.0\r\n",
        "  echo hits=%RESUME_IR_QUERY_TOP_K%\r\n",
        "  echo resume-ir-query-end\r\n",
        "  set /a request_index+=1\r\n",
        ")\r\n",
    )
}

fn file_sha256_hex(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(fs::read(path).unwrap());
    format!("{:x}", hasher.finalize())
}

#[cfg(unix)]
fn file_mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s304-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}
