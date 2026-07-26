use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ImportServiceError {
    Initializing,
    Blocked,
    CapabilityUnavailable,
}

impl ImportServiceError {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::Initializing => "daemon import ipc is initializing",
            Self::Blocked => "daemon import ipc is blocked",
            Self::CapabilityUnavailable => "daemon import ipc capability unavailable",
        }
    }
}

pub(crate) fn parse_http_status(status_line: &str) -> Option<u16> {
    let mut parts = status_line.split_ascii_whitespace();
    if !matches!(parts.next()?, "HTTP/1.0" | "HTTP/1.1") {
        return None;
    }
    let status = parts.next()?.parse().ok()?;
    (100..=599).contains(&status).then_some(status)
}

pub(crate) fn parse_import_service_error(
    body: &str,
    http_status: u16,
) -> Option<ImportServiceError> {
    if http_status != 503 {
        return None;
    }
    let body: Value = serde_json::from_str(body).ok()?;
    if !has_exact_keys(&body, &["schema_version", "status", "error"])
        || string(&body, "schema_version") != Some("resume-ir.error.v2")
        || string(&body, "status") != Some("error")
    {
        return None;
    }
    let error = body.get("error")?;
    if !has_exact_keys(error, &["code", "action", "capability", "reason"]) {
        return None;
    }
    match (
        string(error, "code")?,
        string(error, "action")?,
        nullable_string(error, "capability")?,
        nullable_string(error, "reason")?,
    ) {
        (
            "SERVICE_INITIALIZING",
            "wait_for_service",
            None,
            Some("metadata_initializing" | "migration_rebuild" | "artifact_unavailable"),
        ) => Some(ImportServiceError::Initializing),
        (
            "SERVICE_BLOCKED",
            "repair_required" | "retry",
            None,
            Some(
                "source_unavailable"
                | "runtime_invariant"
                | "unsupported_store_schema"
                | "metadata_unavailable",
            ),
        ) => Some(ImportServiceError::Blocked),
        (
            "CAPABILITY_UNAVAILABLE",
            "select_supported_mode",
            Some("text_import"),
            Some("embedding_unavailable" | "classifier_unavailable"),
        ) => Some(ImportServiceError::CapabilityUnavailable),
        _ => None,
    }
}

fn has_exact_keys(value: &Value, keys: &[&str]) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == keys.len() && object.keys().all(|field| keys.contains(&field.as_str()))
    })
}

fn string<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}

fn nullable_string<'a>(value: &'a Value, field: &str) -> Option<Option<&'a str>> {
    match value.get(field)? {
        Value::Null => Some(None),
        Value::String(value) => Some(Some(value)),
        _ => None,
    }
}
