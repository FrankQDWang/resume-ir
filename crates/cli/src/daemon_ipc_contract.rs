use std::net::SocketAddr;
use std::str::FromStr;

use serde_json::Value;

mod error;
mod status;

pub(crate) use error::{parse_http_status, parse_import_service_error};
pub(crate) use status::valid_status;

pub(crate) const DISCOVERY_SCHEMA: &str = "resume-ir.daemon-ipc.v3";
pub(crate) const AUTH_SCHEMA: &str = "resume-ir.daemon-auth.v3";

const ROUTES: [(&str, &str); 10] = [
    ("status", "status"),
    ("diagnostics", "diagnostics"),
    ("imports", "imports"),
    ("import_cancel", "imports/cancel"),
    ("import_control", "imports/control"),
    ("import_progress", "imports/progress"),
    ("search", "search"),
    ("search_batch", "search/batch"),
    ("details", "details"),
    ("delete", "delete"),
];

#[derive(Clone)]
pub(crate) struct DaemonIpcAuth {
    launch_id: String,
    instance_id: String,
    token: String,
}

impl DaemonIpcAuth {
    pub(crate) fn token(&self) -> &str {
        &self.token
    }
}

pub(crate) struct DaemonIpcDiscovery {
    launch_id: String,
    instance_id: String,
    addr: SocketAddr,
}

impl DaemonIpcDiscovery {
    pub(crate) fn bind(self, auth: DaemonIpcAuth) -> Option<BoundDaemonIpc> {
        (self.launch_id == auth.launch_id && self.instance_id == auth.instance_id).then_some(
            BoundDaemonIpc {
                addr: self.addr,
                token: auth.token,
            },
        )
    }
}

pub(crate) struct BoundDaemonIpc {
    addr: SocketAddr,
    token: String,
}

impl BoundDaemonIpc {
    pub(crate) fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub(crate) fn token(&self) -> &str {
        &self.token
    }
}

pub(crate) fn parse_auth(text: &str) -> Option<DaemonIpcAuth> {
    let value: Value = serde_json::from_str(text).ok()?;
    if !has_exact_keys(
        &value,
        &["schema_version", "launch_id", "instance_id", "token"],
    ) || string(&value, "schema_version")? != AUTH_SCHEMA
    {
        return None;
    }
    let launch_id = string(&value, "launch_id")?;
    let instance_id = string(&value, "instance_id")?;
    let token = string(&value, "token")?;
    if ![launch_id, instance_id, token]
        .into_iter()
        .all(valid_generation_value)
    {
        return None;
    }
    Some(DaemonIpcAuth {
        launch_id: launch_id.to_string(),
        instance_id: instance_id.to_string(),
        token: token.to_string(),
    })
}

pub(crate) fn parse_discovery(text: &str) -> Option<DaemonIpcDiscovery> {
    let value: Value = serde_json::from_str(text).ok()?;
    let mut keys = vec!["schema_version", "launch_id", "instance_id", "owner_mode"];
    keys.extend(ROUTES.map(|(field, _)| field));
    if !has_exact_keys(&value, &keys)
        || string(&value, "schema_version")? != DISCOVERY_SCHEMA
        || !matches!(
            string(&value, "owner_mode")?,
            "standalone" | "desktop_supervised"
        )
    {
        return None;
    }
    let launch_id = string(&value, "launch_id")?;
    let instance_id = string(&value, "instance_id")?;
    if !valid_generation_value(launch_id) || !valid_generation_value(instance_id) {
        return None;
    }

    let mut addr = None;
    for (field, expected_path) in ROUTES {
        let observed = parse_loopback_url(string(&value, field)?, expected_path)?;
        if addr.is_some_and(|current| current != observed) {
            return None;
        }
        addr = Some(observed);
    }
    Some(DaemonIpcDiscovery {
        launch_id: launch_id.to_string(),
        instance_id: instance_id.to_string(),
        addr: addr?,
    })
}

fn parse_loopback_url(value: &str, expected_path: &str) -> Option<SocketAddr> {
    let rest = value.strip_prefix("http://")?;
    let (authority, path) = rest.split_once('/')?;
    if path != expected_path {
        return None;
    }
    let addr = SocketAddr::from_str(authority).ok()?;
    addr.ip().is_loopback().then_some(addr)
}

fn valid_generation_value(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn has_exact_keys(value: &Value, keys: &[&str]) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == keys.len() && object.keys().all(|field| keys.contains(&field.as_str()))
    })
}

fn string<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}

#[cfg(test)]
#[path = "daemon_ipc_contract_tests.rs"]
mod tests;
