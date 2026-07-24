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
    snake_enum!(pub(super) enum DetailFieldType { Name, Email, Phone, Wechat, School, SchoolTier, Degree, Major, Company, Title, Education, Skills, Skill, Certificate, Date, DateRange, YearsExperience, Location, Other });
    snake_enum!(pub(super) enum AcceptedStatus { Accepted });
    snake_enum!(pub(super) enum ImportProfile { Explicit });
    snake_enum!(#[derive(PartialEq, Eq)] pub(super) enum RootControlStatus { Active, Paused });
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
mod search;
mod status;

use serde::{Deserialize, Serialize};

use self::detail::{CancelBody, DetailBody, HydrateBody, ImportBody};
pub(crate) use self::diagnostics::DiagnosticsBody;
use self::enums::RootControlStatus;
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
    Import(ImportBody),
    RootControl(RootControlBody),
    Search(SearchBody),
    Detail(DetailBody),
    Hydrate(HydrateBody),
    Cancel(CancelBody),
    Error(error::ErrorBody),
}

#[derive(Deserialize, Serialize)]
struct RootControlBody {
    schema_version: String,
    status: RootControlStatus,
    changed: bool,
    task_cancel_requested: bool,
    catch_up_queued: bool,
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
        Operation::Import => detail::project_import(body).map(DesktopBody::Import),
        Operation::RootControl => project_root_control(body).map(DesktopBody::RootControl),
        Operation::Search => search::project_search(body, expected).map(DesktopBody::Search),
        Operation::Detail => detail::project_detail(body, expected).map(DesktopBody::Detail),
        Operation::Hydrate => {
            detail::project_hydrate(body, body.len(), expected).map(DesktopBody::Hydrate)
        }
        Operation::Cancel => detail::project_cancel(body, expected).map(DesktopBody::Cancel),
    }
}

fn project_root_control(body: &[u8]) -> Result<RootControlBody, DesktopError> {
    let value: RootControlBody = decode(body)?;
    ensure_schema(&value.schema_version, "daemon.import_root_control.v1")?;
    ensure(!value.task_cancel_requested || value.status == RootControlStatus::Paused)?;
    ensure(!value.catch_up_queued || value.status == RootControlStatus::Active)?;
    ensure(!(value.task_cancel_requested && value.catch_up_queued))?;
    Ok(value)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_control_projection_rejects_state_confusion_and_drops_extra_fields() {
        let body = br#"{"schema_version":"daemon.import_root_control.v1","status":"paused","changed":true,"task_cancel_requested":true,"catch_up_queued":false,"root_path":"synthetic-private-root","private_debug":true}"#;
        let projected = project_root_control(body).unwrap();
        let exposed = serde_json::to_string(&projected).unwrap();
        assert!(!exposed.contains("root_path"));
        assert!(!exposed.contains("synthetic-private-root"));
        assert!(!exposed.contains("private_debug"));

        let confused = br#"{"schema_version":"daemon.import_root_control.v1","status":"active","changed":true,"task_cancel_requested":true,"catch_up_queued":false}"#;
        assert!(project_root_control(confused).is_err());
    }
}
