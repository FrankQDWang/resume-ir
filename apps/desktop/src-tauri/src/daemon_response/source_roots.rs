use serde_json::{Map, Value};

use super::{bounded_chars, protocol_error, DesktopError};
use crate::daemon_exchange::valid_stable_id;

const MAX_ROOTS: usize = 16;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

pub(super) fn project_source_roots(body: &[u8]) -> Result<Value, DesktopError> {
    let value: Value = serde_json::from_slice(body).map_err(|_| protocol_error())?;
    let object = value.as_object().ok_or_else(protocol_error)?;
    if object.get("schema_version").and_then(Value::as_str) != Some("resume-ir.source-roots.v2") {
        return Err(protocol_error());
    }
    let projected = if let Some(roots) = object.get("roots") {
        if object.len() != 3
            || object.get("limit").and_then(Value::as_u64) != Some(MAX_ROOTS as u64)
        {
            return Err(protocol_error());
        }
        let roots = roots.as_array().ok_or_else(protocol_error)?;
        if roots.len() > MAX_ROOTS {
            return Err(protocol_error());
        }
        serde_json::json!({
            "schema_version": "resume-ir.source-roots.v2",
            "limit": MAX_ROOTS,
            "roots": roots.iter().map(project_root).collect::<Result<Vec<_>, _>>()?,
        })
    } else {
        if object.len() != 2 {
            return Err(protocol_error());
        }
        serde_json::json!({
            "schema_version": "resume-ir.source-roots.v2",
            "root": project_root(object.get("root").ok_or_else(protocol_error)?)?,
        })
    };
    Ok(projected)
}

pub(super) fn project_root_deletion(body: &[u8]) -> Result<Value, DesktopError> {
    let value: Value = serde_json::from_slice(body).map_err(|_| protocol_error())?;
    let object = exact_object(
        &value,
        &[
            "schema_version",
            "status",
            "root_id",
            "affected_documents",
            "removed_documents",
            "source_files_deleted",
        ],
    )?;
    if object.get("schema_version").and_then(Value::as_str)
        != Some("resume-ir.root-deletion-receipt.v1")
        || object.get("status").and_then(Value::as_str) != Some("deleting")
        || object.get("source_files_deleted").and_then(Value::as_bool) != Some(false)
    {
        return Err(protocol_error());
    }
    let root_id = object
        .get("root_id")
        .and_then(Value::as_str)
        .filter(|value| valid_stable_id(value, "root-"))
        .ok_or_else(protocol_error)?;
    let affected = safe_count(object.get("affected_documents"))?;
    let removed = safe_count(object.get("removed_documents"))?;
    if removed > affected {
        return Err(protocol_error());
    }
    Ok(serde_json::json!({
        "schema_version": "resume-ir.root-deletion-receipt.v1",
        "status": "deleting",
        "root_id": root_id,
        "affected_documents": affected,
        "removed_documents": removed,
        "source_files_deleted": false,
    }))
}

fn project_root(value: &Value) -> Result<Value, DesktopError> {
    let object = exact_object(
        value,
        &[
            "root_id",
            "display_label",
            "state",
            "watcher_state",
            "current_counts",
            "last_scan",
        ],
    )?;
    let root_id = object
        .get("root_id")
        .and_then(Value::as_str)
        .filter(|value| valid_stable_id(value, "root-"))
        .ok_or_else(protocol_error)?;
    let display_label = object
        .get("display_label")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && bounded_chars(value, 80, 320))
        .ok_or_else(protocol_error)?;
    let state = closed_string(object, "state", &["active", "offline", "deleting"])?;
    let watcher_state = closed_string(
        object,
        "watcher_state",
        &["active", "paused", "unavailable"],
    )?;
    let current_counts =
        project_current_counts(object.get("current_counts").ok_or_else(protocol_error)?)?;
    let last_scan = match object.get("last_scan") {
        Some(Value::Null) => Value::Null,
        Some(value) => project_scan(value)?,
        None => return Err(protocol_error()),
    };
    Ok(serde_json::json!({
        "root_id": root_id,
        "display_label": display_label,
        "state": state,
        "watcher_state": watcher_state,
        "current_counts": current_counts,
        "last_scan": last_scan,
    }))
}

fn project_current_counts(value: &Value) -> Result<Value, DesktopError> {
    let object = exact_object(
        value,
        &[
            "discovered",
            "searchable",
            "non_resume",
            "needs_review",
            "ocr",
            "failed",
        ],
    )?;
    Ok(serde_json::json!({
        "discovered": safe_count(object.get("discovered"))?,
        "searchable": safe_count(object.get("searchable"))?,
        "non_resume": safe_count(object.get("non_resume"))?,
        "needs_review": safe_count(object.get("needs_review"))?,
        "ocr": safe_count(object.get("ocr"))?,
        "failed": safe_count(object.get("failed"))?,
    }))
}

fn project_scan(value: &Value) -> Result<Value, DesktopError> {
    let object = exact_object(
        value,
        &[
            "scan_id",
            "trigger",
            "phase",
            "completeness",
            "counts",
            "rate_per_second",
            "eta_seconds",
            "started_at_seconds",
            "updated_at_seconds",
            "completed_at_seconds",
        ],
    )?;
    let scan_id = object
        .get("scan_id")
        .and_then(Value::as_str)
        .filter(|value| valid_stable_id(value, "imp_"))
        .ok_or_else(protocol_error)?;
    let trigger = closed_string(
        object,
        "trigger",
        &["initial", "manual", "watcher", "periodic", "recovery"],
    )?;
    let phase = closed_string(
        object,
        "phase",
        &[
            "queued",
            "discovering",
            "fingerprinting",
            "classifying",
            "parsing",
            "ocr",
            "publishing",
            "complete",
            "partial",
            "failed",
        ],
    )?;
    let completeness = closed_string(object, "completeness", &["unknown", "complete", "partial"])?;
    let counts = project_counts(object.get("counts").ok_or_else(protocol_error)?)?;
    let rate = nullable_rate(object.get("rate_per_second"))?;
    let eta = nullable_count(object.get("eta_seconds"))?;
    let started = safe_count(object.get("started_at_seconds"))?;
    let updated = safe_count(object.get("updated_at_seconds"))?;
    let completed = nullable_count(object.get("completed_at_seconds"))?;
    let active = matches!(
        phase,
        "queued"
            | "discovering"
            | "fingerprinting"
            | "classifying"
            | "parsing"
            | "ocr"
            | "publishing"
    );
    if updated < started
        || completed.is_some_and(|value| value < updated)
        || (active && (completeness != "unknown" || completed.is_some()))
        || (phase == "complete" && (completeness != "complete" || completed.is_none()))
        || (phase == "partial" && (completeness != "partial" || completed.is_none()))
        || (phase == "failed" && (completeness != "unknown" || completed.is_none()))
    {
        return Err(protocol_error());
    }
    Ok(serde_json::json!({
        "scan_id": scan_id,
        "trigger": trigger,
        "phase": phase,
        "completeness": completeness,
        "counts": counts,
        "rate_per_second": rate,
        "eta_seconds": eta,
        "started_at_seconds": started,
        "updated_at_seconds": updated,
        "completed_at_seconds": completed,
    }))
}

fn project_counts(value: &Value) -> Result<Value, DesktopError> {
    let object = exact_object(
        value,
        &[
            "discovered",
            "searchable",
            "non_resume",
            "needs_review",
            "ocr",
            "failed",
            "ignored",
            "processed",
            "total",
            "errors",
        ],
    )?;
    let discovered = safe_count(object.get("discovered"))?;
    let searchable = safe_count(object.get("searchable"))?;
    let non_resume = safe_count(object.get("non_resume"))?;
    let needs_review = safe_count(object.get("needs_review"))?;
    let ocr = safe_count(object.get("ocr"))?;
    let failed = safe_count(object.get("failed"))?;
    let ignored = safe_count(object.get("ignored"))?;
    let processed = safe_count(object.get("processed"))?;
    let total = nullable_count(object.get("total"))?;
    let errors = safe_count(object.get("errors"))?;
    if total.is_some_and(|total| processed > total) {
        return Err(protocol_error());
    }
    Ok(serde_json::json!({
        "discovered": discovered,
        "searchable": searchable,
        "non_resume": non_resume,
        "needs_review": needs_review,
        "ocr": ocr,
        "failed": failed,
        "ignored": ignored,
        "processed": processed,
        "total": total,
        "errors": errors,
    }))
}

fn exact_object<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, DesktopError> {
    let object = value.as_object().ok_or_else(protocol_error)?;
    if object.len() != keys.len() || !keys.iter().all(|key| object.contains_key(*key)) {
        return Err(protocol_error());
    }
    Ok(object)
}

fn closed_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    allowed: &[&str],
) -> Result<&'a str, DesktopError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| allowed.contains(value))
        .ok_or_else(protocol_error)
}

fn safe_count(value: Option<&Value>) -> Result<u64, DesktopError> {
    value
        .and_then(Value::as_u64)
        .filter(|value| *value <= MAX_SAFE_INTEGER)
        .ok_or_else(protocol_error)
}

fn nullable_count(value: Option<&Value>) -> Result<Option<u64>, DesktopError> {
    match value {
        Some(Value::Null) => Ok(None),
        Some(value) => safe_count(Some(value)).map(Some),
        None => Err(protocol_error()),
    }
}

fn nullable_rate(value: Option<&Value>) -> Result<Option<f64>, DesktopError> {
    match value {
        Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|value| value.is_finite() && *value > 0.0)
            .map(Some)
            .ok_or_else(protocol_error),
        None => Err(protocol_error()),
    }
}
