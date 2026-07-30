use std::net::TcpStream;
use std::path::{Component, Path};
use std::str::FromStr;

use meta_store::{
    ImportProcessingContract, ImportTaskStatus, MetaStoreError, MetaStoreErrorClass,
    OwnedMetaStore, ScanPhase, ScanSnapshot, ScanTrigger, SourceRoot, SourceRootId,
    SourceRootRegistration, SourceRootRegistrationAvailability, SourceRootState,
    SourceWatcherState,
};
use serde::Deserialize;

use super::super::protocol::Request;
use super::{authorized, unauthorized_body, write, write_service_unavailable, RouteResult};
use crate::command_failure::CommandFailure;
use crate::ipc::ServiceErrorCode;

const REGISTER_REQUEST_SCHEMA: &str = "resume-ir.source-root-register-request.v1";
const LEGACY_MIGRATION_REQUEST_SCHEMA: &str = "resume-ir.source-root-legacy-migration-request.v1";
const SCAN_REQUEST_SCHEMA: &str = "resume-ir.source-root-scan-request.v1";
const CONTROL_REQUEST_SCHEMA: &str = "resume-ir.source-root-control-request.v1";
const DELETE_REQUEST_SCHEMA: &str = "resume-ir.source-root-delete-request.v1";
const RESPONSE_SCHEMA: &str = "resume-ir.source-roots.v2";
const MAX_PATH_BYTES: usize = 128 * 1024;
const MAX_LABEL_CHARS: usize = 80;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisterRequest {
    schema_version: String,
    requested_path: String,
    display_label: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyMigrationRequest {
    schema_version: String,
    roots: Vec<LegacyMigrationRoot>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyMigrationRoot {
    requested_path: String,
    display_label: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RootRequest {
    schema_version: String,
    root_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlRequest {
    schema_version: String,
    root_id: String,
    action: ControlAction,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ControlAction {
    Pause,
    Resume,
}

pub(super) fn list(
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    auth_token: &str,
    request: &Request,
    stream: &mut TcpStream,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return write(stream, 401, "application/json", &unauthorized_body());
    }
    let roots = match store.source_roots() {
        Ok(roots) => roots,
        Err(_) => return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable),
    };
    let roots = roots
        .into_iter()
        .map(|root| root_json(store, processing_contract, root))
        .collect::<Result<Vec<_>, _>>();
    let roots = match roots {
        Ok(roots) => roots,
        Err(_) => return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable),
    };
    write(
        stream,
        200,
        "application/json",
        &serde_json::json!({
            "schema_version": RESPONSE_SCHEMA,
            "limit": 16,
            "roots": roots,
        })
        .to_string(),
    )
}

pub(super) fn register(
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    auth_token: &str,
    request: &Request,
    stream: &mut TcpStream,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return write(stream, 401, "application/json", &unauthorized_body());
    }
    let payload = match serde_json::from_slice::<RegisterRequest>(&request.body) {
        Ok(payload) if valid_register_request(&payload) => payload,
        _ => return write_invalid(stream),
    };
    let requested = Path::new(&payload.requested_path);
    let canonical = match crate::source_root_path::canonicalize_authorized_root(requested) {
        Some(path) => path,
        None => return write_invalid(stream),
    };
    let Some(canonical_path) = canonical.to_str() else {
        return write_invalid(stream);
    };
    match source_root_path_is_deleting(store, canonical_path) {
        Ok(true) => return write_source_root_deleting(stream),
        Ok(false) => {}
        Err(()) => {
            return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable);
        }
    }
    let now = match crate::current_timestamp() {
        Ok(now) => now,
        Err(_) => return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable),
    };
    let root = match store.register_source_root(
        canonical_path,
        &payload.requested_path,
        &payload.display_label,
        now,
    ) {
        Ok(root) => root,
        Err(error)
            if error.class() == MetaStoreErrorClass::InvalidTransition
                && source_root_path_is_deleting(store, canonical_path) == Ok(true) =>
        {
            return write_source_root_deleting(stream);
        }
        Err(error) => return write_registration_failure(stream, &error),
    };
    write_root(stream, store, processing_contract, root)
}

pub(super) fn migrate_legacy(
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    auth_token: &str,
    request: &Request,
    stream: &mut TcpStream,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return write(stream, 401, "application/json", &unauthorized_body());
    }
    let payload = match serde_json::from_slice::<LegacyMigrationRequest>(&request.body) {
        Ok(payload)
            if payload.schema_version == LEGACY_MIGRATION_REQUEST_SCHEMA
                && !payload.roots.is_empty()
                && payload.roots.len() <= 16 =>
        {
            payload
        }
        _ => return write_invalid(stream),
    };
    let mut registrations = Vec::with_capacity(payload.roots.len());
    for root in payload.roots {
        let request = RegisterRequest {
            schema_version: REGISTER_REQUEST_SCHEMA.to_string(),
            requested_path: root.requested_path,
            display_label: root.display_label,
        };
        if !valid_register_request(&request) {
            return write_invalid(stream);
        }
        let requested_path = Path::new(&request.requested_path);
        if requested_path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        {
            return write_invalid(stream);
        }
        let (canonical_path, availability) =
            match crate::source_root_path::authorize_legacy_root(requested_path) {
                Ok(crate::source_root_path::LegacyAuthorizedRoot::Available(path)) => {
                    let Some(path) = path.to_str() else {
                        return write_invalid(stream);
                    };
                    (
                        path.to_string(),
                        SourceRootRegistrationAvailability::Available,
                    )
                }
                Ok(crate::source_root_path::LegacyAuthorizedRoot::Offline) => (
                    request.requested_path.clone(),
                    SourceRootRegistrationAvailability::Offline,
                ),
                Err(()) => return write_invalid(stream),
            };
        if canonical_path.len() > MAX_PATH_BYTES || canonical_path.contains('\0') {
            return write_invalid(stream);
        };
        registrations.push(SourceRootRegistration {
            canonical_path,
            requested_path: request.requested_path,
            display_label: request.display_label,
            availability,
        });
    }
    let now = match crate::current_timestamp() {
        Ok(now) => now,
        Err(_) => return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable),
    };
    for registration in &registrations {
        match source_root_path_is_deleting(store, &registration.canonical_path) {
            Ok(true) => return write_source_root_deleting(stream),
            Ok(false) => {}
            Err(()) => {
                return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable);
            }
        }
    }
    if let Err(error) = store.register_source_roots_atomically(&registrations, now) {
        if error.class() == MetaStoreErrorClass::InvalidTransition
            && registrations.iter().any(|registration| {
                source_root_path_is_deleting(store, &registration.canonical_path) == Ok(true)
            })
        {
            return write_source_root_deleting(stream);
        }
        return write_registration_failure(stream, &error);
    }
    list(store, processing_contract, auth_token, request, stream)
}

pub(super) fn scan(
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    auth_token: &str,
    request: &Request,
    stream: &mut TcpStream,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return write(stream, 401, "application/json", &unauthorized_body());
    }
    let root_id = match parse_root_request(&request.body, SCAN_REQUEST_SCHEMA) {
        Ok(root_id) => root_id,
        Err(()) => return write_invalid(stream),
    };
    let root = match store.source_root(&root_id) {
        Ok(Some(root)) => root,
        Ok(None) => return write_not_found(stream),
        Err(_) => return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable),
    };
    if root.state == SourceRootState::Offline {
        return write_source_unavailable(stream);
    }
    match store.source_root_deletion_in_progress(&root_id) {
        Ok(true) => return write_source_root_deleting(stream),
        Ok(false) => {}
        Err(_) => return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable),
    }
    let now = match crate::current_timestamp() {
        Ok(now) => now,
        Err(_) => return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable),
    };
    let trigger = match store.latest_scan_snapshot(&root_id) {
        Ok(None) => ScanTrigger::Initial,
        Ok(Some(_)) => ScanTrigger::Manual,
        Err(_) => {
            return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable);
        }
    };
    match crate::source_scan_coordinator::enqueue(store, processing_contract, &root, trigger, now) {
        Ok(_) => {}
        Err(error) => return write_command_failure(stream, error),
    }
    write_root(stream, store, processing_contract, root)
}

pub(super) fn control(
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    auth_token: &str,
    request: &Request,
    stream: &mut TcpStream,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return write(stream, 401, "application/json", &unauthorized_body());
    }
    let payload = match serde_json::from_slice::<ControlRequest>(&request.body) {
        Ok(payload) if payload.schema_version == CONTROL_REQUEST_SCHEMA => payload,
        _ => return write_invalid(stream),
    };
    let root_id = match SourceRootId::from_str(&payload.root_id) {
        Ok(root_id) => root_id,
        Err(_) => return write_invalid(stream),
    };
    let root = match store.source_root(&root_id) {
        Ok(Some(root)) => root,
        Ok(None) => return write_not_found(stream),
        Err(_) => return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable),
    };
    match store.source_root_deletion_in_progress(&root_id) {
        Ok(true) => return write_source_root_deleting(stream),
        Ok(false) => {}
        Err(_) => return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable),
    }
    if matches!(payload.action, ControlAction::Resume)
        && !crate::source_root_path::is_available(&root.canonical_path)
    {
        let now = match crate::current_timestamp() {
            Ok(now) => now,
            Err(_) => {
                return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable);
            }
        };
        if store
            .set_source_root_state(
                &root_id,
                SourceRootState::Offline,
                SourceWatcherState::Unavailable,
                now,
            )
            .is_err()
        {
            return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable);
        }
        return write_source_unavailable(stream);
    }
    let now = match crate::current_timestamp() {
        Ok(now) => now,
        Err(_) => return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable),
    };
    let root = match payload.action {
        ControlAction::Pause => {
            store.set_source_root_state(&root_id, root.state, SourceWatcherState::Paused, now)
        }
        ControlAction::Resume => store.resume_source_root_monitoring(&root_id, now),
    };
    let root = match root {
        Ok(root) => root,
        Err(_) => return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable),
    };
    let resume_scan_allowed = matches!(payload.action, ControlAction::Resume)
        && match store.latest_scan_snapshot(&root_id) {
            Ok(snapshot) => snapshot.is_some(),
            Err(_) => {
                return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable);
            }
        };
    if resume_scan_allowed {
        match crate::source_scan_coordinator::enqueue(
            store,
            processing_contract,
            &root,
            ScanTrigger::Recovery,
            now,
        ) {
            Ok(_) => {}
            Err(error) => return write_command_failure(stream, error),
        }
    }
    write_root(stream, store, processing_contract, root)
}

pub(super) fn delete(
    data_dir: &Path,
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    auth_token: &str,
    request: &Request,
    stream: &mut TcpStream,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return write(stream, 401, "application/json", &unauthorized_body());
    }
    let root_id = match parse_root_request(&request.body, DELETE_REQUEST_SCHEMA) {
        Ok(root_id) => root_id,
        Err(()) => return write_invalid(stream),
    };
    let sibling = match store.open_sibling() {
        Ok(sibling) => sibling,
        Err(_) => return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable),
    };
    let deletion = match crate::source_root_deletion::request(store, &root_id) {
        Ok(deletion) => deletion,
        Err(error) => return write_command_failure(stream, error),
    };
    let body = serde_json::json!({
        "schema_version": "resume-ir.root-deletion-receipt.v1",
        "status": "deleting",
        "root_id": deletion.receipt.root_id.as_str(),
        "affected_documents": deletion.receipt.affected_documents,
        "removed_documents": deletion.receipt.removed_documents,
        "source_files_deleted": false,
    })
    .to_string();
    acknowledge_then_start_worker(
        || write(stream, 202, "application/json", &body),
        || {
            crate::source_root_deletion::spawn_worker(
                data_dir.to_path_buf(),
                sibling,
                processing_contract.clone(),
                root_id,
            )
        },
    )
}

fn acknowledge_then_start_worker(
    acknowledge: impl FnOnce() -> RouteResult,
    start_worker: impl FnOnce() -> Result<(), CommandFailure>,
) -> RouteResult {
    let response = acknowledge();
    let _ = start_worker();
    response
}

fn write_root(
    stream: &mut TcpStream,
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    root: SourceRoot,
) -> RouteResult {
    let root = match root_json(store, processing_contract, root) {
        Ok(root) => root,
        Err(_) => return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable),
    };
    write(
        stream,
        200,
        "application/json",
        &serde_json::json!({
            "schema_version": RESPONSE_SCHEMA,
            "root": root,
        })
        .to_string(),
    )
}

fn root_json(
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    root: SourceRoot,
) -> meta_store::Result<serde_json::Value> {
    let deleting = store.source_root_deletion_in_progress(&root.id)?;
    let classification = store
        .source_root_classification_counts(&root.id, processing_contract.classifier_epoch())?;
    let current_counts = serde_json::json!({
        "discovered": store.source_root_present_count(&root.id)?,
        "searchable": store.source_root_searchable_count(&root.id)?,
        "non_resume": classification.non_resume,
        "needs_review": classification.needs_review,
        "ocr": classification.ocr_backlog,
        "failed": classification.failed,
    });
    let snapshot = store
        .latest_scan_snapshot(&root.id)?
        .map(|snapshot| live_snapshot(store, snapshot))
        .transpose()?;
    Ok(root_json_with_snapshot(
        root,
        snapshot,
        deleting,
        current_counts,
    ))
}

fn live_snapshot(
    store: &OwnedMetaStore,
    mut snapshot: ScanSnapshot,
) -> meta_store::Result<ScanSnapshot> {
    if !snapshot.phase.is_active() {
        return Ok(snapshot);
    }
    let Ok(task_id) = meta_store::ImportTaskId::from_str(&snapshot.id) else {
        return Ok(snapshot);
    };
    let Some(scope) = store.import_scan_scope_by_task_id(&task_id)? else {
        return Ok(snapshot);
    };
    // The import pipeline persists this counter every bounded batch. Deriving
    // it from outcome buckets is not sound because classification exclusions,
    // unchanged files and crawler-ignored entries have different cardinality
    // semantics. A boundedly stale exact counter is preferable to a fabricated
    // real-time percentage.
    let processed = snapshot.counts.processed;
    snapshot.phase = match store.import_task_by_id(&task_id)?.map(|task| task.status) {
        Some(ImportTaskStatus::Queued | ImportTaskStatus::FailedRetryable) => ScanPhase::Queued,
        Some(ImportTaskStatus::Running) if scope.files_discovered == 0 => ScanPhase::Discovering,
        Some(ImportTaskStatus::Running) if processed < scope.files_discovered => ScanPhase::Parsing,
        Some(ImportTaskStatus::Running) => ScanPhase::Publishing,
        // Task completion and the terminal scan snapshot are committed by
        // separate owners. Do not synthesize a terminal phase here: doing so
        // would pair `complete`/`failed` with the still-active snapshot's
        // unknown completeness and missing completion timestamp. The worker
        // persists the authoritative terminal snapshot immediately after the
        // task, and until then publication is the truthful bounded state.
        Some(ImportTaskStatus::Completed) => ScanPhase::Publishing,
        Some(ImportTaskStatus::FailedPermanent) => snapshot.phase,
        None => snapshot.phase,
    };
    snapshot.counts.discovered = scope.files_discovered;
    snapshot.counts.searchable = scope.searchable_documents;
    snapshot.counts.ocr = scope.ocr_required_documents;
    snapshot.counts.failed = scope.failed_documents;
    snapshot.counts.ignored = scope.ignored_entries;
    snapshot.counts.errors = scope.scan_errors;
    snapshot.counts.processed = processed;
    snapshot.counts.total = Some(scope.files_discovered);
    snapshot.updated_at = scope.updated_at;
    let elapsed = snapshot
        .updated_at
        .as_unix_seconds()
        .saturating_sub(snapshot.started_at.as_unix_seconds());
    // A single fast item produces a wildly unstable ETA for mixed PDF/DOCX
    // corpora. Keep the contract nullable until there is enough elapsed work
    // to make the estimate useful rather than merely precise-looking.
    const MIN_ETA_SAMPLES: u64 = 8;
    const MIN_ETA_ELAPSED_SECONDS: i64 = 3;
    if elapsed >= MIN_ETA_ELAPSED_SECONDS && snapshot.counts.processed >= MIN_ETA_SAMPLES {
        let rate = snapshot.counts.processed as f64 / elapsed as f64;
        snapshot.rate_per_second = Some(rate);
        snapshot.eta_seconds = snapshot.counts.total.map(|total| {
            let remaining = total.saturating_sub(snapshot.counts.processed);
            (remaining as f64 / rate).ceil() as u64
        });
    }
    Ok(snapshot)
}

fn root_json_with_snapshot(
    root: SourceRoot,
    snapshot: Option<ScanSnapshot>,
    deleting: bool,
    current_counts: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "root_id": root.id.as_str(),
        "display_label": root.display_label,
        "state": if deleting { "deleting" } else { source_root_state(root.state) },
        "watcher_state": watcher_state(root.watcher_state),
        "current_counts": current_counts,
        "last_scan": snapshot.map(scan_json),
    })
}

fn scan_json(snapshot: ScanSnapshot) -> serde_json::Value {
    serde_json::json!({
        "scan_id": snapshot.id,
        "trigger": scan_trigger(snapshot.trigger),
        "phase": scan_phase(snapshot.phase),
        "completeness": scan_completeness(snapshot.completeness),
        "counts": {
            "discovered": snapshot.counts.discovered,
            "searchable": snapshot.counts.searchable,
            "non_resume": snapshot.counts.non_resume,
            "needs_review": snapshot.counts.needs_review,
            "ocr": snapshot.counts.ocr,
            "failed": snapshot.counts.failed,
            "ignored": snapshot.counts.ignored,
            "processed": snapshot.counts.processed,
            "total": snapshot.counts.total,
            "errors": snapshot.counts.errors,
        },
        "rate_per_second": snapshot.rate_per_second,
        "eta_seconds": snapshot.eta_seconds,
        "started_at_seconds": snapshot.started_at.as_unix_seconds(),
        "updated_at_seconds": snapshot.updated_at.as_unix_seconds(),
        "completed_at_seconds": snapshot.completed_at.map(|value| value.as_unix_seconds()),
    })
}

fn valid_register_request(request: &RegisterRequest) -> bool {
    request.schema_version == REGISTER_REQUEST_SCHEMA
        && !request.requested_path.is_empty()
        && request.requested_path.len() <= MAX_PATH_BYTES
        && !request.requested_path.contains('\0')
        && Path::new(&request.requested_path).is_absolute()
        && !request.display_label.is_empty()
        && request.display_label.chars().count() <= MAX_LABEL_CHARS
        && !request.display_label.chars().any(char::is_control)
}

fn parse_root_request(body: &[u8], expected_schema: &str) -> Result<SourceRootId, ()> {
    let request = serde_json::from_slice::<RootRequest>(body).map_err(|_| ())?;
    if request.schema_version != expected_schema {
        return Err(());
    }
    SourceRootId::from_str(&request.root_id).map_err(|_| ())
}

fn write_command_failure(stream: &mut TcpStream, error: CommandFailure) -> RouteResult {
    match error {
        CommandFailure::BadRequest(_) | CommandFailure::TooLarge(_) => write_invalid(stream),
        CommandFailure::NotFound(_) => write_not_found(stream),
        CommandFailure::Conflict(_) => write(
            stream,
            409,
            "application/json",
            &super::unified_error_body(None, "CONFLICT", "retry"),
        ),
        CommandFailure::ServiceUnavailable(_) | CommandFailure::Internal => {
            write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable)
        }
    }
}

fn write_registration_failure(stream: &mut TcpStream, error: &MetaStoreError) -> RouteResult {
    match error.class() {
        MetaStoreErrorClass::InvalidValue => write_invalid(stream),
        MetaStoreErrorClass::InvalidTransition | MetaStoreErrorClass::ImmutableIdentityConflict => {
            write_conflict(stream)
        }
        MetaStoreErrorClass::Storage
        | MetaStoreErrorClass::Migration
        | MetaStoreErrorClass::MigrationOwnershipRequired
        | MetaStoreErrorClass::UnsupportedStoreSchema
        | MetaStoreErrorClass::NotFound
        | MetaStoreErrorClass::StorageInvariant
        | MetaStoreErrorClass::WeakPassphrase
        | MetaStoreErrorClass::InvalidBackup
        | MetaStoreErrorClass::Crypto
        | MetaStoreErrorClass::KeyAlreadyExists => {
            write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable)
        }
    }
}

fn write_invalid(stream: &mut TcpStream) -> RouteResult {
    write(
        stream,
        400,
        "application/json",
        &super::unified_error_body(None, "BAD_REQUEST", "correct_request"),
    )
}

fn write_conflict(stream: &mut TcpStream) -> RouteResult {
    write(
        stream,
        409,
        "application/json",
        &super::unified_error_body(None, "CONFLICT", "retry"),
    )
}

fn source_root_path_is_deleting(store: &OwnedMetaStore, canonical_path: &str) -> Result<bool, ()> {
    let root = store
        .source_root_by_canonical_path(canonical_path)
        .map_err(|_| ())?;
    root.map(|root| store.source_root_deletion_in_progress(&root.id))
        .transpose()
        .map(|deleting| deleting.unwrap_or(false))
        .map_err(|_| ())
}

fn write_source_root_deleting(stream: &mut TcpStream) -> RouteResult {
    write(
        stream,
        409,
        "application/json",
        &crate::ipc::response::service_error_body(
            None,
            "CONFLICT",
            "retry",
            None,
            Some("source_root_deleting"),
        ),
    )
}

fn write_not_found(stream: &mut TcpStream) -> RouteResult {
    write(
        stream,
        404,
        "application/json",
        &super::unified_error_body(None, "NOT_FOUND", "refresh_search"),
    )
}

fn write_source_unavailable(stream: &mut TcpStream) -> RouteResult {
    write(
        stream,
        404,
        "application/json",
        &super::unified_error_body(None, "SOURCE_UNAVAILABLE", "rescan_source"),
    )
}

fn source_root_state(state: SourceRootState) -> &'static str {
    match state {
        SourceRootState::Active => "active",
        SourceRootState::Offline => "offline",
    }
}

fn watcher_state(state: SourceWatcherState) -> &'static str {
    match state {
        SourceWatcherState::Active => "active",
        SourceWatcherState::Paused => "paused",
        SourceWatcherState::Unavailable => "unavailable",
    }
}

fn scan_trigger(trigger: ScanTrigger) -> &'static str {
    match trigger {
        ScanTrigger::Initial => "initial",
        ScanTrigger::Manual => "manual",
        ScanTrigger::Watcher => "watcher",
        ScanTrigger::Periodic => "periodic",
        ScanTrigger::Recovery => "recovery",
    }
}

fn scan_phase(phase: meta_store::ScanPhase) -> &'static str {
    match phase {
        meta_store::ScanPhase::Queued => "queued",
        meta_store::ScanPhase::Discovering => "discovering",
        meta_store::ScanPhase::Fingerprinting => "fingerprinting",
        meta_store::ScanPhase::Classifying => "classifying",
        meta_store::ScanPhase::Parsing => "parsing",
        meta_store::ScanPhase::Ocr => "ocr",
        meta_store::ScanPhase::Publishing => "publishing",
        meta_store::ScanPhase::Complete => "complete",
        meta_store::ScanPhase::Partial => "partial",
        meta_store::ScanPhase::Failed => "failed",
    }
}

fn scan_completeness(completeness: meta_store::ScanCompleteness) -> &'static str {
    match completeness {
        meta_store::ScanCompleteness::Unknown => "unknown",
        meta_store::ScanCompleteness::Complete => "complete",
        meta_store::ScanCompleteness::Partial => "partial",
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::acknowledge_then_start_worker;
    use crate::command_failure::CommandFailure;
    use crate::ipc::{RequestFailure, ResponseSinkError};

    #[test]
    fn deletion_acknowledgement_precedes_background_execution() {
        let failure = RequestFailure::ResponseSink(ResponseSinkError::ClientDisconnected);
        for (acknowledge_fails, worker_fails) in [(false, false), (true, false), (false, true)] {
            let order = RefCell::new(Vec::new());
            let result = acknowledge_then_start_worker(
                || {
                    order.borrow_mut().push("acknowledge");
                    (!acknowledge_fails).then_some(()).ok_or(failure)
                },
                || {
                    order.borrow_mut().push("start_worker");
                    (!worker_fails)
                        .then_some(())
                        .ok_or(CommandFailure::Internal)
                },
            );
            assert_eq!(result, (!acknowledge_fails).then_some(()).ok_or(failure));
            assert_eq!(*order.borrow(), ["acknowledge", "start_worker"]);
        }
    }
}
