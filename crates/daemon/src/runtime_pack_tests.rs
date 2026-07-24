use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};
use tempfile::tempdir;

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use super::{
    current_profile, current_target, executable_payload_identity, validate_executable_for_identity,
    ExecutableIdentity, ExecutableRole,
};
use super::{
    sha256_file, validate_classifier, validate_embedding, validate_ocr, validate_ocr_for_identity,
    validate_pack_file_entries, validate_pack_file_entries_with_cancel, windows_ocr_identity,
    OptionalRuntimeReason, PackFile,
};

#[test]
fn embedding_candidate_requires_an_explicit_runtime_pack() {
    let directory = tempdir().unwrap();
    let command = write_file(
        directory.path(),
        "resume-embedding-runtime",
        b"runtime",
        true,
    );

    assert_eq!(
        validate_embedding(&command, "synthetic-model", 384, None),
        Err(OptionalRuntimeReason::Invalid)
    );
    assert_eq!(
        validate_embedding(
            &command,
            "synthetic-model",
            384,
            Some(&directory.path().join("missing-pack")),
        ),
        Err(OptionalRuntimeReason::Invalid)
    );
}

#[test]
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn macos_runtime_payload_identity_accepts_only_signature_blob_drift() {
    let directory = tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let command = write_file(
        &root,
        "resume-embedding-runtime",
        &signed_macho(b"reviewed-code-and-data", b"signature-one"),
        true,
    );
    let payload = executable_payload_identity(&command).unwrap();
    let identity = ExecutableIdentity {
        target_triple: current_target().unwrap(),
        profile: current_profile(),
        file: "resume-embedding-runtime",
        architecture: payload.architecture,
        digest: "sha256_without_code_signature_v1",
        payload_bytes: payload.bytes,
        payload_sha256: &payload.sha256,
    };

    assert!(validate_executable_for_identity(
        &command,
        ExecutableRole::EmbeddingRuntime,
        &identity,
    )
    .is_ok());

    fs::write(
        &command,
        signed_macho(b"reviewed-code-and-data", b"a-longer-signature-two"),
    )
    .unwrap();
    assert!(validate_executable_for_identity(
        &command,
        ExecutableRole::EmbeddingRuntime,
        &identity,
    )
    .is_ok());
}

#[test]
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn macos_executable_attestation_rejects_name_payload_and_shape_drift() {
    let directory = tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let original = signed_macho(b"reviewed-code-and-data", b"signature");
    let command = write_file(&root, "resume-embedding-runtime", &original, true);
    let payload = executable_payload_identity(&command).unwrap();
    let identity = ExecutableIdentity {
        target_triple: current_target().unwrap(),
        profile: current_profile(),
        file: "resume-embedding-runtime",
        architecture: payload.architecture,
        digest: "sha256_without_code_signature_v1",
        payload_bytes: payload.bytes,
        payload_sha256: &payload.sha256,
    };
    let rejected = |path: &Path| {
        assert_eq!(
            validate_executable_for_identity(path, ExecutableRole::EmbeddingRuntime, &identity,)
                .map(|_| ()),
            Err(OptionalRuntimeReason::Invalid)
        );
    };

    let renamed = write_file(&root, "renamed-runtime", &original, true);
    rejected(&renamed);

    let mut code_changed = original.clone();
    code_changed[120] ^= 1;
    fs::write(&command, code_changed).unwrap();
    rejected(&command);

    let mut load_command_changed = original.clone();
    load_command_changed[100] ^= 1;
    fs::write(&command, load_command_changed).unwrap();
    rejected(&command);

    fs::write(&command, [&original[..], b"appended"].concat()).unwrap();
    rejected(&command);
    fs::write(&command, &original[..original.len() - 1]).unwrap();
    rejected(&command);

    fs::write(
        &command,
        signed_macho(b"self-consistent-replacement", b"signature"),
    )
    .unwrap();
    rejected(&command);

    fs::write(&command, &original).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&command, root.join("runtime-link")).unwrap();
        rejected(&root.join("runtime-link"));
    }
}

#[test]
fn configured_ocr_and_classifier_candidates_require_manifests() {
    let directory = tempdir().unwrap();
    let engine = write_file(directory.path(), "renamed-ocr.exe", b"runtime", true);
    let model = write_file(directory.path(), "renamed-model.json", b"{}", false);

    assert_eq!(
        validate_ocr(&engine, None, "eng+chi_sim", None),
        Err(OptionalRuntimeReason::Missing)
    );
    assert_eq!(
        validate_classifier(&model),
        Err(OptionalRuntimeReason::Missing)
    );
}

#[test]
fn windows_named_ocr_binary_is_bound_to_its_manifest_and_digests() {
    let directory = tempdir().unwrap();
    let root = &directory.path().canonicalize().unwrap();
    let engine = write_file(root, "tesseract.exe", b"synthetic executable", true);
    let english = write_file(root, "tessdata/eng.traineddata", b"english", false);
    let chinese = write_file(root, "tessdata/chi_sim.traineddata", b"chinese", false);
    let config = write_file(root, "tessdata/configs/tsv", b"tsv", false);
    let notice = write_file(root, "THIRD-PARTY-NOTICES.json", b"{}", false);
    let license_one = write_file(root, "LICENSES/Tesseract-Apache-2.0.txt", b"one", false);
    let license_two = write_file(root, "LICENSES/Leptonica-BSD-2-Clause.txt", b"two", false);
    let license_three = write_file(
        root,
        "LICENSES/tessdata-fast-Apache-2.0.txt",
        b"three",
        false,
    );
    let files = vec![
        ocr_entry("engine_binary", "tesseract.exe", &engine, true),
        ocr_entry("language_eng", "tessdata/eng.traineddata", &english, false),
        ocr_entry(
            "language_chi_sim",
            "tessdata/chi_sim.traineddata",
            &chinese,
            false,
        ),
        ocr_entry("engine_config", "tessdata/configs/tsv", &config, false),
        ocr_entry(
            "third_party_notice",
            "THIRD-PARTY-NOTICES.json",
            &notice,
            false,
        ),
        ocr_entry(
            "license_text",
            "LICENSES/Tesseract-Apache-2.0.txt",
            &license_one,
            false,
        ),
        ocr_entry(
            "license_text",
            "LICENSES/Leptonica-BSD-2-Clause.txt",
            &license_two,
            false,
        ),
        ocr_entry(
            "license_text",
            "LICENSES/tessdata-fast-Apache-2.0.txt",
            &license_three,
            false,
        ),
    ];
    write_manifest(
        root,
        json!({
            "schema_version": "resume-ir.desktop-ocr-runtime-pack.v1",
            "runtime_pack_id": "tesseract-5.5.2-tessdata-fast-4.1.0-windows-x64-static-r1",
            "target_triple": "x86_64-pc-windows-msvc",
            "engine": "tesseract",
            "engine_version": "5.5.2",
            "renderer": "windows-pdfium-static",
            "languages": ["eng", "chi_sim"],
            "network_access": "disabled",
            "license_reviewed": true,
            "third_party_notice": "THIRD-PARTY-NOTICES.json",
            "files": files,
        }),
    );

    assert_eq!(
        validate_ocr_for_identity(
            &engine,
            None,
            "eng+chi_sim",
            Some(&root.join("tessdata")),
            windows_ocr_identity(),
        ),
        Ok(())
    );

    let renamed = write_file(root, "renamed.exe", b"synthetic executable", true);
    assert_eq!(
        validate_ocr_for_identity(
            &renamed,
            None,
            "eng+chi_sim",
            Some(&root.join("tessdata")),
            windows_ocr_identity(),
        ),
        Err(OptionalRuntimeReason::Invalid)
    );

    fs::write(&english, b"tampered").unwrap();
    assert_eq!(
        validate_ocr_for_identity(
            &engine,
            None,
            "eng+chi_sim",
            Some(&root.join("tessdata")),
            windows_ocr_identity(),
        ),
        Err(OptionalRuntimeReason::Invalid)
    );
}

#[test]
fn self_consistent_classifier_replacement_is_rejected_by_pinned_manifest() {
    let directory = tempdir().unwrap();
    let root = &directory.path().canonicalize().unwrap();
    let model = write_file(
        root,
        "linear-promotion-model.json",
        br#"{"schema":"synthetic"}"#,
        false,
    );
    write_manifest(
        root,
        json!({
            "schema_version": "resume-ir.desktop-classifier-model-pack.v1",
            "classifier_epoch": "precision_first_v4",
            "feature_contract": "bounded_normalized_text_plus_structure_v1",
            "distribution_scope": "user_authorized_internal_test",
            "network_access": "disabled",
            "files": [{
                "role": "linear_promotion_model",
                "file": "linear-promotion-model.json",
                "bytes": fs::metadata(&model).unwrap().len(),
                "sha256": sha256_file(&model).unwrap(),
            }],
        }),
    );

    assert_eq!(
        validate_classifier(&model),
        Err(OptionalRuntimeReason::Invalid)
    );

    let renamed = write_file(
        root,
        "renamed-model.json",
        br#"{"schema":"synthetic"}"#,
        false,
    );
    assert_eq!(
        validate_classifier(&renamed),
        Err(OptionalRuntimeReason::Invalid)
    );

    fs::write(&model, br#"{"schema":"tampered!"}"#).unwrap();
    assert_eq!(
        validate_classifier(&model),
        Err(OptionalRuntimeReason::Invalid)
    );
}

#[test]
fn declared_pack_size_is_bounded_before_any_asset_read() {
    let directory = tempdir().unwrap();
    let entries = (0..3)
        .map(|index| PackFile {
            role: format!("asset_{index}"),
            file: format!("missing-{index}"),
            bytes: 400 * 1024 * 1024,
            sha256: "a".repeat(64),
        })
        .collect::<Vec<_>>();

    assert!(matches!(
        validate_pack_file_entries(directory.path(), &entries),
        Err(OptionalRuntimeReason::Invalid)
    ));
}

#[test]
fn cancellation_aborts_pack_hash_before_entry_is_accepted() {
    let directory = tempdir().unwrap();
    let asset = write_file(
        directory.path(),
        "large-model.bin",
        &vec![0x5a; 3 * 64 * 1024],
        false,
    );
    let entries = [PackFile {
        role: "model".to_owned(),
        file: "large-model.bin".to_owned(),
        bytes: fs::metadata(&asset).unwrap().len(),
        sha256: sha256_file(&asset).unwrap(),
    }];
    let callback_calls = AtomicUsize::new(0);
    let cancelled = || callback_calls.fetch_add(1, Ordering::SeqCst) >= 4;

    assert_eq!(
        validate_pack_file_entries_with_cancel(directory.path(), &entries, &cancelled).map(drop),
        Err(OptionalRuntimeReason::StartFailed)
    );
    assert!(callback_calls.load(Ordering::SeqCst) > 4);
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn signed_macho(payload: &[u8], signature: &[u8]) -> Vec<u8> {
    const HEADER_BYTES: usize = 32;
    const SEGMENT_BYTES: usize = 72;
    const SIGNATURE_COMMAND_BYTES: usize = 16;
    let command_bytes = SEGMENT_BYTES + SIGNATURE_COMMAND_BYTES;
    let signature_offset = HEADER_BYTES + command_bytes + payload.len();
    let mut bytes = vec![0_u8; signature_offset];
    write_u32(&mut bytes, 0, 0xfeedfacf);
    write_u32(&mut bytes, 4, 0x0100000c);
    write_u32(&mut bytes, 12, 2);
    write_u32(&mut bytes, 16, 2);
    write_u32(&mut bytes, 20, command_bytes as u32);

    let segment = HEADER_BYTES;
    write_u32(&mut bytes, segment, 0x19);
    write_u32(&mut bytes, segment + 4, SEGMENT_BYTES as u32);
    bytes[segment + 8..segment + 18].copy_from_slice(b"__LINKEDIT");
    write_u64(&mut bytes, segment + 32, signature.len() as u64);
    write_u64(&mut bytes, segment + 40, signature_offset as u64);
    write_u64(&mut bytes, segment + 48, signature.len() as u64);

    let signature_command = HEADER_BYTES + SEGMENT_BYTES;
    write_u32(&mut bytes, signature_command, 0x1d);
    write_u32(
        &mut bytes,
        signature_command + 4,
        SIGNATURE_COMMAND_BYTES as u32,
    );
    write_u32(&mut bytes, signature_command + 8, signature_offset as u32);
    write_u32(&mut bytes, signature_command + 12, signature.len() as u32);
    bytes[HEADER_BYTES + command_bytes..signature_offset].copy_from_slice(payload);
    bytes.extend_from_slice(signature);
    bytes
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn ocr_entry(role: &str, relative: &str, path: &Path, executable: bool) -> Value {
    json!({
        "role": role,
        "file": relative,
        "bytes": fs::metadata(path).unwrap().len(),
        "sha256": sha256_file(path).unwrap(),
        "executable": executable,
    })
}

fn write_manifest(root: &Path, value: Value) {
    fs::write(
        root.join("runtime-pack.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
}

fn write_file(root: &Path, relative: &str, bytes: &[u8], executable: bool) -> PathBuf {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            &path,
            fs::Permissions::from_mode(if executable { 0o755 } else { 0o644 }),
        )
        .unwrap();
    }
    path
}
