use std::fmt;

use crate::{MetaStoreError, SearchRepairReason};

pub enum SearchMetadataTransactionError<E> {
    Unavailable(SearchMetadataUnavailable),
    Store(MetaStoreError),
    Operation(E),
}

/// Stable error boundary for application-level reads of a ready search
/// publication. Callers never need access to the transaction closure or its
/// implementation-specific error channel.
#[derive(Debug)]
pub enum SearchMetadataReadError {
    Unavailable(SearchMetadataUnavailable),
    Store(MetaStoreError),
}

impl fmt::Display for SearchMetadataReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(unavailable) => unavailable.fmt(formatter),
            Self::Store(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SearchMetadataReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unavailable(unavailable) => Some(unavailable),
            Self::Store(error) => Some(error),
        }
    }
}

impl SearchMetadataReadError {
    pub fn unavailable(&self) -> Option<SearchMetadataUnavailable> {
        match self {
            Self::Unavailable(unavailable) => Some(*unavailable),
            Self::Store(_) => None,
        }
    }

    pub fn store_error(&self) -> Option<&MetaStoreError> {
        match self {
            Self::Unavailable(_) => None,
            Self::Store(error) => Some(error),
        }
    }
}

impl<E: fmt::Debug> fmt::Debug for SearchMetadataTransactionError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(unavailable) => formatter
                .debug_tuple("Unavailable")
                .field(unavailable)
                .finish(),
            Self::Store(error) => formatter.debug_tuple("Store").field(error).finish(),
            Self::Operation(error) => formatter.debug_tuple("Operation").field(error).finish(),
        }
    }
}

impl<E: fmt::Display> fmt::Display for SearchMetadataTransactionError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(unavailable) => unavailable.fmt(formatter),
            Self::Store(error) => error.fmt(formatter),
            Self::Operation(error) => error.fmt(formatter),
        }
    }
}

impl<E> std::error::Error for SearchMetadataTransactionError<E>
where
    E: std::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unavailable(unavailable) => Some(unavailable),
            Self::Store(error) => Some(error),
            Self::Operation(error) => Some(error),
        }
    }
}

impl<E> SearchMetadataTransactionError<E> {
    pub fn store_error(&self) -> Option<&MetaStoreError> {
        match self {
            Self::Unavailable(_) => None,
            Self::Store(error) => Some(error),
            Self::Operation(_) => None,
        }
    }

    pub fn operation_error(&self) -> Option<&E> {
        match self {
            Self::Unavailable(_) | Self::Store(_) => None,
            Self::Operation(error) => Some(error),
        }
    }

    pub fn unavailable(&self) -> Option<SearchMetadataUnavailable> {
        match self {
            Self::Unavailable(unavailable) => Some(*unavailable),
            Self::Store(_) | Self::Operation(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchMetadataUnavailable {
    Repairing(SearchRepairReason),
    RepairBlocked(SearchRepairReason),
}

impl fmt::Display for SearchMetadataUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Repairing(_) => formatter.write_str("search metadata is repairing"),
            Self::RepairBlocked(_) => formatter.write_str("search metadata repair is blocked"),
        }
    }
}

impl std::error::Error for SearchMetadataUnavailable {}
