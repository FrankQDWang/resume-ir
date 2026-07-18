use meta_store::{DocumentId, ResumeVersionId, SearchSelection};

use super::{DetailHydrateError, DetailHydrateRequest, MAX_BODY_PAGE_BYTES};

fn selection() -> SearchSelection {
    SearchSelection {
        document_id: DocumentId::from_non_secret_parts(&["s807", "hydrate-request"]),
        resume_version_id: ResumeVersionId::from_non_secret_parts(&["s807", "hydrate-version"]),
        visible_epoch: 7,
    }
}

fn request(selection: &SearchSelection) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "resume-ir.detail-hydrate-request.v3",
        "request_id": "hydrate-request-1",
        "selection": {
            "doc_id": selection.document_id.as_str(),
            "version_id": selection.resume_version_id.as_str(),
            "visible_epoch": selection.visible_epoch,
        },
        "body_offset_bytes": 0,
        "body_limit_bytes": 4,
    })
}

#[test]
fn request_requires_v3_context_and_rejects_unknown_or_unbounded_input() {
    let selection = selection();
    let valid = request(&selection);
    let parsed = DetailHydrateRequest::parse(&valid.to_string().into_bytes()).unwrap();
    assert_eq!(parsed.context.selection, selection);
    assert_eq!(parsed.context.request_id, "hydrate-request-1");

    let mut wrong_schema = valid.clone();
    wrong_schema["schema_version"] = serde_json::json!("unexpected-schema");
    assert!(matches!(
        DetailHydrateRequest::parse(&wrong_schema.to_string().into_bytes()),
        Err(DetailHydrateError::BadRequest)
    ));

    let mut unknown = valid.clone();
    unknown["legacy_version_id"] = serde_json::json!(selection.resume_version_id.as_str());
    assert!(matches!(
        DetailHydrateRequest::parse(&unknown.to_string().into_bytes()),
        Err(DetailHydrateError::BadRequest)
    ));

    let mut oversized = valid;
    oversized["body_limit_bytes"] = serde_json::json!(MAX_BODY_PAGE_BYTES + 1);
    assert!(matches!(
        DetailHydrateRequest::parse(&oversized.to_string().into_bytes()),
        Err(DetailHydrateError::ResponseTooLarge)
    ));
}
