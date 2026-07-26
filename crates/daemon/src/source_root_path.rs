use std::fs;
use std::path::{Component, Path, PathBuf};

pub(crate) enum LegacyAuthorizedRoot {
    Available(PathBuf),
    Offline,
}

pub(crate) fn canonicalize_authorized_root(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() || !directory_components_are_safe(path) {
        return None;
    }
    let canonical = fs::canonicalize(path).ok()?;
    directory_components_are_safe(&canonical).then_some(canonical)
}

pub(crate) fn authorize_legacy_root(path: &Path) -> Result<LegacyAuthorizedRoot, ()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_dir() || is_link_or_reparse_point(&metadata) {
                return Err(());
            }
            let canonical = canonicalize_authorized_root(path).ok_or(())?;
            if canonical != path {
                return Err(());
            }
            Ok(LegacyAuthorizedRoot::Available(canonical))
        }
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            Ok(LegacyAuthorizedRoot::Offline)
        }
        Err(_) => Err(()),
    }
}

pub(crate) fn is_available(canonical_path: &str) -> bool {
    let path = Path::new(canonical_path);
    canonicalize_authorized_root(path).is_some_and(|observed| observed == path)
}

fn directory_components_are_safe(path: &Path) -> bool {
    let mut observed = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(value) => observed.push(value.as_os_str()),
            Component::RootDir => observed.push(component.as_os_str()),
            Component::Normal(value) => {
                observed.push(value);
                let Ok(metadata) = fs::symlink_metadata(&observed) else {
                    return false;
                };
                if !metadata.is_dir() || is_link_or_reparse_point(&metadata) {
                    return false;
                }
            }
            Component::CurDir | Component::ParentDir => return false,
        }
    }
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
