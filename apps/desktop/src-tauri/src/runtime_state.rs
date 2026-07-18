use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::daemon_client::DesktopError;

pub(crate) struct DesktopRuntimeState {
    data_dir: PathBuf,
}

impl DesktopRuntimeState {
    pub(crate) fn initialize(
        app_local_data_dir: PathBuf,
        debug_override: Option<OsString>,
    ) -> Result<Self, DesktopError> {
        let data_dir = debug_override
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or(app_local_data_dir);
        if !data_dir.is_absolute() {
            return Err(data_dir_unavailable());
        }
        std::fs::create_dir_all(&data_dir).map_err(|_| data_dir_unavailable())?;
        if !data_dir.is_dir() {
            return Err(data_dir_unavailable());
        }
        Ok(Self { data_dir })
    }

    pub(crate) fn data_dir(&self) -> &Path {
        &self.data_dir
    }
}

#[cfg(debug_assertions)]
pub(crate) fn configured_debug_data_dir() -> Option<OsString> {
    std::env::var_os("RESUME_IR_DATA_DIR").filter(|value| !value.is_empty())
}

#[cfg(not(debug_assertions))]
pub(crate) fn configured_debug_data_dir() -> Option<OsString> {
    None
}

fn data_dir_unavailable() -> DesktopError {
    DesktopError::new("data_dir_unavailable", "无法准备本地 resume-ir 数据目录")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temporary_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "resume-ir-desktop-runtime-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn app_local_data_is_created_without_an_environment_override() {
        let app_local = temporary_path("app-local");
        let state = DesktopRuntimeState::initialize(app_local.clone(), None).unwrap();
        assert_eq!(state.data_dir(), app_local);
        assert!(state.data_dir().is_dir());
        fs::remove_dir_all(app_local).unwrap();
    }

    #[test]
    fn an_absolute_debug_override_wins_without_exposing_it_to_webview_state() {
        let app_local = temporary_path("unused-app-local");
        let debug_override = temporary_path("debug-override");
        let state = DesktopRuntimeState::initialize(
            app_local,
            Some(debug_override.clone().into_os_string()),
        )
        .unwrap();
        assert_eq!(state.data_dir(), debug_override);
        assert!(state.data_dir().is_dir());
        fs::remove_dir_all(debug_override).unwrap();
    }

    #[test]
    fn a_relative_debug_override_fails_closed() {
        let result = DesktopRuntimeState::initialize(
            temporary_path("app-local-relative"),
            Some(OsString::from("relative-data")),
        );
        assert!(result.is_err());
    }
}
