use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use index_fulltext::{FullTextIndex, IndexDocument, IndexSection};
use index_vector::{PersistentVectorIndex, VectorDocument, VectorIndex};
use meta_store::{
    Document, DocumentId, DocumentStatus, FileExtension, MetaStore, ResumeVersion, ResumeVersionId,
    ResumeVisibility, UnixTimestamp,
};

#[test]
fn embed_worker_without_command_reports_blocked_without_path_leak() {
    let data_dir = temp_dir("embed-worker-no-command-data");
    let fixture_root = fixture_root();
    import_fixtures(&data_dir, &fixture_root);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "embed-worker", "--once"])
        .output()
        .expect("run embed worker without command");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("embedding worker blocked: local embedding command not configured"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&fixture_root)));

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn embed_worker_runs_local_command_and_persists_vector_snapshot_without_hiding_search_results() {
    let data_dir = temp_dir("embed-worker-command-data");
    let fixture_root = fixture_root();
    let command = write_fixture_executable(
        "fixture-embedding-worker",
        r#"#!/bin/sh
if [ ! -s "$RESUME_IR_EMBEDDING_INPUT_PATH" ]; then
  exit 7
fi
printf 'resume-ir-embedding-v1\n'
printf 'model_id=fixture-local-model\n'
printf 'dimension=4\n'
awk -F '\t' '/^input=/ { sub(/^input=/, "", $1); printf "vector=%s\t0.5,0.5,0.5,0.5\n", $1 }' "$RESUME_IR_EMBEDDING_INPUT_PATH"
printf 'metadata=synthetic-fixture\n'
"#,
    );
    import_fixtures(&data_dir, &fixture_root);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "embed-worker",
            "--once",
            "--command",
            path_str(&command),
            "--model-id",
            "fixture-local-model",
            "--dimension",
            "4",
            "--max-docs",
            "8",
            "--max-text-bytes",
            "100000",
        ])
        .output()
        .expect("run embed worker with local command");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("embedding worker: completed"));
    assert!(stdout.contains("model id: fixture-local-model"));
    assert!(stdout.contains("dimension: 4"));
    assert!(stdout.contains("documents considered: 2"));
    assert!(stdout.contains("documents embedded: 2"));
    assert!(stdout.contains("vector index: available (vector snapshot)"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));

    let status = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "status"])
        .output()
        .expect("run resume-cli status");
    assert!(status.status.success());
    assert!(status.stderr.is_empty());
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("searchable documents: 2"));
    assert!(status_stdout.contains("vector index: available (vector snapshot)"));
    assert!(status_stdout.contains("vector index vectors: 2"));
    assert!(!status_stdout.contains(path_str(&data_dir)));
    assert!(!status_stdout.contains(path_str(&fixture_root)));

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Java"])
        .output()
        .expect("run resume-cli search after embedding");
    assert!(search.status.success());
    assert!(search.stderr.is_empty());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(search_stdout.contains("results: 2"));
    assert!(search_stdout.contains("synthetic-java-platform.pdf"));
    assert!(search_stdout.contains("synthetic-java-engineer.docx"));

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn semantic_and_hybrid_search_use_persistent_vector_snapshot_with_local_query_embedding() {
    let data_dir = temp_dir("semantic-search-data");
    let fixture_root = fixture_root();
    let command = write_fixture_executable(
        "fixture-semantic-search-embedding",
        r#"#!/bin/sh
printf 'resume-ir-embedding-v1\n'
printf 'model_id=fixture-local-model\n'
printf 'dimension=4\n'
awk -F '\t' '/^input=/ { id=$1; sub(/^input=/, "", id); printf "vector=%s\t1,0,0,0\n", id }' "$RESUME_IR_EMBEDDING_INPUT_PATH"
"#,
    );
    import_fixtures(&data_dir, &fixture_root);

    let embed = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "embed-worker",
            "--once",
            "--command",
            path_str(&command),
            "--model-id",
            "fixture-local-model",
            "--dimension",
            "4",
            "--max-docs",
            "8",
            "--max-text-bytes",
            "100000",
        ])
        .output()
        .expect("run embed worker before semantic search");
    assert!(
        embed.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&embed.stdout),
        String::from_utf8_lossy(&embed.stderr)
    );

    for mode in ["semantic", "hybrid"] {
        let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
            .args([
                "--data-dir",
                path_str(&data_dir),
                "search",
                "SemanticOnlyToken",
                "--mode",
                mode,
                "--embedding-command",
                path_str(&command),
                "--model-id",
                "fixture-local-model",
                "--top-k",
                "20",
            ])
            .output()
            .expect("run semantic or hybrid search");
        assert!(
            search.status.success(),
            "mode: {mode}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&search.stdout),
            String::from_utf8_lossy(&search.stderr)
        );
        assert!(search.stderr.is_empty());
        let stdout = String::from_utf8_lossy(&search.stdout);
        assert!(
            stdout.contains("results: 2"),
            "mode: {mode}\nstdout:\n{stdout}"
        );
        assert!(stdout.contains("synthetic-java-platform.pdf"));
        assert!(stdout.contains("synthetic-java-engineer.docx"));
        assert!(!stdout.contains("SemanticOnlyToken"));
        assert!(!stdout.contains(path_str(&data_dir)));
        assert!(!stdout.contains(path_str(&fixture_root)));
    }

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn semantic_search_reports_missing_vector_snapshot_even_when_dimension_is_supplied() {
    let data_dir = temp_dir("semantic-missing-vector-snapshot-data");
    let command = write_fixture_executable(
        "fixture-semantic-missing-vector-snapshot-embedding",
        r#"#!/bin/sh
printf 'resume-ir-embedding-v1\n'
printf 'model_id=fixture-local-model\n'
printf 'dimension=4\n'
awk -F '\t' '/^input=/ { id=$1; sub(/^input=/, "", id); printf "vector=%s\t1,0,0,0\n", id }' "$RESUME_IR_EMBEDDING_INPUT_PATH"
"#,
    );

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "SemanticOnlyToken",
            "--mode",
            "semantic",
            "--embedding-command",
            path_str(&command),
            "--model-id",
            "fixture-local-model",
            "--dimension",
            "4",
        ])
        .output()
        .expect("run semantic search without a vector snapshot");

    assert!(!search.status.success());
    assert!(search.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&search.stderr);
    assert!(stderr.contains("semantic search unavailable: vector index is missing"));
    assert!(!stderr.contains("SemanticOnlyToken"));
    assert!(!stderr.contains(path_str(&data_dir)));

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn semantic_search_uses_only_vectors_for_requested_model() {
    let data_dir = temp_dir("semantic-model-scope-data");
    let old_document_id = DocumentId::from_non_secret_parts(&["s54", "old-model-doc"]);
    let old_version_id = ResumeVersionId::from_non_secret_parts(&["s54", "old-model-version"]);
    let current_document_id = DocumentId::from_non_secret_parts(&["s54", "current-model-doc"]);
    let current_version_id =
        ResumeVersionId::from_non_secret_parts(&["s54", "current-model-version"]);
    seed_searchable_metadata(
        &data_dir,
        &old_document_id,
        &old_version_id,
        "synthetic-old-model.pdf",
        "old model only semantic vector",
    );
    seed_searchable_metadata(
        &data_dir,
        &current_document_id,
        &current_version_id,
        "synthetic-current-model.pdf",
        "current model semantic vector",
    );
    seed_fulltext_index(
        &data_dir,
        [
            IndexDocument {
                doc_id: old_document_id.to_string(),
                version_id: old_version_id.to_string(),
                file_name: "synthetic-old-model.pdf".to_string(),
                clean_text: "old model only vector document".to_string(),
                sections: vec![IndexSection {
                    section_type: "experience".to_string(),
                    text: "old model only vector document".to_string(),
                }],
                is_deleted: false,
            },
            IndexDocument {
                doc_id: current_document_id.to_string(),
                version_id: current_version_id.to_string(),
                file_name: "synthetic-current-model.pdf".to_string(),
                clean_text: "current model vector document".to_string(),
                sections: vec![IndexSection {
                    section_type: "experience".to_string(),
                    text: "current model vector document".to_string(),
                }],
                is_deleted: false,
            },
        ],
    );

    let vector_index = PersistentVectorIndex::open(data_dir.join("vector-index"), 4).unwrap();
    vector_index
        .upsert(vec![
            VectorDocument::new_for_model(
                "model-a",
                format!("model-a:{old_version_id}"),
                old_document_id.to_string(),
                vec![1.0, 0.0, 0.0, 0.0],
            )
            .unwrap(),
            VectorDocument::new_for_model(
                "model-b",
                format!("model-b:{current_version_id}"),
                current_document_id.to_string(),
                vec![0.0, 1.0, 0.0, 0.0],
            )
            .unwrap(),
        ])
        .unwrap();

    let command = write_fixture_executable(
        "fixture-semantic-model-scope-embedding",
        r#"#!/bin/sh
printf 'resume-ir-embedding-v1\n'
printf 'model_id=model-b\n'
printf 'dimension=4\n'
awk -F '\t' '/^input=/ { id=$1; sub(/^input=/, "", id); printf "vector=%s\t1,0,0,0\n", id }' "$RESUME_IR_EMBEDDING_INPUT_PATH"
"#,
    );

    for mode in ["semantic", "hybrid"] {
        let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
            .args([
                "--data-dir",
                path_str(&data_dir),
                "search",
                "SemanticOnlyToken",
                "--mode",
                mode,
                "--embedding-command",
                path_str(&command),
                "--model-id",
                "model-b",
                "--dimension",
                "4",
                "--vector-top-k",
                "1",
                "--top-k",
                "1",
            ])
            .output()
            .expect("run semantic or hybrid search with mixed-model vector snapshot");

        assert!(
            search.status.success(),
            "mode: {mode}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&search.stdout),
            String::from_utf8_lossy(&search.stderr)
        );
        assert!(search.stderr.is_empty());
        let stdout = String::from_utf8_lossy(&search.stdout);
        assert!(stdout.contains("results: 1"), "mode: {mode}\n{stdout}");
        assert!(
            stdout.contains("synthetic-current-model.pdf"),
            "mode: {mode}\n{stdout}"
        );
        assert!(
            !stdout.contains("synthetic-old-model.pdf"),
            "mode: {mode}\n{stdout}"
        );
        assert!(!stdout.contains("SemanticOnlyToken"));
        assert!(!stdout.contains(path_str(&data_dir)));
    }

    remove_dir(&data_dir);
}

fn import_fixtures(data_dir: &Path, fixture_root: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(data_dir),
            "import",
            "--root",
            path_str(fixture_root),
        ])
        .output()
        .expect("import fixtures");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn seed_fulltext_index<const N: usize>(data_dir: &Path, documents: [IndexDocument; N]) {
    let index = FullTextIndex::open_or_create(&data_dir.join("search-index")).unwrap();
    index.replace_documents(documents).unwrap();
    index.commit().unwrap();
}

fn seed_searchable_metadata(
    data_dir: &Path,
    document_id: &DocumentId,
    version_id: &ResumeVersionId,
    file_name: &str,
    text: &str,
) {
    let now = UnixTimestamp::from_unix_seconds(1_800_054_000);
    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    store
        .upsert_document(&Document {
            id: document_id.clone(),
            source_uri: format!("synthetic://{file_name}"),
            normalized_path: format!("synthetic/{file_name}"),
            file_name: file_name.to_string(),
            extension: FileExtension::Pdf,
            byte_size: 256,
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
            raw_text: Some(text.to_string()),
            clean_text: Some(text.to_string()),
            quality_score: Some(0.9),
            visibility: ResumeVisibility::Searchable,
        })
        .unwrap();
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/resumes")
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s39-cli-{label}-{unique}"));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

#[cfg(unix)]
fn write_fixture_executable(name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let directory = temp_dir("embed-worker-command-bin");
    let path = directory.join(name);
    std::fs::write(&path, body).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}
