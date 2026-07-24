use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::ipc::OptionalRuntimeReason;

use super::macho::payload_identity;
use super::security::{valid_digest, validate_canonical_executable};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExecutableRole {
    EmbeddingRuntime,
    PdfRenderer,
}

pub(crate) struct ValidatedExecutable(PathBuf);

impl ValidatedExecutable {
    pub(crate) fn into_path(self) -> PathBuf {
        self.0
    }
}

pub(super) struct ExecutableIdentity<'a> {
    pub(super) target_triple: &'a str,
    pub(super) profile: &'a str,
    pub(super) file: &'a str,
    pub(super) architecture: &'a str,
    pub(super) digest: &'a str,
    pub(super) payload_bytes: u64,
    pub(super) payload_sha256: &'a str,
}

pub(crate) fn validated_embedding_command(
    candidate: &Path,
) -> Result<ValidatedExecutable, OptionalRuntimeReason> {
    validate_compiled(candidate, ExecutableRole::EmbeddingRuntime)
}

pub(crate) fn validated_pdf_renderer(
    candidate: &Path,
) -> Result<ValidatedExecutable, OptionalRuntimeReason> {
    validate_compiled(candidate, ExecutableRole::PdfRenderer)
}

fn validate_compiled(
    candidate: &Path,
    role: ExecutableRole,
) -> Result<ValidatedExecutable, OptionalRuntimeReason> {
    let identity = compiled_identity(role)?;
    validate_for_identity(candidate, role, &identity)
}

pub(super) fn validate_for_identity(
    candidate: &Path,
    role: ExecutableRole,
    identity: &ExecutableIdentity<'_>,
) -> Result<ValidatedExecutable, OptionalRuntimeReason> {
    if current_target() != Some(identity.target_triple)
        || current_profile() != identity.profile
        || fixed_runtime_name(role, identity.target_triple) != Some(identity.file)
        || identity.architecture != "arm64"
        || identity.digest != "sha256_without_code_signature_v1"
        || identity.payload_bytes == 0
        || !valid_digest(identity.payload_sha256)
    {
        return Err(OptionalRuntimeReason::Invalid);
    }
    let canonical = validate_canonical_executable(candidate)?;
    if canonical.file_name() != Some(OsStr::new(identity.file)) {
        return Err(OptionalRuntimeReason::Invalid);
    }
    let payload = payload_identity(&canonical)?;
    if payload.architecture != identity.architecture
        || payload.bytes != identity.payload_bytes
        || payload.sha256 != identity.payload_sha256
    {
        return Err(OptionalRuntimeReason::Invalid);
    }
    Ok(ValidatedExecutable(canonical))
}

fn compiled_identity(
    role: ExecutableRole,
) -> Result<ExecutableIdentity<'static>, OptionalRuntimeReason> {
    let target_triple =
        option_env!("RESUME_IR_ATTESTED_TARGET").ok_or(OptionalRuntimeReason::Invalid)?;
    let profile =
        option_env!("RESUME_IR_ATTESTED_PROFILE").ok_or(OptionalRuntimeReason::Invalid)?;
    let (file, architecture, digest, payload_bytes, payload_sha256) = match role {
        ExecutableRole::EmbeddingRuntime => (
            option_env!("RESUME_IR_ATTESTED_EMBEDDING_FILE"),
            option_env!("RESUME_IR_ATTESTED_EMBEDDING_ARCH"),
            option_env!("RESUME_IR_ATTESTED_EMBEDDING_DIGEST"),
            option_env!("RESUME_IR_ATTESTED_EMBEDDING_PAYLOAD_BYTES"),
            option_env!("RESUME_IR_ATTESTED_EMBEDDING_PAYLOAD_SHA256"),
        ),
        ExecutableRole::PdfRenderer => (
            option_env!("RESUME_IR_ATTESTED_PDF_RENDERER_FILE"),
            option_env!("RESUME_IR_ATTESTED_PDF_RENDERER_ARCH"),
            option_env!("RESUME_IR_ATTESTED_PDF_RENDERER_DIGEST"),
            option_env!("RESUME_IR_ATTESTED_PDF_RENDERER_PAYLOAD_BYTES"),
            option_env!("RESUME_IR_ATTESTED_PDF_RENDERER_PAYLOAD_SHA256"),
        ),
    };
    let payload_bytes = payload_bytes
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .ok_or(OptionalRuntimeReason::Invalid)?;
    Ok(ExecutableIdentity {
        target_triple,
        profile,
        file: file.ok_or(OptionalRuntimeReason::Invalid)?,
        architecture: architecture.ok_or(OptionalRuntimeReason::Invalid)?,
        digest: digest.ok_or(OptionalRuntimeReason::Invalid)?,
        payload_bytes,
        payload_sha256: payload_sha256.ok_or(OptionalRuntimeReason::Invalid)?,
    })
}

fn fixed_runtime_name(role: ExecutableRole, target: &str) -> Option<&'static str> {
    match (role, target) {
        (ExecutableRole::EmbeddingRuntime, "aarch64-apple-darwin")
        | (ExecutableRole::EmbeddingRuntime, "x86_64-apple-darwin") => {
            Some("resume-embedding-runtime")
        }
        (ExecutableRole::PdfRenderer, "aarch64-apple-darwin")
        | (ExecutableRole::PdfRenderer, "x86_64-apple-darwin") => Some("resume-pdf-render-runtime"),
        (ExecutableRole::EmbeddingRuntime, "x86_64-pc-windows-msvc") => {
            Some("resume-embedding-runtime.exe")
        }
        (ExecutableRole::PdfRenderer, "x86_64-pc-windows-msvc") => {
            Some("resume-pdf-render-runtime.exe")
        }
        _ => None,
    }
}

pub(super) const fn current_profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

pub(super) const fn current_target() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("aarch64-apple-darwin")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("x86_64-apple-darwin")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("x86_64-pc-windows-msvc")
    } else {
        None
    }
}
