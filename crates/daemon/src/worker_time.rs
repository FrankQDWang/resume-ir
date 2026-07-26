use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::UnixTimestamp;

use crate::daemon_error::{DaemonError, Result};

pub(crate) fn current_timestamp() -> Result<UnixTimestamp> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DaemonError::user("system clock is before unix epoch"))?
        .as_secs();
    let seconds =
        i64::try_from(seconds).map_err(|_| DaemonError::user("system timestamp is too large"))?;
    Ok(UnixTimestamp::from_unix_seconds(seconds))
}

pub(crate) fn timestamp_minus_seconds(now: UnixTimestamp, seconds: i64) -> UnixTimestamp {
    UnixTimestamp::from_unix_seconds(now.as_unix_seconds().saturating_sub(seconds))
}

pub(crate) fn timestamp_at_or_after(now: UnixTimestamp, floor: UnixTimestamp) -> UnixTimestamp {
    UnixTimestamp::from_unix_seconds(now.as_unix_seconds().max(floor.as_unix_seconds()))
}

pub(crate) fn u64_to_usize(value: u64) -> Result<usize> {
    usize::try_from(value).map_err(|_| DaemonError::user("scan budget is too large"))
}
