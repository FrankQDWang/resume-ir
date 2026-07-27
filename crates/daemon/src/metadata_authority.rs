use std::path::Path;

use serde::Serialize;

use crate::daemon_error::{DaemonError, Result};

const RECEIPT_SCHEMA: &str = "resume-ir.metadata-authority.v1";
const MAX_RECEIPT_BYTES: usize = 4 * 1024;

#[derive(Serialize)]
struct MetadataAuthorityReceipt {
    schema_version: &'static str,
    generation: String,
    visible_epoch: u64,
    service_state: &'static str,
    publication_state: &'static str,
    projection_digest: String,
    fulltext_generation: String,
    fulltext_manifest_schema: &'static str,
    fulltext_index_schema: &'static str,
    fulltext_document_count: u64,
    fulltext_projection_digest: String,
    fulltext_logical_content_digest: String,
    vector_generation: String,
    vector_manifest_schema: &'static str,
    vector_index_schema: &'static str,
    vector_mode: &'static str,
    vector_model_id: Option<String>,
    vector_dimension: Option<u32>,
    vector_projection_count: u64,
    vector_coverage_digest: String,
    vector_count: u64,
    vector_document_count: u64,
    vector_projection_digest: String,
    vector_logical_content_digest: String,
}

pub(crate) fn run(data_dir: &Path) -> Result<()> {
    let authority =
        meta_store::inspect_metadata_logical_authority(data_dir).map_err(DaemonError::store)?;
    let output = render(authority)?;
    println!("{output}");
    Ok(())
}

fn render(authority: meta_store::MetadataLogicalAuthority) -> Result<String> {
    let receipt = MetadataAuthorityReceipt {
        schema_version: RECEIPT_SCHEMA,
        generation: authority.generation,
        visible_epoch: authority.visible_epoch,
        service_state: "ready",
        publication_state: "ready",
        projection_digest: authority.projection_digest,
        fulltext_generation: authority.fulltext_generation,
        fulltext_manifest_schema: meta_store::FULLTEXT_MANIFEST_SCHEMA_V3,
        fulltext_index_schema: meta_store::FULLTEXT_INDEX_SCHEMA_V3,
        fulltext_document_count: authority.fulltext_document_count,
        fulltext_projection_digest: authority.fulltext_projection_digest,
        fulltext_logical_content_digest: authority.fulltext_logical_content_digest,
        vector_generation: authority.vector_generation,
        vector_manifest_schema: meta_store::VECTOR_MANIFEST_SCHEMA_V4,
        vector_index_schema: meta_store::VECTOR_INDEX_SCHEMA_V4,
        vector_mode: authority.vector_mode,
        vector_model_id: authority.vector_model_id,
        vector_dimension: authority.vector_dimension,
        vector_projection_count: authority.vector_projection_count,
        vector_coverage_digest: authority.vector_coverage_digest,
        vector_count: authority.vector_count,
        vector_document_count: authority.vector_document_count,
        vector_projection_digest: authority.vector_projection_digest,
        vector_logical_content_digest: authority.vector_logical_content_digest,
    };
    let output = serde_json::to_string(&receipt).map_err(|_| DaemonError::runtime_integrity())?;
    if output.len() > MAX_RECEIPT_BYTES {
        return Err(DaemonError::runtime_integrity());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_is_bounded_closed_json_without_storage_authority() {
        let digest = format!("sha256:{}", "1".repeat(64));
        let output = render(meta_store::MetadataLogicalAuthority {
            generation: "synthetic-generation".to_string(),
            visible_epoch: 1,
            projection_digest: digest.clone(),
            fulltext_generation: "synthetic-generation".to_string(),
            fulltext_document_count: 0,
            fulltext_projection_digest: digest.clone(),
            fulltext_logical_content_digest: digest.clone(),
            vector_generation: "synthetic-generation".to_string(),
            vector_mode: "disabled",
            vector_model_id: None,
            vector_dimension: None,
            vector_projection_count: 0,
            vector_coverage_digest: digest.clone(),
            vector_count: 0,
            vector_document_count: 0,
            vector_projection_digest: digest.clone(),
            vector_logical_content_digest: digest,
        })
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema_version"], "resume-ir.metadata-authority.v1");
        assert_eq!(value["service_state"], "ready");
        assert_eq!(value["publication_state"], "ready");
        assert_eq!(value.as_object().unwrap().len(), 24);
        assert!(!output.contains("metadata-secrets"));
        assert!(!output.contains("sqlite3"));
        assert!(output.len() <= MAX_RECEIPT_BYTES);
    }
}
