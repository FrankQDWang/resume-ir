//! Daemon lifecycle skeleton for local resume indexing.

use meta_store::MetadataStore;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Returns the crate name for smoke tests and workspace metadata.
#[must_use]
pub fn crate_name() -> &'static str {
    "resume-daemon"
}

/// Runs the daemon command with explicit arguments and output sink.
pub fn run_with_args<I, S, W>(args: I, output: &mut W) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
    W: Write,
{
    let options = DaemonOptions::parse(args)?;
    if !options.foreground {
        return Err(
            "Daemon currently supports foreground mode only. Use --foreground.".to_string(),
        );
    }

    let store = open_store(&options.data_dir)?;
    let status = store
        .status()
        .map_err(|error| error.user_message().to_string())?;
    writeln!(output, "daemon foreground started").map_err(|error| error.to_string())?;
    writeln!(output, "metadata schema: {}", status.schema_version)
        .map_err(|error| error.to_string())?;
    writeln!(
        output,
        "queued imports: {}",
        status.queued_import_task_count
    )
    .map_err(|error| error.to_string())?;

    if options.once {
        writeln!(output, "daemon foreground stopped").map_err(|error| error.to_string())?;
    } else {
        writeln!(output, "daemon foreground skeleton exited").map_err(|error| error.to_string())?;
    }
    Ok(())
}

struct DaemonOptions {
    data_dir: PathBuf,
    foreground: bool,
    once: bool,
}

impl DaemonOptions {
    fn parse<I, S>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut data_dir = PathBuf::from("local-data");
        let mut foreground = false;
        let mut once = false;
        let mut args = args.into_iter();
        let _program = args.next();

        while let Some(arg) = args.next() {
            match arg.as_ref() {
                "--data-dir" => {
                    let Some(value) = args.next() else {
                        return Err("Missing value for --data-dir.".to_string());
                    };
                    data_dir = PathBuf::from(value.as_ref());
                }
                "--foreground" => foreground = true,
                "--once" => once = true,
                _ => {
                    return Err(
                        "Usage: resume-daemon [--data-dir <path>] --foreground [--once]"
                            .to_string(),
                    );
                }
            }
        }

        Ok(Self {
            data_dir,
            foreground,
            once,
        })
    }
}

fn open_store(data_dir: &Path) -> Result<MetadataStore, String> {
    fs::create_dir_all(data_dir)
        .map_err(|error| format!("Could not create local data directory: {error}"))?;
    let store = MetadataStore::open(data_dir.join("metadata.sqlite"))
        .map_err(|error| error.user_message().to_string())?;
    store
        .run_migrations()
        .map_err(|error| error.user_message().to_string())?;
    Ok(store)
}

#[cfg(test)]
mod tests {
    use super::run_with_args;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn foreground_once_initializes_store_and_exits() -> Result<(), String> {
        let data_dir = unique_data_dir("daemon")?;
        let mut output = Vec::new();

        run_with_args(
            [
                "resume-daemon",
                "--data-dir",
                data_dir.as_str(),
                "--foreground",
                "--once",
            ],
            &mut output,
        )?;

        let text = String::from_utf8(output).map_err(|error| error.to_string())?;
        assert!(text.contains("daemon foreground started"));
        assert!(text.contains("metadata schema: 2"));
        assert!(text.contains("daemon foreground stopped"));
        assert!(data_dir.join("metadata.sqlite").is_file());
        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn foreground_without_once_is_supported_by_skeleton() -> Result<(), String> {
        let data_dir = unique_data_dir("daemon-foreground")?;
        let mut output = Vec::new();

        run_with_args(
            [
                "resume-daemon",
                "--data-dir",
                data_dir.as_str(),
                "--foreground",
            ],
            &mut output,
        )?;

        let text = String::from_utf8(output).map_err(|error| error.to_string())?;
        assert!(text.contains("daemon foreground started"));
        assert!(text.contains("daemon foreground skeleton exited"));
        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    fn unique_data_dir(label: &str) -> Result<TestPath, String> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "resume-daemon-{label}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        Ok(TestPath(path))
    }

    struct TestPath(std::path::PathBuf);

    impl TestPath {
        fn join(&self, path: &str) -> Self {
            Self(self.0.join(path))
        }

        fn as_str(&self) -> &str {
            self.0.to_str().unwrap_or("")
        }

        fn is_file(&self) -> bool {
            self.0.is_file()
        }
    }

    impl AsRef<std::path::Path> for TestPath {
        fn as_ref(&self) -> &std::path::Path {
            &self.0
        }
    }
}
