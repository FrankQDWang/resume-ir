use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{EntityType, MetaStore};

#[test]
fn filtered_search_uses_persisted_field_mentions_without_reextracting_clean_text() {
    let data_dir = temp_dir("persisted-fields-data");
    import_fixtures(&data_dir);

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let versions = store
        .visible_documents()
        .unwrap()
        .into_iter()
        .filter(|document| document.file_name != "synthetic-scanned-resume.pdf")
        .flat_map(|document| store.resume_versions_for_document(&document.id).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(versions.len(), 2);
    for version in &versions {
        let mentions = store.entity_mentions_for_version(&version.id).unwrap();
        assert!(mentions
            .iter()
            .any(|mention| mention.entity_type == EntityType::Degree));
        assert!(mentions
            .iter()
            .any(|mention| mention.entity_type == EntityType::Skill));
        assert!(mentions
            .iter()
            .any(|mention| { mention.entity_type == EntityType::YearsExperience }));
    }

    for mut version in versions {
        version.raw_text = None;
        version.clean_text = None;
        store.upsert_resume_version(&version).unwrap();
    }

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "Java",
            "--degree",
            "bachelor",
            "--skills-any",
            "java",
            "--years-experience-min",
            "4",
            "--top-k",
            "20",
        ])
        .output()
        .expect("run persisted-field filtered search");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("results: 2"));
    assert!(stdout.contains("synthetic-java-engineer.docx"));
    assert!(stdout.contains("synthetic-java-platform.pdf"));

    remove_dir(&data_dir);
}

fn import_fixtures(data_dir: &Path) {
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/resumes");
    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(data_dir),
            "import",
            "--root",
            path_str(&fixture_root),
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

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s16-cli-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}
