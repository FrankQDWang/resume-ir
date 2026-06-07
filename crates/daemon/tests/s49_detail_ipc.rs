use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use meta_store::{
    Document, DocumentId, DocumentStatus, EntityMention, EntityMentionId, EntityType,
    FileExtension, MetaStore, ResumeVersion, ResumeVersionId, ResumeVisibility, UnixTimestamp,
};

const IPC_ENDPOINT_TIMEOUT: Duration = Duration::from_secs(30);

#[test]
fn daemon_detail_ipc_authenticates_and_returns_redacted_structured_detail() {
    let data_dir = temp_dir("detail-ipc-data");
    let doc_id = DocumentId::from_non_secret_parts(&["s49", "daemon-detail-doc"]);
    let version_id = ResumeVersionId::from_non_secret_parts(&["s49", "daemon-detail-version"]);
    let old_version_id = ResumeVersionId::from_non_secret_parts(&["s49", "daemon-detail-old"]);
    seed_detail_resume(&data_dir, &doc_id, &old_version_id, &version_id);

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
    let response = http_post_detail_command(
        &endpoint,
        Some(&token),
        serde_json::json!({
            "doc_id": doc_id.to_string()
        }),
    );

    assert!(response.contains("HTTP/1.1 200 OK"));
    assert!(!response.contains(path_str(&data_dir)));
    assert!(!response.contains(&token));
    assert!(!response.contains("candidate@example.test"));
    assert!(!response.contains("155-555-0199"));
    assert!(!response.contains("PRIVATE_TRAILING_MARKER_SHOULD_NOT_APPEAR"));
    assert!(!response.contains("OLD_VERSION_SHOULD_NOT_APPEAR"));
    assert!(!response.contains("private/resumes"));
    assert!(!response.contains("/Users/frank/private/field-value"));
    let body = response.split("\r\n\r\n").nth(1).unwrap_or_default();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["schema_version"], "daemon.detail.v1");
    assert_eq!(payload["status"], "ok");
    let document = &payload["document"];
    assert_eq!(document["doc_id"], doc_id.to_string());
    assert_eq!(document["version_id"], version_id.to_string());
    assert_eq!(document["extension"], "pdf");
    assert_eq!(document["document_status"], "searchable");
    assert_eq!(document["visibility"], "searchable");
    assert_eq!(document["byte_size"], 8192);
    assert!(document["fields"].as_array().unwrap().len() >= 4);
    assert!(document["snippet"].as_str().unwrap().contains("Java"));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_detail_ipc_rejects_unauthorized_invalid_and_missing_docs_without_secret_leaks() {
    let data_dir = temp_dir("detail-ipc-invalid-data");
    let missing_doc_id = DocumentId::from_non_secret_parts(&["s49", "missing-secret-doc"]);
    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "4",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = BufReader::new(stdout);
    let endpoint = read_ipc_endpoint(&mut child, &mut stdout);
    let token = read_ipc_auth_token(&data_dir);

    let unauthorized = http_post_detail_command(
        &endpoint,
        None,
        serde_json::json!({
            "doc_id": missing_doc_id.to_string()
        }),
    );
    assert!(unauthorized.contains("HTTP/1.1 401 Unauthorized"));
    assert!(!unauthorized.contains(missing_doc_id.as_str()));
    assert!(!unauthorized.contains(path_str(&data_dir)));

    let invalid_json = raw_ipc_request(
        &endpoint,
        format!(
            "POST /details HTTP/1.1\r\nHost: local\r\nAuthorization: Bearer {token}\r\nContent-Length: 8\r\nConnection: close\r\n\r\nnot json"
        )
        .as_bytes(),
    );
    assert!(invalid_json.contains("HTTP/1.1 400 Bad Request"));
    assert!(!invalid_json.contains("not json"));

    let invalid_doc = http_post_detail_command(
        &endpoint,
        Some(&token),
        serde_json::json!({
            "doc_id": "not a valid id"
        }),
    );
    assert!(invalid_doc.contains("HTTP/1.1 400 Bad Request"));
    assert!(!invalid_doc.contains("not a valid id"));

    let missing = http_post_detail_command(
        &endpoint,
        Some(&token),
        serde_json::json!({
            "doc_id": missing_doc_id.to_string()
        }),
    );
    assert!(missing.contains("HTTP/1.1 404 Not Found"));
    assert!(!missing.contains(missing_doc_id.as_str()));
    assert!(!missing.contains(path_str(&data_dir)));
    assert!(!missing.contains(&token));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_detail_ipc_hides_deleted_documents_without_returning_version_data() {
    let data_dir = temp_dir("detail-ipc-deleted-data");
    let doc_id = DocumentId::from_non_secret_parts(&["s49", "daemon-detail-deleted-doc"]);
    let version_id = ResumeVersionId::from_non_secret_parts(&["s49", "daemon-detail-deleted"]);
    seed_deleted_resume(&data_dir, &doc_id, &version_id);

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
    let response = http_post_detail_command(
        &endpoint,
        Some(&token),
        serde_json::json!({
            "doc_id": doc_id.to_string()
        }),
    );

    assert!(response.contains("HTTP/1.1 404 Not Found"));
    assert!(!response.contains("DELETED_VERSION_SHOULD_NOT_APPEAR"));
    assert!(!response.contains(path_str(&data_dir)));
    assert!(!response.contains(&token));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

fn seed_detail_resume(
    data_dir: &Path,
    document_id: &DocumentId,
    old_version_id: &ResumeVersionId,
    version_id: &ResumeVersionId,
) {
    let now = UnixTimestamp::from_unix_seconds(1_800_049_000);
    let private_path = "/Users/frank/private/resumes/candidate@example.test-java.pdf";
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    store
        .upsert_document(&Document {
            id: document_id.clone(),
            source_uri: format!("file://{private_path}"),
            normalized_path: private_path.to_string(),
            file_name: "candidate@example.test-java.pdf".to_string(),
            extension: FileExtension::Pdf,
            byte_size: 8192,
            mtime: now,
            content_hash: Some("s49-daemon-detail-content-hash".to_string()),
            text_hash: None,
            is_deleted: false,
            created_at: now,
            updated_at: now,
            status: DocumentStatus::Searchable,
        })
        .unwrap();
    store
        .upsert_resume_version(&ResumeVersion {
            id: old_version_id.clone(),
            document_id: document_id.clone(),
            candidate_id: None,
            parse_version: "parser-v0".to_string(),
            schema_version: "schema-v1".to_string(),
            language_set: vec!["en".to_string()],
            page_count: Some(1),
            raw_text: Some("OLD_VERSION_SHOULD_NOT_APPEAR".to_string()),
            clean_text: Some("OLD_VERSION_SHOULD_NOT_APPEAR".to_string()),
            quality_score: Some(0.3),
            visibility: ResumeVisibility::Searchable,
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
            page_count: Some(2),
            raw_text: Some("raw candidate@example.test 155-555-0199".to_string()),
            clean_text: Some(format!(
                "Java platform engineer candidate@example.test 155-555-0199 {private_path} \
                 led payment routing with Rust and Kubernetes. {} PRIVATE_TRAILING_MARKER_SHOULD_NOT_APPEAR",
                "skill evidence ".repeat(30)
            )),
            quality_score: Some(0.91),
            visibility: ResumeVisibility::Searchable,
        })
        .unwrap();
    store
        .replace_entity_mentions(
            version_id,
            &[
                entity_mention(version_id, "degree", EntityType::Degree, "master", 0.96),
                entity_mention(version_id, "skill", EntityType::Skill, "Kubernetes", 0.94),
                entity_mention(
                    version_id,
                    "company",
                    EntityType::Company,
                    "Acme Payments",
                    0.9,
                ),
                entity_mention(
                    version_id,
                    "title",
                    EntityType::Title,
                    "Staff Engineer",
                    0.9,
                ),
                entity_mention(
                    version_id,
                    "email",
                    EntityType::Email,
                    "candidate@example.test",
                    0.99,
                ),
                entity_mention(version_id, "phone", EntityType::Phone, "155-555-0199", 0.99),
                entity_mention(
                    version_id,
                    "other",
                    EntityType::Other("/Users/frank/private/entity-type".to_string()),
                    "/Users/frank/private/field-value",
                    0.8,
                ),
            ],
        )
        .unwrap();
}

fn seed_deleted_resume(data_dir: &Path, document_id: &DocumentId, version_id: &ResumeVersionId) {
    let now = UnixTimestamp::from_unix_seconds(1_800_049_001);
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    store
        .upsert_document(&Document {
            id: document_id.clone(),
            source_uri: "file:///Users/frank/private/deleted.pdf".to_string(),
            normalized_path: "/Users/frank/private/deleted.pdf".to_string(),
            file_name: "deleted.pdf".to_string(),
            extension: FileExtension::Pdf,
            byte_size: 512,
            mtime: now,
            content_hash: Some("s49-deleted-content-hash".to_string()),
            text_hash: None,
            is_deleted: true,
            created_at: now,
            updated_at: now,
            status: DocumentStatus::Deleted,
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
            raw_text: Some("DELETED_VERSION_SHOULD_NOT_APPEAR".to_string()),
            clean_text: Some("DELETED_VERSION_SHOULD_NOT_APPEAR".to_string()),
            quality_score: Some(0.1),
            visibility: ResumeVisibility::Searchable,
        })
        .unwrap();
}

fn entity_mention(
    version_id: &ResumeVersionId,
    label: &str,
    entity_type: EntityType,
    value: &str,
    confidence: f32,
) -> EntityMention {
    EntityMention {
        id: EntityMentionId::from_non_secret_parts(&["s49", version_id.as_str(), label]),
        resume_version_id: version_id.clone(),
        section_id: None,
        entity_type,
        raw_value: value.to_string(),
        normalized_value: Some(value.to_string()),
        span_start: Some(0),
        span_end: Some(value.len()),
        confidence,
        extractor: "s49-test".to_string(),
    }
}

fn http_post_detail_command(
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
        "POST /details HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n{authorization}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(addr).expect("connect daemon ipc");
    stream
        .write_all(request.as_bytes())
        .expect("write detail request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read detail response");
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
    let deadline = Instant::now() + IPC_ENDPOINT_TIMEOUT;
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
    let _ = child.kill();
    let _ = child.wait();
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
    let path = std::env::temp_dir().join(format!("resume-ir-s49-daemon-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}
