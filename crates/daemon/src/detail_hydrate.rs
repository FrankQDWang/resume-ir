use meta_store::{
    MetaStore, SearchTextBytePage, SearchTextBytePageRequest, SearchTextBytePageResolution,
};
use serde::Deserialize;

use crate::detail_ipc::{
    map_read_error, request_context, selection_json, DetailError, RequestContext,
    WireSearchSelection,
};

pub(crate) use crate::detail_ipc::DetailError as DetailHydrateError;

pub(crate) const REQUEST_SCHEMA_VERSION: &str = "resume-ir.detail-hydrate-request.v3";
pub(crate) const RESPONSE_SCHEMA_VERSION: &str = "resume-ir.detail-hydrate-response.v3";
pub(crate) const MAX_BODY_PAGE_BYTES: usize = 32 * 1024;
pub(crate) const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MIN_BODY_PAGE_BYTES: usize = 4;

pub(crate) fn execute(store: &MetaStore, body: &[u8]) -> Result<String, DetailHydrateError> {
    let request = DetailHydrateRequest::parse(body)?;
    let page_request = SearchTextBytePageRequest::new(
        request.context.selection.clone(),
        request.body_offset_bytes,
        request.body_limit_bytes,
    )
    .map_err(|_| DetailError::BadRequest)?;
    let page = store
        .search_text_byte_page(&page_request)
        .map_err(map_read_error)?;
    let page = match page {
        SearchTextBytePageResolution::Current(page) => page,
        SearchTextBytePageResolution::Stale => return Err(DetailError::StaleSelection),
        SearchTextBytePageResolution::NotFound => return Err(DetailError::NotFound),
        SearchTextBytePageResolution::InvalidOffset => return Err(DetailError::BadRequest),
    };
    encode_response(request.context, page)
}

fn encode_response(
    context: RequestContext,
    page: SearchTextBytePage,
) -> Result<String, DetailHydrateError> {
    let response = serde_json::json!({
        "schema_version": RESPONSE_SCHEMA_VERSION,
        "request_id": context.request_id,
        "selection": selection_json(&context.selection),
        "status": "ok",
        "document": {
            "body_page": {
                "encoding": "utf-8",
                "offset_bytes": page.offset_bytes,
                "next_offset_bytes": page.next_offset_bytes,
                "total_bytes": page.total_bytes,
                "complete": page.next_offset_bytes == page.total_bytes,
                "text": page.text,
            }
        },
        "privacy": {
            "local_authenticated_only": true,
            "public_output_allowed": false,
        },
        "limits": {
            "max_body_page_bytes": MAX_BODY_PAGE_BYTES,
            "max_response_bytes": MAX_RESPONSE_BYTES,
        }
    })
    .to_string();
    if response.len() > MAX_RESPONSE_BYTES {
        return Err(DetailError::ResponseTooLarge);
    }
    Ok(response)
}

pub(crate) struct DetailHydrateRequest {
    pub(crate) context: RequestContext,
    body_offset_bytes: u64,
    body_limit_bytes: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireDetailHydrateRequest {
    schema_version: String,
    request_id: String,
    selection: WireSearchSelection,
    body_offset_bytes: u64,
    body_limit_bytes: u32,
}

impl DetailHydrateRequest {
    pub(crate) fn parse(body: &[u8]) -> Result<Self, DetailHydrateError> {
        let wire: WireDetailHydrateRequest =
            serde_json::from_slice(body).map_err(|_| DetailError::BadRequest)?;
        if wire.schema_version != REQUEST_SCHEMA_VERSION
            || wire.body_limit_bytes
                < u32::try_from(MIN_BODY_PAGE_BYTES).map_err(|_| DetailError::BadRequest)?
        {
            return Err(DetailError::BadRequest);
        }
        if wire.body_limit_bytes
            > u32::try_from(MAX_BODY_PAGE_BYTES).map_err(|_| DetailError::BadRequest)?
        {
            return Err(DetailError::ResponseTooLarge);
        }
        Ok(Self {
            context: request_context(wire.request_id, wire.selection)?,
            body_offset_bytes: wire.body_offset_bytes,
            body_limit_bytes: wire.body_limit_bytes,
        })
    }
}

#[cfg(test)]
#[path = "detail_hydrate_tests.rs"]
mod tests;
