use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use meta_store::{
    Document, DocumentId, DocumentStatus, FileExtension, MetaStore, ResumeVersion, ResumeVersionId,
    ResumeVisibility, UnixTimestamp,
};

#[cfg(unix)]
#[test]
fn daemon_embedding_worker_once_runs_local_command_and_persists_vector_snapshot() {
    let data_dir = temp_dir("embedding-worker-once-data");
    let private_root = seed_searchable_resume_versions(&data_dir);
    let command = write_fixture_executable(
        "fixture-daemon-embedding-worker-once",
        r#"#!/bin/sh
if [ ! -s "$RESUME_IR_EMBEDDING_INPUT_PATH" ]; then
  exit 7
fi
printf 'resume-ir-embedding-v1\n'
printf 'model_id=fixture-local-model\n'
printf 'dimension=4\n'
awk -F '\t' '/^input=/ { id=$1; sub(/^input=/, "", id); printf "vector=%s\t0.25,0.25,0.25,0.25\n", id }' "$RESUME_IR_EMBEDDING_INPUT_PATH"
"#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
            "--work-embeddings-once",
            "--embedding-command",
            path_str(&command),
            "--embedding-model-id",
            "fixture-local-model",
            "--embedding-dimension",
            "4",
            "--embedding-max-docs",
            "8",
            "--embedding-max-text-bytes",
            "100000",
        ])
        .output()
        .expect("run daemon embedding worker once");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("embedding worker processed: 2"));
    assert!(stdout.contains("embedding worker vector writes: 2"));
    assert!(stdout.contains("embedding worker failed: 0"));
    assert!(!stdout.contains("S51PrivateEmbeddingText"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_root)));
    assert!(!stdout.contains(path_str(&command)));

    assert_vector_snapshot(&data_dir, 4, 2);

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn daemon_embedding_worker_loop_serves_status_ipc_while_persisting_vectors() {
    let data_dir = temp_dir("embedding-worker-loop-data");
    let private_root = seed_searchable_resume_versions(&data_dir);
    let command = write_fixture_executable(
        "fixture-daemon-embedding-worker-loop",
        r#"#!/bin/sh
printf 'resume-ir-embedding-v1\n'
printf 'model_id=fixture-local-model\n'
printf 'dimension=4\n'
awk -F '\t' '/^input=/ { id=$1; sub(/^input=/, "", id); printf "vector=%s\t1,0,0,0\n", id }' "$RESUME_IR_EMBEDDING_INPUT_PATH"
"#,
    );

    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-embeddings",
            "--embedding-command",
            path_str(&command),
            "--embedding-model-id",
            "fixture-local-model",
            "--embedding-dimension",
            "4",
            "--embedding-max-docs",
            "8",
            "--embedding-max-text-bytes",
            "100000",
            "--worker-interval-ms",
            "25",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "40",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start daemon embedding worker loop with IPC");

    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = BufReader::new(stdout);
    let endpoint = read_ipc_endpoint(&mut child, &mut stdout);
    let used_requests = wait_for_vector_snapshot_with_status_requests(&data_dir, &endpoint, 2, 40);
    drain_status_requests(&endpoint, 40 - used_requests);
    let mut daemon_stdout = String::new();
    stdout
        .read_to_string(&mut daemon_stdout)
        .expect("read daemon stdout tail");

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());
    assert!(daemon_stdout.contains("embedding worker processed: 2"));
    assert!(!daemon_stdout.contains("S51PrivateEmbeddingText"));
    assert!(!daemon_stdout.contains(path_str(&data_dir)));
    assert!(!daemon_stdout.contains(path_str(&private_root)));
    assert!(!daemon_stdout.contains(path_str(&command)));

    assert_vector_snapshot(&data_dir, 4, 2);

    remove_dir(&data_dir);
}

fn seed_searchable_resume_versions(data_dir: &Path) -> PathBuf {
    let now = UnixTimestamp::from_unix_seconds(1_800_051_000);
    let private_root = data_dir.join("private-resumes");
    fs::create_dir_all(&private_root).unwrap();
    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();

    for index in 0..2 {
        let file_name = format!("synthetic-s51-embedding-{index}.pdf");
        let document_path = private_root.join(&file_name);
        fs::write(&document_path, b"%PDF-1.4 synthetic text-layer resume").unwrap();
        let doc_id = DocumentId::from_non_secret_parts(&["s51", "embedding", &index.to_string()]);
        let version_id = ResumeVersionId::from_non_secret_parts(&[
            "s51",
            "embedding",
            "version",
            doc_id.as_str(),
        ]);
        store
            .upsert_document(&Document {
                id: doc_id.clone(),
                source_uri: format!("file://{}", path_str(&document_path)),
                normalized_path: path_str(&document_path).to_string(),
                file_name,
                extension: FileExtension::Pdf,
                byte_size: fs::metadata(&document_path).unwrap().len(),
                mtime: now,
                content_hash: Some(format!("s51-embedding-content-hash-{index}")),
                text_hash: Some(format!("s51-embedding-text-hash-{index}")),
                is_deleted: false,
                created_at: now,
                updated_at: now,
                status: DocumentStatus::Searchable,
            })
            .unwrap();
        store
            .upsert_resume_version(&ResumeVersion {
                id: version_id,
                document_id: doc_id,
                candidate_id: None,
                parse_version: "s51-fixture-parser".to_string(),
                schema_version: "s51-fixture-schema".to_string(),
                language_set: vec!["en".to_string()],
                page_count: Some(1),
                raw_text: None,
                clean_text: Some(format!(
                    "S51PrivateEmbeddingText synthetic searchable resume {index}"
                )),
                quality_score: Some(0.91),
                visibility: ResumeVisibility::Searchable,
            })
            .unwrap();
    }

    private_root
}

fn assert_vector_snapshot(data_dir: &Path, expected_dimension: usize, expected_vectors: usize) {
    let snapshot = fs::read_to_string(data_dir.join("vector-index").join("vector.snapshot"))
        .expect("read vector snapshot");
    let mut lines = snapshot.lines();
    let expected_header = format!("resume-ir-vector-index-v1\tdimension\t{expected_dimension}");
    assert_eq!(lines.next(), Some(expected_header.as_str()));
    let vectors = lines.filter(|line| line.starts_with("V\t")).count();
    assert_eq!(vectors, expected_vectors);
}

fn http_get(endpoint: &str) -> String {
    try_http_get(endpoint).expect("read status response")
}

fn try_http_get(endpoint: &str) -> io::Result<String> {
    let rest = endpoint
        .strip_prefix("http://")
        .expect("endpoint has http scheme");
    let (addr, path) = rest.split_once('/').expect("endpoint has path");
    let request = format!("GET /{path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    let mut stream = TcpStream::connect(addr)?;
    stream.write_all(request.as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

fn wait_for_vector_snapshot_with_status_requests(
    data_dir: &Path,
    endpoint: &str,
    expected_vectors: usize,
    max_requests: usize,
) -> usize {
    let deadline = Instant::now() + Duration::from_secs(5);
    let snapshot_path = data_dir.join("vector-index").join("vector.snapshot");
    let mut requests = 0_usize;
    loop {
        requests += 1;
        let response = http_get(endpoint);
        assert!(!response.contains(path_str(data_dir)));
        if fs::read_to_string(&snapshot_path)
            .map(|snapshot| {
                snapshot
                    .lines()
                    .filter(|line| line.starts_with("V\t"))
                    .count()
            })
            .unwrap_or(0)
            == expected_vectors
        {
            return requests;
        }
        assert!(
            requests < max_requests,
            "daemon embedding worker did not persist vector snapshot; last response:\n{response}"
        );
        assert!(
            Instant::now() < deadline,
            "daemon embedding worker timed out; last response:\n{response}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn drain_status_requests(endpoint: &str, count: usize) {
    for _ in 0..count {
        if try_http_get(endpoint).is_err() {
            return;
        }
    }
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
    let path = std::env::temp_dir().join(format!("resume-ir-s51-daemon-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

#[cfg(unix)]
fn write_fixture_executable(name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let directory = temp_dir("embedding-worker-command-bin");
    let path = directory.join(name);
    fs::write(&path, body).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).unwrap();
    path
}
