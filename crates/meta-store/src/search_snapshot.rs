use rusqlite::{Connection, TransactionBehavior};

use crate::{MetaStoreError, MetadataStore, MetadataStoreAccess, Result};

mod error;
mod filter;
mod head;
mod selection;
mod text_page;

pub use error::{
    SearchMetadataReadError, SearchMetadataTransactionError, SearchMetadataUnavailable,
};
pub use filter::{
    BoundedFilterSelection, ExactHitHydration, ExactHitHydrationFailure,
    ExactHitHydrationFailureKind, SearchFilterCase, SearchHitMetadata, SearchHitMetadataLimit,
    SearchProjectionFilter, SearchProjectionFilterError, SearchProjectionPredicate,
    MAX_BOUNDED_FILTER_SELECTION, MAX_EXACT_HIT_HYDRATION, MAX_SEARCH_FILTER_PREDICATES,
    MAX_SEARCH_FILTER_VALUES,
};
pub use head::SearchMetadataHead;
pub use selection::{
    SearchSelectionDetails, SearchSelectionDetailsResolution, SearchSelectionLimit,
    SearchSelectionVersion, MAX_SEARCH_SELECTION_MENTIONS,
};
pub(crate) use selection::{MAX_MENTION_EXTRACTOR_BYTES, MAX_MENTION_VALUE_BYTES};
pub use text_page::{
    SearchTextBytePage, SearchTextBytePageRequest, SearchTextBytePageResolution, SearchTextPage,
    SearchTextPageCursor, SearchTextPageCursorError, SearchTextPageRequest,
    SearchTextPageRequestError, SearchTextPageResolution, MAX_SEARCH_TEXT_BYTE_PAGE_BYTES,
    MAX_SEARCH_TEXT_PAGE_CODE_POINTS,
};

/// Immutable detail data and its first bounded text page, resolved inside one
/// metadata transaction against one active search publication.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchSelectionDetailBundle {
    pub details: Box<SearchSelectionDetails>,
    pub text_page: SearchTextBytePage,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SearchSelectionDetailResolution {
    Current(Box<SearchSelectionDetailBundle>),
    Stale,
    NotFound,
    InvalidOffset,
    LimitExceeded(SearchSelectionLimit),
}

use head::{audit_active_projection, read_ready_head, SearchMetadataOpenError};

/// Read-only metadata view pinned to one SQLite snapshot and one ready search
/// publication. The transaction cannot escape `with_search_metadata_snapshot`.
pub struct SearchMetadataSnapshot<'transaction> {
    connection: &'transaction Connection,
    head: SearchMetadataHead,
}

impl SearchMetadataSnapshot<'_> {
    pub fn head(&self) -> &SearchMetadataHead {
        &self.head
    }
}

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    /// Resolves exact-version detail metadata and a bounded text page in one
    /// SQLite snapshot. This is the application boundary for detail requests;
    /// callers cannot accidentally split the two reads across publications.
    pub fn search_selection_detail(
        &self,
        page_request: &SearchTextBytePageRequest,
    ) -> std::result::Result<SearchSelectionDetailResolution, SearchMetadataReadError> {
        flatten_read_result(self.with_search_metadata_snapshot(|snapshot| {
            let details = match snapshot.selection_details(page_request.selection())? {
                SearchSelectionDetailsResolution::Current(details) => details,
                SearchSelectionDetailsResolution::Stale => {
                    return Ok(SearchSelectionDetailResolution::Stale);
                }
                SearchSelectionDetailsResolution::NotFound => {
                    return Ok(SearchSelectionDetailResolution::NotFound);
                }
                SearchSelectionDetailsResolution::LimitExceeded(limit) => {
                    return Ok(SearchSelectionDetailResolution::LimitExceeded(limit));
                }
            };
            let text_page = match snapshot.clean_text_byte_page(page_request)? {
                SearchTextBytePageResolution::Current(page) => page,
                SearchTextBytePageResolution::Stale => {
                    return Ok(SearchSelectionDetailResolution::Stale);
                }
                SearchTextBytePageResolution::NotFound => {
                    return Ok(SearchSelectionDetailResolution::NotFound);
                }
                SearchTextBytePageResolution::InvalidOffset => {
                    return Ok(SearchSelectionDetailResolution::InvalidOffset);
                }
            };
            Ok(SearchSelectionDetailResolution::Current(Box::new(
                SearchSelectionDetailBundle { details, text_page },
            )))
        }))
    }

    /// Reads one bounded UTF-8 byte page for an exact active selection without
    /// exposing transaction ownership to the caller.
    pub fn search_text_byte_page(
        &self,
        request: &SearchTextBytePageRequest,
    ) -> std::result::Result<SearchTextBytePageResolution, SearchMetadataReadError> {
        flatten_read_result(
            self.with_search_metadata_snapshot(|snapshot| snapshot.clean_text_byte_page(request)),
        )
    }

    pub fn with_search_metadata_snapshot<T, E>(
        &self,
        operation: impl for<'transaction> FnOnce(
            &SearchMetadataSnapshot<'transaction>,
        ) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, SearchMetadataTransactionError<E>> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(MetaStoreError::storage)
            .map_err(SearchMetadataTransactionError::Store)?;
        let head = read_ready_head(&transaction).map_err(|error| match error {
            SearchMetadataOpenError::Unavailable(unavailable) => {
                SearchMetadataTransactionError::Unavailable(unavailable)
            }
            SearchMetadataOpenError::Store(error) => SearchMetadataTransactionError::Store(error),
        })?;
        let snapshot = SearchMetadataSnapshot {
            connection: &transaction,
            head,
        };
        let result = operation(&snapshot);
        drop(snapshot);
        match result {
            Ok(value) => {
                transaction
                    .commit()
                    .map_err(MetaStoreError::storage)
                    .map_err(SearchMetadataTransactionError::Store)?;
                Ok(value)
            }
            Err(error) => {
                drop(transaction);
                Err(SearchMetadataTransactionError::Operation(error))
            }
        }
    }

    /// Performs the O(n) projection audit used by repair and diagnostics.
    /// Request hot paths intentionally use only the constant-size ready head;
    /// callers must not invoke this audit per search or detail request.
    pub fn validate_search_projection_integrity(&self) -> Result<()> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(MetaStoreError::storage)?;
        let head = read_ready_head(&transaction).map_err(|error| match error {
            SearchMetadataOpenError::Unavailable(_) => MetaStoreError::storage_invariant(),
            SearchMetadataOpenError::Store(error) => error,
        })?;
        let _ = audit_active_projection(&transaction, &head)?;
        transaction.commit().map_err(MetaStoreError::storage)
    }
}

fn flatten_read_result<T>(
    result: std::result::Result<T, SearchMetadataTransactionError<MetaStoreError>>,
) -> std::result::Result<T, SearchMetadataReadError> {
    match result {
        Ok(value) => Ok(value),
        Err(SearchMetadataTransactionError::Unavailable(unavailable)) => {
            Err(SearchMetadataReadError::Unavailable(unavailable))
        }
        Err(
            SearchMetadataTransactionError::Store(error)
            | SearchMetadataTransactionError::Operation(error),
        ) => Err(SearchMetadataReadError::Store(error)),
    }
}
