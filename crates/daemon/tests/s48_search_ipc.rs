use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use index_fulltext::{FullTextIndex, IndexDocument, IndexSection};
use meta_store::{
    Candidate, CandidateId, ContactHash, Document, DocumentId, DocumentStatus, EntityMention,
    EntityMentionId, EntityType, FileExtension, MetaStore, ResumeVersion, ResumeVersionId,
    ResumeVisibility, UnixTimestamp,
};

#[test]
fn daemon_search_ipc_authenticates_filters_and_redacts_results() {
    let data_dir = temp_dir("search-ipc-data");
    let first_doc = DocumentId::from_non_secret_parts(&["s48", "first"]);
    let first_version = ResumeVersionId::from_non_secret_parts(&["s48", "first-version"]);
    let second_doc = DocumentId::from_non_secret_parts(&["s48", "second"]);
    let second_version = ResumeVersionId::from_non_secret_parts(&["s48", "second-version"]);
    seed_searchable_resume(SeedResume {
        data_dir: &data_dir,
        document_id: &first_doc,
        version_id: &first_version,
        file_name: "candidate@example.test-java.pdf",
        clean_text: "Java platform engineer candidate@example.test 155-555-0199 Kubernetes",
        degree: "master",
        skill: "Kubernetes",
        years: 7.0,
        school: "",
        school_tier: "985",
        certificate: "",
        company: "",
        title: "",
    });
    seed_searchable_resume(SeedResume {
        data_dir: &data_dir,
        document_id: &second_doc,
        version_id: &second_version,
        file_name: "synthetic-rust-java.pdf",
        clean_text: "Java Rust engineer with two years of experience",
        degree: "bachelor",
        skill: "Rust",
        years: 2.0,
        school: "",
        school_tier: "overseas",
        certificate: "",
        company: "",
        title: "",
    });
    seed_fulltext_index(
        &data_dir,
        [
            IndexDocument {
                doc_id: first_doc.to_string(),
                version_id: first_version.to_string(),
                file_name: "candidate@example.test-java.pdf".to_string(),
                clean_text: "Java platform engineer candidate@example.test 155-555-0199 Kubernetes"
                    .to_string(),
                sections: vec![IndexSection {
                    section_type: "experience".to_string(),
                    text: "Java Kubernetes".to_string(),
                }],
                is_deleted: false,
            },
            IndexDocument {
                doc_id: second_doc.to_string(),
                version_id: second_version.to_string(),
                file_name: "synthetic-rust-java.pdf".to_string(),
                clean_text: "Java Rust engineer with two years of experience".to_string(),
                sections: vec![IndexSection {
                    section_type: "experience".to_string(),
                    text: "Java Rust".to_string(),
                }],
                is_deleted: false,
            },
        ],
    );

    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "2",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = BufReader::new(stdout);
    let endpoint = read_ipc_endpoint(&mut child, &mut stdout);
    let token = read_ipc_auth_token(&data_dir);
    let response = http_post_search_command(
        &endpoint,
        Some(&token),
        serde_json::json!({
            "query": "Java",
            "mode": "fulltext",
            "top_k": 5,
            "filters": {
                "degree_min": "master",
                "skills_any": ["kubernetes"],
                "years_experience_min": 5.0,
                "school_tiers_any": ["985"]
            }
        }),
    );

    assert!(response.contains("HTTP/1.1 200 OK"));
    assert!(!response.contains(path_str(&data_dir)));
    assert!(!response.contains(&token));
    assert!(!response.contains("candidate@example.test"));
    assert!(!response.contains("155-555-0199"));
    let body = response.split("\r\n\r\n").nth(1).unwrap_or_default();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["schema_version"], "daemon.search.v1");
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["mode"], "fulltext");
    assert_eq!(payload["search_index"], "available");
    assert_eq!(payload["result_count"], 1);
    let results = payload["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["rank"], 1);
    assert_eq!(results[0]["doc_id"], first_doc.to_string());
    assert_eq!(results[0]["version_id"], first_version.to_string());

    let status_response = http_get_status(&endpoint, &token);
    assert!(status_response.contains("HTTP/1.1 200 OK"));
    assert!(!status_response.contains("Java"));
    assert!(!status_response.contains(path_str(&data_dir)));
    assert!(!status_response.contains(&token));
    let status_body = status_response.split("\r\n\r\n").nth(1).unwrap_or_default();
    let status_payload: serde_json::Value = serde_json::from_str(status_body).unwrap();
    assert_eq!(status_payload["schema_version"], "daemon.status.v1");
    assert_eq!(status_payload["query_latency"]["sample_count"], 1);
    assert!(status_payload["query_latency"]["p50_ms"].as_u64().is_some());
    assert!(status_payload["query_latency"]["p95_ms"].as_u64().is_some());
    assert!(status_payload["query_latency"]["p99_ms"].as_u64().is_some());

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_search_ipc_includes_redacted_soft_dedupe_hints() {
    let data_dir = temp_dir("search-ipc-soft-dedupe-data");
    let first_doc = DocumentId::from_non_secret_parts(&["s48", "soft-first"]);
    let first_version = ResumeVersionId::from_non_secret_parts(&["s48", "soft-first-version"]);
    let second_doc = DocumentId::from_non_secret_parts(&["s48", "soft-second"]);
    let second_version = ResumeVersionId::from_non_secret_parts(&["s48", "soft-second-version"]);
    seed_soft_dedupe_resume(
        &data_dir,
        &first_doc,
        &first_version,
        "synthetic-soft-ipc-a.pdf",
        "Java backend payments",
        "Synthetic Candidate",
        "synthetic candidate",
        "Synthetic University",
        "synthetic university",
        "Java",
        "java",
    );
    seed_soft_dedupe_resume(
        &data_dir,
        &second_doc,
        &second_version,
        "synthetic-soft-ipc-b.pdf",
        "Java backend search",
        "Synthetic Candidate",
        "synthetic candidate",
        "Synthetic University",
        "synthetic university",
        "Java",
        "java",
    );
    seed_fulltext_index(
        &data_dir,
        [
            IndexDocument {
                doc_id: first_doc.to_string(),
                version_id: first_version.to_string(),
                file_name: "synthetic-soft-ipc-a.pdf".to_string(),
                clean_text: "Java backend payments".to_string(),
                sections: vec![IndexSection {
                    section_type: "experience".to_string(),
                    text: "Java payments".to_string(),
                }],
                is_deleted: false,
            },
            IndexDocument {
                doc_id: second_doc.to_string(),
                version_id: second_version.to_string(),
                file_name: "synthetic-soft-ipc-b.pdf".to_string(),
                clean_text: "Java backend search".to_string(),
                sections: vec![IndexSection {
                    section_type: "experience".to_string(),
                    text: "Java search".to_string(),
                }],
                is_deleted: false,
            },
        ],
    );

    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = BufReader::new(stdout);
    let endpoint = read_ipc_endpoint(&mut child, &mut stdout);
    let token = read_ipc_auth_token(&data_dir);
    let response = http_post_search_command(
        &endpoint,
        Some(&token),
        serde_json::json!({
            "query": "Java",
            "mode": "fulltext",
            "top_k": 5
        }),
    );

    assert!(response.contains("HTTP/1.1 200 OK"));
    assert!(!response.contains("Synthetic Candidate"));
    assert!(!response.contains("Synthetic University"));
    let body = response.split("\r\n\r\n").nth(1).unwrap_or_default();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    let results = payload["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["soft_dedupe"]["suspected_versions"], 1);
    assert!(
        results[0]["soft_dedupe"]["max_confidence"]
            .as_f64()
            .unwrap()
            > 0.70
    );
    assert_eq!(results[0]["soft_dedupe"]["folded"], false);

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_search_ipc_prefilters_unknown_school_tier_before_fulltext_top_k_cutoff() {
    let data_dir = temp_dir("search-ipc-unknown-tier-data");
    let target_doc = DocumentId::from_non_secret_parts(&["s48", "unknown-tier-target"]);
    let target_version =
        ResumeVersionId::from_non_secret_parts(&["s48", "unknown-tier-target-version"]);
    let noisy_query_text = std::iter::repeat_n("needle", 100)
        .collect::<Vec<_>>()
        .join(" ");
    let mut index_documents = Vec::new();

    for index in 0..5 {
        let document_id =
            DocumentId::from_non_secret_parts(&["s48", &format!("tier-decoy-{index}")]);
        let version_id = ResumeVersionId::from_non_secret_parts(&[
            "s48",
            &format!("tier-decoy-version-{index}"),
        ]);
        let file_name = format!("known-tier-decoy-{index}.pdf");
        let clean_text = format!("Known tier decoy {index} {noisy_query_text}");
        seed_searchable_resume(SeedResume {
            data_dir: &data_dir,
            document_id: &document_id,
            version_id: &version_id,
            file_name: &file_name,
            clean_text: &clean_text,
            degree: "bachelor",
            skill: "Java",
            years: 4.0,
            school: "",
            school_tier: "985",
            certificate: "",
            company: "",
            title: "",
        });
        index_documents.push(IndexDocument {
            doc_id: document_id.to_string(),
            version_id: version_id.to_string(),
            file_name,
            clean_text: clean_text.clone(),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: clean_text,
            }],
            is_deleted: false,
        });
    }

    seed_searchable_resume(SeedResume {
        data_dir: &data_dir,
        document_id: &target_doc,
        version_id: &target_version,
        file_name: "unknown-tier-target.pdf",
        clean_text: "Unknown tier target needle",
        degree: "bachelor",
        skill: "Java",
        years: 4.0,
        school: "",
        school_tier: "",
        certificate: "",
        company: "",
        title: "",
    });
    index_documents.push(IndexDocument {
        doc_id: target_doc.to_string(),
        version_id: target_version.to_string(),
        file_name: "unknown-tier-target.pdf".to_string(),
        clean_text: "Unknown tier target needle".to_string(),
        sections: vec![IndexSection {
            section_type: "experience".to_string(),
            text: "needle".to_string(),
        }],
        is_deleted: false,
    });
    seed_fulltext_index_vec(&data_dir, index_documents);

    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = BufReader::new(stdout);
    let endpoint = read_ipc_endpoint(&mut child, &mut stdout);
    let token = read_ipc_auth_token(&data_dir);
    let response = http_post_search_command(
        &endpoint,
        Some(&token),
        serde_json::json!({
            "query": "needle",
            "mode": "fulltext",
            "top_k": 1,
            "filters": {
                "school_tiers_any": ["unknown"]
            }
        }),
    );

    assert!(response.contains("HTTP/1.1 200 OK"));
    let body = response.split("\r\n\r\n").nth(1).unwrap_or_default();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["result_count"], 1);
    let results = payload["results"].as_array().unwrap();
    assert_eq!(results[0]["doc_id"], target_doc.to_string());
    assert_eq!(results[0]["version_id"], target_version.to_string());
    assert!(!response.contains("known-tier-decoy-"));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_search_ipc_prefilters_certificates_before_fulltext_top_k_cutoff() {
    let data_dir = temp_dir("search-ipc-certificate-data");
    let target_doc = DocumentId::from_non_secret_parts(&["s48", "certificate-target"]);
    let target_version =
        ResumeVersionId::from_non_secret_parts(&["s48", "certificate-target-version"]);
    let noisy_query_text = std::iter::repeat_n("needle", 100)
        .collect::<Vec<_>>()
        .join(" ");
    let mut index_documents = Vec::new();

    for index in 0..5 {
        let document_id =
            DocumentId::from_non_secret_parts(&["s48", &format!("certificate-decoy-{index}")]);
        let version_id = ResumeVersionId::from_non_secret_parts(&[
            "s48",
            &format!("certificate-decoy-version-{index}"),
        ]);
        let file_name = format!("certificate-decoy-{index}.pdf");
        let clean_text = format!("Certificate decoy {index} {noisy_query_text}");
        seed_searchable_resume(SeedResume {
            data_dir: &data_dir,
            document_id: &document_id,
            version_id: &version_id,
            file_name: &file_name,
            clean_text: &clean_text,
            degree: "bachelor",
            skill: "Java",
            years: 4.0,
            school: "",
            school_tier: "",
            certificate: "",
            company: "",
            title: "",
        });
        index_documents.push(IndexDocument {
            doc_id: document_id.to_string(),
            version_id: version_id.to_string(),
            file_name,
            clean_text: clean_text.clone(),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: clean_text,
            }],
            is_deleted: false,
        });
    }

    seed_searchable_resume(SeedResume {
        data_dir: &data_dir,
        document_id: &target_doc,
        version_id: &target_version,
        file_name: "certificate-target.pdf",
        clean_text: "Certificate target needle",
        degree: "bachelor",
        skill: "Java",
        years: 4.0,
        school: "",
        school_tier: "",
        certificate: "pmp",
        company: "",
        title: "",
    });
    index_documents.push(IndexDocument {
        doc_id: target_doc.to_string(),
        version_id: target_version.to_string(),
        file_name: "certificate-target.pdf".to_string(),
        clean_text: "Certificate target needle".to_string(),
        sections: vec![IndexSection {
            section_type: "experience".to_string(),
            text: "needle".to_string(),
        }],
        is_deleted: false,
    });
    seed_fulltext_index_vec(&data_dir, index_documents);

    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = BufReader::new(stdout);
    let endpoint = read_ipc_endpoint(&mut child, &mut stdout);
    let token = read_ipc_auth_token(&data_dir);
    let response = http_post_search_command(
        &endpoint,
        Some(&token),
        serde_json::json!({
            "query": "needle",
            "mode": "fulltext",
            "top_k": 1,
            "filters": {
                "certificates_any": ["pmp"]
            }
        }),
    );

    assert!(response.contains("HTTP/1.1 200 OK"));
    let body = response.split("\r\n\r\n").nth(1).unwrap_or_default();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["result_count"], 1);
    let results = payload["results"].as_array().unwrap();
    assert_eq!(results[0]["doc_id"], target_doc.to_string());
    assert_eq!(results[0]["version_id"], target_version.to_string());
    assert!(!response.contains("certificate-decoy-"));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_search_ipc_prefilters_school_before_fulltext_top_k_cutoff() {
    let data_dir = temp_dir("search-ipc-school-data");
    let target_doc = DocumentId::from_non_secret_parts(&["s48", "school-target"]);
    let target_version = ResumeVersionId::from_non_secret_parts(&["s48", "school-target-version"]);
    let noisy_query_text = std::iter::repeat_n("needle", 100)
        .collect::<Vec<_>>()
        .join(" ");
    let mut index_documents = Vec::new();

    for index in 0..5 {
        let document_id =
            DocumentId::from_non_secret_parts(&["s48", &format!("school-decoy-{index}")]);
        let version_id =
            ResumeVersionId::from_non_secret_parts(&["s48", &format!("school-decoy-{index}")]);
        let file_name = format!("school-decoy-{index}.pdf");
        let clean_text = format!("School decoy {index} {noisy_query_text}");
        seed_searchable_resume(SeedResume {
            data_dir: &data_dir,
            document_id: &document_id,
            version_id: &version_id,
            file_name: &file_name,
            clean_text: &clean_text,
            degree: "bachelor",
            skill: "Java",
            years: 4.0,
            school: "synthetic search college",
            school_tier: "",
            certificate: "",
            company: "",
            title: "",
        });
        index_documents.push(IndexDocument {
            doc_id: document_id.to_string(),
            version_id: version_id.to_string(),
            file_name,
            clean_text: clean_text.clone(),
            sections: vec![IndexSection {
                section_type: "education".to_string(),
                text: clean_text,
            }],
            is_deleted: false,
        });
    }

    seed_searchable_resume(SeedResume {
        data_dir: &data_dir,
        document_id: &target_doc,
        version_id: &target_version,
        file_name: "school-target.pdf",
        clean_text: "School target needle",
        degree: "bachelor",
        skill: "Java",
        years: 4.0,
        school: "synthetic institute of technology",
        school_tier: "",
        certificate: "",
        company: "",
        title: "",
    });
    index_documents.push(IndexDocument {
        doc_id: target_doc.to_string(),
        version_id: target_version.to_string(),
        file_name: "school-target.pdf".to_string(),
        clean_text: "School target needle".to_string(),
        sections: vec![IndexSection {
            section_type: "education".to_string(),
            text: "needle".to_string(),
        }],
        is_deleted: false,
    });
    seed_fulltext_index_vec(&data_dir, index_documents);

    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = BufReader::new(stdout);
    let endpoint = read_ipc_endpoint(&mut child, &mut stdout);
    let token = read_ipc_auth_token(&data_dir);
    let response = http_post_search_command(
        &endpoint,
        Some(&token),
        serde_json::json!({
            "query": "needle",
            "mode": "fulltext",
            "top_k": 1,
            "filters": {
                "schools_any": ["synthetic institute of technology"]
            }
        }),
    );

    assert!(response.contains("HTTP/1.1 200 OK"));
    let body = response.split("\r\n\r\n").nth(1).unwrap_or_default();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["result_count"], 1);
    let results = payload["results"].as_array().unwrap();
    assert_eq!(results[0]["doc_id"], target_doc.to_string());
    assert_eq!(results[0]["version_id"], target_version.to_string());
    assert!(!response.contains("school-decoy-"));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_search_ipc_prefilters_major_before_fulltext_top_k_cutoff() {
    let data_dir = temp_dir("search-ipc-major-data");
    let target_doc = DocumentId::from_non_secret_parts(&["s48", "major-target"]);
    let target_version = ResumeVersionId::from_non_secret_parts(&["s48", "major-target-version"]);
    let noisy_query_text = std::iter::repeat_n("needle", 100)
        .collect::<Vec<_>>()
        .join(" ");
    let mut index_documents = Vec::new();

    for index in 0..5 {
        let document_id =
            DocumentId::from_non_secret_parts(&["s48", &format!("major-decoy-{index}")]);
        let version_id =
            ResumeVersionId::from_non_secret_parts(&["s48", &format!("major-decoy-{index}")]);
        let file_name = format!("major-decoy-{index}.pdf");
        let clean_text = format!("Major decoy {index} {noisy_query_text}");
        seed_searchable_resume(SeedResume {
            data_dir: &data_dir,
            document_id: &document_id,
            version_id: &version_id,
            file_name: &file_name,
            clean_text: &clean_text,
            degree: "bachelor",
            skill: "Java",
            years: 4.0,
            school: "",
            school_tier: "",
            certificate: "",
            company: "",
            title: "",
        });
        append_major_mention(&data_dir, &version_id, "economics");
        index_documents.push(IndexDocument {
            doc_id: document_id.to_string(),
            version_id: version_id.to_string(),
            file_name,
            clean_text: clean_text.clone(),
            sections: vec![IndexSection {
                section_type: "education".to_string(),
                text: clean_text,
            }],
            is_deleted: false,
        });
    }

    seed_searchable_resume(SeedResume {
        data_dir: &data_dir,
        document_id: &target_doc,
        version_id: &target_version,
        file_name: "major-target.pdf",
        clean_text: "Major target needle",
        degree: "bachelor",
        skill: "Java",
        years: 4.0,
        school: "",
        school_tier: "",
        certificate: "",
        company: "",
        title: "",
    });
    append_major_mention(&data_dir, &target_version, "computer_science");
    index_documents.push(IndexDocument {
        doc_id: target_doc.to_string(),
        version_id: target_version.to_string(),
        file_name: "major-target.pdf".to_string(),
        clean_text: "Major target needle".to_string(),
        sections: vec![IndexSection {
            section_type: "education".to_string(),
            text: "needle".to_string(),
        }],
        is_deleted: false,
    });
    seed_fulltext_index_vec(&data_dir, index_documents);

    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = BufReader::new(stdout);
    let endpoint = read_ipc_endpoint(&mut child, &mut stdout);
    let token = read_ipc_auth_token(&data_dir);
    let response = http_post_search_command(
        &endpoint,
        Some(&token),
        serde_json::json!({
            "query": "needle",
            "mode": "fulltext",
            "top_k": 1,
            "filters": {
                "majors_any": ["computer_science"]
            }
        }),
    );

    assert!(response.contains("HTTP/1.1 200 OK"));
    let body = response.split("\r\n\r\n").nth(1).unwrap_or_default();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["result_count"], 1);
    let results = payload["results"].as_array().unwrap();
    assert_eq!(results[0]["doc_id"], target_doc.to_string());
    assert_eq!(results[0]["version_id"], target_version.to_string());
    assert!(!response.contains("major-decoy-"));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_search_ipc_prefilters_date_range_before_fulltext_top_k_cutoff() {
    let data_dir = temp_dir("search-ipc-date-range-data");
    let target_doc = DocumentId::from_non_secret_parts(&["s48", "date-range-target"]);
    let target_version =
        ResumeVersionId::from_non_secret_parts(&["s48", "date-range-target-version"]);
    let noisy_query_text = std::iter::repeat_n("needle", 100)
        .collect::<Vec<_>>()
        .join(" ");
    let mut index_documents = Vec::new();

    for index in 0..5 {
        let document_id =
            DocumentId::from_non_secret_parts(&["s48", &format!("date-range-decoy-{index}")]);
        let version_id = ResumeVersionId::from_non_secret_parts(&[
            "s48",
            &format!("date-range-decoy-version-{index}"),
        ]);
        let file_name = format!("date-range-decoy-{index}.pdf");
        let clean_text = format!("Date range decoy {index} {noisy_query_text}");
        seed_searchable_resume(SeedResume {
            data_dir: &data_dir,
            document_id: &document_id,
            version_id: &version_id,
            file_name: &file_name,
            clean_text: &clean_text,
            degree: "bachelor",
            skill: "Java",
            years: 2.0,
            school: "",
            school_tier: "",
            certificate: "",
            company: "",
            title: "",
        });
        append_date_range_mention(&data_dir, &version_id, "2017-01/2018-12");
        index_documents.push(IndexDocument {
            doc_id: document_id.to_string(),
            version_id: version_id.to_string(),
            file_name,
            clean_text: clean_text.clone(),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: clean_text,
            }],
            is_deleted: false,
        });
    }

    seed_searchable_resume(SeedResume {
        data_dir: &data_dir,
        document_id: &target_doc,
        version_id: &target_version,
        file_name: "date-range-target.pdf",
        clean_text: "Date range target needle",
        degree: "bachelor",
        skill: "Java",
        years: 4.0,
        school: "",
        school_tier: "",
        certificate: "",
        company: "",
        title: "",
    });
    append_date_range_mention(&data_dir, &target_version, "2020-03/2022-06");
    index_documents.push(IndexDocument {
        doc_id: target_doc.to_string(),
        version_id: target_version.to_string(),
        file_name: "date-range-target.pdf".to_string(),
        clean_text: "Date range target needle".to_string(),
        sections: vec![IndexSection {
            section_type: "experience".to_string(),
            text: "needle".to_string(),
        }],
        is_deleted: false,
    });
    seed_fulltext_index_vec(&data_dir, index_documents);

    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = BufReader::new(stdout);
    let endpoint = read_ipc_endpoint(&mut child, &mut stdout);
    let token = read_ipc_auth_token(&data_dir);
    let response = http_post_search_command(
        &endpoint,
        Some(&token),
        serde_json::json!({
            "query": "needle",
            "mode": "fulltext",
            "top_k": 1,
            "filters": {
                "date_range_overlaps": "2021-01/2021-12"
            }
        }),
    );

    assert!(response.contains("HTTP/1.1 200 OK"));
    let body = response.split("\r\n\r\n").nth(1).unwrap_or_default();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["result_count"], 1);
    let results = payload["results"].as_array().unwrap();
    assert_eq!(results[0]["doc_id"], target_doc.to_string());
    assert_eq!(results[0]["version_id"], target_version.to_string());
    assert!(!response.contains("date-range-decoy-"));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_search_ipc_prefilters_company_and_title_before_fulltext_top_k_cutoff() {
    let data_dir = temp_dir("search-ipc-company-title-data");
    let target_doc = DocumentId::from_non_secret_parts(&["s48", "company-title-target"]);
    let target_version =
        ResumeVersionId::from_non_secret_parts(&["s48", "company-title-target-version"]);
    let noisy_query_text = std::iter::repeat_n("needle", 100)
        .collect::<Vec<_>>()
        .join(" ");
    let mut index_documents = Vec::new();

    for index in 0..5 {
        let document_id =
            DocumentId::from_non_secret_parts(&["s48", &format!("company-title-decoy-{index}")]);
        let version_id = ResumeVersionId::from_non_secret_parts(&[
            "s48",
            &format!("company-title-decoy-version-{index}"),
        ]);
        let file_name = format!("company-title-decoy-{index}.pdf");
        let clean_text = format!("Company title decoy {index} {noisy_query_text}");
        seed_searchable_resume(SeedResume {
            data_dir: &data_dir,
            document_id: &document_id,
            version_id: &version_id,
            file_name: &file_name,
            clean_text: &clean_text,
            degree: "bachelor",
            skill: "Java",
            years: 4.0,
            school: "",
            school_tier: "",
            certificate: "",
            company: "synthetic search",
            title: "product_manager",
        });
        index_documents.push(IndexDocument {
            doc_id: document_id.to_string(),
            version_id: version_id.to_string(),
            file_name,
            clean_text: clean_text.clone(),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: clean_text,
            }],
            is_deleted: false,
        });
    }

    seed_searchable_resume(SeedResume {
        data_dir: &data_dir,
        document_id: &target_doc,
        version_id: &target_version,
        file_name: "company-title-target.pdf",
        clean_text: "Company title target needle",
        degree: "bachelor",
        skill: "Java",
        years: 4.0,
        school: "",
        school_tier: "",
        certificate: "",
        company: "synthetic payments",
        title: "backend_engineer",
    });
    index_documents.push(IndexDocument {
        doc_id: target_doc.to_string(),
        version_id: target_version.to_string(),
        file_name: "company-title-target.pdf".to_string(),
        clean_text: "Company title target needle".to_string(),
        sections: vec![IndexSection {
            section_type: "experience".to_string(),
            text: "needle".to_string(),
        }],
        is_deleted: false,
    });
    seed_fulltext_index_vec(&data_dir, index_documents);

    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = BufReader::new(stdout);
    let endpoint = read_ipc_endpoint(&mut child, &mut stdout);
    let token = read_ipc_auth_token(&data_dir);
    let response = http_post_search_command(
        &endpoint,
        Some(&token),
        serde_json::json!({
            "query": "needle",
            "mode": "fulltext",
            "top_k": 1,
            "filters": {
                "companies_any": ["synthetic payments"],
                "titles_any": ["backend_engineer"]
            }
        }),
    );

    assert!(response.contains("HTTP/1.1 200 OK"));
    let body = response.split("\r\n\r\n").nth(1).unwrap_or_default();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["result_count"], 1);
    let results = payload["results"].as_array().unwrap();
    assert_eq!(results[0]["doc_id"], target_doc.to_string());
    assert_eq!(results[0]["version_id"], target_version.to_string());
    assert!(!response.contains("company-title-decoy-"));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_search_ipc_prefilters_location_before_fulltext_top_k_cutoff() {
    let data_dir = temp_dir("search-ipc-location-data");
    let target_doc = DocumentId::from_non_secret_parts(&["s48", "location-target"]);
    let target_version =
        ResumeVersionId::from_non_secret_parts(&["s48", "location-target-version"]);
    let noisy_query_text = std::iter::repeat_n("needle", 100)
        .collect::<Vec<_>>()
        .join(" ");
    let mut index_documents = Vec::new();

    for index in 0..5 {
        let document_id =
            DocumentId::from_non_secret_parts(&["s48", &format!("location-decoy-{index}")]);
        let version_id = ResumeVersionId::from_non_secret_parts(&[
            "s48",
            &format!("location-decoy-version-{index}"),
        ]);
        let file_name = format!("location-decoy-{index}.pdf");
        let clean_text = format!("Location decoy {index} {noisy_query_text}");
        seed_searchable_resume(SeedResume {
            data_dir: &data_dir,
            document_id: &document_id,
            version_id: &version_id,
            file_name: &file_name,
            clean_text: &clean_text,
            degree: "bachelor",
            skill: "Java",
            years: 4.0,
            school: "",
            school_tier: "",
            certificate: "",
            company: "",
            title: "",
        });
        append_location_mention(&data_dir, &version_id, "beijing");
        index_documents.push(IndexDocument {
            doc_id: document_id.to_string(),
            version_id: version_id.to_string(),
            file_name,
            clean_text: clean_text.clone(),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: clean_text,
            }],
            is_deleted: false,
        });
    }

    seed_searchable_resume(SeedResume {
        data_dir: &data_dir,
        document_id: &target_doc,
        version_id: &target_version,
        file_name: "location-target.pdf",
        clean_text: "Location target needle",
        degree: "bachelor",
        skill: "Java",
        years: 4.0,
        school: "",
        school_tier: "",
        certificate: "",
        company: "",
        title: "",
    });
    append_location_mention(&data_dir, &target_version, "shanghai");
    index_documents.push(IndexDocument {
        doc_id: target_doc.to_string(),
        version_id: target_version.to_string(),
        file_name: "location-target.pdf".to_string(),
        clean_text: "Location target needle".to_string(),
        sections: vec![IndexSection {
            section_type: "experience".to_string(),
            text: "needle".to_string(),
        }],
        is_deleted: false,
    });
    seed_fulltext_index_vec(&data_dir, index_documents);

    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = BufReader::new(stdout);
    let endpoint = read_ipc_endpoint(&mut child, &mut stdout);
    let token = read_ipc_auth_token(&data_dir);
    let response = http_post_search_command(
        &endpoint,
        Some(&token),
        serde_json::json!({
            "query": "needle",
            "mode": "fulltext",
            "top_k": 1,
            "filters": {
                "locations_any": ["shanghai"]
            }
        }),
    );

    assert!(response.contains("HTTP/1.1 200 OK"));
    let body = response.split("\r\n\r\n").nth(1).unwrap_or_default();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["result_count"], 1);
    let results = payload["results"].as_array().unwrap();
    assert_eq!(results[0]["doc_id"], target_doc.to_string());
    assert_eq!(results[0]["version_id"], target_version.to_string());
    assert!(!response.contains("location-decoy-"));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_search_ipc_prefilters_contact_hash_before_fulltext_top_k_cutoff() {
    let data_dir = temp_dir("search-ipc-contact-data");
    let target_hash = ContactHash::from_keyed_digest("a".repeat(64)).unwrap();
    let target_doc = DocumentId::from_non_secret_parts(&["s48", "contact-target"]);
    let target_version = ResumeVersionId::from_non_secret_parts(&["s48", "contact-target-version"]);
    let noisy_query_text = std::iter::repeat_n("needle", 100)
        .collect::<Vec<_>>()
        .join(" ");
    let mut index_documents = Vec::new();

    for index in 0..5 {
        let document_id =
            DocumentId::from_non_secret_parts(&["s48", &format!("contact-decoy-{index}")]);
        let version_id = ResumeVersionId::from_non_secret_parts(&[
            "s48",
            &format!("contact-decoy-version-{index}"),
        ]);
        let file_name = format!("contact-decoy-{index}.pdf");
        let clean_text = format!("Contact decoy {index} {noisy_query_text}");
        seed_searchable_resume(SeedResume {
            data_dir: &data_dir,
            document_id: &document_id,
            version_id: &version_id,
            file_name: &file_name,
            clean_text: &clean_text,
            degree: "bachelor",
            skill: "Java",
            years: 4.0,
            school: "",
            school_tier: "",
            certificate: "",
            company: "",
            title: "",
        });
        index_documents.push(IndexDocument {
            doc_id: document_id.to_string(),
            version_id: version_id.to_string(),
            file_name,
            clean_text: clean_text.clone(),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: clean_text,
            }],
            is_deleted: false,
        });
    }

    seed_searchable_resume(SeedResume {
        data_dir: &data_dir,
        document_id: &target_doc,
        version_id: &target_version,
        file_name: "contact-target.pdf",
        clean_text: "Contact target needle target-contact@example.test 212-555-0199",
        degree: "bachelor",
        skill: "Java",
        years: 4.0,
        school: "",
        school_tier: "",
        certificate: "",
        company: "",
        title: "",
    });
    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let candidate = Candidate {
        id: CandidateId::from_non_secret_parts(&["s48", "contact-target-candidate"]),
        primary_name: None,
        phone_hash: None,
        email_hash: Some(target_hash.clone()),
        dedupe_key: None,
        merge_confidence: Some(1.0),
        version_count: 0,
    };
    store.upsert_candidate(&candidate).unwrap();
    store
        .assign_candidate_to_version(&target_version, &candidate.id)
        .unwrap();
    index_documents.push(IndexDocument {
        doc_id: target_doc.to_string(),
        version_id: target_version.to_string(),
        file_name: "contact-target.pdf".to_string(),
        clean_text: "Contact target needle target-contact@example.test 212-555-0199".to_string(),
        sections: vec![IndexSection {
            section_type: "experience".to_string(),
            text: "needle".to_string(),
        }],
        is_deleted: false,
    });
    seed_fulltext_index_vec(&data_dir, index_documents);

    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = BufReader::new(stdout);
    let endpoint = read_ipc_endpoint(&mut child, &mut stdout);
    let token = read_ipc_auth_token(&data_dir);
    let response = http_post_search_command(
        &endpoint,
        Some(&token),
        serde_json::json!({
            "query": "needle",
            "mode": "fulltext",
            "top_k": 1,
            "filters": {
                "contact_hashes_any": [target_hash.as_str()]
            }
        }),
    );

    assert!(response.contains("HTTP/1.1 200 OK"));
    let body = response.split("\r\n\r\n").nth(1).unwrap_or_default();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["result_count"], 1);
    let results = payload["results"].as_array().unwrap();
    assert_eq!(results[0]["doc_id"], target_doc.to_string());
    assert_eq!(results[0]["version_id"], target_version.to_string());
    assert!(!response.contains("contact-decoy-"));
    assert!(!response.contains("target-contact@example.test"));
    assert!(!response.contains("212-555-0199"));
    assert!(!response.contains(target_hash.as_str()));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_search_ipc_requires_bearer_token_without_leaking_query() {
    let data_dir = temp_dir("search-ipc-auth-data");
    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = BufReader::new(stdout);
    let endpoint = read_ipc_endpoint(&mut child, &mut stdout);
    let response = http_post_search_command(
        &endpoint,
        None,
        serde_json::json!({
            "query": "secret-query",
            "mode": "fulltext",
            "top_k": 1
        }),
    );

    assert!(response.contains("HTTP/1.1 401 Unauthorized"));
    assert!(!response.contains("secret-query"));
    assert!(!response.contains(path_str(&data_dir)));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_search_ipc_rejects_invalid_requests_without_leaking_query() {
    let data_dir = temp_dir("search-ipc-invalid-data");
    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "5",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = BufReader::new(stdout);
    let endpoint = read_ipc_endpoint(&mut child, &mut stdout);
    let token = read_ipc_auth_token(&data_dir);

    let wrong_token = http_post_search_command(
        &endpoint,
        Some("0000000000000000000000000000000000000000000000000000000000000000"),
        serde_json::json!({
            "query": "secret-query",
            "mode": "fulltext",
            "top_k": 1
        }),
    );
    assert!(wrong_token.contains("HTTP/1.1 401 Unauthorized"));
    assert!(!wrong_token.contains("secret-query"));

    let invalid_json = raw_ipc_request(
        &endpoint,
        format!(
            "POST /search HTTP/1.1\r\nHost: local\r\nAuthorization: Bearer {token}\r\nContent-Length: 8\r\nConnection: close\r\n\r\nnot json"
        )
        .as_bytes(),
    );
    assert!(invalid_json.contains("HTTP/1.1 400 Bad Request"));
    assert!(!invalid_json.contains("not json"));

    let empty_query = http_post_search_command(
        &endpoint,
        Some(&token),
        serde_json::json!({
            "query": "",
            "mode": "fulltext",
            "top_k": 1
        }),
    );
    assert!(empty_query.contains("HTTP/1.1 400 Bad Request"));

    let unsupported_mode = http_post_search_command(
        &endpoint,
        Some(&token),
        serde_json::json!({
            "query": "secret-query",
            "mode": "semantic",
            "top_k": 1
        }),
    );
    assert!(unsupported_mode.contains("HTTP/1.1 400 Bad Request"));
    assert!(!unsupported_mode.contains("secret-query"));

    let malformed_filters = http_post_search_command(
        &endpoint,
        Some(&token),
        serde_json::json!({
            "query": "secret-query",
            "mode": "fulltext",
            "top_k": 1,
            "filters": {
                "skills_any": "not-array"
            }
        }),
    );
    assert!(malformed_filters.contains("HTTP/1.1 400 Bad Request"));
    assert!(!malformed_filters.contains("secret-query"));
    assert!(!malformed_filters.contains(path_str(&data_dir)));
    assert!(!malformed_filters.contains(&token));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_search_ipc_reports_not_ready_without_opening_local_results() {
    let data_dir = temp_dir("search-ipc-not-ready-data");
    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = BufReader::new(stdout);
    let endpoint = read_ipc_endpoint(&mut child, &mut stdout);
    let token = read_ipc_auth_token(&data_dir);
    let response = http_post_search_command(
        &endpoint,
        Some(&token),
        serde_json::json!({
            "query": "secret-query",
            "mode": "fulltext",
            "top_k": 1
        }),
    );

    assert!(response.contains("HTTP/1.1 200 OK"));
    assert!(response.contains("\"search_index\":\"not_ready\""));
    assert!(response.contains("\"result_count\":0"));
    assert!(!response.contains("secret-query"));
    assert!(!response.contains(path_str(&data_dir)));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

struct SeedResume<'a> {
    data_dir: &'a Path,
    document_id: &'a DocumentId,
    version_id: &'a ResumeVersionId,
    file_name: &'a str,
    clean_text: &'a str,
    degree: &'a str,
    skill: &'a str,
    years: f32,
    school: &'a str,
    school_tier: &'a str,
    certificate: &'a str,
    company: &'a str,
    title: &'a str,
}

fn seed_searchable_resume(seed: SeedResume<'_>) {
    let now = UnixTimestamp::from_unix_seconds(1_800_048_000);
    let store = MetaStore::open_data_dir(seed.data_dir).unwrap();
    store.run_migrations().unwrap();
    store
        .upsert_document(&Document {
            id: seed.document_id.clone(),
            source_uri: format!("synthetic://{}", seed.file_name),
            normalized_path: format!("synthetic/{}", seed.file_name),
            file_name: seed.file_name.to_string(),
            extension: FileExtension::Pdf,
            byte_size: 128,
            mtime: now,
            content_hash: Some(format!("{}-hash", seed.file_name)),
            text_hash: None,
            is_deleted: false,
            created_at: now,
            updated_at: now,
            status: DocumentStatus::Searchable,
        })
        .unwrap();
    store
        .upsert_resume_version(&ResumeVersion {
            id: seed.version_id.clone(),
            document_id: seed.document_id.clone(),
            candidate_id: None,
            parse_version: "parser-v1".to_string(),
            schema_version: "schema-v1".to_string(),
            language_set: vec!["en".to_string()],
            page_count: Some(1),
            raw_text: Some(seed.clean_text.to_string()),
            clean_text: Some(seed.clean_text.to_string()),
            quality_score: Some(0.9),
            visibility: ResumeVisibility::Searchable,
        })
        .unwrap();
    let years = seed.years.to_string();
    let mut mentions = vec![
        entity_mention(
            seed.version_id,
            "degree",
            EntityType::Degree,
            seed.degree,
            0.95,
        ),
        entity_mention(
            seed.version_id,
            "skill",
            EntityType::Skill,
            seed.skill,
            0.95,
        ),
        entity_mention(
            seed.version_id,
            "years",
            EntityType::YearsExperience,
            &years,
            0.95,
        ),
    ];
    if !seed.school_tier.is_empty() {
        mentions.push(entity_mention(
            seed.version_id,
            "school-tier",
            EntityType::SchoolTier,
            seed.school_tier,
            0.95,
        ));
    }
    if !seed.school.is_empty() {
        mentions.push(entity_mention(
            seed.version_id,
            "school",
            EntityType::School,
            seed.school,
            0.95,
        ));
    }
    if !seed.certificate.is_empty() {
        mentions.push(entity_mention(
            seed.version_id,
            "certificate",
            EntityType::Certificate,
            seed.certificate,
            0.95,
        ));
    }
    if !seed.company.is_empty() {
        mentions.push(entity_mention(
            seed.version_id,
            "company",
            EntityType::Company,
            seed.company,
            0.95,
        ));
    }
    if !seed.title.is_empty() {
        mentions.push(entity_mention(
            seed.version_id,
            "title",
            EntityType::Title,
            seed.title,
            0.95,
        ));
    }
    store
        .replace_entity_mentions(seed.version_id, &mentions)
        .unwrap();
}

fn append_date_range_mention(
    data_dir: &Path,
    version_id: &ResumeVersionId,
    normalized_value: &str,
) {
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    let mut mentions = store.entity_mentions_for_version(version_id).unwrap();
    mentions.push(entity_mention(
        version_id,
        "date-range",
        EntityType::DateRange,
        normalized_value,
        0.95,
    ));
    store
        .replace_entity_mentions(version_id, &mentions)
        .unwrap();
}

fn append_location_mention(data_dir: &Path, version_id: &ResumeVersionId, normalized_value: &str) {
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    let mut mentions = store.entity_mentions_for_version(version_id).unwrap();
    mentions.push(entity_mention(
        version_id,
        "location",
        EntityType::Location,
        normalized_value,
        0.95,
    ));
    store
        .replace_entity_mentions(version_id, &mentions)
        .unwrap();
}

fn append_major_mention(data_dir: &Path, version_id: &ResumeVersionId, normalized_value: &str) {
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    let mut mentions = store.entity_mentions_for_version(version_id).unwrap();
    mentions.push(entity_mention(
        version_id,
        "major",
        EntityType::Major,
        normalized_value,
        0.95,
    ));
    store
        .replace_entity_mentions(version_id, &mentions)
        .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn seed_soft_dedupe_resume(
    data_dir: &Path,
    document_id: &DocumentId,
    version_id: &ResumeVersionId,
    file_name: &str,
    clean_text: &str,
    raw_name: &str,
    normalized_name: &str,
    raw_school: &str,
    normalized_school: &str,
    raw_skill: &str,
    normalized_skill: &str,
) {
    let now = UnixTimestamp::from_unix_seconds(1_800_048_000);
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    store
        .upsert_document(&Document {
            id: document_id.clone(),
            source_uri: format!("synthetic://{file_name}"),
            normalized_path: format!("synthetic/{file_name}"),
            file_name: file_name.to_string(),
            extension: FileExtension::Pdf,
            byte_size: 128,
            mtime: now,
            content_hash: Some(format!("{file_name}-hash")),
            text_hash: None,
            is_deleted: false,
            created_at: now,
            updated_at: now,
            status: DocumentStatus::Searchable,
        })
        .unwrap();
    store
        .upsert_resume_version(&ResumeVersion {
            id: version_id.clone(),
            document_id: document_id.clone(),
            candidate_id: None,
            parse_version: "parser-v1".to_string(),
            schema_version: "schema-v1".to_string(),
            language_set: vec!["en".to_string()],
            page_count: Some(1),
            raw_text: Some(clean_text.to_string()),
            clean_text: Some(clean_text.to_string()),
            quality_score: Some(0.9),
            visibility: ResumeVisibility::Searchable,
        })
        .unwrap();
    store
        .replace_entity_mentions(
            version_id,
            &[
                entity_mention_with_raw(
                    version_id,
                    "name",
                    EntityType::Name,
                    raw_name,
                    normalized_name,
                    0.95,
                ),
                entity_mention_with_raw(
                    version_id,
                    "school",
                    EntityType::School,
                    raw_school,
                    normalized_school,
                    0.95,
                ),
                entity_mention_with_raw(
                    version_id,
                    "skill",
                    EntityType::Skill,
                    raw_skill,
                    normalized_skill,
                    0.95,
                ),
            ],
        )
        .unwrap();
}

fn entity_mention(
    version_id: &ResumeVersionId,
    label: &str,
    entity_type: EntityType,
    normalized_value: &str,
    confidence: f32,
) -> EntityMention {
    EntityMention {
        id: EntityMentionId::from_non_secret_parts(&["s48", version_id.as_str(), label]),
        resume_version_id: version_id.clone(),
        section_id: None,
        entity_type,
        raw_value: normalized_value.to_string(),
        normalized_value: Some(normalized_value.to_string()),
        span_start: Some(0),
        span_end: Some(normalized_value.len()),
        confidence,
        extractor: "s48-test".to_string(),
    }
}

fn entity_mention_with_raw(
    version_id: &ResumeVersionId,
    label: &str,
    entity_type: EntityType,
    raw_value: &str,
    normalized_value: &str,
    confidence: f32,
) -> EntityMention {
    EntityMention {
        id: EntityMentionId::from_non_secret_parts(&["s48", version_id.as_str(), label]),
        resume_version_id: version_id.clone(),
        section_id: None,
        entity_type,
        raw_value: raw_value.to_string(),
        normalized_value: Some(normalized_value.to_string()),
        span_start: Some(0),
        span_end: Some(raw_value.len()),
        confidence,
        extractor: "s48-test".to_string(),
    }
}

fn seed_fulltext_index<const N: usize>(data_dir: &Path, documents: [IndexDocument; N]) {
    seed_fulltext_index_vec(data_dir, documents.into_iter().collect());
}

fn seed_fulltext_index_vec(data_dir: &Path, documents: Vec<IndexDocument>) {
    let index = FullTextIndex::open_or_create(&data_dir.join("search-index")).unwrap();
    index.replace_documents(documents).unwrap();
    index.commit().unwrap();
}

fn http_post_search_command(
    endpoint: &str,
    token: Option<&str>,
    payload: serde_json::Value,
) -> String {
    let rest = endpoint
        .strip_prefix("http://")
        .expect("endpoint has http scheme");
    let (addr, _path) = rest.split_once('/').expect("endpoint has path");
    let body = payload.to_string();
    let authorization = token
        .map(|token| format!("Authorization: Bearer {token}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "POST /search HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n{authorization}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(addr).expect("connect daemon ipc");
    stream
        .write_all(request.as_bytes())
        .expect("write search request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read search response");
    response
}

fn http_get_status(endpoint: &str, token: &str) -> String {
    let rest = endpoint
        .strip_prefix("http://")
        .expect("endpoint has http scheme");
    let (addr, _path) = rest.split_once('/').expect("endpoint has path");
    let request = format!(
        "GET /status HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n",
    );
    let mut stream = TcpStream::connect(addr).expect("connect daemon ipc");
    stream
        .write_all(request.as_bytes())
        .expect("write status request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read status response");
    response
}

fn raw_ipc_request(endpoint: &str, request: &[u8]) -> String {
    let rest = endpoint
        .strip_prefix("http://")
        .expect("endpoint has http scheme");
    let (addr, _path) = rest.split_once('/').expect("endpoint has path");
    let mut stream = TcpStream::connect(addr).expect("connect daemon ipc");
    stream.write_all(request).expect("write raw request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read raw response");
    response
}

fn read_ipc_endpoint(child: &mut Child, stdout: &mut BufReader<impl Read>) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut line = String::new();
    while Instant::now() < deadline {
        line.clear();
        let bytes = stdout.read_line(&mut line).expect("read daemon stdout");
        if bytes == 0 {
            if let Ok(Some(status)) = child.try_wait() {
                panic!("daemon exited before endpoint: {status}");
            }
            continue;
        }
        if let Some(endpoint) = line.trim().strip_prefix("ipc status endpoint: ") {
            return endpoint.to_string();
        }
    }
    panic!("daemon did not print ipc status endpoint");
}

fn read_ipc_auth_token(data_dir: &Path) -> String {
    let token = fs::read_to_string(data_dir.join("ipc.auth")).expect("read daemon ipc auth token");
    token.trim().to_string()
}

fn wait_child(child: Child) -> ChildOutput {
    let output = child.wait_with_output().expect("wait daemon");
    ChildOutput {
        success: output.status.success(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

struct ChildOutput {
    success: bool,
    stderr: String,
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s48-daemon-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}
