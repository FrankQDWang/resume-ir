use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use uuid::Uuid;

use crate::daemon_client::DesktopError;

const MAX_REGISTERED_ROOTS: usize = 16;
const MAX_DISPLAY_LABEL_CHARS: usize = 80;
const MAX_MANAGED_ROOT_PATH_BYTES: usize = 128 * 1024;
const MAX_MANAGED_ROOT_LEDGER_BYTES: u64 = 64 * 1024;
const MANAGED_ROOT_LEDGER_FILE: &str = "managed-roots.v1.json";
const MANAGED_ROOT_SCHEMA: &str = "resume-ir.desktop-managed-roots.v1";
pub(crate) const MAX_DIAGNOSTICS_EXPORT_BYTES: usize = 256 * 1024;
const MAX_EXPORT_LABEL_CHARS: usize = 80;

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct SelectedImportRoot {
    root_handle: String,
    display_label: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RootAvailability {
    Available,
    Unavailable,
}

#[derive(Serialize)]
pub(crate) struct ManagedRoots {
    schema_version: &'static str,
    limit: usize,
    roots: Vec<ManagedRoot>,
}

#[derive(Serialize)]
pub(crate) struct ManagedRoot {
    root_handle: String,
    display_label: String,
    availability: RootAvailability,
}

#[derive(Serialize)]
pub(crate) struct DiagnosticsExportReceipt {
    status: &'static str,
    file_label: String,
}

#[derive(Clone)]
struct RegisteredRoot {
    handle: String,
    path: PathBuf,
    display_label: String,
}

pub(crate) struct PreparedImportRoot {
    path: PathBuf,
    display_label: String,
}

#[derive(Clone, Default)]
struct RootRegistry {
    roots: VecDeque<RegisteredRoot>,
}

pub(crate) struct NativeImportState {
    roots: Mutex<RootRegistry>,
    ledger_path: PathBuf,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ManagedRootLedger {
    schema_version: String,
    roots: Vec<PersistedManagedRoot>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedManagedRoot {
    root_handle: String,
    display_label: String,
    canonical_path: String,
}

impl NativeImportState {
    pub(crate) fn initialize(data_dir: &Path) -> Result<Self, DesktopError> {
        if !data_dir.is_absolute() {
            return Err(managed_roots_invalid());
        }
        fs::create_dir_all(data_dir).map_err(|_| managed_roots_invalid())?;
        if !data_dir.is_dir() {
            return Err(managed_roots_invalid());
        }
        let ledger_path = data_dir.join(MANAGED_ROOT_LEDGER_FILE);
        let roots = load_managed_roots(&ledger_path)?;
        Ok(Self {
            roots: Mutex::new(roots),
            ledger_path,
        })
    }

    pub(crate) fn register(
        &self,
        prepared: PreparedImportRoot,
    ) -> Result<SelectedImportRoot, DesktopError> {
        let current = self
            .roots
            .lock()
            .map_err(|_| DesktopError::internal())?
            .clone();
        if let Some(existing) = current.roots.iter().find(|root| root.path == prepared.path) {
            return Ok(SelectedImportRoot {
                root_handle: existing.handle.clone(),
                display_label: existing.display_label.clone(),
            });
        }
        if current.roots.len() >= MAX_REGISTERED_ROOTS {
            return Err(DesktopError::new(
                "managed_root_limit",
                "已达到本地授权目录数量上限",
            ));
        }
        if current.roots.iter().any(|root| {
            prepared.path.starts_with(&root.path) || root.path.starts_with(&prepared.path)
        }) {
            return Err(DesktopError::new(
                "managed_root_overlap",
                "所选目录与现有授权目录重叠",
            ));
        }

        let handle = format!("root-{}", Uuid::new_v4().simple());
        let selected = SelectedImportRoot {
            root_handle: handle.clone(),
            display_label: prepared.display_label.clone(),
        };
        let mut next = current;
        next.roots.push_back(RegisteredRoot {
            handle: handle.clone(),
            path: prepared.path,
            display_label: prepared.display_label,
        });
        persist_managed_roots(&self.ledger_path, &next)?;
        *self.roots.lock().map_err(|_| DesktopError::internal())? = next;
        Ok(selected)
    }

    pub(crate) fn resolve(&self, handle: &str) -> Result<PathBuf, DesktopError> {
        if !valid_root_handle(handle) {
            return Err(invalid_handle());
        }
        let roots = self.roots.lock().map_err(|_| DesktopError::internal())?;
        roots
            .roots
            .iter()
            .find(|root| root.handle == handle)
            .map(|root| root.path.clone())
            .ok_or_else(invalid_handle)
    }

    pub(crate) fn reauthorize(
        &self,
        handle: &str,
        prepared: PreparedImportRoot,
    ) -> Result<SelectedImportRoot, DesktopError> {
        if !valid_root_handle(handle) {
            return Err(invalid_handle());
        }
        let roots = self.roots.lock().map_err(|_| DesktopError::internal())?;
        let existing = roots
            .roots
            .iter()
            .find(|root| root.handle == handle)
            .ok_or_else(invalid_handle)?;
        if existing.path != prepared.path {
            return Err(DesktopError::new(
                "managed_root_mismatch",
                "所选目录与待恢复授权不一致",
            ));
        }
        Ok(SelectedImportRoot {
            root_handle: existing.handle.clone(),
            display_label: existing.display_label.clone(),
        })
    }

    pub(crate) fn resolve_for_import(&self, handle: &str) -> Result<PathBuf, DesktopError> {
        let authorized = self.resolve(handle)?;
        let current = fs::canonicalize(&authorized).map_err(|_| import_root_unavailable())?;
        if current != authorized || !current.is_dir() {
            return Err(import_root_unavailable());
        }
        Ok(current)
    }

    pub(crate) fn managed_roots(&self) -> Result<ManagedRoots, DesktopError> {
        let roots = self
            .roots
            .lock()
            .map_err(|_| DesktopError::internal())?
            .clone();
        Ok(ManagedRoots {
            schema_version: MANAGED_ROOT_SCHEMA,
            limit: MAX_REGISTERED_ROOTS,
            roots: roots
                .roots
                .into_iter()
                .map(|root| ManagedRoot {
                    root_handle: root.handle,
                    display_label: root.display_label,
                    availability: if authorized_root_is_available(&root.path) {
                        RootAvailability::Available
                    } else {
                        RootAvailability::Unavailable
                    },
                })
                .collect(),
        })
    }
}

pub(crate) fn prepare_import_root(
    selected_path: &Path,
) -> Result<PreparedImportRoot, DesktopError> {
    let metadata = fs::metadata(selected_path)
        .map_err(|_| DesktopError::new("import_root_unreadable", "所选目录不存在或当前不可读取"))?;
    if !metadata.is_dir() {
        return Err(DesktopError::new("import_root_invalid", "所选位置不是目录"));
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

pub(crate) async fn pick_reauthorization_root() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title("重新授权原目录")
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

fn invalid_handle() -> DesktopError {
    DesktopError::new("import_root_expired", "所选目录句柄已失效，请重新选择")
}

fn managed_roots_invalid() -> DesktopError {
    DesktopError::new("managed_roots_invalid", "本地授权目录记录无效，已停止加载")
}

fn import_root_unavailable() -> DesktopError {
    DesktopError::new(
        "import_root_unavailable",
        "授权目录当前不可读取，请恢复磁盘或权限",
    )
}

fn authorized_root_is_available(path: &Path) -> bool {
    fs::canonicalize(path).is_ok_and(|current| current == path && current.is_dir())
}

fn load_managed_roots(path: &Path) -> Result<RootRegistry, DesktopError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RootRegistry::default());
        }
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
    let ledger =
        serde_json::from_slice::<ManagedRootLedger>(&bytes).map_err(|_| managed_roots_invalid())?;
    if ledger.schema_version != MANAGED_ROOT_SCHEMA || ledger.roots.len() > MAX_REGISTERED_ROOTS {
        return Err(managed_roots_invalid());
    }

    let mut roots = VecDeque::with_capacity(ledger.roots.len());
    for persisted in ledger.roots {
        if !valid_root_handle(&persisted.root_handle) {
            return Err(managed_roots_invalid());
        }
        let path = PathBuf::from(&persisted.canonical_path);
        if !path.is_absolute()
            || persisted.canonical_path.is_empty()
            || persisted.canonical_path.len() > MAX_MANAGED_ROOT_PATH_BYTES
            || persisted.canonical_path.as_bytes().contains(&0)
            || persisted.display_label != bounded_display_label(&path)
            || persisted.display_label.chars().any(char::is_control)
        {
            return Err(managed_roots_invalid());
        }
        if roots.iter().any(|existing: &RegisteredRoot| {
            existing.handle == persisted.root_handle
                || existing.path == path
                || existing.path.starts_with(&path)
                || path.starts_with(&existing.path)
        }) {
            return Err(managed_roots_invalid());
        }
        roots.push_back(RegisteredRoot {
            handle: persisted.root_handle,
            path,
            display_label: persisted.display_label,
        });
    }
    Ok(RootRegistry { roots })
}

fn persist_managed_roots(path: &Path, roots: &RootRegistry) -> Result<(), DesktopError> {
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || !owner_only_permissions(&metadata) =>
        {
            return Err(managed_roots_invalid());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(managed_roots_invalid()),
    }
    let ledger = ManagedRootLedger {
        schema_version: MANAGED_ROOT_SCHEMA.to_string(),
        roots: roots
            .roots
            .iter()
            .map(|root| {
                let canonical_path = root
                    .path
                    .to_str()
                    .filter(|value| !value.is_empty() && value.len() <= MAX_MANAGED_ROOT_PATH_BYTES)
                    .ok_or_else(managed_roots_invalid)?;
                Ok(PersistedManagedRoot {
                    root_handle: root.handle.clone(),
                    display_label: root.display_label.clone(),
                    canonical_path: canonical_path.to_string(),
                })
            })
            .collect::<Result<Vec<_>, DesktopError>>()?,
    };
    let bytes = serde_json::to_vec(&ledger).map_err(|_| managed_roots_invalid())?;
    if bytes.len() as u64 > MAX_MANAGED_ROOT_LEDGER_BYTES {
        return Err(managed_roots_invalid());
    }
    let parent = path.parent().ok_or_else(managed_roots_invalid)?;
    let mut temporary = NamedTempFile::new_in(parent).map_err(|_| managed_roots_invalid())?;
    #[cfg(unix)]
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o600))
        .map_err(|_| managed_roots_invalid())?;
    temporary
        .write_all(&bytes)
        .and_then(|_| temporary.flush())
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|_| managed_roots_invalid())?;
    let persisted = temporary
        .persist(path)
        .map_err(|_| managed_roots_invalid())?;
    persisted.sync_all().map_err(|_| managed_roots_invalid())?;
    #[cfg(unix)]
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| managed_roots_invalid())?;
    Ok(())
}

fn valid_root_handle(handle: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn selected_root_serialization_is_bounded_and_never_contains_the_path() {
        let temp_root = temp_dir("private-parent");
        let nested = temp_root.join("a".repeat(90));
        fs::create_dir(&nested).unwrap();
        let state = NativeImportState::initialize(&temp_root.join("state")).unwrap();

        let selected = state
            .register(prepare_import_root(&nested).unwrap())
            .unwrap();
        let serialized = serde_json::to_string(&selected).unwrap();

        assert!(serialized.contains("root-"));
        assert!(serialized.contains(&format!("{}...", "a".repeat(77))));
        assert!(!serialized.contains(temp_root.to_str().unwrap()));
        assert_eq!(
            state.resolve(&selected.root_handle).unwrap(),
            fs::canonicalize(&nested).unwrap()
        );
        fs::remove_dir_all(temp_root).unwrap();
    }

    #[test]
    fn registry_rejects_overflow_without_evicting_authorized_handles() {
        let temp_root = temp_dir("bounded-registry");
        let state = NativeImportState::initialize(&temp_root.join("state")).unwrap();
        let mut handles = Vec::new();
        for index in 0..MAX_REGISTERED_ROOTS {
            let root = temp_root.join(format!("root-{index}"));
            fs::create_dir(&root).unwrap();
            handles.push(
                state
                    .register(prepare_import_root(&root).unwrap())
                    .unwrap()
                    .root_handle,
            );
        }
        let overflow = temp_root.join("overflow");
        fs::create_dir(&overflow).unwrap();

        let error = state
            .register(prepare_import_root(&overflow).unwrap())
            .err()
            .unwrap();
        assert_eq!(
            serde_json::to_value(error).unwrap()["code"],
            "managed_root_limit"
        );
        assert!(state.resolve(&handles[0]).is_ok());
        assert!(state.resolve(handles.last().unwrap()).is_ok());
        fs::remove_dir_all(temp_root).unwrap();
    }

    #[test]
    fn managed_root_survives_restart_with_the_same_opaque_handle() {
        let data_dir = temp_dir("managed-root-data");
        let private_parent = temp_dir("managed-root-private-parent");
        let root = private_parent.join("synthetic-managed-root");
        fs::create_dir(&root).unwrap();

        let state = NativeImportState::initialize(&data_dir).unwrap();
        let selected = state.register(prepare_import_root(&root).unwrap()).unwrap();
        drop(state);

        let restarted = NativeImportState::initialize(&data_dir).unwrap();
        let managed = restarted.managed_roots().unwrap();
        let serialized = serde_json::to_string(&managed).unwrap();

        assert_eq!(managed.schema_version, "resume-ir.desktop-managed-roots.v1");
        assert_eq!(managed.limit, MAX_REGISTERED_ROOTS);
        assert_eq!(managed.roots.len(), 1);
        assert_eq!(managed.roots[0].root_handle, selected.root_handle);
        assert_eq!(managed.roots[0].display_label, "synthetic-managed-root");
        assert_eq!(managed.roots[0].availability, RootAvailability::Available);
        assert_eq!(
            restarted.resolve(&selected.root_handle).unwrap(),
            fs::canonicalize(&root).unwrap()
        );
        assert!(!serialized.contains(private_parent.to_str().unwrap()));

        fs::remove_dir_all(data_dir).unwrap();
        fs::remove_dir_all(private_parent).unwrap();
    }

    #[test]
    fn unavailable_managed_root_remains_listed_after_restart() {
        let data_dir = temp_dir("managed-root-offline-data");
        let private_parent = temp_dir("managed-root-offline-parent");
        let root = private_parent.join("synthetic-offline-root");
        fs::create_dir(&root).unwrap();
        let canonical_root = fs::canonicalize(&root).unwrap();
        let state = NativeImportState::initialize(&data_dir).unwrap();
        let selected = state.register(prepare_import_root(&root).unwrap()).unwrap();
        drop(state);
        fs::remove_dir_all(&root).unwrap();

        let restarted = NativeImportState::initialize(&data_dir).unwrap();
        let managed = restarted.managed_roots().unwrap();

        assert_eq!(managed.roots.len(), 1);
        assert_eq!(managed.roots[0].root_handle, selected.root_handle);
        assert_eq!(managed.roots[0].availability, RootAvailability::Unavailable);
        assert_eq!(
            restarted.resolve(&selected.root_handle).unwrap(),
            canonical_root
        );
        assert_eq!(
            serde_json::to_value(
                restarted
                    .resolve_for_import(&selected.root_handle)
                    .err()
                    .unwrap()
            )
            .unwrap()["code"],
            "import_root_unavailable"
        );

        fs::remove_dir_all(data_dir).unwrap();
        fs::remove_dir_all(private_parent).unwrap();
    }

    #[test]
    fn duplicate_selection_reuses_the_persisted_handle() {
        let data_dir = temp_dir("managed-root-duplicate-data");
        let private_parent = temp_dir("managed-root-duplicate-parent");
        let root = private_parent.join("synthetic-duplicate-root");
        fs::create_dir(&root).unwrap();
        let state = NativeImportState::initialize(&data_dir).unwrap();

        let first = state.register(prepare_import_root(&root).unwrap()).unwrap();
        let second = state.register(prepare_import_root(&root).unwrap()).unwrap();
        let restarted = NativeImportState::initialize(&data_dir).unwrap();

        assert_eq!(first.root_handle, second.root_handle);
        assert_eq!(restarted.managed_roots().unwrap().roots.len(), 1);

        fs::remove_dir_all(data_dir).unwrap();
        fs::remove_dir_all(private_parent).unwrap();
    }

    #[test]
    fn reauthorization_accepts_only_the_same_root_without_registry_mutation() {
        let data_dir = temp_dir("managed-root-reauthorize-data");
        let private_parent = temp_dir("managed-root-reauthorize-parent");
        let root = private_parent.join("synthetic-authorized-root");
        let different = private_parent.join("synthetic-different-root");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&different).unwrap();
        let state = NativeImportState::initialize(&data_dir).unwrap();
        let selected = state.register(prepare_import_root(&root).unwrap()).unwrap();
        let ledger_before = fs::read(data_dir.join(MANAGED_ROOT_LEDGER_FILE)).unwrap();
        fs::remove_dir(&root).unwrap();
        assert_eq!(
            state.managed_roots().unwrap().roots[0].availability,
            RootAvailability::Unavailable
        );
        fs::create_dir(&root).unwrap();

        let restored = state
            .reauthorize(&selected.root_handle, prepare_import_root(&root).unwrap())
            .unwrap();
        assert_eq!(restored.root_handle, selected.root_handle);
        assert_eq!(state.managed_roots().unwrap().roots.len(), 1);

        let mismatch = state
            .reauthorize(
                &selected.root_handle,
                prepare_import_root(&different).unwrap(),
            )
            .err()
            .unwrap();
        assert_eq!(
            serde_json::to_value(mismatch).unwrap()["code"],
            "managed_root_mismatch"
        );
        assert_eq!(state.managed_roots().unwrap().roots.len(), 1);
        assert_eq!(
            state.resolve(&selected.root_handle).unwrap(),
            fs::canonicalize(&root).unwrap()
        );
        assert_eq!(
            fs::read(data_dir.join(MANAGED_ROOT_LEDGER_FILE)).unwrap(),
            ledger_before
        );

        drop(state);
        let restarted = NativeImportState::initialize(&data_dir).unwrap();
        assert_eq!(restarted.managed_roots().unwrap().roots.len(), 1);
        assert_eq!(
            restarted.managed_roots().unwrap().roots[0].root_handle,
            selected.root_handle
        );
        assert_eq!(
            restarted.resolve(&selected.root_handle).unwrap(),
            fs::canonicalize(&root).unwrap()
        );

        fs::remove_dir_all(data_dir).unwrap();
        fs::remove_dir_all(private_parent).unwrap();
    }

    #[test]
    fn malformed_or_oversized_managed_root_ledgers_fail_closed() {
        let malformed_data = temp_dir("managed-root-malformed-data");
        let malformed_path = malformed_data.join(MANAGED_ROOT_LEDGER_FILE);
        fs::write(&malformed_path, b"{}").unwrap();
        set_owner_only(&malformed_path);
        assert_managed_roots_invalid(NativeImportState::initialize(&malformed_data));

        let oversized_data = temp_dir("managed-root-oversized-data");
        let oversized_path = oversized_data.join(MANAGED_ROOT_LEDGER_FILE);
        fs::write(
            &oversized_path,
            vec![b'x'; MAX_MANAGED_ROOT_LEDGER_BYTES as usize + 1],
        )
        .unwrap();
        set_owner_only(&oversized_path);
        assert_managed_roots_invalid(NativeImportState::initialize(&oversized_data));

        fs::remove_dir_all(malformed_data).unwrap();
        fs::remove_dir_all(oversized_data).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_or_group_readable_managed_root_ledgers_fail_closed() {
        use std::os::unix::fs::symlink;

        let symlink_data = temp_dir("managed-root-symlink-data");
        let target = symlink_data.join("synthetic-target.json");
        fs::write(&target, b"{}").unwrap();
        set_owner_only(&target);
        symlink(&target, symlink_data.join(MANAGED_ROOT_LEDGER_FILE)).unwrap();
        assert_managed_roots_invalid(NativeImportState::initialize(&symlink_data));

        let permissions_data = temp_dir("managed-root-permissions-data");
        let permissions_root = permissions_data.join("synthetic-root");
        fs::create_dir(&permissions_root).unwrap();
        let state = NativeImportState::initialize(&permissions_data).unwrap();
        state
            .register(prepare_import_root(&permissions_root).unwrap())
            .unwrap();
        let ledger = permissions_data.join(MANAGED_ROOT_LEDGER_FILE);
        fs::set_permissions(&ledger, fs::Permissions::from_mode(0o644)).unwrap();
        assert_managed_roots_invalid(NativeImportState::initialize(&permissions_data));

        fs::remove_dir_all(symlink_data).unwrap();
        fs::remove_dir_all(permissions_data).unwrap();
    }

    fn assert_managed_roots_invalid(result: Result<NativeImportState, DesktopError>) {
        let error = result.err().unwrap();
        assert_eq!(
            serde_json::to_value(error).unwrap()["code"],
            "managed_roots_invalid"
        );
    }

    fn set_owner_only(path: &Path) {
        #[cfg(unix)]
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[test]
    fn export_writes_bounded_json_and_returns_no_parent_path() {
        let parent = temp_dir("private-export");
        let path = parent.join("redacted-diagnostics.json");
        let payload = serde_json::json!({
            "schema_version": "resume-ir.desktop-diagnostics.v1",
            "contains_resume_paths": false,
        });

        let receipt =
            write_diagnostics_export(&path, &serde_json::to_vec(&payload).unwrap()).unwrap();
        let receipt = serde_json::to_string(&receipt).unwrap();
        let written: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();

        assert_eq!(written, payload);
        assert!(receipt.contains("redacted-diagnostics.json"));
        assert!(!receipt.contains(parent.to_str().unwrap()));
        fs::remove_dir_all(parent).unwrap();
    }

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "resume-ir-desktop-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        path
    }
}
