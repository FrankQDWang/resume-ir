macro_rules! snake_enum {
    ($(#[$attribute:meta])* $vis:vis enum $name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(serde::Deserialize, serde::Serialize)]
        $(#[$attribute])*
        #[serde(rename_all = "snake_case")]
        $vis enum $name { $($variant),+ }
    };
}

// Exact daemon wire vocabularies accepted by the desktop bridge.
#[rustfmt::skip]
mod enums {
    snake_enum!(pub(super) enum OkStatus { Ok });
    snake_enum!(pub(super) enum PrivacyBoundary { RedactedLocalAggregate });
    snake_enum!(pub(super) enum EvidenceLane { GuiManual });
    snake_enum!(pub(super) enum EvidenceStatus { Unaccepted });
    snake_enum!(pub(super) enum ScanErrorClass { PermissionDenied, SourceUnavailable, LockedOrUnreadable, Io });
    snake_enum!(pub(super) enum ScanErrorOperation { NormalizePath, ReadDirectory, ReadMetadata, Fingerprint });
    snake_enum!(pub(super) enum SourceRootDeletionPhase { Requested, Quiescing, Publishing, Purging, Verifying });
    snake_enum!(pub(super) enum SourceRootDeletionErrorCode { ImportQuiescenceTimeout, OcrQuiescenceTimeout, PublicationFailed, MetadataPurgeFailed, PrivacyCleanupFailed, ReceiptCompletionFailed, Internal });
    snake_enum!(pub(super) enum DetailFieldType { Name, Email, Phone, Wechat, School, SchoolTier, Degree, Major, Company, Title, Education, Skills, Skill, Certificate, Date, DateRange, YearsExperience, Location, Other });
    snake_enum!(pub(super) enum CancelStatus { Cancelled, CancelRequested, Complete });
    snake_enum!(#[derive(PartialEq, Eq)] pub(super) enum SearchStatus { Ok, Cancelled });
    snake_enum!(pub(super) enum QueryMode { Keyword, FieldFilter, Hybrid, Semantic });
    snake_enum!(pub(super) enum PartialReason { SearchIndexNotReady, DeadlineExceeded, EmbeddingRuntimeUnavailable });
    snake_enum!(pub(super) enum ErrorStatus { Error });
}

mod detail;
mod diagnostics;
mod error;
mod health_contract;
mod preview;
mod search;
mod source_roots;
mod status;

use serde::{Deserialize, Serialize};

use self::detail::{CancelBody, DetailBody, HydrateBody};
pub(crate) use self::diagnostics::DiagnosticsBody;
use self::search::SearchBody;
use self::status::StatusBody;
use crate::daemon_client::DesktopError;
use crate::daemon_exchange::ExpectedResponse;
use crate::daemon_request::Operation;

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
#[derive(Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(transparent)]
struct SafeCount(u64);

impl SafeCount {
    fn value(self) -> u64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for SafeCount {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = u64::deserialize(deserializer)?;
        if value <= MAX_SAFE_INTEGER {
            Ok(Self(value))
        } else {
            Err(serde::de::Error::custom(
                "count exceeds JavaScript safe integer",
            ))
        }
    }
}

#[derive(Serialize)]
pub(crate) struct DesktopResponse {
    pub(crate) http_status: u16,
    body: DesktopBody,
}

impl DesktopResponse {
    pub(crate) fn diagnostics(&self) -> Option<&DiagnosticsBody> {
        match &self.body {
            DesktopBody::Diagnostics(body) => Some(body.as_ref()),
            _ => None,
        }
    }
}

#[derive(Serialize)]
#[serde(untagged)]
enum DesktopBody {
    Status(Box<StatusBody>),
    Diagnostics(Box<DiagnosticsBody>),
    SourceRoots(serde_json::Value),
    RootDeletion(serde_json::Value),
    Search(SearchBody),
    Detail(DetailBody),
    Hydrate(HydrateBody),
    Preview(serde_json::Value),
    Cancel(CancelBody),
    Error(error::ErrorBody),
}

pub(crate) fn project_response(
    http_status: u16,
    body: &[u8],
    expected: &ExpectedResponse,
) -> Result<DesktopResponse, DesktopError> {
    let projected = if (200..300).contains(&http_status) {
        project_success(body, expected)?
    } else {
        error::project_error(body, http_status, expected).map(DesktopBody::Error)?
    };
    Ok(DesktopResponse {
        http_status,
        body: projected,
    })
}

fn project_success(body: &[u8], expected: &ExpectedResponse) -> Result<DesktopBody, DesktopError> {
    match expected.operation() {
        Operation::Status => status::project_status(body)
            .map(Box::new)
            .map(DesktopBody::Status),
        Operation::Diagnostics => diagnostics::project_diagnostics(body)
            .map(Box::new)
            .map(DesktopBody::Diagnostics),
        Operation::RootControl => Err(protocol_error()),
        Operation::SourceRoots => {
            source_roots::project_source_roots(body).map(DesktopBody::SourceRoots)
        }
        Operation::RootDeletion => {
            source_roots::project_root_deletion(body).map(DesktopBody::RootDeletion)
        }
        Operation::Search => search::project_search(body, expected).map(DesktopBody::Search),
        Operation::Detail => detail::project_detail(body, expected).map(DesktopBody::Detail),
        Operation::Hydrate => {
            detail::project_hydrate(body, body.len(), expected).map(DesktopBody::Hydrate)
        }
        Operation::PreviewCreate | Operation::PreviewRange | Operation::PreviewClose => {
            preview::project_preview(body, expected).map(DesktopBody::Preview)
        }
        Operation::Cancel => detail::project_cancel(body, expected).map(DesktopBody::Cancel),
    }
}

fn decode<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, DesktopError> {
    serde_json::from_slice(body).map_err(|_| protocol_error())
}

fn ensure_schema(actual: &str, expected: &str) -> Result<(), DesktopError> {
    ensure(actual == expected)
}

fn ensure(condition: bool) -> Result<(), DesktopError> {
    if condition {
        Ok(())
    } else {
        Err(protocol_error())
    }
}

fn bounded_chars(value: &str, max_chars: usize, max_bytes: usize) -> bool {
    value.len() <= max_bytes && value.chars().count() <= max_chars
}

fn protocol_error() -> DesktopError {
    DesktopError::new("daemon_protocol", "daemon 响应合同无效")
}
