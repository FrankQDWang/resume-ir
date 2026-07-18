use core_domain::{ContentDigest, SearchProjectionDigest};
use serde::{Deserialize, Serialize};

pub(crate) const SNAPSHOT_MANIFEST_SCHEMA_VERSION: &str = "fulltext.snapshot.v2";
pub(crate) const FULLTEXT_INDEX_SCHEMA_VERSION: &str = "tantivy.fulltext.v2";
pub(crate) const SNAPSHOT_HEADER_ENCRYPTED_V2: &str = "resume-ir-fulltext-snapshot-encrypted-v2";
pub(crate) const MAX_MANIFEST_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FullTextSnapshotSchema {
    manifest_schema: &'static str,
    index_schema: &'static str,
}

impl FullTextSnapshotSchema {
    pub const fn manifest_schema(self) -> &'static str {
        self.manifest_schema
    }

    pub const fn index_schema(self) -> &'static str {
        self.index_schema
    }
}

pub const FULLTEXT_SNAPSHOT_SCHEMA_V2: FullTextSnapshotSchema = FullTextSnapshotSchema {
    manifest_schema: SNAPSHOT_MANIFEST_SCHEMA_VERSION,
    index_schema: FULLTEXT_INDEX_SCHEMA_VERSION,
};

/// Validated identity and size metadata for one immutable full-text generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedSnapshotMetadata {
    generation: String,
    document_count: usize,
    projection_digest: SearchProjectionDigest,
    logical_content_digest: ContentDigest,
    artifact_digest: ContentDigest,
}

impl PublishedSnapshotMetadata {
    pub fn generation(&self) -> &str {
        &self.generation
    }

    pub const fn schema(&self) -> FullTextSnapshotSchema {
        FULLTEXT_SNAPSHOT_SCHEMA_V2
    }

    pub fn document_count(&self) -> usize {
        self.document_count
    }

    pub fn projection_digest(&self) -> &SearchProjectionDigest {
        &self.projection_digest
    }

    pub fn logical_content_digest(&self) -> &ContentDigest {
        &self.logical_content_digest
    }

    pub fn artifact_digest(&self) -> &ContentDigest {
        &self.artifact_digest
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ManifestError {
    Corrupt,
    SchemaMismatch,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SnapshotManifest {
    schema_version: String,
    index_schema: String,
    encrypted_snapshot: String,
    generation: String,
    document_count: u64,
    projection_digest: String,
    logical_content_digest: String,
    artifact_digest: String,
}

pub(crate) fn encode_manifest(
    generation: &str,
    document_count: usize,
    projection_digest: &SearchProjectionDigest,
    logical_content_digest: &ContentDigest,
    artifact_digest: &ContentDigest,
) -> Result<Vec<u8>, ManifestError> {
    let manifest = SnapshotManifest {
        schema_version: SNAPSHOT_MANIFEST_SCHEMA_VERSION.to_string(),
        index_schema: FULLTEXT_INDEX_SCHEMA_VERSION.to_string(),
        encrypted_snapshot: SNAPSHOT_HEADER_ENCRYPTED_V2.to_string(),
        generation: generation.to_string(),
        document_count: u64::try_from(document_count).map_err(|_| ManifestError::Corrupt)?,
        projection_digest: projection_digest.as_str().to_string(),
        logical_content_digest: logical_content_digest.as_str().to_string(),
        artifact_digest: artifact_digest.as_str().to_string(),
    };
    let mut bytes = serde_json::to_vec(&manifest).map_err(|_| ManifestError::Corrupt)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestError::Corrupt);
    }
    Ok(bytes)
}

pub(crate) fn decode_manifest(
    bytes: &[u8],
    expected_generation: &str,
) -> Result<PublishedSnapshotMetadata, ManifestError> {
    if bytes.is_empty() || bytes.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestError::Corrupt);
    }
    let manifest: SnapshotManifest =
        serde_json::from_slice(bytes).map_err(|_| ManifestError::Corrupt)?;
    if manifest.schema_version != SNAPSHOT_MANIFEST_SCHEMA_VERSION
        || manifest.index_schema != FULLTEXT_INDEX_SCHEMA_VERSION
        || manifest.encrypted_snapshot != SNAPSHOT_HEADER_ENCRYPTED_V2
        || manifest.generation != expected_generation
    {
        return Err(ManifestError::SchemaMismatch);
    }
    let document_count =
        usize::try_from(manifest.document_count).map_err(|_| ManifestError::Corrupt)?;
    let projection_digest = manifest
        .projection_digest
        .parse()
        .map_err(|_| ManifestError::Corrupt)?;
    let logical_content_digest = manifest
        .logical_content_digest
        .parse()
        .map_err(|_| ManifestError::Corrupt)?;
    let artifact_digest = manifest
        .artifact_digest
        .parse()
        .map_err(|_| ManifestError::Corrupt)?;
    Ok(PublishedSnapshotMetadata {
        generation: manifest.generation,
        document_count,
        projection_digest,
        logical_content_digest,
        artifact_digest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_strict_and_generation_bound() {
        let projection_digest = SearchProjectionDigest::from_pairs([(
            "doc_00000000000000000000000000000001",
            "ver_00000000000000000000000000000001",
        )])
        .unwrap();
        let logical_content_digest = ContentDigest::from_bytes(b"synthetic logical contents");
        let artifact_digest = ContentDigest::from_bytes(b"synthetic encrypted snapshot");
        let bytes = encode_manifest(
            "fulltext-generation-a",
            7,
            &projection_digest,
            &logical_content_digest,
            &artifact_digest,
        )
        .unwrap();
        let metadata = decode_manifest(&bytes, "fulltext-generation-a").unwrap();
        assert_eq!(metadata.generation(), "fulltext-generation-a");
        assert_eq!(metadata.document_count(), 7);
        assert_eq!(metadata.projection_digest(), &projection_digest);
        assert_eq!(metadata.logical_content_digest(), &logical_content_digest);
        assert_eq!(metadata.artifact_digest(), &artifact_digest);

        assert_eq!(
            decode_manifest(&bytes, "fulltext-generation-b"),
            Err(ManifestError::SchemaMismatch)
        );
        let with_unknown = String::from_utf8(bytes)
            .unwrap()
            .replace("\n", ",\"legacy_alias\":true}\n")
            .replace("},\"legacy_alias", ",\"legacy_alias");
        assert_eq!(
            decode_manifest(with_unknown.as_bytes(), "fulltext-generation-a"),
            Err(ManifestError::Corrupt)
        );
    }
}
