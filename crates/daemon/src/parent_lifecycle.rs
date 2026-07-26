use std::io::{self, Read};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

#[cfg(unix)]
use process_containment::CurrentProcessGroupLeader;

use crate::daemon_error::{DaemonError, Result};

#[cfg(unix)]
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ParentLifecycleMode {
    #[default]
    Unmanaged,
    Stdin,
}

pub(crate) fn start(mode: ParentLifecycleMode) -> Result<Option<Arc<AtomicBool>>> {
    if mode != ParentLifecycleMode::Stdin {
        return Ok(None);
    }
    #[cfg(unix)]
    let process_group_leader = CurrentProcessGroupLeader::acquire().map_err(|_| {
        DaemonError::user("parent lifecycle stdin requires an isolated process group")
    })?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let watcher_shutdown = Arc::clone(&shutdown);
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buffer = [0_u8; 1];
        loop {
            match stdin.read(&mut buffer) {
                Ok(0) | Err(_) => {
                    watcher_shutdown.store(true, Ordering::Release);
                    #[cfg(unix)]
                    {
                        thread::sleep(SHUTDOWN_GRACE);
                        let _ = process_group_leader.kill_process_group();
                    }
                    return;
                }
                Ok(_) => {}
            }
        }
    });
    Ok(Some(shutdown))
}
