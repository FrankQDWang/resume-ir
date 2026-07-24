use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::ipc::OptionalRuntimeReason;

use super::attestation::validated_pdf_renderer;
use super::security::{
    canonical_input_directory, ensure_not_cancelled, matches_declared_executable,
    read_manifest_pinned_with_cancel, read_manifest_with_cancel, validate_canonical_executable,
    validate_pack_file_entries_with_cancel, PackFile,
};

const SCHEMA: &str = "resume-ir.desktop-ocr-runtime-pack.v1";
const VERSION: &str = "5.5.2";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const MAC_PACK_ID: &str = "tesseract-5.5.2-tessdata-fast-4.1.0-macos-arm64-r1";
#[cfg(test)]
const WINDOWS_PACK_ID: &str = "tesseract-5.5.2-tessdata-fast-4.1.0-windows-x64-static-r1";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const MAC_MANIFEST_SHA256: &str =
    "bd86a197f5b9518df622196c4d4c16201567295237bab06a133b2d3496528ad1";

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const MAC_LICENSES: [&str; 10] = [
    "LICENSES/0BSD.txt",
    "LICENSES/Apache-2.0.txt",
    "LICENSES/BSD-2-Clause.txt",
    "LICENSES/BSD-3-Clause.txt",
    "LICENSES/CC0-1.0.txt",
    "LICENSES/IJG.txt",
    "LICENSES/MIT.txt",
    "LICENSES/Zlib.txt",
    "LICENSES/libpng-2.0.txt",
    "LICENSES/libtiff.txt",
];
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const MAC_LIBRARIES: [&str; 15] = [
    "lib/libarchive.13.dylib",
    "lib/libb2.1.dylib",
    "lib/libgif.7.2.0.dylib",
    "lib/libjpeg.8.3.2.dylib",
    "lib/libleptonica.6.dylib",
    "lib/liblz4.1.10.0.dylib",
    "lib/liblzma.5.dylib",
    "lib/libopenjp2.2.5.4.dylib",
    "lib/libpng16.16.dylib",
    "lib/libsharpyuv.0.1.2.dylib",
    "lib/libtesseract.5.dylib",
    "lib/libtiff.6.dylib",
    "lib/libwebp.7.2.0.dylib",
    "lib/libwebpmux.3.1.2.dylib",
    "lib/libzstd.1.5.7.dylib",
];
#[cfg(test)]
const WINDOWS_LICENSES: [&str; 3] = [
    "LICENSES/Tesseract-Apache-2.0.txt",
    "LICENSES/Leptonica-BSD-2-Clause.txt",
    "LICENSES/tessdata-fast-Apache-2.0.txt",
];

#[cfg(test)]
pub(super) fn validate(
    engine: &Path,
    renderer: Option<&Path>,
    requested_languages: &str,
    tessdata_dir: Option<&Path>,
) -> Result<(), OptionalRuntimeReason> {
    validated_runtime(engine, renderer, requested_languages, tessdata_dir).map(drop)
}

pub(crate) struct ValidatedOcrRuntime {
    engine: PathBuf,
    renderer: PathBuf,
}

impl ValidatedOcrRuntime {
    pub(crate) fn into_paths(self) -> (PathBuf, PathBuf) {
        (self.engine, self.renderer)
    }
}

pub(crate) fn validated_runtime(
    engine: &Path,
    renderer: Option<&Path>,
    requested_languages: &str,
    tessdata_dir: Option<&Path>,
) -> Result<ValidatedOcrRuntime, OptionalRuntimeReason> {
    validated_runtime_with_cancel(engine, renderer, requested_languages, tessdata_dir, &|| {
        false
    })
}

pub(crate) fn validated_runtime_with_cancel(
    engine: &Path,
    renderer: Option<&Path>,
    requested_languages: &str,
    tessdata_dir: Option<&Path>,
    cancelled: &dyn Fn() -> bool,
) -> Result<ValidatedOcrRuntime, OptionalRuntimeReason> {
    ensure_not_cancelled(cancelled)?;
    let renderer = renderer.ok_or(OptionalRuntimeReason::Missing)?;
    let engine = validate_pack_for_identity_with_cancel(
        engine,
        requested_languages,
        tessdata_dir,
        host_identity()?,
        cancelled,
    )?;
    ensure_not_cancelled(cancelled)?;
    let renderer = validated_pdf_renderer(renderer)?.into_path();
    ensure_not_cancelled(cancelled)?;
    Ok(ValidatedOcrRuntime { engine, renderer })
}

#[cfg(test)]
pub(super) fn validate_for_identity(
    engine: &Path,
    _renderer: Option<&Path>,
    requested_languages: &str,
    tessdata_dir: Option<&Path>,
    identity: PackIdentity,
) -> Result<(), OptionalRuntimeReason> {
    validate_pack_for_identity_with_cancel(
        engine,
        requested_languages,
        tessdata_dir,
        identity,
        &|| false,
    )
    .map(drop)
}

fn validate_pack_for_identity_with_cancel(
    engine: &Path,
    requested_languages: &str,
    tessdata_dir: Option<&Path>,
    identity: PackIdentity,
    cancelled: &dyn Fn() -> bool,
) -> Result<PathBuf, OptionalRuntimeReason> {
    ensure_not_cancelled(cancelled)?;
    let engine = validate_canonical_executable(engine)?;
    let root = engine.parent().ok_or(OptionalRuntimeReason::Invalid)?;
    let manifest: Manifest = match identity.manifest_sha256 {
        Some(digest) => read_manifest_pinned_with_cancel(root, digest, cancelled)?,
        None => read_manifest_with_cancel(root, cancelled)?,
    };
    if manifest.schema_version != SCHEMA
        || manifest.runtime_pack_id != identity.pack_id
        || manifest.target_triple != identity.target_triple
        || manifest.engine != "tesseract"
        || manifest.engine_version != VERSION
        || manifest.renderer != identity.renderer
        || manifest.network_access != "disabled"
        || !manifest.license_reviewed
        || manifest.third_party_notice != "THIRD-PARTY-NOTICES.json"
        || manifest.languages != ["eng", "chi_sim"]
        || requested_languages != "eng+chi_sim"
    {
        return Err(OptionalRuntimeReason::Invalid);
    }
    if !validate_file_contract(&manifest.files, identity) {
        return Err(OptionalRuntimeReason::Invalid);
    }
    let entries = manifest
        .files
        .iter()
        .map(|file| PackFile {
            role: file.role.clone(),
            file: file.file.clone(),
            bytes: file.bytes,
            sha256: file.sha256.clone(),
        })
        .collect::<Vec<_>>();
    let files = validate_pack_file_entries_with_cancel(root, &entries, cancelled)?;
    if manifest.files.iter().any(|declared| {
        files
            .iter()
            .find(|validated| validated.file == declared.file)
            .is_none_or(|validated| {
                !matches_declared_executable(&validated.path, declared.executable)
            })
    }) {
        return Err(OptionalRuntimeReason::Invalid);
    }
    let engine_entry = manifest
        .files
        .iter()
        .find(|entry| entry.role == "engine_binary")
        .ok_or(OptionalRuntimeReason::Invalid)?;
    let engine_file = files
        .iter()
        .find(|entry| entry.role == "engine_binary")
        .ok_or(OptionalRuntimeReason::Invalid)?;
    if !engine_entry.executable
        || engine_file.path != engine
        || !files.iter().any(|entry| entry.role == "language_eng")
        || !files.iter().any(|entry| entry.role == "language_chi_sim")
        || !files.iter().any(|entry| entry.role == "engine_config")
        || !files.iter().any(|entry| {
            entry.role == "third_party_notice" && entry.file == manifest.third_party_notice
        })
    {
        return Err(OptionalRuntimeReason::Invalid);
    }
    ensure_not_cancelled(cancelled)?;
    let tessdata = tessdata_dir.ok_or(OptionalRuntimeReason::Missing)?;
    if canonical_input_directory(tessdata)?
        != root
            .join("tessdata")
            .canonicalize()
            .map_err(|_| OptionalRuntimeReason::Missing)?
    {
        return Err(OptionalRuntimeReason::Invalid);
    }
    ensure_not_cancelled(cancelled)?;
    Ok(engine)
}

#[derive(Clone, Copy)]
pub(super) struct PackIdentity {
    pack_id: &'static str,
    target_triple: &'static str,
    renderer: &'static str,
    engine_file: &'static str,
    licenses: &'static [&'static str],
    libraries: &'static [&'static str],
    manifest_sha256: Option<&'static str>,
}

fn host_identity() -> Result<PackIdentity, OptionalRuntimeReason> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Ok(mac_identity());
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Err(OptionalRuntimeReason::Invalid);
    }
    #[allow(unreachable_code)]
    Err(OptionalRuntimeReason::Invalid)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const fn mac_identity() -> PackIdentity {
    PackIdentity {
        pack_id: MAC_PACK_ID,
        target_triple: "aarch64-apple-darwin",
        renderer: "macos-pdfkit-coregraphics",
        engine_file: "tesseract",
        licenses: &MAC_LICENSES,
        libraries: &MAC_LIBRARIES,
        manifest_sha256: Some(MAC_MANIFEST_SHA256),
    }
}

#[cfg(test)]
pub(super) const fn windows_identity() -> PackIdentity {
    PackIdentity {
        pack_id: WINDOWS_PACK_ID,
        target_triple: "x86_64-pc-windows-msvc",
        renderer: "windows-pdfium-static",
        engine_file: "tesseract.exe",
        licenses: &WINDOWS_LICENSES,
        libraries: &[],
        manifest_sha256: None,
    }
}

fn validate_file_contract(files: &[ManifestFile], identity: PackIdentity) -> bool {
    let expected_count = 5_usize
        .checked_add(identity.licenses.len())
        .and_then(|count| count.checked_add(identity.libraries.len()));
    if expected_count != Some(files.len()) {
        return false;
    }
    let expected = [
        ("engine_binary", identity.engine_file),
        ("language_eng", "tessdata/eng.traineddata"),
        ("language_chi_sim", "tessdata/chi_sim.traineddata"),
        ("engine_config", "tessdata/configs/tsv"),
        ("third_party_notice", "THIRD-PARTY-NOTICES.json"),
    ];
    expected.iter().all(|(role, file)| {
        files
            .iter()
            .filter(|entry| entry.role == *role && entry.file == *file)
            .count()
            == 1
    }) && identity.licenses.iter().all(|file| {
        files
            .iter()
            .filter(|entry| entry.role == "license_text" && entry.file == *file)
            .count()
            == 1
    }) && identity.libraries.iter().all(|file| {
        files
            .iter()
            .filter(|entry| entry.role == "engine_library" && entry.file == *file)
            .count()
            == 1
    }) && files.iter().all(|entry| {
        expected
            .iter()
            .any(|(role, file)| entry.role == *role && entry.file == *file)
            || identity
                .licenses
                .iter()
                .any(|file| entry.role == "license_text" && entry.file == *file)
            || identity
                .libraries
                .iter()
                .any(|file| entry.role == "engine_library" && entry.file == *file)
    }) && files
        .iter()
        .all(|entry| entry.executable == (entry.role == "engine_binary"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: String,
    runtime_pack_id: String,
    target_triple: String,
    engine: String,
    engine_version: String,
    renderer: String,
    languages: Vec<String>,
    network_access: String,
    license_reviewed: bool,
    third_party_notice: String,
    files: Vec<ManifestFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    role: String,
    file: String,
    bytes: u64,
    sha256: String,
    executable: bool,
}
