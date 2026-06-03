pub fn crate_name() -> &'static str {
    "embedder"
}

use std::{
    collections::BTreeMap,
    fmt,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::{
    fs::{DirBuilderExt, OpenOptionsExt},
    process::CommandExt,
};

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const EMBEDDING_OUTPUT_MAX_BYTES: usize = 4 * 1024 * 1024;
const EMBEDDING_POLL_INTERVAL_MS: u64 = 10;
static EMBEDDING_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub trait Embedder {
    fn model_id(&self) -> &str;
    fn dimension(&self) -> usize;
    fn embed_batch(
        &self,
        inputs: &[EmbeddingInput],
        budget: EmbeddingBudget,
    ) -> Result<Vec<EmbeddingVector>, EmbeddingError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmbeddingBudget {
    max_inputs: usize,
    max_text_bytes: usize,
}

impl EmbeddingBudget {
    pub fn new(max_inputs: usize, max_text_bytes: usize) -> Self {
        Self {
            max_inputs,
            max_text_bytes,
        }
    }

    pub fn max_inputs(self) -> usize {
        self.max_inputs
    }

    pub fn max_text_bytes(self) -> usize {
        self.max_text_bytes
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct EmbeddingInput {
    id: String,
    text: String,
}

impl EmbeddingInput {
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

impl fmt::Debug for EmbeddingInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbeddingInput")
            .field("id", &self.id)
            .field("text", &"<redacted>")
            .field("text_bytes", &self.text.len())
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct EmbeddingVector {
    id: String,
    model_id: String,
    values: Vec<f32>,
}

impl EmbeddingVector {
    pub fn new(
        id: impl Into<String>,
        model_id: impl Into<String>,
        values: Vec<f32>,
    ) -> Result<Self, EmbeddingError> {
        if values.is_empty() {
            return Err(EmbeddingError::InvalidDimension);
        }

        Ok(Self {
            id: id.into(),
            model_id: model_id.into(),
            values,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }
}

impl fmt::Debug for EmbeddingVector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbeddingVector")
            .field("id", &self.id)
            .field("model_id", &self.model_id)
            .field("dimension", &self.values.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LocalEmbeddingCommandSpec {
    program: PathBuf,
    args: Vec<String>,
    model_id: String,
    dimension: usize,
    timeout_ms: u64,
}

impl LocalEmbeddingCommandSpec {
    pub fn new<I, S>(
        program: impl Into<PathBuf>,
        args: I,
        model_id: impl Into<String>,
        dimension: usize,
    ) -> Result<Self, EmbeddingError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let program = program.into();
        let model_id = model_id.into();
        if program.as_os_str().is_empty() || model_id.trim().is_empty() || dimension == 0 {
            return Err(EmbeddingError::InvalidRequest);
        }

        Ok(Self {
            program,
            args: args.into_iter().map(Into::into).collect(),
            model_id,
            dimension,
            timeout_ms: 30_000,
        })
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Result<Self, EmbeddingError> {
        if timeout_ms == 0 {
            return Err(EmbeddingError::InvalidRequest);
        }
        self.timeout_ms = timeout_ms;
        Ok(self)
    }
}

impl fmt::Debug for LocalEmbeddingCommandSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalEmbeddingCommandSpec")
            .field("program", &"<redacted>")
            .field("args_count", &self.args.len())
            .field("model_id", &self.model_id)
            .field("dimension", &self.dimension)
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalEmbeddingCommandEmbedder {
    spec: LocalEmbeddingCommandSpec,
}

impl LocalEmbeddingCommandEmbedder {
    pub fn new(spec: LocalEmbeddingCommandSpec) -> Self {
        Self { spec }
    }
}

impl Embedder for LocalEmbeddingCommandEmbedder {
    fn model_id(&self) -> &str {
        &self.spec.model_id
    }

    fn dimension(&self) -> usize {
        self.spec.dimension
    }

    fn embed_batch(
        &self,
        inputs: &[EmbeddingInput],
        budget: EmbeddingBudget,
    ) -> Result<Vec<EmbeddingVector>, EmbeddingError> {
        validate_embedding_inputs(inputs, budget)?;
        let input = EmbeddingTempInput::write(self, inputs)?;
        let mut child = spawn_embedding_command(&self.spec, input.path(), inputs.len())?;
        let stdout = child.stdout.take().ok_or(EmbeddingError::EngineFailed)?;
        let stderr = child.stderr.take().ok_or(EmbeddingError::EngineFailed)?;
        let stdout_reader = spawn_output_reader(stdout, EMBEDDING_OUTPUT_MAX_BYTES);
        let stderr_reader = spawn_output_reader(stderr, EMBEDDING_OUTPUT_MAX_BYTES);

        let status = match wait_for_embedding_child(&mut child, self.spec.timeout_ms) {
            Ok(status) => status,
            Err(error) => {
                let _ = join_output_reader(stdout_reader);
                let _ = join_output_reader(stderr_reader);
                return Err(error);
            }
        };
        #[cfg(unix)]
        terminate_process_group(child.id());
        let stdout = join_output_reader(stdout_reader)?;
        let _stderr = join_output_reader(stderr_reader)?;
        if !status.success() {
            return Err(EmbeddingError::EngineFailed);
        }

        parse_embedding_output(inputs, &stdout, self.model_id(), self.dimension())
    }
}

/// Deterministic local embedder for tests and interface wiring only.
///
/// It is a lexical hash vectorizer, not a licensed model and not a semantic
/// quality claim.
#[derive(Clone, PartialEq)]
pub struct DeterministicTestEmbedder {
    model_id: String,
    dimension: usize,
}

impl DeterministicTestEmbedder {
    pub fn new(model_id: impl Into<String>, dimension: usize) -> Result<Self, EmbeddingError> {
        if dimension == 0 {
            return Err(EmbeddingError::InvalidDimension);
        }

        Ok(Self {
            model_id: model_id.into(),
            dimension,
        })
    }
}

impl fmt::Debug for DeterministicTestEmbedder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeterministicTestEmbedder")
            .field("model_id", &self.model_id)
            .field("dimension", &self.dimension)
            .finish()
    }
}

impl Embedder for DeterministicTestEmbedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn embed_batch(
        &self,
        inputs: &[EmbeddingInput],
        budget: EmbeddingBudget,
    ) -> Result<Vec<EmbeddingVector>, EmbeddingError> {
        validate_embedding_inputs(inputs, budget)?;

        inputs
            .iter()
            .map(|input| {
                EmbeddingVector::new(
                    input.id(),
                    self.model_id(),
                    deterministic_values(input.text(), self.dimension),
                )
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmbeddingError {
    InvalidDimension,
    InvalidRequest,
    WorkerUnavailable,
    EngineFailed,
    Timeout,
    BudgetExceeded { limit: usize, actual: usize },
    TextBudgetExceeded { limit: usize, actual: usize },
}

impl fmt::Display for EmbeddingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimension => formatter.write_str("embedding dimension must be positive"),
            Self::InvalidRequest => formatter.write_str("embedding request is invalid"),
            Self::WorkerUnavailable => formatter.write_str("embedding worker is unavailable"),
            Self::EngineFailed => formatter.write_str("embedding engine failed"),
            Self::Timeout => formatter.write_str("embedding request timed out"),
            Self::BudgetExceeded { limit, actual } => {
                write!(
                    formatter,
                    "embedding batch limit {limit} exceeded by {actual}"
                )
            }
            Self::TextBudgetExceeded { limit, actual } => {
                write!(
                    formatter,
                    "embedding text byte limit {limit} exceeded by {actual}"
                )
            }
        }
    }
}

impl std::error::Error for EmbeddingError {}

fn validate_embedding_inputs(
    inputs: &[EmbeddingInput],
    budget: EmbeddingBudget,
) -> Result<(), EmbeddingError> {
    if inputs.len() > budget.max_inputs() {
        return Err(EmbeddingError::BudgetExceeded {
            limit: budget.max_inputs(),
            actual: inputs.len(),
        });
    }

    for input in inputs {
        if input.id().trim().is_empty()
            || input.id().contains('\n')
            || input.id().contains('\r')
            || input.id().contains('\t')
        {
            return Err(EmbeddingError::InvalidRequest);
        }
        if input.text().len() > budget.max_text_bytes() {
            return Err(EmbeddingError::TextBudgetExceeded {
                limit: budget.max_text_bytes(),
                actual: input.text().len(),
            });
        }
    }

    Ok(())
}

fn spawn_embedding_command(
    spec: &LocalEmbeddingCommandSpec,
    input_path: &Path,
    input_count: usize,
) -> Result<Child, EmbeddingError> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .env("RESUME_IR_EMBEDDING_INPUT_PATH", input_path.as_os_str())
        .env("RESUME_IR_EMBEDDING_MODEL_ID", &spec.model_id)
        .env("RESUME_IR_EMBEDDING_DIMENSION", spec.dimension.to_string())
        .env("RESUME_IR_EMBEDDING_INPUT_COUNT", input_count.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_isolation(&mut command);

    command.spawn().map_err(|error| match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied => {
            EmbeddingError::WorkerUnavailable
        }
        _ => EmbeddingError::EngineFailed,
    })
}

#[cfg(unix)]
fn configure_process_isolation(command: &mut Command) {
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_isolation(_command: &mut Command) {}

fn wait_for_embedding_child(
    child: &mut Child,
    timeout_ms: u64,
) -> Result<std::process::ExitStatus, EmbeddingError> {
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(timeout_ms))
        .ok_or(EmbeddingError::InvalidRequest)?;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(_) => {
                terminate_child(child);
                return Err(EmbeddingError::EngineFailed);
            }
        }

        let now = Instant::now();
        if now >= deadline {
            terminate_child(child);
            return Err(EmbeddingError::Timeout);
        }

        thread::sleep(Duration::from_millis(EMBEDDING_POLL_INTERVAL_MS).min(deadline - now));
    }
}

fn terminate_child(child: &mut Child) {
    #[cfg(unix)]
    {
        signal_process_group(child.id(), UnixSignal::Term);
        if wait_for_child_exit(child, Duration::from_millis(100)) {
            return;
        }
        signal_process_group(child.id(), UnixSignal::Kill);
        if wait_for_child_exit(child, Duration::from_millis(100)) {
            return;
        }
    }

    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn terminate_process_group(process_group_id: u32) {
    signal_process_group(process_group_id, UnixSignal::Term);
    thread::sleep(Duration::from_millis(10));
    signal_process_group(process_group_id, UnixSignal::Kill);
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
        .arg(format!("-{process_group_id}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(unix)]
fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) => {}
            Err(_) => return true,
        }

        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn spawn_output_reader<R>(
    mut reader: R,
    max_bytes: usize,
) -> JoinHandle<Result<Vec<u8>, EmbeddingError>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut output = Vec::new();
        let mut buffer = [0_u8; 8192];
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|_| EmbeddingError::EngineFailed)?;
            if read == 0 {
                break;
            }
            if output.len().saturating_add(read) > max_bytes {
                return Err(EmbeddingError::EngineFailed);
            }
            output.extend_from_slice(&buffer[..read]);
        }
        Ok(output)
    })
}

fn join_output_reader(
    handle: JoinHandle<Result<Vec<u8>, EmbeddingError>>,
) -> Result<Vec<u8>, EmbeddingError> {
    handle.join().map_err(|_| EmbeddingError::EngineFailed)?
}

fn parse_embedding_output(
    inputs: &[EmbeddingInput],
    stdout: &[u8],
    expected_model_id: &str,
    expected_dimension: usize,
) -> Result<Vec<EmbeddingVector>, EmbeddingError> {
    let output = std::str::from_utf8(stdout).map_err(|_| EmbeddingError::EngineFailed)?;
    let mut lines = output.lines();
    if lines.next() != Some("resume-ir-embedding-v1") {
        return Err(EmbeddingError::EngineFailed);
    }
    let model_id = lines
        .next()
        .and_then(|line| line.strip_prefix("model_id="))
        .ok_or(EmbeddingError::EngineFailed)?;
    if model_id != expected_model_id {
        return Err(EmbeddingError::EngineFailed);
    }
    let dimension = lines
        .next()
        .and_then(|line| line.strip_prefix("dimension="))
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or(EmbeddingError::EngineFailed)?;
    if dimension != expected_dimension {
        return Err(EmbeddingError::EngineFailed);
    }

    let mut by_id = BTreeMap::<String, Vec<f32>>::new();
    for line in lines {
        if line.trim().is_empty() || line.starts_with("metadata=") {
            continue;
        }
        let Some(rest) = line.strip_prefix("vector=") else {
            return Err(EmbeddingError::EngineFailed);
        };
        let Some((id, values)) = rest.split_once('\t') else {
            return Err(EmbeddingError::EngineFailed);
        };
        if id.trim().is_empty() || by_id.contains_key(id) {
            return Err(EmbeddingError::EngineFailed);
        }
        let values = values
            .split(',')
            .map(|value| {
                let parsed = value
                    .parse::<f32>()
                    .map_err(|_| EmbeddingError::EngineFailed)?;
                if !parsed.is_finite() {
                    return Err(EmbeddingError::EngineFailed);
                }
                Ok(parsed)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if values.len() != expected_dimension {
            return Err(EmbeddingError::EngineFailed);
        }
        by_id.insert(id.to_string(), values);
    }

    inputs
        .iter()
        .map(|input| {
            let values = by_id
                .remove(input.id())
                .ok_or(EmbeddingError::EngineFailed)?;
            EmbeddingVector::new(input.id(), expected_model_id, values)
        })
        .collect()
}

struct EmbeddingTempInput {
    directory: PathBuf,
    path: PathBuf,
}

impl EmbeddingTempInput {
    fn write(
        embedder: &LocalEmbeddingCommandEmbedder,
        inputs: &[EmbeddingInput],
    ) -> Result<Self, EmbeddingError> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| EmbeddingError::InvalidRequest)?
            .as_nanos();
        let sequence = EMBEDDING_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "resume-ir-embedding-{}-{unique}-{sequence}",
            std::process::id()
        ));
        create_private_directory(&directory)?;
        let path = directory.join("embedding-input.txt");
        let mut file = create_private_file(&path)?;
        writeln!(file, "resume-ir-embedding-input-v1").map_err(|_| EmbeddingError::EngineFailed)?;
        writeln!(file, "model_id={}", embedder.model_id())
            .map_err(|_| EmbeddingError::EngineFailed)?;
        writeln!(file, "dimension={}", embedder.dimension())
            .map_err(|_| EmbeddingError::EngineFailed)?;
        writeln!(file, "count={}", inputs.len()).map_err(|_| EmbeddingError::EngineFailed)?;
        for input in inputs {
            writeln!(file, "input={}\t{}", input.id(), input.text().len())
                .map_err(|_| EmbeddingError::EngineFailed)?;
            writeln!(file, "text:").map_err(|_| EmbeddingError::EngineFailed)?;
            writeln!(file, "{}", input.text()).map_err(|_| EmbeddingError::EngineFailed)?;
            writeln!(file, "--resume-ir-embedding-input-boundary--")
                .map_err(|_| EmbeddingError::EngineFailed)?;
        }
        file.flush().map_err(|_| EmbeddingError::EngineFailed)?;

        Ok(Self { directory, path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for EmbeddingTempInput {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> Result<(), EmbeddingError> {
    fs::DirBuilder::new()
        .mode(0o700)
        .create(path)
        .map_err(|_| EmbeddingError::EngineFailed)
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> Result<(), EmbeddingError> {
    fs::create_dir(path).map_err(|_| EmbeddingError::EngineFailed)
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> Result<fs::File, EmbeddingError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| EmbeddingError::EngineFailed)
}

#[cfg(not(unix))]
fn create_private_file(path: &Path) -> Result<fs::File, EmbeddingError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| EmbeddingError::EngineFailed)
}

fn deterministic_values(text: &str, dimension: usize) -> Vec<f32> {
    let mut values = vec![0.0; dimension];

    for token in text.split_whitespace() {
        let normalized = token.to_ascii_lowercase();
        let hash = stable_hash(normalized.as_bytes());
        let index = hash as usize % dimension;
        values[index] += 1.0;
    }

    let magnitude = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if magnitude > 0.0 {
        for value in &mut values {
            *value /= magnitude;
        }
    }

    values
}

fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}
