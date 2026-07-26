use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Component, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::daemon_client::{read_bounded_http_response, DesktopError};
use crate::daemon_connection::{self, ConnectionGenerationSource, DaemonConnection};
use crate::daemon_exchange::SearchSelection;

const MAX_RESPONSE_BYTES: u64 = 128 * 1024;
const SOURCE_FILE_VERIFICATION_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Serialize)]
struct RevealTargetRequest<'a> {
    schema_version: &'static str,
    request_id: &'a str,
    selection: &'a SearchSelection,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevealTargetResponse {
    schema_version: String,
    request_id: String,
    status: String,
    path: String,
    byte_size: u64,
    content_hash: String,
}

#[derive(Serialize)]
pub(crate) struct RevealReceipt {
    schema_version: &'static str,
    status: &'static str,
}

pub(crate) fn reveal(
    data_dir: &std::path::Path,
    generation_source: &impl ConnectionGenerationSource,
    selection: &SearchSelection,
) -> Result<RevealReceipt, DesktopError> {
    if !selection.is_valid() {
        return Err(DesktopError::new(
            "source_reveal_invalid",
            "无法定位该来源文件",
        ));
    }
    let request_id = format!("native-reveal-{}", uuid::Uuid::new_v4());
    let body = serde_json::to_vec(&RevealTargetRequest {
        schema_version: "resume-ir.source-reveal-request.v1",
        request_id: &request_id,
        selection,
    })
    .map_err(|_| DesktopError::internal())?;
    let target =
        daemon_connection::with_connection_lease(data_dir, generation_source, |connection| {
            resolve_target(connection, &request_id, &body)
        })?;
    validate_native_reveal_target(&target)?;
    tauri_plugin_opener::reveal_item_in_dir(&target.path)
        .map_err(|_| DesktopError::new("source_reveal_failed", "无法在系统中显示来源文件"))?;
    Ok(RevealReceipt {
        schema_version: "resume-ir.source-reveal.v1",
        status: "revealed",
    })
}

fn resolve_target(
    connection: &DaemonConnection,
    request_id: &str,
    body: &[u8],
) -> Result<NativeRevealTarget, DesktopError> {
    let mut stream = TcpStream::connect_timeout(&connection.addr(), Duration::from_millis(500))
        .map_err(|_| DesktopError::new("daemon_unavailable", "无法连接本地 daemon"))?;
    stream
        .set_read_timeout(Some(SOURCE_FILE_VERIFICATION_TIMEOUT))
        .map_err(|_| DesktopError::new("daemon_unavailable", "无法配置来源定位时限"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .map_err(|_| DesktopError::new("daemon_unavailable", "无法配置来源定位时限"))?;
    write!(
        stream,
        "POST /source-reveal/resolve HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        connection.addr(),
        connection.token(),
        body.len(),
    )
    .and_then(|()| stream.write_all(body))
    .map_err(|_| DesktopError::new("daemon_unavailable", "无法发送来源定位请求"))?;
    let response = read_bounded_http_response(&mut stream, MAX_RESPONSE_BYTES)?;
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| DesktopError::new("daemon_protocol", "来源定位响应无效"))?;
    let head = std::str::from_utf8(&response[..split])
        .map_err(|_| DesktopError::new("daemon_protocol", "来源定位响应无效"))?;
    if !head.starts_with("HTTP/1.1 200 ") {
        return Err(DesktopError::new(
            "source_reveal_unavailable",
            "来源文件已移动、变化或不再可用",
        ));
    }
    let value: RevealTargetResponse = serde_json::from_slice(&response[split + 4..])
        .map_err(|_| DesktopError::new("daemon_protocol", "来源定位响应无效"))?;
    if value.schema_version != "resume-ir.source-reveal-target.v1"
        || value.request_id != request_id
        || value.status != "ok"
        || value.path.is_empty()
        || value.path.len() > 128 * 1024
        || value.path.contains('\0')
        || value.byte_size == 0
        || value.byte_size > 256 * 1024 * 1024
        || !valid_sha256_digest(&value.content_hash)
    {
        return Err(DesktopError::new("daemon_protocol", "来源定位响应无效"));
    }
    let path = PathBuf::from(value.path);
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(DesktopError::new("daemon_protocol", "来源定位响应无效"));
    }
    Ok(NativeRevealTarget {
        path,
        byte_size: value.byte_size,
        content_hash: value.content_hash,
    })
}

struct NativeRevealTarget {
    path: PathBuf,
    byte_size: u64,
    content_hash: String,
}

fn validate_native_reveal_target(target: &NativeRevealTarget) -> Result<(), DesktopError> {
    let mut observed = PathBuf::new();
    let mut components = target.path.components().peekable();
    while let Some(component) = components.next() {
        match component {
            Component::Prefix(value) => observed.push(value.as_os_str()),
            Component::RootDir => observed.push(component.as_os_str()),
            Component::Normal(value) => observed.push(value),
            Component::CurDir | Component::ParentDir => {
                return Err(DesktopError::new(
                    "source_reveal_unavailable",
                    "来源文件已移动、变化或不再可用",
                ));
            }
        }
        if !matches!(component, Component::Normal(_)) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&observed).map_err(|_| {
            DesktopError::new(
                "source_reveal_unavailable",
                "来源文件已移动、变化或不再可用",
            )
        })?;
        if native_path_component_is_link(&metadata)
            || (components.peek().is_some() && !metadata.is_dir())
            || (components.peek().is_none() && !metadata.is_file())
        {
            return Err(DesktopError::new(
                "source_reveal_unavailable",
                "来源文件已移动、变化或不再可用",
            ));
        }
    }
    let mut file = std::fs::File::open(&target.path).map_err(|_| {
        DesktopError::new(
            "source_reveal_unavailable",
            "来源文件已移动、变化或不再可用",
        )
    })?;
    let metadata = file.metadata().map_err(|_| {
        DesktopError::new(
            "source_reveal_unavailable",
            "来源文件已移动、变化或不再可用",
        )
    })?;
    if !metadata.is_file() || metadata.len() != target.byte_size {
        return Err(DesktopError::new(
            "source_reveal_unavailable",
            "来源文件已移动、变化或不再可用",
        ));
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut observed_bytes = 0_u64;
    loop {
        let read = file.read(&mut buffer).map_err(|_| {
            DesktopError::new(
                "source_reveal_unavailable",
                "来源文件已移动、变化或不再可用",
            )
        })?;
        if read == 0 {
            break;
        }
        observed_bytes = observed_bytes
            .checked_add(u64::try_from(read).map_err(|_| DesktopError::internal())?)
            .ok_or_else(DesktopError::internal)?;
        if observed_bytes > target.byte_size {
            return Err(DesktopError::new(
                "source_reveal_unavailable",
                "来源文件已移动、变化或不再可用",
            ));
        }
        hasher.update(&buffer[..read]);
    }
    let digest = format!("sha256:{:x}", hasher.finalize());
    if observed_bytes != target.byte_size || digest != target.content_hash {
        return Err(DesktopError::new(
            "source_reveal_unavailable",
            "来源文件已移动、变化或不再可用",
        ));
    }
    Ok(())
}

fn valid_sha256_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(not(windows))]
fn native_path_component_is_link(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn native_path_component_is_link(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}
