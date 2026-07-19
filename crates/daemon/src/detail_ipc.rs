use std::str::FromStr;

use meta_store::{
    EntityMention, EntityType, ReadMetaStore, SearchMetadataReadError, SearchMetadataUnavailable,
    SearchSelection, SearchSelectionDetailBundle, SearchSelectionDetailResolution,
    SearchTextBytePageRequest,
};
use privacy::redact_contact_values;
use serde::Deserialize;

pub(crate) const REQUEST_SCHEMA_VERSION: &str = "resume-ir.detail-request.v3";
pub(crate) const RESPONSE_SCHEMA_VERSION: &str = "resume-ir.detail-response.v3";
pub(crate) const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const REQUEST_ID_MAX_BYTES: usize = 128;
const DETAIL_FIELD_LIMIT: usize = 256;

#[derive(Debug)]
pub(crate) enum DetailError {
    BadRequest,
    StaleSelection,
    NotFound,
    ResponseTooLarge,
    Repairing,
    QueryServiceUnavailable,
    MetadataUnavailable,
}

pub(crate) fn execute(store: &ReadMetaStore, body: &[u8]) -> Result<String, DetailError> {
    let context = DetailRequest::parse(body)?;
    let snippet_request = SearchTextBytePageRequest::new(context.selection.clone(), 0, 240)
        .map_err(|_| DetailError::BadRequest)?;
    let bundle = match store
        .search_selection_detail(&snippet_request)
        .map_err(map_read_error)?
    {
        SearchSelectionDetailResolution::Current(bundle) => bundle,
        SearchSelectionDetailResolution::Stale => return Err(DetailError::StaleSelection),
        SearchSelectionDetailResolution::NotFound => return Err(DetailError::NotFound),
        SearchSelectionDetailResolution::InvalidOffset => return Err(DetailError::BadRequest),
        SearchSelectionDetailResolution::LimitExceeded(_) => {
            return Err(DetailError::ResponseTooLarge);
        }
    };
    encode_response(context, *bundle)
}

pub(crate) fn request_id(body: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<serde_json::Value>(body).ok()?;
    let request_id = value.get("request_id")?.as_str()?;
    valid_request_id(request_id).then(|| request_id.to_string())
}

fn encode_response(
    context: RequestContext,
    bundle: SearchSelectionDetailBundle,
) -> Result<String, DetailError> {
    let details = bundle.details;
    let snippet = redact_short_text(&bundle.text_page.text, 240);
    let field_count_total = details.mentions.len();
    let fields = details
        .mentions
        .iter()
        .take(DETAIL_FIELD_LIMIT)
        .map(detail_field)
        .collect::<Vec<_>>();
    let field_count_returned = fields.len();
    let response = serde_json::json!({
        "schema_version": RESPONSE_SCHEMA_VERSION,
        "request_id": context.request_id,
        "selection": selection_json(&context.selection),
        "status": "ok",
        "document": {
            "source_byte_size": details.version.source_byte_size,
            "parse_version": details.version.parse_version,
            "schema_version": details.version.schema_version,
            "language_set": details.version.language_set,
            "page_count": details.version.page_count,
            "quality_score": details.version.quality_score,
            "field_limit": DETAIL_FIELD_LIMIT,
            "field_count_total": field_count_total,
            "field_count_returned": field_count_returned,
            "fields_truncated": field_count_returned < field_count_total,
            "fields": fields,
            "snippet": snippet,
        },
        "limits": {
            "max_fields": DETAIL_FIELD_LIMIT,
            "max_response_bytes": MAX_RESPONSE_BYTES,
        }
    })
    .to_string();
    if response.len() > MAX_RESPONSE_BYTES {
        return Err(DetailError::ResponseTooLarge);
    }
    Ok(response)
}

pub(crate) fn map_read_error(error: SearchMetadataReadError) -> DetailError {
    match error {
        SearchMetadataReadError::Unavailable(SearchMetadataUnavailable::Repairing(_)) => {
            DetailError::Repairing
        }
        SearchMetadataReadError::Unavailable(SearchMetadataUnavailable::RepairBlocked(_)) => {
            DetailError::QueryServiceUnavailable
        }
        SearchMetadataReadError::Store(_) => DetailError::MetadataUnavailable,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DetailRequest {
    schema_version: String,
    request_id: String,
    selection: WireSearchSelection,
}

impl DetailRequest {
    fn parse(body: &[u8]) -> Result<RequestContext, DetailError> {
        let wire: Self = serde_json::from_slice(body).map_err(|_| DetailError::BadRequest)?;
        if wire.schema_version != REQUEST_SCHEMA_VERSION {
            return Err(DetailError::BadRequest);
        }
        request_context(wire.request_id, wire.selection)
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireSearchSelection {
    doc_id: String,
    version_id: String,
    visible_epoch: u64,
}

#[derive(Clone)]
pub(crate) struct RequestContext {
    pub(crate) request_id: String,
    pub(crate) selection: SearchSelection,
}

pub(crate) fn request_context(
    request_id: String,
    selection: WireSearchSelection,
) -> Result<RequestContext, DetailError> {
    if !valid_request_id(&request_id)
        || selection.visible_epoch == 0
        || selection.visible_epoch > i64::MAX as u64
    {
        return Err(DetailError::BadRequest);
    }
    let document_id =
        meta_store::DocumentId::from_str(&selection.doc_id).map_err(|_| DetailError::BadRequest)?;
    let resume_version_id = meta_store::ResumeVersionId::from_str(&selection.version_id)
        .map_err(|_| DetailError::BadRequest)?;
    Ok(RequestContext {
        request_id,
        selection: SearchSelection {
            document_id,
            resume_version_id,
            visible_epoch: selection.visible_epoch,
        },
    })
}

pub(crate) fn selection_json(selection: &SearchSelection) -> serde_json::Value {
    serde_json::json!({
        "doc_id": selection.document_id.as_str(),
        "version_id": selection.resume_version_id.as_str(),
        "visible_epoch": selection.visible_epoch,
    })
}

fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= REQUEST_ID_MAX_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn detail_field(mention: &EntityMention) -> serde_json::Value {
    let value = mention
        .normalized_value
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&mention.raw_value);
    serde_json::json!({
        "type": entity_type_label(&mention.entity_type),
        "value": redact_short_text(value, 120),
        "confidence": f64::from(mention.confidence.clamp(0.0, 1.0)),
        "evidence": redact_short_text(&mention.raw_value, 120),
        "extractor": redact_short_text(&mention.extractor, 80),
    })
}

fn redact_short_text(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&redact_contact_values(&compact), max_chars)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index == max_chars {
            output.push_str("...");
            break;
        }
        output.push(ch);
    }
    output
}

fn entity_type_label(entity_type: &EntityType) -> String {
    match entity_type {
        EntityType::Name => "name".to_string(),
        EntityType::Email => "email".to_string(),
        EntityType::Phone => "phone".to_string(),
        EntityType::WeChat => "wechat".to_string(),
        EntityType::School => "school".to_string(),
        EntityType::SchoolTier => "school_tier".to_string(),
        EntityType::Degree => "degree".to_string(),
        EntityType::Major => "major".to_string(),
        EntityType::Company => "company".to_string(),
        EntityType::Title => "title".to_string(),
        EntityType::Education => "education".to_string(),
        EntityType::Skills => "skills".to_string(),
        EntityType::Skill => "skill".to_string(),
        EntityType::Certificate => "certificate".to_string(),
        EntityType::Date => "date".to_string(),
        EntityType::DateRange => "date_range".to_string(),
        EntityType::YearsExperience => "years_experience".to_string(),
        EntityType::Location => "location".to_string(),
        EntityType::Other(_) => "other".to_string(),
    }
}
