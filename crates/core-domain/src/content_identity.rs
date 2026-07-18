use std::{collections::BTreeSet, fmt, str::FromStr};

use sha2::{Digest, Sha256};

use crate::{DocumentId, ResumeVersionId, SourceRevisionId, ID_DIGEST_HEX_LEN};

const SHA256_PREFIX: &str = "sha256:";
const SHA256_HEX_LEN: usize = 64;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentDigest(String);

impl ContentDigest {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(format!("sha256:{:x}", Sha256::digest(bytes)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Canonical digest of a complete one-to-one document/version search mapping.
///
/// The input order is irrelevant. Duplicate documents or versions are
/// rejected so the digest cannot bless an ambiguous active projection.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SearchProjectionDigest(ContentDigest);

impl SearchProjectionDigest {
    pub fn from_pairs<I, D, V>(pairs: I) -> Result<Self, SearchProjectionDigestError>
    where
        I: IntoIterator<Item = (D, V)>,
        D: AsRef<str>,
        V: AsRef<str>,
    {
        let mut canonical = Vec::new();
        let mut document_ids = BTreeSet::new();
        let mut resume_version_ids = BTreeSet::new();
        for (document_id, resume_version_id) in pairs {
            let document_id = document_id.as_ref();
            let resume_version_id = resume_version_id.as_ref();
            DocumentId::from_str(document_id)
                .map_err(|_| SearchProjectionDigestError::InvalidIdentity)?;
            ResumeVersionId::from_str(resume_version_id)
                .map_err(|_| SearchProjectionDigestError::InvalidIdentity)?;
            if !document_ids.insert(document_id.to_string()) {
                return Err(SearchProjectionDigestError::DuplicateDocument);
            }
            if !resume_version_ids.insert(resume_version_id.to_string()) {
                return Err(SearchProjectionDigestError::DuplicateResumeVersion);
            }
            canonical.push((document_id.to_string(), resume_version_id.to_string()));
        }
        canonical.sort_unstable();

        let mut hasher = Sha256::new();
        update_content_addressed_part(&mut hasher, b"resume-ir.search-projection.v1");
        hasher.update((canonical.len() as u64).to_le_bytes());
        for (document_id, resume_version_id) in canonical {
            update_content_addressed_part(&mut hasher, document_id.as_bytes());
            update_content_addressed_part(&mut hasher, resume_version_id.as_bytes());
        }
        Ok(Self(ContentDigest(format!(
            "sha256:{:x}",
            hasher.finalize()
        ))))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for SearchProjectionDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SearchProjectionDigest(<redacted>)")
    }
}

impl FromStr for SearchProjectionDigest {
    type Err = ContentDigestParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse::<ContentDigest>().map(Self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchProjectionDigestError {
    InvalidIdentity,
    DuplicateDocument,
    DuplicateResumeVersion,
}

impl fmt::Display for SearchProjectionDigestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("search projection identity mapping is invalid")
    }
}

impl std::error::Error for SearchProjectionDigestError {}

impl FromStr for ContentDigest {
    type Err = ContentDigestParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let digest = value
            .strip_prefix(SHA256_PREFIX)
            .ok_or(ContentDigestParseError::InvalidFormat)?;
        if digest.len() != SHA256_HEX_LEN
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ContentDigestParseError::InvalidFormat);
        }
        Ok(Self(value.to_string()))
    }
}

impl fmt::Debug for ContentDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ContentDigest(<redacted>)")
    }
}

impl fmt::Display for ContentDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted-content-digest>")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentDigestParseError {
    InvalidFormat,
}

impl fmt::Display for ContentDigestParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("content digest must be a lowercase sha256 value")
    }
}

impl std::error::Error for ContentDigestParseError {}

#[derive(Clone, PartialEq, Eq)]
pub struct SourceRevision {
    pub id: SourceRevisionId,
    pub document_id: DocumentId,
    pub content_hash: ContentDigest,
    pub byte_size: u64,
}

impl SourceRevision {
    pub fn for_content(
        document_id: DocumentId,
        content_hash: ContentDigest,
        byte_size: u64,
    ) -> Self {
        let id = SourceRevisionId::from_content_identity(&document_id, &content_hash);
        Self {
            id,
            document_id,
            content_hash,
            byte_size,
        }
    }
}

impl fmt::Debug for SourceRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SourceRevision")
            .field("id", &self.id)
            .field("document_id", &self.document_id)
            .field("content_hash", &self.content_hash)
            .field("byte_size", &self.byte_size)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SearchSelection {
    pub document_id: DocumentId,
    pub resume_version_id: ResumeVersionId,
    pub visible_epoch: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ActiveSearchProjection {
    pub document_id: DocumentId,
    pub resume_version_id: ResumeVersionId,
}

impl fmt::Debug for ActiveSearchProjection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActiveSearchProjection")
            .field("document_id", &self.document_id)
            .field("resume_version_id", &self.resume_version_id)
            .finish()
    }
}

impl SourceRevisionId {
    pub fn from_content_identity(document_id: &DocumentId, content: &ContentDigest) -> Self {
        Self::from_content_hex_digest(content_addressed_id_digest(
            "SourceRevisionId",
            &[document_id.as_str().as_bytes(), content.as_str().as_bytes()],
        ))
    }
}

impl ResumeVersionId {
    pub fn from_content_identity(
        document_id: &DocumentId,
        source_revision_id: &SourceRevisionId,
        normalized_text: &ContentDigest,
        parser_version: &str,
        schema_version: &str,
    ) -> Self {
        Self::from_content_hex_digest(content_addressed_id_digest(
            "ResumeVersionId",
            &[
                document_id.as_str().as_bytes(),
                source_revision_id.as_str().as_bytes(),
                normalized_text.as_str().as_bytes(),
                parser_version.as_bytes(),
                schema_version.as_bytes(),
            ],
        ))
    }
}

fn content_addressed_id_digest(namespace: &str, parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(namespace.len().to_le_bytes());
    hasher.update(namespace.as_bytes());
    hasher.update(parts.len().to_le_bytes());
    for part in parts {
        hasher.update(part.len().to_le_bytes());
        hasher.update(part);
    }
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(ID_DIGEST_HEX_LEN);
    for byte in &digest[..ID_DIGEST_HEX_LEN / 2] {
        use std::fmt::Write;
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

fn update_content_addressed_part(hasher: &mut Sha256, part: &[u8]) {
    hasher.update((part.len() as u64).to_le_bytes());
    hasher.update(part);
}
