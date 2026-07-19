use std::str::FromStr;

use import_pipeline::{
    SearchProjectionRemoval, SearchProjectionRemovalReason, SearchPublicationVectorization,
};
use meta_store::{DocumentId, DocumentStatus, OwnedMetaStore};

use crate::command_failure::CommandFailure;

pub(crate) fn execute(store: &OwnedMetaStore, body: &[u8]) -> Result<String, CommandFailure> {
    let payload = serde_json::from_slice::<serde_json::Value>(body)
        .map_err(|_| CommandFailure::BadRequest("invalid json"))?;
    let document_id = parse(&payload)?;
    let now = crate::current_timestamp().map_err(|_| CommandFailure::Internal)?;
    let Some(document) = store
        .document_by_id(&document_id)
        .map_err(|_| CommandFailure::Internal)?
    else {
        return Err(CommandFailure::NotFound("delete document was not found"));
    };
    if document.is_deleted || document.status == DocumentStatus::Deleted {
        return Err(CommandFailure::NotFound("delete document was not found"));
    }
    let publication = import_pipeline::publish_search_projection_removals(
        store,
        &[SearchProjectionRemoval {
            document_id: document_id.clone(),
            reason: SearchProjectionRemovalReason::ConfirmedSourceDeletion,
        }],
        now,
        &SearchPublicationVectorization::default(),
    )
    .map_err(|_| CommandFailure::Internal)?;
    Ok(serde_json::json!({
        "schema_version": "resume-ir.delete-response.v2",
        "status": "ok",
        "doc_id": document_id.as_str(),
        "publication_committed": true,
        "indexed_documents": publication.active_projection_count,
    })
    .to_string())
}

fn parse(payload: &serde_json::Value) -> Result<DocumentId, CommandFailure> {
    let value = payload
        .get("doc_id")
        .and_then(serde_json::Value::as_str)
        .ok_or(CommandFailure::BadRequest("doc_id is required"))?;
    DocumentId::from_str(value).map_err(|_| CommandFailure::BadRequest("doc_id is invalid"))
}
