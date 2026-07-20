use std::collections::BTreeSet;

use core_domain::{ActiveSearchProjection, ContentDigest, SearchProjectionDigest};

use crate::model::{VectorDocument, VectorIndexError};
use crate::model_contract::VectorModelContract;
use crate::publish_control::VectorSnapshotPublishControl;
use crate::snapshot_model::{projection_digest_with_control, validate_projection_with_control};

pub(crate) fn canonical_projection_with_control(
    projection: &[ActiveSearchProjection],
    control: VectorSnapshotPublishControl<'_>,
) -> Result<Vec<ActiveSearchProjection>, VectorIndexError> {
    validate_projection_with_control(projection, control)?;
    let mut canonical = Vec::with_capacity(projection.len());
    for (index, entry) in projection.iter().enumerate() {
        canonical.push(entry.clone());
        control.check_after_record(index + 1)?;
    }
    control.check()?;
    canonical.sort_unstable_by(|left, right| {
        left.document_id
            .cmp(&right.document_id)
            .then_with(|| left.resume_version_id.cmp(&right.resume_version_id))
    });
    control.check()?;
    Ok(canonical)
}

pub(crate) fn logical_content_digest_with_control(
    model_contract: &VectorModelContract,
    projection: &[ActiveSearchProjection],
    documents: &[VectorDocument],
    control: VectorSnapshotPublishControl<'_>,
) -> Result<ContentDigest, VectorIndexError> {
    control.check()?;
    let projection_digest = projection_digest_with_control(projection, control)?;
    let mut canonical = Vec::with_capacity(256 + documents.len() * 71);
    canonical.extend_from_slice(b"resume-ir.vector.logical-content.v4");
    match model_contract {
        VectorModelContract::Disabled => canonical.push(0),
        VectorModelContract::Enabled {
            model_id,
            dimension,
        } => {
            canonical.push(1);
            append_value_digest(&mut canonical, model_id.as_bytes());
            canonical.extend_from_slice(&(*dimension as u64).to_le_bytes());
        }
    }
    canonical.extend_from_slice(projection_digest.as_str().as_bytes());

    let mut sorted_documents = Vec::with_capacity(documents.len());
    for (index, document) in documents.iter().enumerate() {
        sorted_documents.push(document);
        control.check_after_record(index + 1)?;
    }
    control.check()?;
    sorted_documents.sort_unstable_by(|left, right| left.vector_id().cmp(right.vector_id()));
    control.check()?;
    canonical.extend_from_slice(&(sorted_documents.len() as u64).to_le_bytes());
    for (document_index, document) in sorted_documents.into_iter().enumerate() {
        let mut vector = Vec::with_capacity(4 * 71 + 8 + document.values().len() * 4);
        for identity in [
            document.vector_id(),
            document.document_id(),
            document.resume_version_id(),
            document.model_id(),
        ] {
            append_value_digest(&mut vector, identity.as_bytes());
        }
        vector.extend_from_slice(&(document.values().len() as u64).to_le_bytes());
        for (value_index, value) in document.values().iter().enumerate() {
            vector.extend_from_slice(&value.to_bits().to_le_bytes());
            control.check_after_record(value_index + 1)?;
        }
        append_value_digest(&mut canonical, &vector);
        control.check_after_record(document_index + 1)?;
    }
    control.check()?;
    Ok(ContentDigest::from_bytes(&canonical))
}

pub(crate) fn coverage_digest_with_control(
    documents: &[VectorDocument],
    control: VectorSnapshotPublishControl<'_>,
) -> Result<SearchProjectionDigest, VectorIndexError> {
    control.check()?;
    let mut pairs = BTreeSet::new();
    for (index, document) in documents.iter().enumerate() {
        pairs.insert((document.document_id(), document.resume_version_id()));
        control.check_after_record(index + 1)?;
    }
    let digest = SearchProjectionDigest::from_pairs(pairs)
        .map_err(|_| VectorIndexError::PublicationProjectionMismatch)?;
    control.check()?;
    Ok(digest)
}

fn append_value_digest(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(ContentDigest::from_bytes(value).as_str().as_bytes());
}
