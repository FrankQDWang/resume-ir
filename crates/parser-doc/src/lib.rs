use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parser_common::{
    FileProbe, ParseInput, ParseOutput, ParseStatus, Parser, ParserError, ResourceBudget, Result,
    SupportLevel,
};

const DOC_CONVERTER_ENV: &str = "RESUME_IR_DOC_TEXT_COMMAND";
const DOC_CONVERTER_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CONVERTED_TEXT_BYTES: u64 = 1_000_000;

pub fn crate_name() -> &'static str {
    "parser-doc"
}

#[derive(Clone, Default)]
pub struct DocParser {
    converter: Option<PathBuf>,
}

impl DocParser {
    pub fn with_converter(converter: impl Into<PathBuf>) -> Self {
        Self {
            converter: Some(converter.into()),
        }
    }
}

impl fmt::Debug for DocParser {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DocParser")
            .field("converter", &self.converter.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl Parser for DocParser {
    fn supports(&self, probe: &FileProbe) -> SupportLevel {
        match (probe.extension(), probe.has_ole_header()) {
            (Some("doc"), true) => SupportLevel::Supported,
            (Some("doc"), false) | (_, true) => SupportLevel::Possible,
            _ => SupportLevel::Unsupported,
        }
    }

    fn parse(&self, input: ParseInput<'_>, budget: ResourceBudget) -> Result<ParseOutput> {
        let parse_budget = budget.begin(input.bytes().len())?;

        if self.supports(input.probe()) == SupportLevel::Unsupported {
            return Err(ParserError::unsupported(
                "doc parser received unsupported probe",
            ));
        }
        if !input.probe().has_ole_header() {
            return Err(ParserError::corrupted("doc OLE header is missing"));
        }

        let converter = self
            .converter
            .clone()
            .or_else(default_converter)
            .ok_or_else(|| ParserError::unsupported("doc converter is unavailable"))?;
        let temp = DocTempDir::new()?;
        let input_path = temp.path().join("input.doc");
        let output_path = temp.path().join("output.txt");
        fs::write(&input_path, input.bytes())
            .map_err(|_| ParserError::io("doc parser could not write private temp input"))?;

        parse_budget.check_deadline()?;
        run_converter(&converter, &input_path, &output_path)?;
        parse_budget.check_deadline()?;
        let text = read_converted_text(&output_path)?;
        if text.trim().is_empty() {
            return Err(ParserError::corrupted("doc converter produced empty text"));
        }

        Ok(ParseOutput::new(
            ParseStatus::TextExtracted,
            trim_trailing_line_breaks(&text).to_string(),
        ))
    }
}

fn default_converter() -> Option<PathBuf> {
    if let Some(command) = std::env::var_os(DOC_CONVERTER_ENV)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
    {
        return Some(command);
    }

    #[cfg(target_os = "macos")]
    {
        let textutil = PathBuf::from("/usr/bin/textutil");
        if is_executable_file(&textutil) {
            return Some(textutil);
        }
    }

    None
}

fn run_converter(converter: &Path, input_path: &Path, output_path: &Path) -> Result<()> {
    let mut child = Command::new(converter)
        .args(["-convert", "txt", "-encoding", "UTF-8", "-output"])
        .arg(output_path)
        .arg("--")
        .arg(input_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ParserError::io("doc converter could not start"))?;
    let deadline = Instant::now() + DOC_CONVERTER_TIMEOUT;

    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|_| ParserError::io("doc converter status check failed"))?
        {
            if status.success() && output_path.exists() {
                return Ok(());
            }
            return Err(ParserError::corrupted("doc converter failed"));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ParserError::timeout("doc converter timed out"));
        }

        std::thread::sleep(Duration::from_millis(10));
    }
}

fn read_converted_text(path: &Path) -> Result<String> {
    let metadata = fs::metadata(path)
        .map_err(|_| ParserError::io("doc parser could not read converter output metadata"))?;
    if metadata.len() > MAX_CONVERTED_TEXT_BYTES {
        return Err(ParserError::resource_exhausted(
            "doc converter output exceeds parser budget",
        ));
    }

    let bytes = fs::read(path)
        .map_err(|_| ParserError::io("doc parser could not read converter output"))?;
    String::from_utf8(bytes)
        .map_err(|_| ParserError::corrupted("doc converter output is not utf-8"))
}

fn trim_trailing_line_breaks(value: &str) -> &str {
    value.trim_end_matches(['\n', '\r'])
}

#[cfg(target_os = "macos")]
fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}

struct DocTempDir {
    path: PathBuf,
}

impl DocTempDir {
    fn new() -> Result<Self> {
        let unique = format!(
            "resume-ir-parser-doc-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| ParserError::internal("system clock is before unix epoch"))?
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir(&path)
            .map_err(|_| ParserError::io("doc parser could not create private temp directory"))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(&path)
                .map_err(|_| ParserError::io("doc parser could not inspect temp directory"))?
                .permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(&path, permissions)
                .map_err(|_| ParserError::io("doc parser could not protect temp directory"))?;
        }

        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for DocTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
