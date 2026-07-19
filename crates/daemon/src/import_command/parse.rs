use std::fs;
use std::path::PathBuf;

use meta_store::{ImportRootPreset, ImportScanProfile};

use crate::command_failure::CommandFailure;

pub(super) struct CanonicalImportRoot {
    pub(super) requested: PathBuf,
    pub(super) canonical: PathBuf,
}

pub(super) fn root_preset(
    payload: &serde_json::Value,
) -> Result<Option<ImportRootPreset>, CommandFailure> {
    let Some(value) = payload.get("root_preset") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    match value.as_str() {
        Some("local-discovery") => Ok(Some(ImportRootPreset::LocalDiscovery)),
        _ => Err(CommandFailure::BadRequest("invalid root_preset")),
    }
}

pub(super) fn roots(payload: &serde_json::Value) -> Result<Vec<PathBuf>, CommandFailure> {
    let roots = payload
        .get("roots")
        .and_then(serde_json::Value::as_array)
        .filter(|roots| !roots.is_empty())
        .ok_or(CommandFailure::BadRequest(
            "roots must be a non-empty array",
        ))?;
    if roots.len() > 64 {
        return Err(CommandFailure::BadRequest("too many roots"));
    }
    roots
        .iter()
        .map(|root| {
            let value = root
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .ok_or(CommandFailure::BadRequest("roots must be strings"))?;
            Ok(PathBuf::from(value))
        })
        .collect()
}

pub(super) fn profile(payload: &serde_json::Value) -> Result<ImportScanProfile, CommandFailure> {
    match payload
        .get("profile")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("explicit")
    {
        "explicit" => Ok(ImportScanProfile::Explicit),
        "discovery" => Ok(ImportScanProfile::Discovery),
        _ => Err(CommandFailure::BadRequest("invalid profile")),
    }
}

pub(super) fn max_files(payload: &serde_json::Value) -> Result<Option<usize>, CommandFailure> {
    let Some(value) = payload.get("max_files") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let value = value
        .as_u64()
        .filter(|value| *value > 0)
        .ok_or(CommandFailure::BadRequest("max_files must be positive"))?;
    usize::try_from(value)
        .map(Some)
        .map_err(|_| CommandFailure::BadRequest("max_files is too large"))
}

pub(super) fn canonical_roots(
    requested_roots: &[PathBuf],
) -> Result<Vec<CanonicalImportRoot>, CommandFailure> {
    let mut roots = requested_roots
        .iter()
        .map(|requested_root| {
            let metadata = fs::metadata(requested_root).map_err(|_| {
                CommandFailure::BadRequest("import root must exist and be a directory")
            })?;
            if !metadata.is_dir() {
                return Err(CommandFailure::BadRequest(
                    "import root must exist and be a directory",
                ));
            }
            let canonical = fs::canonicalize(requested_root).map_err(|_| {
                CommandFailure::BadRequest("import root must exist and be a directory")
            })?;
            Ok(CanonicalImportRoot {
                requested: requested_root.clone(),
                canonical,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    roots.sort_by(|left, right| left.canonical.cmp(&right.canonical));
    for window in roots.windows(2) {
        let [left, right] = window else {
            continue;
        };
        if left.canonical == right.canonical || right.canonical.starts_with(&left.canonical) {
            return Err(CommandFailure::BadRequest(
                "import roots must be distinct and non-overlapping",
            ));
        }
    }
    Ok(roots)
}
