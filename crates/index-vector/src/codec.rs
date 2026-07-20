use crate::model::{
    validate_documents_with_control, VectorDocument, VectorDocumentIdentity, VectorIndexError,
};
use crate::model_contract::VectorModelContract;
use crate::private_storage::{
    create_private_file, decode_fixed_hex, decode_hex, encode_hex, load_or_create_key,
    random_bytes, read_key, read_private_bytes, read_private_bytes_bounded, sync_directory,
    write_private_bytes,
};
use crate::publish_control::VectorSnapshotPublishControl;
use crate::snapshot_identity::{
    canonical_projection_with_control, coverage_digest_with_control,
    logical_content_digest_with_control,
};
use crate::snapshot_model::{
    projection_digest_with_control, VectorSnapshotDigests, VectorSnapshotManifestMetadata,
    VectorSnapshotSummary, VECTOR_SNAPSHOT_SCHEMA_V4,
};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use core_domain::{
    ActiveSearchProjection, ContentDigest, DocumentId, ResumeVersionId, SearchProjectionDigest,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;
use std::str::FromStr;

pub(crate) const SNAPSHOT_FILE: &str = "vector.snapshot.enc";
pub(crate) const MANIFEST_FILE: &str = "snapshot-manifest.json";
pub(crate) const KEY_FILE: &str = "vector.snapshot.key-v4";
const PAYLOAD_SCHEMA: &str = "vector.payload.v4";
const SEARCH_BACKEND: &str = "hnsw_ann";
const ENCRYPTION: &str = "xchacha20poly1305.v1";
const ENCRYPTED_HEADER: &str = "resume-ir-vector-index-encrypted-v4";
const NONCE_LEN: usize = 24;
const ENCRYPTED_WRITE_CHUNK_BYTES: usize = 64 * 1024;
pub(crate) const MAX_MANIFEST_BYTES: usize = 4 * 1024;

#[cfg(test)]
pub(crate) fn write_snapshot(
    snapshot_dir: &Path,
    key_path: &Path,
    generation: &str,
    model_contract: &VectorModelContract,
    projection: &[ActiveSearchProjection],
    documents: &[VectorDocument],
) -> Result<VectorSnapshotSummary, VectorIndexError> {
    write_snapshot_with_control(
        snapshot_dir,
        key_path,
        generation,
        model_contract,
        projection,
        documents,
        VectorSnapshotPublishControl::disabled(),
    )
}

pub(crate) fn write_snapshot_with_control(
    snapshot_dir: &Path,
    key_path: &Path,
    generation: &str,
    model_contract: &VectorModelContract,
    projection: &[ActiveSearchProjection],
    documents: &[VectorDocument],
    control: VectorSnapshotPublishControl<'_>,
) -> Result<VectorSnapshotSummary, VectorIndexError> {
    control.check()?;
    let projection = canonical_projection_with_control(projection, control)?;
    validate_documents_with_control(model_contract, &projection, documents, control)?;
    let projection_digest = projection_digest_with_control(&projection, control)?;
    let coverage_digest = coverage_digest_with_control(documents, control)?;
    let logical_content_digest =
        logical_content_digest_with_control(model_contract, &projection, documents, control)?;
    let mut active_projection = Vec::with_capacity(projection.len());
    for (index, entry) in projection.iter().enumerate() {
        active_projection.push(json!({
            "document_id": entry.document_id.as_str(),
            "resume_version_id": entry.resume_version_id.as_str(),
        }));
        control.check_after_record(index + 1)?;
    }
    let mut vectors = Vec::with_capacity(documents.len());
    for (index, document) in documents.iter().enumerate() {
        vectors.push(json!({
            "vector_id": document.vector_id(),
            "document_id": document.document_id(),
            "resume_version_id": document.resume_version_id(),
            "model_id": document.model_id(),
            "values": document.values(),
        }));
        control.check_after_record(index + 1)?;
    }
    control.check()?;
    let plaintext = serde_json::to_vec(&json!({
        "schema_version": PAYLOAD_SCHEMA,
        "generation": generation,
        "model_id": model_contract.model_id(),
        "dimension": model_contract.dimension(),
        "active_projection": active_projection,
        "vectors": vectors,
    }))
    .map_err(|_| VectorIndexError::Storage)?;
    control.check()?;
    let artifact_digest = write_encrypted_payload(snapshot_dir, key_path, &plaintext, control)?;
    let summary = VectorSnapshotSummary::from_contents_with_control(
        generation.to_string(),
        model_contract.clone(),
        &projection,
        documents,
        VectorSnapshotDigests::new(
            projection_digest,
            coverage_digest,
            logical_content_digest,
            artifact_digest,
        ),
        control,
    )?;
    control.check()?;
    write_manifest(snapshot_dir, &summary)?;
    control.check()?;
    sync_directory(snapshot_dir)?;
    control.check()?;
    Ok(summary)
}

pub(crate) fn read_snapshot(
    snapshot_dir: &Path,
    key_path: &Path,
    expected_generation: &str,
    expected_model_contract: &VectorModelContract,
) -> Result<
    (
        Vec<ActiveSearchProjection>,
        Vec<VectorDocument>,
        VectorSnapshotSummary,
    ),
    VectorIndexError,
> {
    read_snapshot_with_control(
        snapshot_dir,
        key_path,
        expected_generation,
        expected_model_contract,
        VectorSnapshotPublishControl::disabled(),
    )
}

pub(crate) fn read_snapshot_with_control(
    snapshot_dir: &Path,
    key_path: &Path,
    expected_generation: &str,
    expected_model_contract: &VectorModelContract,
    control: VectorSnapshotPublishControl<'_>,
) -> Result<
    (
        Vec<ActiveSearchProjection>,
        Vec<VectorDocument>,
        VectorSnapshotSummary,
    ),
    VectorIndexError,
> {
    control.check()?;
    expected_model_contract.validate()?;
    let manifest_metadata = decode_manifest_metadata(
        &read_private_bytes_bounded(&snapshot_dir.join(MANIFEST_FILE), MAX_MANIFEST_BYTES)?,
        expected_generation,
        expected_model_contract,
    )?;
    let manifest_projection_digest = manifest_metadata.projection_digest().clone();
    let manifest_coverage_digest = manifest_metadata.coverage_digest().clone();
    let manifest_logical_content_digest = manifest_metadata.logical_content_digest().clone();
    let manifest_artifact_digest = manifest_metadata.artifact_digest().clone();
    control.check()?;
    let plaintext = read_encrypted_payload(snapshot_dir, key_path, &manifest_artifact_digest)?;
    control.check()?;
    let payload: Value =
        serde_json::from_slice(&plaintext).map_err(|_| VectorIndexError::CorruptSnapshot)?;
    control.check()?;
    let payload = payload
        .as_object()
        .ok_or(VectorIndexError::CorruptSnapshot)?;
    ensure_exact_keys(
        payload,
        &[
            "schema_version",
            "generation",
            "model_id",
            "dimension",
            "active_projection",
            "vectors",
        ],
    )?;
    if string_field(payload, "schema_version")? != PAYLOAD_SCHEMA {
        return Err(VectorIndexError::SchemaMismatch);
    }
    if string_field(payload, "generation")? != expected_generation {
        return Err(VectorIndexError::CorruptSnapshot);
    }
    let payload_model_contract = parse_model_contract(payload)?;
    if &payload_model_contract != expected_model_contract {
        return Err(VectorIndexError::CorruptSnapshot);
    }
    let projection_values = payload
        .get("active_projection")
        .and_then(Value::as_array)
        .ok_or(VectorIndexError::CorruptSnapshot)?;
    let mut projection = Vec::with_capacity(projection_values.len());
    for (index, value) in projection_values.iter().enumerate() {
        projection.push(parse_projection_entry(value)?);
        control.check_after_record(index + 1)?;
    }
    let projection = canonical_projection_with_control(&projection, control)
        .map_err(map_staging_validation_error)?;
    let document_values = payload
        .get("vectors")
        .and_then(Value::as_array)
        .ok_or(VectorIndexError::CorruptSnapshot)?;
    let mut documents = Vec::with_capacity(document_values.len());
    for (index, value) in document_values.iter().enumerate() {
        documents.push(parse_document(value)?);
        control.check_after_record(index + 1)?;
    }
    validate_documents_with_control(expected_model_contract, &projection, &documents, control)
        .map_err(map_staging_validation_error)?;
    let actual_projection_digest = projection_digest_with_control(&projection, control)
        .map_err(map_staging_validation_error)?;
    if actual_projection_digest != manifest_projection_digest {
        return Err(VectorIndexError::CorruptSnapshot);
    }
    let actual_coverage_digest =
        coverage_digest_with_control(&documents, control).map_err(map_staging_validation_error)?;
    if actual_coverage_digest != manifest_coverage_digest {
        return Err(VectorIndexError::CorruptSnapshot);
    }
    let actual_logical_content_digest = logical_content_digest_with_control(
        expected_model_contract,
        &projection,
        &documents,
        control,
    )
    .map_err(map_staging_validation_error)?;
    if actual_logical_content_digest != manifest_logical_content_digest {
        return Err(VectorIndexError::CorruptSnapshot);
    }
    let summary = VectorSnapshotSummary::from_contents_with_control(
        expected_generation.to_string(),
        expected_model_contract.clone(),
        &projection,
        &documents,
        VectorSnapshotDigests::new(
            actual_projection_digest,
            actual_coverage_digest,
            actual_logical_content_digest,
            manifest_artifact_digest,
        ),
        control,
    )?;
    if manifest_metadata.vector_count() != summary.vector_count()
        || manifest_metadata.projection_count() != summary.projection_count()
        || manifest_metadata.vector_document_count() != summary.vector_document_count()
    {
        return Err(VectorIndexError::CorruptSnapshot);
    }
    control.check()?;
    Ok((projection, documents, summary))
}

fn map_staging_validation_error(error: VectorIndexError) -> VectorIndexError {
    if error == VectorIndexError::Cancelled {
        error
    } else {
        VectorIndexError::CorruptSnapshot
    }
}

pub(crate) fn decode_manifest_metadata(
    bytes: &[u8],
    expected_generation: &str,
    expected_model_contract: &VectorModelContract,
) -> Result<VectorSnapshotManifestMetadata, VectorIndexError> {
    if bytes.is_empty() || bytes.len() > MAX_MANIFEST_BYTES {
        return Err(VectorIndexError::CorruptSnapshot);
    }
    expected_model_contract.validate()?;
    let manifest =
        serde_json::from_slice::<Value>(bytes).map_err(|_| VectorIndexError::CorruptSnapshot)?;
    let manifest = manifest
        .as_object()
        .ok_or(VectorIndexError::CorruptSnapshot)?;
    ensure_exact_keys(
        manifest,
        &[
            "schema_version",
            "index_schema",
            "generation",
            "model_id",
            "dimension",
            "vector_count",
            "projection_count",
            "vector_document_count",
            "projection_digest",
            "coverage_digest",
            "logical_content_digest",
            "artifact_digest",
            "search_backend",
            "encryption",
        ],
    )?;
    validate_manifest(manifest, expected_generation, expected_model_contract)?;
    let vector_count = usize_field(manifest, "vector_count")?;
    let projection_count = usize_field(manifest, "projection_count")?;
    let vector_document_count = usize_field(manifest, "vector_document_count")?;
    if vector_document_count > vector_count
        || vector_document_count > projection_count
        || (matches!(expected_model_contract, VectorModelContract::Disabled)
            && (vector_count != 0 || vector_document_count != 0))
    {
        return Err(VectorIndexError::CorruptSnapshot);
    }
    Ok(VectorSnapshotManifestMetadata::new(
        expected_generation.to_string(),
        expected_model_contract.clone(),
        vector_count,
        projection_count,
        vector_document_count,
        VectorSnapshotDigests::new(
            digest_field::<SearchProjectionDigest>(manifest, "projection_digest")?,
            digest_field::<SearchProjectionDigest>(manifest, "coverage_digest")?,
            digest_field::<ContentDigest>(manifest, "logical_content_digest")?,
            digest_field::<ContentDigest>(manifest, "artifact_digest")?,
        ),
    ))
}

fn write_encrypted_payload(
    snapshot_dir: &Path,
    key_path: &Path,
    plaintext: &[u8],
    control: VectorSnapshotPublishControl<'_>,
) -> Result<ContentDigest, VectorIndexError> {
    control.check()?;
    let key = load_or_create_key(key_path)?;
    let nonce = random_bytes::<NONCE_LEN>()?;
    control.check()?;
    let ciphertext = XChaCha20Poly1305::new((&key).into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: ENCRYPTED_HEADER.as_bytes(),
            },
        )
        .map_err(|_| VectorIndexError::Storage)?;
    control.check()?;
    let encoded_capacity = ciphertext
        .len()
        .checked_mul(2)
        .and_then(|ciphertext_size| ENCRYPTED_HEADER.len().checked_add(ciphertext_size))
        .and_then(|size| size.checked_add(1 + NONCE_LEN * 2 + 1))
        .and_then(|size| size.checked_add(1))
        .ok_or(VectorIndexError::Storage)?;
    let mut encrypted = Vec::with_capacity(encoded_capacity);
    encrypted.extend_from_slice(ENCRYPTED_HEADER.as_bytes());
    encrypted.push(b'\n');
    encrypted.extend_from_slice(encode_hex(&nonce).as_bytes());
    encrypted.push(b'\n');
    append_hex_with_control(&mut encrypted, &ciphertext, control)?;
    encrypted.push(b'\n');
    control.check()?;
    let content_digest = ContentDigest::from_bytes(&encrypted);
    let mut snapshot = create_private_file(&snapshot_dir.join(SNAPSHOT_FILE))?;
    for chunk in encrypted.chunks(ENCRYPTED_WRITE_CHUNK_BYTES) {
        control.check()?;
        snapshot
            .write_all(chunk)
            .map_err(|_| VectorIndexError::Storage)?;
    }
    control.check()?;
    snapshot.sync_all().map_err(|_| VectorIndexError::Storage)?;
    control.check()?;
    Ok(content_digest)
}

fn append_hex_with_control(
    output: &mut Vec<u8>,
    bytes: &[u8],
    control: VectorSnapshotPublishControl<'_>,
) -> Result<(), VectorIndexError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for (index, byte) in bytes.iter().copied().enumerate() {
        output.push(HEX[usize::from(byte >> 4)]);
        output.push(HEX[usize::from(byte & 0x0f)]);
        control.check_after_record(index + 1)?;
    }
    Ok(())
}

fn read_encrypted_payload(
    snapshot_dir: &Path,
    key_path: &Path,
    expected_artifact_digest: &ContentDigest,
) -> Result<Vec<u8>, VectorIndexError> {
    let encrypted = read_private_bytes(&snapshot_dir.join(SNAPSHOT_FILE))?;
    if &ContentDigest::from_bytes(&encrypted) != expected_artifact_digest {
        return Err(VectorIndexError::CorruptSnapshot);
    }
    let encrypted =
        std::str::from_utf8(&encrypted).map_err(|_| VectorIndexError::CorruptSnapshot)?;
    let mut lines = encrypted.lines();
    let header = lines.next().ok_or(VectorIndexError::CorruptSnapshot)?;
    if header != ENCRYPTED_HEADER {
        return Err(VectorIndexError::SchemaMismatch);
    }
    let nonce =
        decode_fixed_hex::<NONCE_LEN>(lines.next().ok_or(VectorIndexError::CorruptSnapshot)?)?;
    let ciphertext = decode_hex(lines.next().ok_or(VectorIndexError::CorruptSnapshot)?)?;
    if lines.next().is_some() {
        return Err(VectorIndexError::CorruptSnapshot);
    }
    let key = read_key(key_path)?;
    XChaCha20Poly1305::new((&key).into())
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: ENCRYPTED_HEADER.as_bytes(),
            },
        )
        .map_err(|_| VectorIndexError::CorruptSnapshot)
}

fn write_manifest(
    snapshot_dir: &Path,
    summary: &VectorSnapshotSummary,
) -> Result<(), VectorIndexError> {
    let bytes = serde_json::to_vec(&json!({
        "schema_version": VECTOR_SNAPSHOT_SCHEMA_V4.manifest_schema(),
        "index_schema": VECTOR_SNAPSHOT_SCHEMA_V4.index_schema(),
        "generation": summary.generation(),
        "model_id": summary.model_contract().model_id(),
        "dimension": summary.model_contract().dimension(),
        "vector_count": summary.vector_count(),
        "projection_count": summary.projection_count(),
        "vector_document_count": summary.vector_document_count(),
        "projection_digest": summary.projection_digest().as_str(),
        "coverage_digest": summary.coverage_digest().as_str(),
        "logical_content_digest": summary.logical_content_digest().as_str(),
        "artifact_digest": summary.artifact_digest().as_str(),
        "search_backend": SEARCH_BACKEND,
        "encryption": ENCRYPTION,
    }))
    .map_err(|_| VectorIndexError::Storage)?;
    write_private_bytes(&snapshot_dir.join(MANIFEST_FILE), &bytes)
}

fn validate_manifest(
    manifest: &Map<String, Value>,
    generation: &str,
    expected_model_contract: &VectorModelContract,
) -> Result<(), VectorIndexError> {
    if string_field(manifest, "schema_version")? != VECTOR_SNAPSHOT_SCHEMA_V4.manifest_schema()
        || string_field(manifest, "index_schema")? != VECTOR_SNAPSHOT_SCHEMA_V4.index_schema()
        || string_field(manifest, "search_backend")? != SEARCH_BACKEND
        || string_field(manifest, "encryption")? != ENCRYPTION
    {
        return Err(VectorIndexError::SchemaMismatch);
    }
    if string_field(manifest, "generation")? != generation
        || &parse_model_contract(manifest)? != expected_model_contract
    {
        return Err(VectorIndexError::CorruptSnapshot);
    }
    Ok(())
}

fn parse_projection_entry(value: &Value) -> Result<ActiveSearchProjection, VectorIndexError> {
    let object = value.as_object().ok_or(VectorIndexError::CorruptSnapshot)?;
    ensure_exact_keys(object, &["document_id", "resume_version_id"])?;
    Ok(ActiveSearchProjection {
        document_id: DocumentId::from_str(string_field(object, "document_id")?)
            .map_err(|_| VectorIndexError::CorruptSnapshot)?,
        resume_version_id: ResumeVersionId::from_str(string_field(object, "resume_version_id")?)
            .map_err(|_| VectorIndexError::CorruptSnapshot)?,
    })
}

fn parse_model_contract(
    object: &Map<String, Value>,
) -> Result<VectorModelContract, VectorIndexError> {
    match (object.get("model_id"), object.get("dimension")) {
        (Some(Value::Null), Some(Value::Null)) => Ok(VectorModelContract::Disabled),
        (Some(Value::String(model_id)), Some(Value::Number(dimension))) => {
            let dimension = dimension
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or(VectorIndexError::CorruptSnapshot)?;
            VectorModelContract::enabled(model_id, dimension)
                .map_err(|_| VectorIndexError::CorruptSnapshot)
        }
        _ => Err(VectorIndexError::CorruptSnapshot),
    }
}

fn digest_field<D>(object: &Map<String, Value>, field: &str) -> Result<D, VectorIndexError>
where
    D: FromStr,
{
    string_field(object, field)?
        .parse()
        .map_err(|_| VectorIndexError::CorruptSnapshot)
}

fn parse_document(value: &Value) -> Result<VectorDocument, VectorIndexError> {
    let object = value.as_object().ok_or(VectorIndexError::CorruptSnapshot)?;
    ensure_exact_keys(
        object,
        &[
            "vector_id",
            "document_id",
            "resume_version_id",
            "model_id",
            "values",
        ],
    )?;
    let values = object
        .get("values")
        .and_then(Value::as_array)
        .ok_or(VectorIndexError::CorruptSnapshot)?
        .iter()
        .map(|value| {
            value
                .as_f64()
                .map(|value| value as f32)
                .filter(|value| value.is_finite())
                .ok_or(VectorIndexError::CorruptSnapshot)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let identity = VectorDocumentIdentity::new(
        string_field(object, "vector_id")?,
        string_field(object, "document_id")?,
        string_field(object, "resume_version_id")?,
        string_field(object, "model_id")?,
    )
    .map_err(|_| VectorIndexError::CorruptSnapshot)?;
    VectorDocument::new(identity, values).map_err(|_| VectorIndexError::CorruptSnapshot)
}

fn ensure_exact_keys(
    object: &Map<String, Value>,
    expected: &[&str],
) -> Result<(), VectorIndexError> {
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual == expected {
        Ok(())
    } else {
        Err(VectorIndexError::CorruptSnapshot)
    }
}

fn string_field<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a str, VectorIndexError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or(VectorIndexError::CorruptSnapshot)
}

fn usize_field(object: &Map<String, Value>, field: &str) -> Result<usize, VectorIndexError> {
    object
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(VectorIndexError::CorruptSnapshot)
}
