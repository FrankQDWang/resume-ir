//! Cross-platform ownership for bounded local worker process trees.
//!
//! Unix children run in a dedicated process group. Windows children are
//! attached to a non-breakaway Job Object with `KILL_ON_JOB_CLOSE`. Spawning
//! fails closed when containment cannot be established.

use std::fmt;
use std::io;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus};

#[cfg(unix)]
use nix::sys::signal::{killpg, Signal};
#[cfg(unix)]
use nix::unistd::{getpgrp, getpid, getppid, Pid};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(unix)]
use std::process::Stdio;
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
const GRACEFUL_TERMINATION: Duration = Duration::from_millis(100);

/// Owns a direct child and the operating-system primitive that contains all of
/// its descendants.
pub struct ContainedChild {
    child: Child,
    #[cfg(unix)]
    process_group_id: u32,
    #[cfg(windows)]
    job: Option<win32job::Job>,
    cleaned_up: bool,
}

impl ContainedChild {
    /// Spawns `command` only when the platform containment primitive is ready.
    ///
    /// On Windows, a child that cannot be assigned to its Job Object is killed
    /// and reaped before this method returns an error.
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        #[cfg(unix)]
        command.process_group(0);

        #[cfg(windows)]
        let job = create_windows_job()?;

        let child = command.spawn()?;

        #[cfg(windows)]
        let child = {
            let mut child = child;
            if let Err(error) = job.assign_process(child.as_raw_handle() as isize) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.into());
            }
            child
        };

        #[cfg(unix)]
        let process_group_id = child.id();

        Ok(Self {
            child,
            #[cfg(unix)]
            process_group_id,
            #[cfg(windows)]
            job: Some(job),
            cleaned_up: false,
        })
    }

    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }

    /// Returns the operating-system identifier of the directly owned child.
    ///
    /// The identifier is observational only; lifecycle operations must still
    /// go through this containment owner so descendants remain covered.
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        if status.is_some() {
            self.cleanup_descendants();
        }
        Ok(status)
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait()?;
        self.cleanup_descendants();
        Ok(status)
    }

    /// Terminates the contained process tree and reaps the direct child.
    pub fn terminate(&mut self) {
        if self.cleaned_up {
            let _ = self.child.wait();
            return;
        }

        #[cfg(windows)]
        {
            self.job.take();
        }

        #[cfg(unix)]
        {
            signal_process_group(self.process_group_id, UnixSignal::Term);
            wait_for_direct_child(&mut self.child, GRACEFUL_TERMINATION);
            signal_process_group(self.process_group_id, UnixSignal::Kill);
        }

        let _ = self.child.kill();
        let _ = self.child.wait();
        self.cleaned_up = true;
    }

    fn cleanup_descendants(&mut self) {
        if self.cleaned_up {
            return;
        }

        #[cfg(windows)]
        {
            self.job.take();
        }

        #[cfg(unix)]
        {
            signal_process_group(self.process_group_id, UnixSignal::Term);
            thread::sleep(Duration::from_millis(10));
            signal_process_group(self.process_group_id, UnixSignal::Kill);
        }

        self.cleaned_up = true;
    }
}

impl fmt::Debug for ContainedChild {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContainedChild")
            .field("tree_contained", &true)
            .field("cleaned_up", &self.cleaned_up)
            .finish()
    }
}

impl Drop for ContainedChild {
    fn drop(&mut self) {
        if !self.cleaned_up {
            self.terminate();
        }
    }
}

/// Proof that the current Unix process owns its process group.
///
/// Acquisition fails unless the current process ID equals its process-group
/// ID. This prevents a lifecycle watchdog from signaling an unrelated caller
/// such as a shell or test runner.
#[cfg(unix)]
pub struct CurrentProcessGroupLeader {
    process_group_id: Pid,
}

#[cfg(unix)]
impl CurrentProcessGroupLeader {
    /// Verifies that the current process is its process-group leader.
    pub fn acquire() -> io::Result<Self> {
        let process_id = getpid();
        let process_group_id = getpgrp();
        if process_id != process_group_id {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "current process does not own its process group",
            ));
        }
        Ok(Self { process_group_id })
    }

    /// Sends the final `SIGKILL` fallback to the verified current process group.
    pub fn kill_process_group(self) -> io::Result<()> {
        killpg(self.process_group_id, Signal::SIGKILL).map_err(Into::into)
    }
}

#[cfg(unix)]
impl fmt::Debug for CurrentProcessGroupLeader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CurrentProcessGroupLeader")
            .field("ownership_verified", &true)
            .finish()
    }
}

/// Captures the non-init Unix parent of a supervised leaf process.
///
/// The identity can be polled without signals or unsafe code. A changed parent
/// means the original supervisor exited and the leaf was reparented.
#[cfg(unix)]
pub struct VerifiedParentProcess {
    parent_process_id: Pid,
}

#[cfg(unix)]
impl VerifiedParentProcess {
    /// Captures the current parent, rejecting an already orphaned process.
    pub fn capture() -> io::Result<Self> {
        let parent_process_id = getppid();
        if parent_process_id.as_raw() <= 1 {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "supervised process does not have a live parent",
            ));
        }
        Ok(Self { parent_process_id })
    }

    /// Returns whether the original supervisor is still the current parent.
    pub fn is_current_parent(&self) -> bool {
        getppid() == self.parent_process_id
    }
}

#[cfg(unix)]
impl fmt::Debug for VerifiedParentProcess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedParentProcess")
            .field("parent_captured", &true)
            .finish()
    }
}

/// Owns a leaf child without creating a nested Unix process group.
///
/// Unix leaf children inherit the caller's process group so an outer
/// [`ContainedChild`] can terminate the whole tree. The leaf remains directly
/// terminable and reapable by its immediate owner. Windows leaf children keep
/// an independent kill-on-close Job Object so they cannot survive an abrupt
/// owner exit.
pub struct OwnedLeafChild {
    child: Child,
    #[cfg(windows)]
    job: Option<win32job::Job>,
    cleaned_up: bool,
}

impl OwnedLeafChild {
    /// Spawns a direct child using platform-specific leaf ownership.
    ///
    /// On Windows, a child that cannot be assigned to its Job Object is killed
    /// and reaped before this method returns an error.
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        #[cfg(windows)]
        let job = create_windows_job()?;

        let child = command.spawn()?;

        #[cfg(windows)]
        let child = {
            let mut child = child;
            if let Err(error) = job.assign_process(child.as_raw_handle() as isize) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.into());
            }
            child
        };

        Ok(Self {
            child,
            #[cfg(windows)]
            job: Some(job),
            cleaned_up: false,
        })
    }

    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        if status.is_some() {
            self.cleanup_after_exit();
        }
        Ok(status)
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait()?;
        self.cleanup_after_exit();
        Ok(status)
    }

    /// Terminates and reaps the directly owned child.
    pub fn terminate(&mut self) {
        if self.cleaned_up {
            let _ = self.child.wait();
            return;
        }

        #[cfg(windows)]
        {
            self.job.take();
        }

        #[cfg(unix)]
        {
            signal_process(self.child.id(), UnixSignal::Term);
            wait_for_direct_child(&mut self.child, GRACEFUL_TERMINATION);
        }

        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        self.cleaned_up = true;
    }

    fn cleanup_after_exit(&mut self) {
        if self.cleaned_up {
            return;
        }

        #[cfg(windows)]
        {
            self.job.take();
        }

        self.cleaned_up = true;
    }
}

impl fmt::Debug for OwnedLeafChild {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedLeafChild")
            .field("direct_child_owned", &true)
            .field("cleaned_up", &self.cleaned_up)
            .finish()
    }
}

impl Drop for OwnedLeafChild {
    fn drop(&mut self) {
        if !self.cleaned_up {
            self.terminate();
        }
    }
}

#[cfg(windows)]
fn create_windows_job() -> io::Result<win32job::Job> {
    let mut limits = win32job::ExtendedLimitInfo::new();
    limits.limit_kill_on_job_close();
    win32job::Job::create_with_limit_info(&limits).map_err(Into::into)
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum UnixSignal {
    Term,
    Kill,
}

#[cfg(unix)]
impl UnixSignal {
    fn as_kill_arg(self) -> &'static str {
        match self {
            Self::Term => "-TERM",
            Self::Kill => "-KILL",
        }
    }
}

#[cfg(unix)]
fn signal_process_group(process_group_id: u32, signal: UnixSignal) {
    let _ = Command::new("/bin/kill")
        .arg(signal.as_kill_arg())
        .arg("--")
        .arg(format!("-{process_group_id}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(unix)]
fn signal_process(process_id: u32, signal: UnixSignal) {
    let _ = Command::new("/bin/kill")
        .arg(signal.as_kill_arg())
        .arg("--")
        .arg(process_id.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(unix)]
fn wait_for_direct_child(child: &mut Child, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::io::{BufRead, BufReader, Read, Write};
    #[cfg(unix)]
    use std::path::Path;
    #[cfg(unix)]
    use std::process::Stdio;
    #[test]
    fn debug_output_does_not_expose_process_identity() {
        let mut command = immediate_success_command();
        let mut child = ContainedChild::spawn(&mut command).unwrap();
        let output = format!("{child:?}");
        assert_eq!(
            output,
            "ContainedChild { tree_contained: true, cleaned_up: false }"
        );
        child.terminate();
    }

    #[test]
    fn successful_child_is_reaped_with_its_containment() {
        let mut command = immediate_success_command();
        let mut child = ContainedChild::spawn(&mut command).unwrap();
        assert!(child.wait().unwrap().success());
    }

    #[cfg(unix)]
    #[test]
    fn termination_reaps_a_process_group_without_waiting_for_descendants() {
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("sleep 30 & wait")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = ContainedChild::spawn(&mut command).unwrap();
        let started = Instant::now();
        child.terminate();
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn owned_leaf_inherits_callers_process_group_and_can_be_reaped() {
        let parent_process_group = process_group_id(std::process::id());
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "trap '' TERM; printf 'ready\\n'; exec /bin/sleep 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = OwnedLeafChild::spawn(&mut command).unwrap();
        let child_process_id = child.child.id();
        let mut ready = String::new();
        BufReader::new(child.take_stdout().unwrap())
            .read_line(&mut ready)
            .unwrap();
        assert_eq!(ready, "ready\n");

        assert_eq!(process_group_id(child_process_id), parent_process_group);
        let started = Instant::now();
        child.terminate();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(child.child.try_wait().unwrap().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn group_leader_watchdog_terminates_an_owned_leaf_after_stdin_eof() {
        let marker = std::env::temp_dir().join(format!(
            "resume-ir-process-group-leaf-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&marker);
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "trap '' TERM; exec \"$RESUME_IR_PROCESS_GROUP_HELPER_EXE\" --exact tests::process_group_termination_helper --nocapture",
            ])
            .env("RESUME_IR_PROCESS_GROUP_HELPER", "1")
            .env(
                "RESUME_IR_PROCESS_GROUP_HELPER_EXE",
                std::env::current_exe().unwrap(),
            )
            .env("RESUME_IR_PROCESS_GROUP_LEAF_MARKER", &marker)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command.process_group(0);
        let mut child = command.spawn().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            assert_ne!(stdout.read_line(&mut line).unwrap(), 0);
            if line.contains("process group helper ready") {
                break;
            }
        }
        drop(child.stdin.take());

        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                signal_process_group(child.id(), UnixSignal::Kill);
                let _ = child.kill();
                let _ = child.wait();
                panic!("watchdog did not terminate group");
            }
            thread::sleep(Duration::from_millis(10));
        };
        assert!(!status.success());
        wait_until_process_exits(&marker);
        let _ = std::fs::remove_file(marker);
    }

    #[cfg(unix)]
    #[test]
    fn process_group_termination_helper() {
        if std::env::var_os("RESUME_IR_PROCESS_GROUP_HELPER").is_none() {
            return;
        }
        let leader = CurrentProcessGroupLeader::acquire().unwrap();
        let marker = std::env::var_os("RESUME_IR_PROCESS_GROUP_LEAF_MARKER").unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "trap '' TERM; printf '%s' \"$$\" > \"$RESUME_IR_PROCESS_GROUP_LEAF_MARKER\"; exec /bin/sleep 30",
            ])
            .env("RESUME_IR_PROCESS_GROUP_LEAF_MARKER", &marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let _leaf = OwnedLeafChild::spawn(&mut command).unwrap();
        let marker = Path::new(&marker);
        while !marker.exists() {
            thread::sleep(Duration::from_millis(5));
        }
        println!("process group helper ready");
        io::stdout().flush().unwrap();
        let _ = io::stdin().read(&mut [0_u8; 1]);
        thread::sleep(Duration::from_millis(50));
        leader.kill_process_group().unwrap();
        thread::sleep(Duration::from_secs(5));
        panic!("process group termination returned without terminating helper");
    }

    #[cfg(unix)]
    #[test]
    fn verified_parent_identity_detects_reparenting() {
        let base =
            std::env::temp_dir().join(format!("resume-ir-parent-identity-{}", std::process::id()));
        let ready = base.with_extension("ready");
        let changed = base.with_extension("changed");
        let child_process_id = base.with_extension("pid");
        for path in [&ready, &changed, &child_process_id] {
            let _ = std::fs::remove_file(path);
        }
        let status = Command::new("/bin/sh")
            .args([
                "-c",
                "\"$RESUME_IR_PARENT_HELPER_EXE\" --exact tests::parent_identity_change_helper --nocapture >/dev/null 2>/dev/null & child=$!; printf '%s' \"$child\" > \"$RESUME_IR_PARENT_PID\"; attempts=0; while [ ! -f \"$RESUME_IR_PARENT_READY\" ] && [ \"$attempts\" -lt 200 ]; do attempts=$((attempts + 1)); sleep 0.01; done; test -f \"$RESUME_IR_PARENT_READY\"",
            ])
            .env("RESUME_IR_PARENT_IDENTITY_HELPER", "1")
            .env("RESUME_IR_PARENT_HELPER_EXE", std::env::current_exe().unwrap())
            .env("RESUME_IR_PARENT_READY", &ready)
            .env("RESUME_IR_PARENT_CHANGED", &changed)
            .env("RESUME_IR_PARENT_PID", &child_process_id)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());

        let deadline = Instant::now() + Duration::from_secs(2);
        while !changed.exists() {
            if Instant::now() >= deadline {
                let process_id = std::fs::read_to_string(&child_process_id).unwrap_or_default();
                let _ = Command::new("/bin/kill")
                    .args(["-KILL", "--", process_id.trim()])
                    .status();
                panic!("parent identity did not detect reparenting");
            }
            thread::sleep(Duration::from_millis(10));
        }
        for path in [&ready, &changed, &child_process_id] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[cfg(unix)]
    #[test]
    fn parent_identity_change_helper() {
        if std::env::var_os("RESUME_IR_PARENT_IDENTITY_HELPER").is_none() {
            return;
        }
        let parent = VerifiedParentProcess::capture().unwrap();
        std::fs::write(
            std::env::var_os("RESUME_IR_PARENT_READY").unwrap(),
            b"ready",
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while parent.is_current_parent() {
            assert!(Instant::now() < deadline, "parent identity did not change");
            thread::sleep(Duration::from_millis(10));
        }
        std::fs::write(
            std::env::var_os("RESUME_IR_PARENT_CHANGED").unwrap(),
            b"changed",
        )
        .unwrap();
    }

    #[cfg(unix)]
    fn wait_until_process_exits(marker: &Path) {
        let process_id = std::fs::read_to_string(marker).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while Command::new("/bin/kill")
            .args(["-0", "--", process_id.trim()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        {
            assert!(Instant::now() < deadline, "owned leaf survived group exit");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(unix)]
    fn process_group_id(process_id: u32) -> u32 {
        let output = Command::new("/bin/ps")
            .args(["-o", "pgid=", "-p", &process_id.to_string()])
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    fn immediate_success_command() -> Command {
        #[cfg(windows)]
        {
            let mut command = Command::new("cmd.exe");
            command.args(["/C", "exit", "0"]);
            command
        }
        #[cfg(not(windows))]
        {
            let mut command = Command::new("/usr/bin/true");
            command.stdin(Stdio::null());
            command
        }
    }
}
