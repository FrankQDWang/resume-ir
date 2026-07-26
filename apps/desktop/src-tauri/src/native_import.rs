use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use serde::{Deserialize, Serialize};

use crate::daemon_client::DesktopError;

const MAX_REGISTERED_ROOTS: usize = 16;
const MAX_DISPLAY_LABEL_CHARS: usize = 80;
const MAX_MANAGED_ROOT_PATH_BYTES: usize = 128 * 1024;
const MAX_MANAGED_ROOT_LEDGER_BYTES: u64 = 64 * 1024;
const LEGACY_MANAGED_ROOT_LEDGER_FILE: &str = "managed-roots.v1.json";
const LEGACY_MANAGED_ROOT_SCHEMA: &str = "resume-ir.desktop-managed-roots.v1";
pub(crate) const MAX_DIAGNOSTICS_EXPORT_BYTES: usize = 256 * 1024;
const MAX_EXPORT_LABEL_CHARS: usize = 80;

#[derive(Serialize)]
pub(crate) struct DiagnosticsExportReceipt {
    status: &'static str,
    file_label: String,
}

#[derive(Clone)]
pub(crate) struct PreparedImportRoot {
    path: PathBuf,
    display_label: String,
}

impl PreparedImportRoot {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn display_label(&self) -> &str {
        &self.display_label
    }
}

/// One-shot reader for the retired desktop managed-roots v1 ledger.
///
/// New directory authority is owned exclusively by the daemon metadata store.
/// This state exists only long enough to register validated legacy roots there
/// and permanently remove the old ledger.
pub(crate) struct LegacyManagedRootsMigration {
    state: Mutex<LegacyManagedRootsMigrationState>,
    ledger_path: PathBuf,
}

enum LegacyManagedRootsMigrationState {
    Pending(Vec<PreparedImportRoot>),
    Invalid,
    Retired,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyManagedRootLedger {
    schema_version: String,
    roots: Vec<LegacyPersistedManagedRoot>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyPersistedManagedRoot {
    root_handle: String,
    display_label: String,
    canonical_path: String,
}

impl LegacyManagedRootsMigration {
    pub(crate) fn initialize(data_dir: &Path) -> Result<Self, DesktopError> {
        if !data_dir.is_absolute() {
            return Err(managed_roots_invalid());
        }
        fs::create_dir_all(data_dir).map_err(|_| managed_roots_invalid())?;
        if !data_dir.is_dir() {
            return Err(managed_roots_invalid());
        }
        let ledger_path = data_dir.join(LEGACY_MANAGED_ROOT_LEDGER_FILE);
        let state = match load_legacy_managed_roots(&ledger_path) {
            Ok(roots) => LegacyManagedRootsMigrationState::Pending(roots),
            Err(_) => LegacyManagedRootsMigrationState::Invalid,
        };
        Ok(Self {
            state: Mutex::new(state),
            ledger_path,
        })
    }

    pub(crate) fn pending_roots(&self) -> Result<Vec<PreparedImportRoot>, DesktopError> {
        let state = self.state.lock().map_err(|_| DesktopError::internal())?;
        match &*state {
            LegacyManagedRootsMigrationState::Pending(roots) => Ok(roots.clone()),
            LegacyManagedRootsMigrationState::Invalid => Err(managed_roots_invalid()),
            LegacyManagedRootsMigrationState::Retired => Ok(Vec::new()),
        }
    }

    pub(crate) fn retire(&self) -> Result<(), DesktopError> {
        let mut state = self.state.lock().map_err(|_| DesktopError::internal())?;
        match &*state {
            LegacyManagedRootsMigrationState::Invalid => return Err(managed_roots_invalid()),
            LegacyManagedRootsMigrationState::Retired => return Ok(()),
            LegacyManagedRootsMigrationState::Pending(_) => {}
        }
        match fs::symlink_metadata(&self.ledger_path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(managed_roots_invalid());
            }
            Ok(_) => fs::remove_file(&self.ledger_path).map_err(|_| managed_roots_invalid())?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(managed_roots_invalid()),
        }
        *state = LegacyManagedRootsMigrationState::Retired;
        Ok(())
    }
}

pub(crate) fn prepare_import_root(
    selected_path: &Path,
) -> Result<PreparedImportRoot, DesktopError> {
    let metadata = fs::symlink_metadata(selected_path)
        .map_err(|_| DesktopError::new("import_root_unreadable", "所选目录不存在或当前不可读取"))?;
    if !metadata.is_dir() || is_link_or_reparse_point(&metadata) {
        return Err(DesktopError::new(
            "import_root_invalid",
            "所选位置不是安全的目录",
        ));
    }
    let path = fs::canonicalize(selected_path)
        .map_err(|_| DesktopError::new("import_root_unreadable", "所选目录不存在或当前不可读取"))?;
    let path_text = path
        .to_str()
        .filter(|value| !value.is_empty() && value.len() <= MAX_MANAGED_ROOT_PATH_BYTES)
        .ok_or_else(|| DesktopError::new("import_root_invalid", "所选目录无法用于本地导入"))?;
    if path_text.as_bytes().contains(&0) {
        return Err(DesktopError::new(
            "import_root_invalid",
            "所选目录无法用于本地导入",
        ));
    }
    let display_label = bounded_display_label(&path);
    if display_label.chars().any(char::is_control) {
        return Err(DesktopError::new(
            "import_root_invalid",
            "所选目录无法用于本地导入",
        ));
    }
    Ok(PreparedImportRoot {
        path,
        display_label,
    })
}

pub(crate) async fn pick_import_root() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title("选择要导入的简历目录")
        .pick_folder()
        .await
        .map(|handle| handle.path().to_path_buf())
}

pub(crate) async fn pick_diagnostics_export_path() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title("导出脱敏诊断")
        .set_file_name("resume-ir-diagnostics.json")
        .add_filter("JSON", &["json"])
        .save_file()
        .await
        .map(|handle| handle.path().to_path_buf())
}

pub(crate) fn write_diagnostics_export(
    path: &Path,
    payload: &[u8],
) -> Result<DiagnosticsExportReceipt, DesktopError> {
    let mut body = payload.to_vec();
    if !body.ends_with(b"\n") {
        body.push(b'\n');
    }
    if body.len() > MAX_DIAGNOSTICS_EXPORT_BYTES {
        return Err(DesktopError::new(
            "diagnostics_too_large",
            "脱敏诊断超过本地导出上限",
        ));
    }
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| DesktopError::new("diagnostics_export_failed", "无法写入所选导出位置"))?;
    file.write_all(&body)
        .and_then(|_| file.flush())
        .map_err(|_| DesktopError::new("diagnostics_export_failed", "无法写入所选导出位置"))?;
    Ok(DiagnosticsExportReceipt {
        status: "saved",
        file_label: bounded_label(path, MAX_EXPORT_LABEL_CHARS, "resume-ir-diagnostics.json"),
    })
}

fn managed_roots_invalid() -> DesktopError {
    DesktopError::new(
        "managed_roots_invalid",
        "旧版本地授权目录记录无效，已停止迁移",
    )
}

fn load_legacy_managed_roots(path: &Path) -> Result<Vec<PreparedImportRoot>, DesktopError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(managed_roots_invalid()),
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_MANAGED_ROOT_LEDGER_BYTES
        || !owner_only_permissions(&metadata)
    {
        return Err(managed_roots_invalid());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| {
            file.take(MAX_MANAGED_ROOT_LEDGER_BYTES + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|_| managed_roots_invalid())?;
    if bytes.len() as u64 > MAX_MANAGED_ROOT_LEDGER_BYTES {
        return Err(managed_roots_invalid());
    }
    let ledger = serde_json::from_slice::<LegacyManagedRootLedger>(&bytes)
        .map_err(|_| managed_roots_invalid())?;
    if ledger.schema_version != LEGACY_MANAGED_ROOT_SCHEMA
        || ledger.roots.len() > MAX_REGISTERED_ROOTS
    {
        return Err(managed_roots_invalid());
    }

    let mut handles = HashSet::with_capacity(ledger.roots.len());
    let mut paths = Vec::<PathBuf>::with_capacity(ledger.roots.len());
    let mut roots = Vec::with_capacity(ledger.roots.len());
    for persisted in ledger.roots {
        if !valid_legacy_root_handle(&persisted.root_handle)
            || !handles.insert(persisted.root_handle)
        {
            return Err(managed_roots_invalid());
        }
        let path = PathBuf::from(&persisted.canonical_path);
        if !path.is_absolute()
            || persisted.canonical_path.is_empty()
            || persisted.canonical_path.len() > MAX_MANAGED_ROOT_PATH_BYTES
            || persisted.canonical_path.as_bytes().contains(&0)
            || persisted.display_label != bounded_display_label(&path)
            || persisted.display_label.chars().any(char::is_control)
            || paths.iter().any(|existing| {
                existing == &path || existing.starts_with(&path) || path.starts_with(existing)
            })
        {
            return Err(managed_roots_invalid());
        }
        paths.push(path.clone());
        roots.push(PreparedImportRoot {
            path,
            display_label: persisted.display_label,
        });
    }
    Ok(roots)
}

fn valid_legacy_root_handle(handle: &str) -> bool {
    handle.strip_prefix("root-").is_some_and(|suffix| {
        suffix.len() == 32
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

#[cfg(unix)]
fn owner_only_permissions(metadata: &fs::Metadata) -> bool {
    metadata.permissions().mode() & 0o077 == 0
}

#[cfg(not(unix))]
fn owner_only_permissions(_metadata: &fs::Metadata) -> bool {
    true
}

#[cfg(not(windows))]
fn is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn bounded_display_label(path: &Path) -> String {
    bounded_label(path, MAX_DISPLAY_LABEL_CHARS, "已选择目录")
}

fn bounded_label(path: &Path, max_chars: usize, fallback: &str) -> String {
    let label = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback);
    let label_chars = label.chars().count();
    let visible_chars = if label_chars > max_chars {
        max_chars.saturating_sub(3)
    } else {
        max_chars
    };
    let mut bounded = label.chars().take(visible_chars).collect::<String>();
    if label_chars > max_chars {
        bounded.push_str("...");
    }
    bounded
}
