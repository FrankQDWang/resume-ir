use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

#[path = "src/runtime_pack/macho_payload.rs"]
mod macho_payload;

const ATTESTATION_ENV: &str = "RESUME_IR_RUNTIME_EXECUTABLE_ATTESTATION";
const ATTESTATION_SCHEMA: &str = "resume-ir.runtime-executable-attestation.v1";
const MAX_ATTESTATION_BYTES: u64 = 16 * 1024;
const MAX_EXECUTABLE_BYTES: u64 = macho_payload::MAX_EXECUTABLE_BYTES;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attestation {
    schema_version: String,
    target_triple: String,
    profile: String,
    executables: Vec<Executable>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Executable {
    role: String,
    build_file: String,
    runtime_file: String,
    architecture: String,
    digest: String,
    payload_bytes: u64,
    payload_sha256: String,
}

fn main() {
    println!("cargo:rerun-if-env-changed={ATTESTATION_ENV}");
    let Some(attestation_path) = env::var_os(ATTESTATION_ENV) else {
        return;
    };
    let target = env::var("TARGET").expect("Cargo TARGET must be available");
    let profile = env::var("PROFILE").expect("Cargo PROFILE must be available");
    let path = PathBuf::from(attestation_path);
    let attestation = read_attestation(&path);
    if attestation.schema_version != ATTESTATION_SCHEMA
        || attestation.target_triple != target
        || attestation.profile != profile
    {
        panic!("runtime executable attestation build identity is invalid");
    }
    let expected_names = expected_names(&target);
    if attestation.executables.len() != expected_names.len() {
        panic!("runtime executable attestation role set is invalid");
    }
    let root = path
        .parent()
        .expect("runtime executable attestation must have a parent");
    let mut executables = BTreeMap::new();
    for executable in attestation.executables {
        if executables
            .insert(executable.role.clone(), executable)
            .is_some()
        {
            panic!("runtime executable attestation contains duplicate roles");
        }
    }
    for (role, expected_build_name, expected_runtime_name) in expected_names {
        let executable = executables
            .get(role)
            .unwrap_or_else(|| panic!("runtime executable attestation is missing {role}"));
        if executable.build_file != expected_build_name
            || executable.runtime_file != expected_runtime_name
            || !valid_direct_name(&executable.build_file)
            || !valid_direct_name(&executable.runtime_file)
            || executable.architecture != "arm64"
            || executable.digest != "sha256_without_code_signature_v1"
            || executable.payload_bytes == 0
            || executable.payload_bytes > MAX_EXECUTABLE_BYTES
            || !valid_digest(&executable.payload_sha256)
        {
            panic!("runtime executable attestation {role} identity is invalid");
        }
        let executable_path = root.join(&executable.build_file);
        validate_attested_executable(&executable_path, executable);
        println!("cargo:rerun-if-changed={}", executable_path.display());
        emit(role, "FILE", &executable.runtime_file);
        emit(role, "ARCH", &executable.architecture);
        emit(role, "DIGEST", &executable.digest);
        emit(role, "PAYLOAD_BYTES", &executable.payload_bytes.to_string());
        emit(role, "PAYLOAD_SHA256", &executable.payload_sha256);
    }
    println!("cargo:rerun-if-changed={}", path.display());
    println!("cargo:rustc-env=RESUME_IR_ATTESTED_TARGET={target}");
    println!("cargo:rustc-env=RESUME_IR_ATTESTED_PROFILE={profile}");
}

fn read_attestation(path: &Path) -> Attestation {
    if !path.is_absolute() {
        panic!("runtime executable attestation path must be absolute");
    }
    let metadata = fs::symlink_metadata(path)
        .unwrap_or_else(|_| panic!("runtime executable attestation is missing"));
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_ATTESTATION_BYTES
    {
        panic!("runtime executable attestation file is invalid");
    }
    serde_json::from_slice(
        &fs::read(path).expect("runtime executable attestation must remain readable"),
    )
    .expect("runtime executable attestation JSON is invalid")
}

fn expected_names(target: &str) -> [(&'static str, &'static str, &'static str); 2] {
    match target {
        "aarch64-apple-darwin" => [
            (
                "embedding_runtime",
                "resume-embedding-runtime-aarch64-apple-darwin",
                "resume-embedding-runtime",
            ),
            (
                "pdf_renderer",
                "resume-pdf-render-runtime-aarch64-apple-darwin",
                "resume-pdf-render-runtime",
            ),
        ],
        _ => panic!("runtime executable attestation target is unsupported"),
    }
}

fn validate_attested_executable(path: &Path, expected: &Executable) {
    let metadata = fs::symlink_metadata(path)
        .unwrap_or_else(|_| panic!("attested runtime executable is missing"));
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        panic!("attested runtime executable shape changed");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();
        if mode & 0o111 == 0 || mode & 0o022 != 0 {
            panic!("attested runtime executable permissions are invalid");
        }
    }
    let (architecture, payload_bytes, payload_sha256) = macho_payload_identity(path);
    if architecture != expected.architecture
        || payload_bytes != expected.payload_bytes
        || payload_sha256 != expected.payload_sha256
    {
        panic!("attested runtime executable digest is stale or tampered");
    }
}

fn macho_payload_identity(path: &Path) -> (&'static str, u64, String) {
    let payload = macho_payload::read_canonical_payload(path)
        .expect("attested runtime executable is not a canonical bounded arm64 Mach-O");
    let payload_bytes = payload.bytes.len() as u64;
    let mut digest = Sha256::new();
    digest.update(&payload.bytes);
    (
        payload.architecture,
        payload_bytes,
        format!("{:x}", digest.finalize()),
    )
}

fn valid_direct_name(value: &str) -> bool {
    let path = Path::new(value);
    path.file_name() == Some(OsStr::new(value))
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn emit(role: &str, field: &str, value: &str) {
    let role = match role {
        "embedding_runtime" => "EMBEDDING",
        "pdf_renderer" => "PDF_RENDERER",
        _ => panic!("runtime executable attestation role is unsupported"),
    };
    println!("cargo:rustc-env=RESUME_IR_ATTESTED_{role}_{field}={value}");
}
