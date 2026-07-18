#![allow(dead_code)]

use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::process::{Command, Output};

pub const TEST_DAEMON_INSTANCE_ID: &str =
    "abababababababababababababababababababababababababababababababab";

pub fn write_daemon_auth(path: &Path, token: &str) {
    fs::write(
        path,
        serde_json::json!({
            "schema_version": "resume-ir.daemon-auth.v2",
            "instance_id": TEST_DAEMON_INSTANCE_ID,
            "token": token.trim(),
        })
        .to_string(),
    )
    .expect("write daemon auth fixture");
}

pub fn write_daemon_discovery(data_dir: &Path, addr: SocketAddr, token: &str) {
    fs::create_dir_all(data_dir).expect("create daemon discovery fixture directory");
    let manifest = serde_json::json!({
        "schema_version": "resume-ir.daemon-ipc.v2",
        "instance_id": TEST_DAEMON_INSTANCE_ID,
        "owner_mode": "standalone",
        "status": format!("http://{addr}/status"),
        "diagnostics": format!("http://{addr}/diagnostics"),
        "imports": format!("http://{addr}/imports"),
        "import_cancel": format!("http://{addr}/imports/cancel"),
        "import_control": format!("http://{addr}/imports/control"),
        "import_progress": format!("http://{addr}/imports/progress"),
        "search": format!("http://{addr}/search"),
        "search_batch": format!("http://{addr}/search/batch"),
        "details": format!("http://{addr}/details"),
        "delete": format!("http://{addr}/delete"),
    });
    fs::write(data_dir.join("ipc.endpoints.json"), manifest.to_string())
        .expect("write daemon discovery manifest fixture");
    write_daemon_auth(&data_dir.join("ipc.auth"), token);
}

pub fn ready_daemon_status_body() -> &'static str {
    "{\"schema_version\":\"daemon.status.v2\",\"status\":\"ok\",\"process_state\":\"ready\",\"index_health\":\"ready\"}"
}

pub fn import_text_resumes<N: AsRef<str>, T: AsRef<str>>(
    data_dir: &Path,
    source_root: &Path,
    files: &[(N, T)],
) -> Output {
    fs::create_dir_all(source_root).expect("create synthetic source root");
    for (file_name, text) in files {
        fs::write(source_root.join(file_name.as_ref()), text.as_ref())
            .expect("write synthetic resume fixture");
    }

    import_existing_root(data_dir, source_root)
}

pub fn import_existing_root(data_dir: &Path, source_root: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(data_dir),
            "import",
            "--root",
            path_str(source_root),
            "--parse-workers",
            "1",
        ])
        .output()
        .expect("run resume-cli import")
}

pub fn assert_import_succeeded(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn path_str(path: &Path) -> &str {
    path.to_str().expect("synthetic fixture path is utf-8")
}
