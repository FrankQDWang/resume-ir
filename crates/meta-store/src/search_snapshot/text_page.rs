use std::fmt;

use rusqlite::{params, OptionalExtension};

use super::SearchMetadataSnapshot;
use crate::{MetaStoreError, Result, SearchSelection, SearchSelectionResolution};

pub const MAX_SEARCH_TEXT_PAGE_CODE_POINTS: u32 = 8_192;
pub const MAX_SEARCH_TEXT_BYTE_PAGE_BYTES: u32 = 32 * 1024;
const MIN_SEARCH_TEXT_BYTE_PAGE_BYTES: u32 = 4;
const UTF8_MAX_TRAILING_BYTES: u32 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchTextBytePageRequest {
    selection: SearchSelection,
    offset_bytes: u64,
    limit_bytes: u32,
}

impl SearchTextBytePageRequest {
    pub fn new(
        selection: SearchSelection,
        offset_bytes: u64,
        limit_bytes: u32,
    ) -> std::result::Result<Self, SearchTextPageRequestError> {
        if !(MIN_SEARCH_TEXT_BYTE_PAGE_BYTES..=MAX_SEARCH_TEXT_BYTE_PAGE_BYTES)
            .contains(&limit_bytes)
        {
            return Err(SearchTextPageRequestError::InvalidLimit);
        }
        if offset_bytes > i64::MAX as u64 {
            return Err(SearchTextPageRequestError::InvalidOffset);
        }
        Ok(Self {
            selection,
            offset_bytes,
            limit_bytes,
        })
    }

    pub fn selection(&self) -> &SearchSelection {
        &self.selection
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchTextBytePage {
    pub selection: SearchSelection,
    pub offset_bytes: u64,
    pub next_offset_bytes: u64,
    pub total_bytes: u64,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchTextBytePageResolution {
    Current(SearchTextBytePage),
    Stale,
    NotFound,
    InvalidOffset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchTextPageRequest {
    selection: SearchSelection,
    cursor: SearchTextPageCursor,
    limit_code_points: u32,
}

impl SearchTextPageRequest {
    pub fn new(
        selection: SearchSelection,
        cursor: Option<SearchTextPageCursor>,
        limit_code_points: u32,
    ) -> std::result::Result<Self, SearchTextPageRequestError> {
        if !(1..=MAX_SEARCH_TEXT_PAGE_CODE_POINTS).contains(&limit_code_points) {
            return Err(SearchTextPageRequestError::InvalidLimit);
        }
        Ok(Self {
            selection,
            cursor: cursor.unwrap_or_else(SearchTextPageCursor::start),
            limit_code_points,
        })
    }

    pub fn selection(&self) -> &SearchSelection {
        &self.selection
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchTextPageRequestError {
    InvalidLimit,
    InvalidOffset,
}

/// Opaque continuation cursor. Internally this is a Unicode code-point offset
/// matching SQLite TEXT `substr`/`length`; clients must only echo the token.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SearchTextPageCursor {
    code_point_offset: u64,
}

impl SearchTextPageCursor {
    fn start() -> Self {
        Self {
            code_point_offset: 0,
        }
    }

    fn for_offset(code_point_offset: u64) -> Self {
        Self { code_point_offset }
    }

    pub fn from_opaque_token(token: &str) -> std::result::Result<Self, SearchTextPageCursorError> {
        let encoded = token
            .strip_prefix("cp1:")
            .filter(|encoded| encoded.len() == 16)
            .ok_or(SearchTextPageCursorError::Invalid)?;
        let code_point_offset =
            u64::from_str_radix(encoded, 16).map_err(|_| SearchTextPageCursorError::Invalid)?;
        if code_point_offset > i64::MAX as u64 {
            return Err(SearchTextPageCursorError::Invalid);
        }
        Ok(Self { code_point_offset })
    }

    pub fn to_opaque_token(self) -> String {
        format!("cp1:{:016x}", self.code_point_offset)
    }
}

impl fmt::Debug for SearchTextPageCursor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SearchTextPageCursor(<redacted>)")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchTextPageCursorError {
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchTextPage {
    pub selection: SearchSelection,
    pub cursor: SearchTextPageCursor,
    pub next_cursor: Option<SearchTextPageCursor>,
    pub total_code_points: u64,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchTextPageResolution {
    Current(SearchTextPage),
    Stale,
    NotFound,
    InvalidOffset,
}

impl SearchMetadataSnapshot<'_> {
    pub fn clean_text_byte_page(
        &self,
        request: &SearchTextBytePageRequest,
    ) -> Result<SearchTextBytePageResolution> {
        let selection = match self.resolve_search_selection(request.selection())? {
            SearchSelectionResolution::Current { selection } => selection,
            SearchSelectionResolution::Stale => return Ok(SearchTextBytePageResolution::Stale),
            SearchSelectionResolution::NotFound => {
                return Ok(SearchTextBytePageResolution::NotFound);
            }
        };
        let start = i64::try_from(request.offset_bytes)
            .ok()
            .and_then(|offset| offset.checked_add(1))
            .ok_or_else(|| MetaStoreError::invalid_value("search_text_page.offset_bytes"))?;
        let read_limit = request
            .limit_bytes
            .checked_add(UTF8_MAX_TRAILING_BYTES)
            .ok_or_else(MetaStoreError::storage_invariant)?;
        let (total, bytes) = self
            .connection
            .query_row(
                "SELECT length(CAST(clean_text AS BLOB)),
                        substr(CAST(clean_text AS BLOB), ?3, ?4)
                 FROM resume_version
                 WHERE id = ?1 AND document_id = ?2 AND clean_text IS NOT NULL",
                params![
                    selection.resume_version_id.as_str(),
                    selection.document_id.as_str(),
                    start,
                    i64::from(read_limit),
                ],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .optional()
            .map_err(MetaStoreError::storage)?
            .ok_or_else(MetaStoreError::storage_invariant)?;
        let total_bytes = u64::try_from(total)
            .map_err(|_| MetaStoreError::invalid_value("resume_version.clean_text"))?;
        if request.offset_bytes > total_bytes {
            return Ok(SearchTextBytePageResolution::InvalidOffset);
        }
        let valid_bytes = match std::str::from_utf8(&bytes) {
            Ok(_) => bytes.len(),
            Err(error) if error.error_len().is_none() && error.valid_up_to() > 0 => {
                error.valid_up_to()
            }
            Err(_) => return Ok(SearchTextBytePageResolution::InvalidOffset),
        };
        let valid = std::str::from_utf8(&bytes[..valid_bytes])
            .map_err(|_| MetaStoreError::storage_invariant())?;
        let mut page_bytes = usize::try_from(request.limit_bytes)
            .map_err(|_| MetaStoreError::storage_invariant())?
            .min(valid.len());
        while page_bytes > 0 && !valid.is_char_boundary(page_bytes) {
            page_bytes -= 1;
        }
        let text = valid[..page_bytes].to_string();
        let next_offset_bytes = request
            .offset_bytes
            .checked_add(
                u64::try_from(page_bytes).map_err(|_| MetaStoreError::storage_invariant())?,
            )
            .ok_or_else(MetaStoreError::storage_invariant)?;
        Ok(SearchTextBytePageResolution::Current(SearchTextBytePage {
            selection,
            offset_bytes: request.offset_bytes,
            next_offset_bytes,
            total_bytes,
            text,
        }))
    }

    pub fn clean_text_page(
        &self,
        request: &SearchTextPageRequest,
    ) -> Result<SearchTextPageResolution> {
        let selection = match self.resolve_search_selection(request.selection())? {
            SearchSelectionResolution::Current { selection } => selection,
            SearchSelectionResolution::Stale => return Ok(SearchTextPageResolution::Stale),
            SearchSelectionResolution::NotFound => return Ok(SearchTextPageResolution::NotFound),
        };
        let offset_code_points = request.cursor.code_point_offset;
        let start = i64::try_from(offset_code_points)
            .ok()
            .and_then(|offset| offset.checked_add(1))
            .ok_or_else(|| MetaStoreError::invalid_value("search_text_page.offset_code_points"))?;
        let (total, text) = self
            .connection
            .query_row(
                "SELECT length(clean_text), substr(clean_text, ?3, ?4)
                 FROM resume_version
                 WHERE id = ?1 AND document_id = ?2 AND clean_text IS NOT NULL",
                params![
                    selection.resume_version_id.as_str(),
                    selection.document_id.as_str(),
                    start,
                    i64::from(request.limit_code_points),
                ],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(MetaStoreError::storage)?
            .ok_or_else(MetaStoreError::storage_invariant)?;
        let total_code_points = u64::try_from(total)
            .map_err(|_| MetaStoreError::invalid_value("resume_version.clean_text"))?;
        if offset_code_points > total_code_points {
            return Ok(SearchTextPageResolution::InvalidOffset);
        }
        let returned =
            u64::try_from(text.chars().count()).map_err(|_| MetaStoreError::storage_invariant())?;
        let next_offset_code_points = offset_code_points
            .checked_add(returned)
            .ok_or_else(MetaStoreError::storage_invariant)?;
        Ok(SearchTextPageResolution::Current(SearchTextPage {
            selection,
            cursor: request.cursor,
            next_cursor: (next_offset_code_points < total_code_points)
                .then(|| SearchTextPageCursor::for_offset(next_offset_code_points)),
            total_code_points,
            text,
        }))
    }
}
