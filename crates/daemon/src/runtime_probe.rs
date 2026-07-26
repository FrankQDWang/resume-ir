use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use process_containment::ContainedChild;

use crate::ipc::OptionalRuntimeReason;

const OCR_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const OCR_PROBE_OUTPUT_MAX_BYTES: u64 = 64 * 1024;

pub(crate) fn probe_ocr_with_cancel(
    engine: &Path,
    requested_languages: &str,
    tessdata_dir: &Path,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<(), OptionalRuntimeReason> {
    probe_ocr_with_timeout(
        engine,
        requested_languages,
        tessdata_dir,
        OCR_PROBE_TIMEOUT,
        is_cancelled,
    )
}

fn probe_ocr_with_timeout(
    engine: &Path,
    requested_languages: &str,
    tessdata_dir: &Path,
    timeout: Duration,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<(), OptionalRuntimeReason> {
    if is_cancelled() {
        return Err(OptionalRuntimeReason::StartFailed);
    }
    let mut command = Command::new(engine);
    command
        .arg("--list-langs")
        .env("TESSDATA_PREFIX", tessdata_dir)
        .env("OMP_THREAD_LIMIT", "1");
    probe_ocr_command_with_timeout(command, requested_languages, timeout, is_cancelled)
}

fn probe_ocr_command_with_timeout(
    mut command: Command,
    requested_languages: &str,
    timeout: Duration,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<(), OptionalRuntimeReason> {
    if is_cancelled() {
        return Err(OptionalRuntimeReason::StartFailed);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child =
        ContainedChild::spawn(&mut command).map_err(|_| OptionalRuntimeReason::StartFailed)?;
    let stdout = child
        .take_stdout()
        .ok_or(OptionalRuntimeReason::StartFailed)?;
    let stderr = child
        .take_stderr()
        .ok_or(OptionalRuntimeReason::StartFailed)?;
    let stdout_reader = thread::spawn(move || read_bounded(stdout));
    let stderr_reader = thread::spawn(move || read_bounded(stderr));
    let deadline = Instant::now()
        .checked_add(timeout)
        .unwrap_or_else(Instant::now);
    let status = loop {
        if is_cancelled() {
            child.terminate();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(OptionalRuntimeReason::StartFailed);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) | Err(_) => {
                child.terminate();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(OptionalRuntimeReason::StartFailed);
            }
        }
    };
    let stdout = stdout_reader
        .join()
        .ok()
        .and_then(Result::ok)
        .ok_or(OptionalRuntimeReason::StartFailed)?;
    let stderr = stderr_reader
        .join()
        .ok()
        .and_then(Result::ok)
        .ok_or(OptionalRuntimeReason::StartFailed)?;
    if !status.success() {
        return Err(OptionalRuntimeReason::StartFailed);
    }
    let requested = requested_languages.split('+').collect::<Vec<_>>();
    let output = stdout
        .split(|byte| *byte == b'\n' || *byte == b'\r')
        .chain(stderr.split(|byte| *byte == b'\n' || *byte == b'\r'))
        .filter_map(|line| std::str::from_utf8(line).ok())
        .map(str::trim)
        .collect::<Vec<_>>();
    if requested.is_empty()
        || requested
            .iter()
            .any(|requested| !output.contains(requested))
    {
        return Err(OptionalRuntimeReason::StartFailed);
    }
    Ok(())
}

fn read_bounded(mut reader: impl Read) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    reader
        .by_ref()
        .take(OCR_PROBE_OUTPUT_MAX_BYTES + 1)
        .read_to_end(&mut output)?;
    if output.len() as u64 > OCR_PROBE_OUTPUT_MAX_BYTES {
        return Err(std::io::Error::other("runtime probe output exceeded bound"));
    }
    Ok(output)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn ocr_probe_distinguishes_start_failure_from_static_pack_validation() {
        let ready = shell("printf 'List of available languages (2):\\neng\\nchi_sim\\n'");
        assert_eq!(
            probe_ocr_command_with_timeout(ready, "eng+chi_sim", OCR_PROBE_TIMEOUT, &|| false,),
            Ok(())
        );

        let failed = shell("exit 1");
        assert_eq!(
            probe_ocr_command_with_timeout(failed, "eng+chi_sim", OCR_PROBE_TIMEOUT, &|| false,),
            Err(OptionalRuntimeReason::StartFailed)
        );

        let hung = shell("sleep 5");
        assert_eq!(
            probe_ocr_command_with_timeout(
                hung,
                "eng+chi_sim",
                Duration::from_millis(100),
                &|| false,
            ),
            Err(OptionalRuntimeReason::StartFailed)
        );
    }

    #[test]
    fn cancelled_ocr_probe_does_not_spawn_the_runtime() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("started");
        let command = shell(&format!("touch '{}'", marker.display()));

        assert_eq!(
            probe_ocr_command_with_timeout(command, "eng+chi_sim", OCR_PROBE_TIMEOUT, &|| true,),
            Err(OptionalRuntimeReason::StartFailed)
        );
        assert!(!marker.exists());
    }

    fn shell(body: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", body]);
        command
    }
}
