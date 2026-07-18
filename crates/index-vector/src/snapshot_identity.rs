use std::collections::BTreeSet;

use core_domain::{ActiveSearchProjection, ContentDigest, SearchProjectionDigest};

use crate::model::{VectorDocument, VectorIndexError};
use crate::model_contract::VectorModelContract;
use crate::snapshot_model::{projection_digest, validate_projection};

pub(crate) fn canonical_projection(
    projection: &[ActiveSearchProjection],
) -> Result<Vec<ActiveSearchProjection>, VectorIndexError> {
    validate_projection(projection)?;
    let mut projection = projection.to_vec();
    projection.sort_unstable_by(|left, right| {
        left.document_id
            .cmp(&right.document_id)
            .then_with(|| left.resume_version_id.cmp(&right.resume_version_id))
    });
    Ok(projection)
}

pub(crate) fn logical_content_digest(
    model_contract: &VectorModelContract,
    projection: &[ActiveSearchProjection],
    documents: &[VectorDocument],
) -> Result<ContentDigest, VectorIndexError> {
    let projection_digest = projection_digest(projection)?;
    let mut canonical = Vec::with_capacity(256 + documents.len() * 71);
    canonical.extend_from_slice(b"resume-ir.vector.logical-content.v3");
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

    let mut documents = documents.iter().collect::<Vec<_>>();
    documents.sort_unstable_by(|left, right| left.vector_id().cmp(right.vector_id()));
    canonical.extend_from_slice(&(documents.len() as u64).to_le_bytes());
    for document in documents {
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
        for value in document.values() {
            vector.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        append_value_digest(&mut canonical, &vector);
    }
    Ok(ContentDigest::from_bytes(&canonical))
}

pub(crate) fn coverage_digest(
    documents: &[VectorDocument],
) -> Result<SearchProjectionDigest, VectorIndexError> {
    let pairs = documents
        .iter()
        .map(|document| (document.document_id(), document.resume_version_id()))
        .collect::<BTreeSet<_>>();
    SearchProjectionDigest::from_pairs(pairs)
        .map_err(|_| VectorIndexError::PublicationProjectionMismatch)
}

fn append_value_digest(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(ContentDigest::from_bytes(value).as_str().as_bytes());
}
