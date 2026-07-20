use std::str::FromStr;

use rusqlite::Connection;

use crate::{ContentDigest, MetaStoreError, Result, SearchProjectionDigest};

pub(crate) const LEGACY_FULLTEXT_MANIFEST: &str = "fulltext.snapshot.v2";
pub(crate) const LEGACY_FULLTEXT_INDEX: &str = "tantivy.fulltext.v2";
pub(crate) const LEGACY_VECTOR_MANIFEST: &str = "vector.snapshot.v3";
pub(crate) const LEGACY_VECTOR_INDEX: &str = "hnsw-vector.v3";
pub(crate) const CURRENT_FULLTEXT_MANIFEST: &str = "fulltext.snapshot.v3";
pub(crate) const CURRENT_FULLTEXT_INDEX: &str = "tantivy.fulltext.v3";
pub(crate) const CURRENT_VECTOR_MANIFEST: &str = "vector.snapshot.v4";
pub(crate) const CURRENT_VECTOR_INDEX: &str = "hnsw-vector.v4";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DescriptorContract {
    None,
    Legacy,
    Current,
}

pub(crate) struct PublicationDescriptorRecord {
    pub(crate) generation: String,
    pub(crate) expected_visible_epoch: u64,
    pub(crate) classifier_epoch: String,
    pub(crate) projection_digest: SearchProjectionDigest,
    pub(crate) publication_fingerprint: Option<ContentDigest>,
    pub(crate) state: String,
    pub(crate) fulltext_generation: Option<String>,
    pub(crate) fulltext_document_count: Option<u64>,
    pub(crate) fulltext_projection_digest: Option<SearchProjectionDigest>,
    pub(crate) fulltext_logical_content_digest: Option<ContentDigest>,
    pub(crate) vector_generation: Option<String>,
    pub(crate) vector_mode: Option<String>,
    pub(crate) vector_model_id: Option<String>,
    pub(crate) vector_dimension: Option<u32>,
    pub(crate) vector_projection_count: Option<u64>,
    pub(crate) vector_coverage_digest: Option<SearchProjectionDigest>,
    pub(crate) vector_count: Option<u64>,
    pub(crate) vector_document_count: Option<u64>,
    pub(crate) vector_resume_version_count: Option<u64>,
    pub(crate) vector_projection_digest: Option<SearchProjectionDigest>,
    pub(crate) vector_logical_content_digest: Option<ContentDigest>,
    pub(crate) contract: DescriptorContract,
}

pub(crate) fn publication_descriptor_records(
    connection: &Connection,
) -> Result<Vec<PublicationDescriptorRecord>> {
    let mut statement = connection
        .prepare(
            "SELECT generation, expected_visible_epoch, classifier_epoch,
                    projection_digest, publication_fingerprint, state,
                    fulltext_generation, fulltext_manifest_schema, fulltext_index_schema,
                    fulltext_document_count, fulltext_projection_digest,
                    fulltext_logical_content_digest, vector_generation,
                    vector_manifest_schema, vector_index_schema, vector_mode,
                    vector_model_id, vector_dimension, vector_projection_count,
                    vector_coverage_digest, vector_count, vector_document_count,
                    vector_resume_version_count, vector_projection_digest,
                    vector_logical_content_digest
             FROM search_publication_journal ORDER BY generation",
        )
        .map_err(MetaStoreError::storage)?;
    let mut rows = statement.query([]).map_err(MetaStoreError::storage)?;
    let mut records = Vec::new();
    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        let fulltext_manifest_schema = row
            .get::<_, Option<String>>(7)
            .map_err(MetaStoreError::storage)?;
        let fulltext_index_schema = row
            .get::<_, Option<String>>(8)
            .map_err(MetaStoreError::storage)?;
        let vector_manifest_schema = row
            .get::<_, Option<String>>(13)
            .map_err(MetaStoreError::storage)?;
        let vector_index_schema = row
            .get::<_, Option<String>>(14)
            .map_err(MetaStoreError::storage)?;
        let contract = descriptor_contract(
            fulltext_manifest_schema.as_deref(),
            fulltext_index_schema.as_deref(),
            vector_manifest_schema.as_deref(),
            vector_index_schema.as_deref(),
        )?;
        let record = PublicationDescriptorRecord {
            generation: row.get(0).map_err(MetaStoreError::storage)?,
            expected_visible_epoch: required_u64(row.get(1).map_err(MetaStoreError::storage)?)?,
            classifier_epoch: row.get(2).map_err(MetaStoreError::storage)?,
            projection_digest: parse_projection_digest(
                row.get(3).map_err(MetaStoreError::storage)?,
            )?,
            publication_fingerprint: optional_content_digest(
                row.get(4).map_err(MetaStoreError::storage)?,
            )?,
            state: row.get(5).map_err(MetaStoreError::storage)?,
            fulltext_generation: row.get(6).map_err(MetaStoreError::storage)?,
            fulltext_document_count: optional_u64(row.get(9).map_err(MetaStoreError::storage)?)?,
            fulltext_projection_digest: optional_projection_digest(
                row.get(10).map_err(MetaStoreError::storage)?,
            )?,
            fulltext_logical_content_digest: optional_content_digest(
                row.get(11).map_err(MetaStoreError::storage)?,
            )?,
            vector_generation: row.get(12).map_err(MetaStoreError::storage)?,
            vector_mode: row.get(15).map_err(MetaStoreError::storage)?,
            vector_model_id: row.get(16).map_err(MetaStoreError::storage)?,
            vector_dimension: optional_u32(row.get(17).map_err(MetaStoreError::storage)?)?,
            vector_projection_count: optional_u64(row.get(18).map_err(MetaStoreError::storage)?)?,
            vector_coverage_digest: optional_projection_digest(
                row.get(19).map_err(MetaStoreError::storage)?,
            )?,
            vector_count: optional_u64(row.get(20).map_err(MetaStoreError::storage)?)?,
            vector_document_count: optional_u64(row.get(21).map_err(MetaStoreError::storage)?)?,
            vector_resume_version_count: optional_u64(
                row.get(22).map_err(MetaStoreError::storage)?,
            )?,
            vector_projection_digest: optional_projection_digest(
                row.get(23).map_err(MetaStoreError::storage)?,
            )?,
            vector_logical_content_digest: optional_content_digest(
                row.get(24).map_err(MetaStoreError::storage)?,
            )?,
            contract,
        };
        validate_descriptor_record(connection, &record)?;
        records.push(record);
    }
    Ok(records)
}

pub(crate) fn descriptor_contract(
    fulltext_manifest: Option<&str>,
    fulltext_index: Option<&str>,
    vector_manifest: Option<&str>,
    vector_index: Option<&str>,
) -> Result<DescriptorContract> {
    match (
        fulltext_manifest,
        fulltext_index,
        vector_manifest,
        vector_index,
    ) {
        (None, None, None, None) => Ok(DescriptorContract::None),
        (
            Some(LEGACY_FULLTEXT_MANIFEST),
            Some(LEGACY_FULLTEXT_INDEX),
            Some(LEGACY_VECTOR_MANIFEST),
            Some(LEGACY_VECTOR_INDEX),
        ) => Ok(DescriptorContract::Legacy),
        (
            Some(CURRENT_FULLTEXT_MANIFEST),
            Some(CURRENT_FULLTEXT_INDEX),
            Some(CURRENT_VECTOR_MANIFEST),
            Some(CURRENT_VECTOR_INDEX),
        ) => Ok(DescriptorContract::Current),
        _ => Err(MetaStoreError::storage_invariant()),
    }
}

fn validate_descriptor_record(
    connection: &Connection,
    record: &PublicationDescriptorRecord,
) -> Result<()> {
    if record.contract == DescriptorContract::None {
        if record.publication_fingerprint.is_some()
            || record.fulltext_generation.is_some()
            || record.fulltext_document_count.is_some()
            || record.fulltext_projection_digest.is_some()
            || record.fulltext_logical_content_digest.is_some()
            || record.vector_generation.is_some()
            || record.vector_mode.is_some()
            || record.vector_model_id.is_some()
            || record.vector_dimension.is_some()
            || record.vector_projection_count.is_some()
            || record.vector_coverage_digest.is_some()
            || record.vector_count.is_some()
            || record.vector_document_count.is_some()
            || record.vector_resume_version_count.is_some()
            || record.vector_projection_digest.is_some()
            || record.vector_logical_content_digest.is_some()
            || !matches!(record.state.as_str(), "preparing" | "abandoned")
        {
            return Err(MetaStoreError::storage_invariant());
        }
        return Ok(());
    }
    let fingerprint = record
        .publication_fingerprint
        .as_ref()
        .ok_or_else(MetaStoreError::storage_invariant)?;
    let fulltext_count = record
        .fulltext_document_count
        .ok_or_else(MetaStoreError::storage_invariant)?;
    let vector_projection_count = record
        .vector_projection_count
        .ok_or_else(MetaStoreError::storage_invariant)?;
    let vector_count = record
        .vector_count
        .ok_or_else(MetaStoreError::storage_invariant)?;
    let vector_document_count = record
        .vector_document_count
        .ok_or_else(MetaStoreError::storage_invariant)?;
    let vector_resume_version_count = record
        .vector_resume_version_count
        .ok_or_else(MetaStoreError::storage_invariant)?;
    if record.fulltext_generation.as_deref() != Some(record.generation.as_str())
        || record.vector_generation.as_deref() != Some(record.generation.as_str())
        || record.fulltext_projection_digest.as_ref() != Some(&record.projection_digest)
        || record.vector_projection_digest.as_ref() != Some(&record.projection_digest)
        || fulltext_count != vector_projection_count
        || vector_document_count > vector_projection_count
        || vector_resume_version_count != vector_document_count
        || vector_count < vector_document_count
        || record.state == "preparing"
    {
        return Err(MetaStoreError::storage_invariant());
    }
    let empty_coverage = SearchProjectionDigest::from_pairs::<_, &str, &str>([])
        .map_err(|_| MetaStoreError::storage_invariant())?;
    match record.vector_mode.as_deref() {
        Some("disabled")
            if record.vector_model_id.is_none()
                && record.vector_dimension.is_none()
                && vector_count == 0
                && vector_document_count == 0
                && vector_resume_version_count == 0
                && record.vector_coverage_digest.as_ref() == Some(&empty_coverage) => {}
        Some("enabled")
            if record
                .vector_model_id
                .as_deref()
                .is_some_and(|model| !model.trim().is_empty() && model.chars().count() <= 128)
                && record
                    .vector_dimension
                    .is_some_and(|dimension| dimension != 0) => {}
        _ => return Err(MetaStoreError::storage_invariant()),
    }
    if record.contract == DescriptorContract::Legacy && legacy_fingerprint(record)? != *fingerprint
    {
        return Err(MetaStoreError::storage_invariant());
    }
    if record.contract == DescriptorContract::Current {
        let publication = crate::search_publication::search_publication_in_connection(
            connection,
            &record.generation,
        )?
        .ok_or_else(MetaStoreError::storage_invariant)?;
        let fulltext = publication
            .fulltext
            .as_ref()
            .ok_or_else(MetaStoreError::storage_invariant)?;
        let vector = publication
            .vector
            .as_ref()
            .ok_or_else(MetaStoreError::storage_invariant)?;
        let canonical = crate::search_publication_fingerprint(
            &publication.classifier_epoch,
            &publication.projection_digest,
            fulltext,
            vector,
        );
        if canonical != *fingerprint {
            return Err(MetaStoreError::storage_invariant());
        }
    }
    Ok(())
}

pub(crate) fn legacy_fingerprint(record: &PublicationDescriptorRecord) -> Result<ContentDigest> {
    let mut canonical = Vec::new();
    append_part(&mut canonical, b"resume-ir.search-publication.v1");
    append_part(&mut canonical, record.classifier_epoch.as_bytes());
    append_part(&mut canonical, record.projection_digest.as_str().as_bytes());
    append_part(&mut canonical, LEGACY_FULLTEXT_MANIFEST.as_bytes());
    append_part(&mut canonical, LEGACY_FULLTEXT_INDEX.as_bytes());
    append_part(
        &mut canonical,
        &record
            .fulltext_document_count
            .ok_or_else(MetaStoreError::storage_invariant)?
            .to_le_bytes(),
    );
    append_part(
        &mut canonical,
        record
            .fulltext_projection_digest
            .as_ref()
            .ok_or_else(MetaStoreError::storage_invariant)?
            .as_str()
            .as_bytes(),
    );
    append_part(
        &mut canonical,
        record
            .fulltext_logical_content_digest
            .as_ref()
            .ok_or_else(MetaStoreError::storage_invariant)?
            .as_str()
            .as_bytes(),
    );
    append_part(&mut canonical, LEGACY_VECTOR_MANIFEST.as_bytes());
    append_part(&mut canonical, LEGACY_VECTOR_INDEX.as_bytes());
    append_part(
        &mut canonical,
        &record
            .vector_projection_count
            .ok_or_else(MetaStoreError::storage_invariant)?
            .to_le_bytes(),
    );
    append_part(
        &mut canonical,
        record
            .vector_projection_digest
            .as_ref()
            .ok_or_else(MetaStoreError::storage_invariant)?
            .as_str()
            .as_bytes(),
    );
    append_part(
        &mut canonical,
        record
            .vector_coverage_digest
            .as_ref()
            .ok_or_else(MetaStoreError::storage_invariant)?
            .as_str()
            .as_bytes(),
    );
    append_part(
        &mut canonical,
        &record
            .vector_count
            .ok_or_else(MetaStoreError::storage_invariant)?
            .to_le_bytes(),
    );
    append_part(
        &mut canonical,
        &record
            .vector_document_count
            .ok_or_else(MetaStoreError::storage_invariant)?
            .to_le_bytes(),
    );
    append_part(
        &mut canonical,
        &record
            .vector_resume_version_count
            .ok_or_else(MetaStoreError::storage_invariant)?
            .to_le_bytes(),
    );
    append_part(
        &mut canonical,
        record
            .vector_logical_content_digest
            .as_ref()
            .ok_or_else(MetaStoreError::storage_invariant)?
            .as_str()
            .as_bytes(),
    );
    match record.vector_mode.as_deref() {
        Some("disabled")
            if record.vector_model_id.is_none() && record.vector_dimension.is_none() =>
        {
            append_part(&mut canonical, b"disabled");
        }
        Some("enabled") => {
            let model_id = record
                .vector_model_id
                .as_deref()
                .ok_or_else(MetaStoreError::storage_invariant)?;
            let dimension = record
                .vector_dimension
                .ok_or_else(MetaStoreError::storage_invariant)?;
            append_part(&mut canonical, b"enabled");
            append_part(&mut canonical, model_id.as_bytes());
            append_part(&mut canonical, &dimension.to_le_bytes());
        }
        _ => return Err(MetaStoreError::storage_invariant()),
    }
    Ok(ContentDigest::from_bytes(&canonical))
}

fn append_part(target: &mut Vec<u8>, part: &[u8]) {
    target.extend_from_slice(&(part.len() as u64).to_le_bytes());
    target.extend_from_slice(part);
}

pub(crate) fn required_u64(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| MetaStoreError::storage_invariant())
}

fn optional_u64(value: Option<i64>) -> Result<Option<u64>> {
    value.map(required_u64).transpose()
}

fn optional_u32(value: Option<i64>) -> Result<Option<u32>> {
    value
        .map(|value| u32::try_from(value).map_err(|_| MetaStoreError::storage_invariant()))
        .transpose()
}

pub(crate) fn parse_projection_digest(value: String) -> Result<SearchProjectionDigest> {
    SearchProjectionDigest::from_str(&value).map_err(|_| MetaStoreError::storage_invariant())
}

fn optional_projection_digest(value: Option<String>) -> Result<Option<SearchProjectionDigest>> {
    value.map(parse_projection_digest).transpose()
}

fn optional_content_digest(value: Option<String>) -> Result<Option<ContentDigest>> {
    value
        .map(|value| {
            ContentDigest::from_str(&value).map_err(|_| MetaStoreError::storage_invariant())
        })
        .transpose()
}
