use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{
    Document, DocumentId, DocumentStatus, FileExtension, IngestJobId, IngestJobKind,
    IngestJobStatus, MetaStore, ResumeVersion, ResumeVersionId, ResumeVisibility, UnixTimestamp,
};

#[cfg(unix)]
#[test]
fn daemon_embedding_worker_once_persists_and_completes_per_version_jobs() {
    let data_dir = temp_dir("embedding-jobs-once-data");
    let (private_root, versions) = seed_searchable_resume_versions(&data_dir);
    let command = write_fixture_executable(
        "fixture-daemon-embedding-jobs-once",
        r#"#!/bin/sh
printf 'resume-ir-embedding-v1\n'
printf 'model_id=fixture-local-model\n'
printf 'dimension=4\n'
awk -F '\t' '/^input=/ { id=$1; sub(/^input=/, "", id); printf "vector=%s\t0.5,0.5,0.5,0.5\n", id }' "$RESUME_IR_EMBEDDING_INPUT_PATH"
"#,
    );

    let output = run_embedding_worker_once(&data_dir, &command);

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
    assert!(!stdout.contains("S52PrivateEmbeddingText"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_root)));
    assert!(!stdout.contains(path_str(&command)));

    assert_vector_snapshot(&data_dir, 4, 2);

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    for (document_id, version_id) in &versions {
        let job_id = embedding_job_id(document_id, version_id, "fixture-local-model", 4);
        let job = store
            .ingest_job_by_id(&job_id)
            .unwrap()
            .expect("embedding job persisted");
        assert_eq!(job.document_id, *document_id);
        assert_eq!(job.resume_version_id.as_ref(), Some(version_id));
        assert_eq!(job.kind, IngestJobKind::UpdateIndex);
        assert_eq!(job.status, IngestJobStatus::Completed);
        assert_eq!(job.attempt_count, 1);
    }
    assert_eq!(store.status_summary().unwrap().embedding_queue_depth, 0);

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn daemon_embedding_worker_once_writes_section_vectors_inside_one_version_job() {
    let data_dir = temp_dir("embedding-jobs-section-data");
    let (private_root, document_id, version_id) = seed_sectionized_resume_version(&data_dir);
    let command = write_fixture_executable(
        "fixture-daemon-embedding-jobs-section",
        r#"#!/bin/sh
printf 'resume-ir-embedding-v1\n'
printf 'model_id=fixture-local-model\n'
printf 'dimension=4\n'
awk -F '\t' '/^input=/ { id=$1; sub(/^input=/, "", id); printf "vector=%s\t0.5,0.5,0.5,0.5\n", id }' "$RESUME_IR_EMBEDDING_INPUT_PATH"
"#,
    );

    let output = run_embedding_worker_once(&data_dir, &command);

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("embedding worker processed: 1"), "{stdout}");
    assert!(
        stdout.contains("embedding worker vector writes: 3"),
        "{stdout}"
    );
    assert!(stdout.contains("embedding worker failed: 0"), "{stdout}");
    assert!(!stdout.contains("S52PrivateMarker"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_root)));
    assert!(!stdout.contains(path_str(&command)));

    assert_vector_snapshot(&data_dir, 4, 3);

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let job_id = embedding_job_id(&document_id, &version_id, "fixture-local-model", 4);
    let job = store
        .ingest_job_by_id(&job_id)
        .unwrap()
        .expect("embedding job persisted");
    assert_eq!(job.status, IngestJobStatus::Completed);
    assert_eq!(job.attempt_count, 1);
    assert_eq!(store.status_summary().unwrap().embedding_queue_depth, 0);

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn daemon_embedding_worker_once_skips_completed_jobs_after_restart() {
    let data_dir = temp_dir("embedding-jobs-restart-data");
    let (_private_root, _versions) = seed_searchable_resume_versions(&data_dir);
    let command = write_fixture_executable(
        "fixture-daemon-embedding-jobs-restart",
        r#"#!/bin/sh
counter="$(dirname "$0")/counter.txt"
count=0
if [ -f "$counter" ]; then
  count="$(cat "$counter")"
fi
count=$((count + 1))
printf '%s\n' "$count" > "$counter"
printf 'resume-ir-embedding-v1\n'
printf 'model_id=fixture-local-model\n'
printf 'dimension=4\n'
awk -F '\t' '/^input=/ { id=$1; sub(/^input=/, "", id); printf "vector=%s\t1,0,0,0\n", id }' "$RESUME_IR_EMBEDDING_INPUT_PATH"
"#,
    );

    let first = run_embedding_worker_once(&data_dir, &command);
    assert!(
        first.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("embedding worker processed: 2"));
    assert_eq!(read_counter(&command), 1);

    let second = run_embedding_worker_once(&data_dir, &command);
    assert!(
        second.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let second_stdout = String::from_utf8_lossy(&second.stdout);
    assert!(second_stdout.contains("embedding worker processed: 0"));
    assert_eq!(read_counter(&command), 1);

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn daemon_embedding_worker_once_reembeds_completed_jobs_for_new_model() {
    let data_dir = temp_dir("embedding-jobs-model-change-data");
    let (_private_root, _versions) = seed_searchable_resume_versions(&data_dir);
    let command = write_fixture_executable(
        "fixture-daemon-embedding-jobs-model-change",
        r#"#!/bin/sh
counter="$(dirname "$0")/counter.txt"
count=0
if [ -f "$counter" ]; then
  count="$(cat "$counter")"
fi
count=$((count + 1))
printf '%s\n' "$count" > "$counter"
model_id="$(awk -F '=' '/^model_id=/ { print $2 }' "$RESUME_IR_EMBEDDING_INPUT_PATH")"
dimension="$(awk -F '=' '/^dimension=/ { print $2 }' "$RESUME_IR_EMBEDDING_INPUT_PATH")"
printf 'resume-ir-embedding-v1\n'
printf 'model_id=%s\n' "$model_id"
printf 'dimension=%s\n' "$dimension"
awk -F '\t' '/^input=/ { id=$1; sub(/^input=/, "", id); printf "vector=%s\t0.25,0.25,0.25,0.25\n", id }' "$RESUME_IR_EMBEDDING_INPUT_PATH"
"#,
    );

    let first = run_embedding_worker_once_with_model(&data_dir, &command, "fixture-local-model-a");
    assert!(
        first.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("embedding worker processed: 2"));
    assert_eq!(read_counter(&command), 1);

    let second = run_embedding_worker_once_with_model(&data_dir, &command, "fixture-local-model-b");
    assert!(
        second.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(String::from_utf8_lossy(&second.stdout).contains("embedding worker processed: 2"));
    assert_eq!(read_counter(&command), 2);
    assert_vector_snapshot(&data_dir, 4, 4);

    remove_dir(&data_dir);
}

#[cfg(unix)]
fn run_embedding_worker_once(data_dir: &Path, command: &Path) -> std::process::Output {
    run_embedding_worker_once_with_model(data_dir, command, "fixture-local-model")
}

#[cfg(unix)]
fn run_embedding_worker_once_with_model(
    data_dir: &Path,
    command: &Path,
    model_id: &str,
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(data_dir),
            "run",
            "--foreground",
            "--once",
            "--work-embeddings-once",
            "--embedding-command",
            path_str(command),
            "--embedding-model-id",
            model_id,
            "--embedding-dimension",
            "4",
            "--embedding-max-docs",
            "8",
            "--embedding-max-text-bytes",
            "100000",
        ])
        .output()
        .expect("run daemon embedding worker once")
}

fn seed_searchable_resume_versions(
    data_dir: &Path,
) -> (PathBuf, Vec<(DocumentId, ResumeVersionId)>) {
    let now = UnixTimestamp::from_unix_seconds(1_800_052_000);
    let private_root = data_dir.join("private-resumes");
    fs::create_dir_all(&private_root).unwrap();
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    let mut versions = Vec::new();

    for index in 0..2 {
        let file_name = format!("synthetic-s52-embedding-{index}.pdf");
        let document_path = private_root.join(&file_name);
        fs::write(&document_path, b"%PDF-1.4 synthetic text-layer resume").unwrap();
        let doc_id = DocumentId::from_non_secret_parts(&["s52", "embedding", &index.to_string()]);
        let version_id = ResumeVersionId::from_non_secret_parts(&[
            "s52",
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
                content_hash: Some(format!("s52-embedding-content-hash-{index}")),
                text_hash: Some(format!("s52-embedding-text-hash-{index}")),
                is_deleted: false,
                created_at: now,
                updated_at: now,
                status: DocumentStatus::Searchable,
            })
            .unwrap();
        store
            .upsert_resume_version(&ResumeVersion {
                id: version_id.clone(),
                document_id: doc_id.clone(),
                candidate_id: None,
                parse_version: "s52-fixture-parser".to_string(),
                schema_version: "s52-fixture-schema".to_string(),
                language_set: vec!["en".to_string()],
                page_count: Some(1),
                raw_text: None,
                clean_text: Some(format!(
                    "S52PrivateEmbeddingText synthetic searchable resume {index}"
                )),
                quality_score: Some(0.91),
                visibility: ResumeVisibility::Searchable,
            })
            .unwrap();
        versions.push((doc_id, version_id));
    }

    (private_root, versions)
}

fn seed_sectionized_resume_version(data_dir: &Path) -> (PathBuf, DocumentId, ResumeVersionId) {
    let now = UnixTimestamp::from_unix_seconds(1_800_052_100);
    let private_root = data_dir.join("private-section-resumes");
    fs::create_dir_all(&private_root).unwrap();
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();

    let file_name = "synthetic-s52-section-embedding.pdf";
    let document_path = private_root.join(file_name);
    fs::write(&document_path, b"%PDF-1.4 synthetic section resume").unwrap();
    let doc_id = DocumentId::from_non_secret_parts(&["s52", "section", "embedding"]);
    let version_id =
        ResumeVersionId::from_non_secret_parts(&["s52", "section", "version", doc_id.as_str()]);
    store
        .upsert_document(&Document {
            id: doc_id.clone(),
            source_uri: format!("file://{}", path_str(&document_path)),
            normalized_path: path_str(&document_path).to_string(),
            file_name: file_name.to_string(),
            extension: FileExtension::Pdf,
            byte_size: fs::metadata(&document_path).unwrap().len(),
            mtime: now,
            content_hash: Some("s52-section-content-hash".to_string()),
            text_hash: Some("s52-section-text-hash".to_string()),
            is_deleted: false,
            created_at: now,
            updated_at: now,
            status: DocumentStatus::Searchable,
        })
        .unwrap();
    store
        .upsert_resume_version(&ResumeVersion {
            id: version_id.clone(),
            document_id: doc_id.clone(),
            candidate_id: None,
            parse_version: "s52-fixture-parser".to_string(),
            schema_version: "s52-fixture-schema".to_string(),
            language_set: vec!["en".to_string()],
            page_count: Some(1),
            raw_text: None,
            clean_text: Some(
                "Summary\nS52PrivateMarkerAlpha synthetic overview.\n\nExperience\nS52PrivateMarkerBeta synthetic delivery record."
                    .to_string(),
            ),
            quality_score: Some(0.91),
            visibility: ResumeVisibility::Searchable,
        })
        .unwrap();

    (private_root, doc_id, version_id)
}

fn embedding_job_id(
    document_id: &DocumentId,
    version_id: &ResumeVersionId,
    model_id: &str,
    dimension: usize,
) -> IngestJobId {
    let dimension = dimension.to_string();
    IngestJobId::from_non_secret_parts(&[
        "embedding-version",
        document_id.as_str(),
        version_id.as_str(),
        model_id,
        dimension.as_str(),
    ])
}

fn assert_vector_snapshot(data_dir: &Path, expected_dimension: usize, expected_vectors: usize) {
    let snapshot = fs::read_to_string(data_dir.join("vector-index").join("vector.snapshot"))
        .expect("read vector snapshot");
    let mut lines = snapshot.lines();
    let expected_header = format!("resume-ir-vector-index-v2\tdimension\t{expected_dimension}");
    assert_eq!(lines.next(), Some(expected_header.as_str()));
    let vectors = lines.filter(|line| line.starts_with("V\t")).count();
    assert_eq!(vectors, expected_vectors);
}

fn read_counter(command: &Path) -> u64 {
    let counter = command.parent().unwrap().join("counter.txt");
    fs::read_to_string(counter)
        .expect("read fixture command counter")
        .trim()
        .parse()
        .unwrap()
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s52-daemon-{label}-{unique}"));
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
