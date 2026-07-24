use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::daemon_client::DesktopError;
use crate::daemon_connection::ConnectionGenerationSource;

#[path = "daemon_lifecycle/policy.rs"]
mod policy;
#[path = "daemon_lifecycle/process.rs"]
mod process;
#[path = "daemon_lifecycle/receipt.rs"]
mod receipt;
#[path = "daemon_lifecycle/runtime_candidates.rs"]
mod runtime_candidates;
#[path = "daemon_lifecycle/supervisor.rs"]
mod supervisor;
#[path = "daemon_lifecycle/supervisor_contract.rs"]
mod supervisor_contract;

use process::ProductionDaemonRuntime;
use receipt::LifecycleReceiptRecorder;
pub(crate) use supervisor::{DaemonLifecycleSnapshot, DaemonLifecycleState, ReadyDaemonIdentity};

impl DaemonLifecycleState {
    pub(crate) fn initialize(
        data_dir: &Path,
        current_exe: &Path,
        embedding_resource_dir: &Path,
        ocr_resource_dir: &Path,
        classifier_resource_dir: &Path,
    ) -> Result<Self, DesktopError> {
        Self::launch(
            ProductionDaemonRuntime::initialize(
                data_dir,
                current_exe,
                embedding_resource_dir,
                ocr_resource_dir,
                classifier_resource_dir,
            ),
            LifecycleReceiptRecorder::initialize(data_dir),
        )
    }
}

impl ConnectionGenerationSource for DaemonLifecycleState {
    fn ready_identity(&self) -> Option<ReadyDaemonIdentity> {
        self.ready_identity()
    }
}

fn configured_daemon_binary() -> Result<PathBuf, DesktopError> {
    let configured = configured_debug_daemon_binary();
    let current_exe = std::env::current_exe().map_err(|_| {
        DesktopError::new(
            "daemon_binary_unavailable",
            "无法定位本地 daemon 可执行文件",
        )
    })?;
    let debug_binary = debug_daemon_binary();
    select_daemon_binary(configured.as_deref(), &current_exe, debug_binary.as_deref()).ok_or_else(
        || {
            DesktopError::new(
                "daemon_binary_unavailable",
                "本地 daemon 可执行文件尚未准备好",
            )
        },
    )
}

#[cfg(debug_assertions)]
fn debug_daemon_binary() -> Option<PathBuf> {
    Some(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../target/debug")
            .join(binary_name()),
    )
}

#[cfg(not(debug_assertions))]
fn debug_daemon_binary() -> Option<PathBuf> {
    None
}

#[cfg(debug_assertions)]
fn configured_debug_daemon_binary() -> Option<OsString> {
    std::env::var_os("RESUME_IR_DAEMON_BINARY").filter(|value| !value.is_empty())
}

#[cfg(not(debug_assertions))]
fn configured_debug_daemon_binary() -> Option<OsString> {
    None
}

fn select_daemon_binary(
    configured: Option<&OsStr>,
    current_exe: &Path,
    debug_binary: Option<&Path>,
) -> Option<PathBuf> {
    let sibling = current_exe
        .parent()
        .map(|parent| parent.join(binary_name()));
    configured
        .map(PathBuf::from)
        .into_iter()
        .chain(sibling)
        .chain(debug_binary.map(PathBuf::from))
        .find(|candidate| candidate.is_file())
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "resume-daemon.exe"
    } else {
        "resume-daemon"
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn explicit_daemon_binary_wins() {
        let root = std::env::temp_dir().join(format!(
            "resume-ir-desktop-lifecycle-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let configured = root.join("configured-daemon");
        let sibling = root.join(binary_name());
        fs::write(&configured, "synthetic").unwrap();
        fs::write(&sibling, "synthetic").unwrap();

        let selected =
            select_daemon_binary(Some(configured.as_os_str()), &root.join("desktop"), None);
        assert_eq!(selected.as_deref(), Some(configured.as_path()));

        fs::remove_dir_all(root).unwrap();
    }
}
