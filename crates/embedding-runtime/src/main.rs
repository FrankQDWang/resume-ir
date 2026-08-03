use std::{
    env,
    fmt::{self, Write as _},
    fs,
    io::{self, BufReader, BufWriter},
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use embedding_protocol::{
    read_frame, write_frame, EmbedRequest, EmbeddingRole, EmbeddingTelemetry, ResidentErrorCode,
    ResidentResponse, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES,
};
use ort::{
    ep::CPU,
    session::{builder::GraphOptimizationLevel, Session},
    value::Tensor,
};
#[cfg(unix)]
use process_containment::VerifiedParentProcess;
#[cfg(unix)]
use std::thread;
use tokenizers::{AddedToken, PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

mod batch;
mod profiling;
mod runtime_pack;

use batch::{mean_pool_batch, tokenized_batch};
use profiling::{
    parse_run_mode, FixedShapePolicy, MemoryPatternPolicy, ProfilingMode, ResidentMode,
    ResidentThreadPolicy, RunMode, PROFILE_OUTPUT_PREFIX_ENV,
};
#[cfg(test)]
use runtime_pack::AssetIdentity;
use runtime_pack::{FileRole, RuntimePack};

const INPUT_SCHEMA: &str = "resume-ir-embedding-input-v1";
const OUTPUT_SCHEMA: &str = "resume-ir-embedding-v1";
const PACK_SCHEMA: &str = "resume-ir.embedding-runtime-pack.v1";
const PACK_ID: &str = "intfloat-multilingual-e5-small-qint8-r1";
const UPSTREAM_MODEL_ID: &str = "intfloat/multilingual-e5-small";
const UPSTREAM_REVISION: &str = "614241f622f53c4eeff9890bdc4f31cfecc418b3";
const MODEL_ID: &str = PACK_ID;
const DIMENSION: usize = 384;
const MAX_INPUTS: usize = 4;
const MAX_TEXT_BYTES: usize = 65_536;
const MAX_INPUT_FILE_BYTES: u64 = 4 * MAX_TEXT_BYTES as u64 + 16 * 1024;
const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_TOKENIZER_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_RUNTIME_PATH_BYTES: usize = 4096;
const BOUNDARY: &str = "--resume-ir-embedding-input-boundary--";
#[cfg(unix)]
const PARENT_IDENTITY_POLL_INTERVAL: Duration = Duration::from_millis(100);

fn main() {
    panic::set_hook(Box::new(|_| {}));
    if let Err(error) = run_with_panic_boundary(run) {
        eprintln!("embedding runtime blocked: {error}");
        std::process::exit(2);
    }
}

fn run_with_panic_boundary(
    operation: impl FnOnce() -> Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    panic::catch_unwind(AssertUnwindSafe(operation))
        .unwrap_or(Err(RuntimeError::RuntimeUnavailable))
}

fn run() -> Result<(), RuntimeError> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match parse_run_mode(&args, || env::var_os(PROFILE_OUTPUT_PREFIX_ENV))? {
        RunMode::OneShot => run_one_shot(),
        RunMode::Resident(mode) => run_resident(mode),
    }
}

fn run_one_shot() -> Result<(), RuntimeError> {
    let environment = RuntimeEnvironment::read()?;
    let pack = RuntimePack::load(&environment.runtime_dir)?;
    if environment.model_id != pack.model_id() || environment.dimension != pack.dimension() {
        return Err(RuntimeError::IdentityMismatch);
    }
    let request = EmbeddingRequest::read(&environment.input_path)?;
    if request.model_id != pack.model_id()
        || request.dimension != pack.dimension()
        || request.inputs.len() != environment.input_count
    {
        return Err(RuntimeError::IdentityMismatch);
    }

    let mut model = initialize_model(&pack, environment.intra_threads, ProfilingMode::Disabled)?;
    let texts = request.inputs.iter().map(prefixed_text).collect::<Vec<_>>();
    let mut vectors = Vec::with_capacity(texts.len());
    for text in &texts {
        vectors.push(model.embed(text)?);
    }
    let output = format_output(&request, vectors)?;
    print!("{output}");
    Ok(())
}

fn run_resident(mode: ResidentMode) -> Result<(), RuntimeError> {
    start_resident_parent_death_guard()?;
    let ResidentMode {
        profiling,
        thread_policy,
        memory_pattern,
        fixed_shape,
    } = mode;
    let environment = ResidentEnvironment::read(thread_policy)?;
    let pack = RuntimePack::load(&environment.runtime_dir)?;
    let model_id = pack.model_id().to_string();
    let dimension = pack.dimension();
    let model_path = pack.file(FileRole::Model)?.to_path_buf();
    if environment.model_id != model_id || environment.dimension != dimension {
        return Err(RuntimeError::IdentityMismatch);
    }
    let mut model = initialize_model_from_path(
        &pack,
        &model_path,
        environment.intra_threads,
        profiling,
        memory_pattern,
        fixed_shape,
    )?;
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());
    write_frame(
        &mut writer,
        &ResidentResponse::ready(&model_id, dimension),
        MAX_RESPONSE_BYTES,
    )
    .map_err(|_| RuntimeError::OutputInvalid)?;

    loop {
        let request = match read_frame::<EmbedRequest>(&mut reader, MAX_REQUEST_BYTES) {
            Ok(Some(request)) => request,
            Ok(None) => {
                model.finish_profiling()?;
                return Ok(());
            }
            Err(embedding_protocol::ProtocolError::InvalidPayload) => {
                write_frame(
                    &mut writer,
                    &ResidentResponse::error(None, ResidentErrorCode::InvalidRequest, false),
                    MAX_RESPONSE_BYTES,
                )
                .map_err(|_| RuntimeError::OutputInvalid)?;
                continue;
            }
            Err(_) => return Err(RuntimeError::InputInvalid),
        };
        if request.validate().is_err() {
            write_frame(
                &mut writer,
                &ResidentResponse::error(
                    Some(request.request_id),
                    ResidentErrorCode::InvalidRequest,
                    false,
                ),
                MAX_RESPONSE_BYTES,
            )
            .map_err(|_| RuntimeError::OutputInvalid)?;
            continue;
        }
        if request.model_id != model_id || request.dimension != dimension {
            write_frame(
                &mut writer,
                &ResidentResponse::error(
                    Some(request.request_id),
                    ResidentErrorCode::IdentityMismatch,
                    false,
                ),
                MAX_RESPONSE_BYTES,
            )
            .map_err(|_| RuntimeError::OutputInvalid)?;
            continue;
        }

        let response = match resident_success_response(&mut model, &request) {
            Ok(response) => response,
            Err(error) => {
                write_frame(
                    &mut writer,
                    &ResidentResponse::error(
                        Some(request.request_id),
                        ResidentErrorCode::InferenceFailed,
                        true,
                    ),
                    MAX_RESPONSE_BYTES,
                )
                .map_err(|_| RuntimeError::OutputInvalid)?;
                return Err(error);
            }
        };
        write_frame(&mut writer, &response, MAX_RESPONSE_BYTES)
            .map_err(|_| RuntimeError::OutputInvalid)?;
    }
}

struct ResidentBatchOutput {
    vectors: Vec<Vec<f32>>,
    telemetry: EmbeddingTelemetry,
}

trait ResidentBatchModel {
    fn embed_resident_batch(
        &mut self,
        texts: &[String],
    ) -> Result<ResidentBatchOutput, RuntimeError>;
}

fn resident_success_response(
    model: &mut impl ResidentBatchModel,
    request: &EmbedRequest,
) -> Result<ResidentResponse, RuntimeError> {
    let texts = request
        .inputs
        .iter()
        .map(|input| prefixed_resident_text(input.role, &input.text))
        .collect::<Vec<_>>();
    let output = model.embed_resident_batch(&texts)?;
    Ok(ResidentResponse::result_with_telemetry(
        request.request_id,
        output.vectors,
        output.telemetry,
    ))
}

#[cfg(unix)]
fn start_resident_parent_death_guard() -> Result<(), RuntimeError> {
    let parent = VerifiedParentProcess::capture().map_err(|_| RuntimeError::RuntimeUnavailable)?;
    thread::Builder::new()
        .name("embedding-parent-guard".to_string())
        .spawn(move || loop {
            if !parent.is_current_parent() {
                std::process::exit(0);
            }
            thread::sleep(PARENT_IDENTITY_POLL_INTERVAL);
        })
        .map(|_| ())
        .map_err(|_| RuntimeError::RuntimeUnavailable)
}

#[cfg(not(unix))]
fn start_resident_parent_death_guard() -> Result<(), RuntimeError> {
    Ok(())
}

fn initialize_model(
    pack: &RuntimePack,
    intra_threads: usize,
    profiling: ProfilingMode,
) -> Result<NativeEmbeddingModel, RuntimeError> {
    initialize_model_from_path(
        pack,
        pack.file(FileRole::Model)?,
        intra_threads,
        profiling,
        MemoryPatternPolicy::Disabled,
        FixedShapePolicy::Disabled,
    )
}

fn initialize_model_from_path(
    pack: &RuntimePack,
    model_path: &Path,
    intra_threads: usize,
    profiling: ProfilingMode,
    memory_pattern: MemoryPatternPolicy,
    fixed_shape: FixedShapePolicy,
) -> Result<NativeEmbeddingModel, RuntimeError> {
    if !ort::init_from(pack.file(FileRole::RuntimeLibrary)?)
        .map_err(|_| RuntimeError::RuntimeUnavailable)?
        .commit()
    {
        return Err(RuntimeError::RuntimeUnavailable);
    }
    tokenizers::utils::parallelism::set_parallelism(false);
    NativeEmbeddingModel::load(
        pack,
        model_path,
        intra_threads,
        profiling,
        memory_pattern,
        fixed_shape,
    )
}

fn prefixed_text(input: &EmbeddingRuntimeInput) -> String {
    let prefix = if input.id == "query" {
        "query: "
    } else {
        "passage: "
    };
    format!("{prefix}{}", input.text)
}

fn prefixed_resident_text(role: EmbeddingRole, text: &str) -> String {
    let prefix = match role {
        EmbeddingRole::Query => "query: ",
        EmbeddingRole::Passage => "passage: ",
    };
    format!("{prefix}{text}")
}

struct NativeEmbeddingModel {
    tokenizer: Tokenizer,
    session: Session,
    fixed_b4x512_session: Option<Session>,
    need_token_type_ids: bool,
    profiling: ProfilingMode,
}

impl NativeEmbeddingModel {
    fn load(
        pack: &RuntimePack,
        model_path: &Path,
        intra_threads: usize,
        profiling: ProfilingMode,
        memory_pattern: MemoryPatternPolicy,
        fixed_shape: FixedShapePolicy,
    ) -> Result<Self, RuntimeError> {
        let builder_error = |_| RuntimeError::ModelUnavailable;
        let build_session = || {
            Session::builder()
                .map_err(|_| RuntimeError::ModelUnavailable)?
                .with_execution_providers([CPU::default().with_arena_allocator(false).build()])
                .map_err(builder_error)?
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .map_err(builder_error)?
                .with_memory_pattern(memory_pattern.enabled())
                .map_err(builder_error)?
                .with_prepacking(false)
                .map_err(builder_error)?
                .with_parallel_execution(false)
                .map_err(builder_error)?
                .with_intra_threads(intra_threads)
                .map_err(builder_error)?
                .with_inter_threads(1)
                .map_err(builder_error)
        };
        let builder = build_session()?;
        let session = profiling
            .configure_builder(builder)?
            .commit_from_file(model_path)
            .map_err(|_| RuntimeError::ModelUnavailable)?;
        let need_token_type_ids = validate_session_contract(&session)?;
        let fixed_b4x512_session = fixed_shape
            .enabled()
            .then(|| {
                build_session()?
                    .with_dimension_override("batch_size", 4)
                    .map_err(builder_error)?
                    .with_dimension_override("sequence_length", 512)
                    .map_err(builder_error)?
                    .commit_from_file(model_path)
                    .map_err(|_| RuntimeError::ModelUnavailable)
            })
            .transpose()?;
        if let Some(fixed_session) = &fixed_b4x512_session {
            if validate_session_contract(fixed_session)? != need_token_type_ids {
                return Err(RuntimeError::ModelUnavailable);
            }
        }
        let tokenizer = load_tokenizer(pack)?;
        Ok(Self {
            tokenizer,
            session,
            fixed_b4x512_session,
            need_token_type_ids,
            profiling,
        })
    }

    fn finish_profiling(&mut self) -> Result<(), RuntimeError> {
        self.profiling.finish(&mut self.session)
    }

    fn embed(&mut self, text: &str) -> Result<Vec<f32>, RuntimeError> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|_| RuntimeError::InferenceFailed)?;
        let input_ids = encoding
            .get_ids()
            .iter()
            .map(|value| i64::from(*value))
            .collect::<Vec<_>>();
        let attention_mask = encoding
            .get_attention_mask()
            .iter()
            .map(|value| i64::from(*value))
            .collect::<Vec<_>>();
        let token_type_ids = encoding
            .get_type_ids()
            .iter()
            .map(|value| i64::from(*value))
            .collect::<Vec<_>>();
        let sequence_length = input_ids.len();
        if sequence_length == 0
            || attention_mask.len() != sequence_length
            || token_type_ids.len() != sequence_length
        {
            return Err(RuntimeError::InferenceFailed);
        }
        let mut inputs = ort::inputs![
            "input_ids" => Tensor::from_array(([1, sequence_length], input_ids))
                .map_err(|_| RuntimeError::InferenceFailed)?,
            "attention_mask" => Tensor::from_array(([1, sequence_length], attention_mask.clone()))
                .map_err(|_| RuntimeError::InferenceFailed)?,
        ];
        if self.need_token_type_ids {
            inputs.push((
                "token_type_ids".into(),
                Tensor::from_array(([1, sequence_length], token_type_ids))
                    .map_err(|_| RuntimeError::InferenceFailed)?
                    .into(),
            ));
        }
        let outputs = self
            .session
            .run(inputs)
            .map_err(|_| RuntimeError::InferenceFailed)?;
        let (shape, values) = outputs
            .get("last_hidden_state")
            .ok_or(RuntimeError::OutputInvalid)?
            .try_extract_tensor::<f32>()
            .map_err(|_| RuntimeError::OutputInvalid)?;
        mean_pool(&shape[..], values, &attention_mask)
    }
}

fn validate_session_contract(session: &Session) -> Result<bool, RuntimeError> {
    let required_inputs = ["input_ids", "attention_mask"];
    if required_inputs.iter().any(|required| {
        !session
            .inputs()
            .iter()
            .any(|input| input.name() == *required)
    }) || !session
        .outputs()
        .iter()
        .any(|output| output.name() == "last_hidden_state")
    {
        return Err(RuntimeError::ModelUnavailable);
    }
    Ok(session
        .inputs()
        .iter()
        .any(|input| input.name() == "token_type_ids"))
}

fn use_fixed_b4x512_session(
    input_count: usize,
    sequence_length: usize,
    fixed_session_available: bool,
) -> bool {
    fixed_session_available && input_count == 4 && sequence_length == 512
}

impl ResidentBatchModel for NativeEmbeddingModel {
    fn embed_resident_batch(
        &mut self,
        texts: &[String],
    ) -> Result<ResidentBatchOutput, RuntimeError> {
        if texts.is_empty() || texts.len() > MAX_INPUTS {
            return Err(RuntimeError::InferenceFailed);
        }
        let child_started = Instant::now();
        let phase_started = Instant::now();
        let encodings = self
            .tokenizer
            .encode_batch(texts.iter().map(String::as_str).collect(), true)
            .map_err(|_| RuntimeError::InferenceFailed)?;
        if encodings.len() != texts.len() {
            return Err(RuntimeError::InferenceFailed);
        }
        let tokenize = phase_started.elapsed();

        let phase_started = Instant::now();
        let tokenized = tokenized_batch(&encodings)?;
        let active_token_count = tokenized
            .attention_masks
            .iter()
            .flatten()
            .filter(|value| **value != 0)
            .count();
        let padded_token_count = tokenized
            .attention_masks
            .iter()
            .map(Vec::len)
            .fold(0_usize, usize::saturating_add);
        let attention_mask = tokenized
            .attention_masks
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        let use_fixed_session = use_fixed_b4x512_session(
            texts.len(),
            tokenized.sequence_length,
            self.fixed_b4x512_session.is_some(),
        );
        let mut inputs = ort::inputs![
            "input_ids" => Tensor::from_array((
                [texts.len(), tokenized.sequence_length],
                tokenized.input_ids,
            ))
            .map_err(|_| RuntimeError::InferenceFailed)?,
            "attention_mask" => Tensor::from_array((
                [texts.len(), tokenized.sequence_length],
                attention_mask,
            ))
            .map_err(|_| RuntimeError::InferenceFailed)?,
        ];
        if self.need_token_type_ids {
            inputs.push((
                "token_type_ids".into(),
                Tensor::from_array((
                    [texts.len(), tokenized.sequence_length],
                    tokenized.token_type_ids,
                ))
                .map_err(|_| RuntimeError::InferenceFailed)?
                .into(),
            ));
        }
        let tensor = phase_started.elapsed();

        let phase_started = Instant::now();
        let session = if use_fixed_session {
            self.fixed_b4x512_session
                .as_mut()
                .ok_or(RuntimeError::RuntimeUnavailable)?
        } else {
            &mut self.session
        };
        let outputs = session
            .run(inputs)
            .map_err(|_| RuntimeError::InferenceFailed)?;
        let (shape, values) = outputs
            .get("last_hidden_state")
            .ok_or(RuntimeError::OutputInvalid)?
            .try_extract_tensor::<f32>()
            .map_err(|_| RuntimeError::OutputInvalid)?;
        let onnx = phase_started.elapsed();

        let phase_started = Instant::now();
        let vectors = mean_pool_batch(&shape[..], values, &tokenized.attention_masks)?;
        let pool = phase_started.elapsed();

        let phase_started = Instant::now();
        let vectors = vectors
            .into_iter()
            .map(normalize_vector)
            .collect::<Result<Vec<_>, _>>()?;
        let normalize = phase_started.elapsed();
        Ok(ResidentBatchOutput {
            vectors,
            telemetry: build_embedding_telemetry(
                texts.len(),
                active_token_count,
                padded_token_count,
                [tokenize, tensor, onnx, pool, normalize],
                child_started.elapsed(),
            ),
        })
    }
}

fn build_embedding_telemetry(
    input_count: usize,
    active_token_count: usize,
    padded_token_count: usize,
    phases: [Duration; 5],
    child_total: Duration,
) -> EmbeddingTelemetry {
    let [tokenize_us, tensor_us, onnx_us, pool_us, normalize_us] = phases.map(saturated_micros);
    let phase_total = tokenize_us
        .saturating_add(tensor_us)
        .saturating_add(onnx_us)
        .saturating_add(pool_us)
        .saturating_add(normalize_us);
    EmbeddingTelemetry {
        input_count: usize_to_u64(input_count),
        active_token_count: usize_to_u64(active_token_count),
        padded_token_count: usize_to_u64(padded_token_count),
        tokenize_us,
        tensor_us,
        onnx_us,
        pool_us,
        normalize_us,
        child_total_us: saturated_micros(child_total).max(phase_total),
        queue_wait_us: 0,
        ipc_wall_us: 0,
    }
}

fn saturated_micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn mean_pool(
    shape: &[i64],
    values: &[f32],
    attention_mask: &[i64],
) -> Result<Vec<f32>, RuntimeError> {
    mean_pool_batch(shape, values, &[attention_mask.to_vec()])?
        .pop()
        .ok_or(RuntimeError::OutputInvalid)
}

fn load_tokenizer(pack: &RuntimePack) -> Result<Tokenizer, RuntimeError> {
    let config: serde_json::Value = serde_json::from_slice(&read_component(
        pack.file(FileRole::ModelConfig)?,
        MAX_TOKENIZER_CONFIG_BYTES,
    )?)
    .map_err(|_| RuntimeError::ModelUnavailable)?;
    let tokenizer_config: serde_json::Value = serde_json::from_slice(&read_component(
        pack.file(FileRole::TokenizerConfig)?,
        MAX_TOKENIZER_CONFIG_BYTES,
    )?)
    .map_err(|_| RuntimeError::ModelUnavailable)?;
    let special_tokens: serde_json::Value = serde_json::from_slice(&read_component(
        pack.file(FileRole::SpecialTokensMap)?,
        MAX_TOKENIZER_CONFIG_BYTES,
    )?)
    .map_err(|_| RuntimeError::ModelUnavailable)?;
    let pad_id = config
        .get("pad_token_id")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(RuntimeError::ModelUnavailable)?;
    let pad_token = tokenizer_config
        .get("pad_token")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .ok_or(RuntimeError::ModelUnavailable)?
        .to_string();
    let model_max_length = tokenizer_config
        .get("model_max_length")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value == 512)
        .ok_or(RuntimeError::ModelUnavailable)?;
    let mut tokenizer = Tokenizer::from_file(pack.file(FileRole::Tokenizer)?)
        .map_err(|_| RuntimeError::ModelUnavailable)?;
    tokenizer
        .with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            pad_token,
            pad_id,
            ..Default::default()
        }))
        .with_truncation(Some(TruncationParams {
            max_length: model_max_length,
            ..Default::default()
        }))
        .map_err(|_| RuntimeError::ModelUnavailable)?;
    let entries = special_tokens
        .as_object()
        .ok_or(RuntimeError::ModelUnavailable)?;
    let tokens = entries
        .values()
        .map(|value| {
            value
                .as_str()
                .or_else(|| value.get("content").and_then(serde_json::Value::as_str))
                .filter(|content| !content.is_empty() && content.len() <= 128)
                .map(|content| AddedToken {
                    content: content.to_string(),
                    special: true,
                    ..Default::default()
                })
                .ok_or(RuntimeError::ModelUnavailable)
        })
        .collect::<Result<Vec<_>, _>>()?;
    tokenizer.add_special_tokens(&tokens);
    Ok(tokenizer)
}

#[derive(Debug)]
enum RuntimeError {
    EnvironmentInvalid,
    RuntimePackInvalid,
    RuntimeUnavailable,
    ModelUnavailable,
    InputInvalid,
    InputBudgetExceeded,
    IdentityMismatch,
    InferenceFailed,
    OutputInvalid,
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EnvironmentInvalid => "environment is invalid",
            Self::RuntimePackInvalid => "runtime pack is invalid",
            Self::RuntimeUnavailable => "ONNX runtime is unavailable",
            Self::ModelUnavailable => "embedding model is unavailable",
            Self::InputInvalid => "input is invalid",
            Self::InputBudgetExceeded => "input budget exceeded",
            Self::IdentityMismatch => "model identity does not match",
            Self::InferenceFailed => "embedding inference failed",
            Self::OutputInvalid => "embedding output is invalid",
        })
    }
}

struct RuntimeEnvironment {
    input_path: PathBuf,
    input_count: usize,
    model_id: String,
    dimension: usize,
    runtime_dir: PathBuf,
    intra_threads: usize,
}

struct ResidentEnvironment {
    model_id: String,
    dimension: usize,
    runtime_dir: PathBuf,
    intra_threads: usize,
}

impl ResidentEnvironment {
    fn read(thread_policy: ResidentThreadPolicy) -> Result<Self, RuntimeError> {
        let runtime_dir = absolute_environment_path("RESUME_IR_EMBEDDING_RUNTIME_DIR")?;
        let dimension = required_environment_usize("RESUME_IR_EMBEDDING_DIMENSION", 1, usize::MAX)?;
        let intra_threads = match thread_policy {
            ResidentThreadPolicy::Production => optional_environment_usize(
                "RESUME_IR_EMBEDDING_INTRA_THREADS",
                /*default*/ 1,
                /*max*/ 3,
            )?,
            #[cfg(feature = "resident-embedding-pool-experiment")]
            ResidentThreadPolicy::PoolExperiment => required_environment_usize(
                "RESUME_IR_EMBEDDING_INTRA_THREADS",
                /*minimum*/ 1,
                /*maximum*/ 4,
            )?,
        };
        let model_id = env::var("RESUME_IR_EMBEDDING_MODEL_ID")
            .ok()
            .filter(|value| !value.is_empty() && value.len() <= 128)
            .ok_or(RuntimeError::EnvironmentInvalid)?;
        Ok(Self {
            model_id,
            dimension,
            runtime_dir,
            intra_threads,
        })
    }
}

impl RuntimeEnvironment {
    fn read() -> Result<Self, RuntimeError> {
        let input_path = absolute_environment_path("RESUME_IR_EMBEDDING_INPUT_PATH")?;
        let runtime_dir = absolute_environment_path("RESUME_IR_EMBEDDING_RUNTIME_DIR")?;
        let input_count =
            required_environment_usize("RESUME_IR_EMBEDDING_INPUT_COUNT", 0, MAX_INPUTS)?;
        let dimension = required_environment_usize("RESUME_IR_EMBEDDING_DIMENSION", 1, usize::MAX)?;
        let intra_threads = optional_environment_usize(
            "RESUME_IR_EMBEDDING_INTRA_THREADS",
            /*default*/ 1,
            /*max*/ 3,
        )?;
        let model_id = env::var("RESUME_IR_EMBEDDING_MODEL_ID")
            .ok()
            .filter(|value| !value.is_empty() && value.len() <= 128)
            .ok_or(RuntimeError::EnvironmentInvalid)?;
        Ok(Self {
            input_path,
            input_count,
            model_id,
            dimension,
            runtime_dir,
            intra_threads,
        })
    }
}

fn absolute_environment_path(name: &str) -> Result<PathBuf, RuntimeError> {
    let value = env::var_os(name).ok_or(RuntimeError::EnvironmentInvalid)?;
    let path = PathBuf::from(value);
    let text = path.to_str().ok_or(RuntimeError::EnvironmentInvalid)?;
    if !path.is_absolute() || text.len() > MAX_RUNTIME_PATH_BYTES {
        return Err(RuntimeError::EnvironmentInvalid);
    }
    Ok(path)
}

fn required_environment_usize(
    name: &str,
    minimum: usize,
    maximum: usize,
) -> Result<usize, RuntimeError> {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (minimum..=maximum).contains(value))
        .ok_or(RuntimeError::EnvironmentInvalid)
}

fn optional_environment_usize(
    name: &str,
    default: usize,
    maximum: usize,
) -> Result<usize, RuntimeError> {
    match env::var(name) {
        Ok(_) => required_environment_usize(name, 1, maximum),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => Err(RuntimeError::EnvironmentInvalid),
    }
}

fn read_component(path: &Path, max_bytes: u64) -> Result<Vec<u8>, RuntimeError> {
    let metadata = path
        .metadata()
        .map_err(|_| RuntimeError::ModelUnavailable)?;
    if metadata.len() > max_bytes {
        return Err(RuntimeError::ModelUnavailable);
    }
    fs::read(path).map_err(|_| RuntimeError::ModelUnavailable)
}

struct EmbeddingRequest {
    model_id: String,
    dimension: usize,
    inputs: Vec<EmbeddingRuntimeInput>,
}

struct EmbeddingRuntimeInput {
    id: String,
    text: String,
}

impl EmbeddingRequest {
    fn read(path: &Path) -> Result<Self, RuntimeError> {
        let metadata = fs::symlink_metadata(path).map_err(|_| RuntimeError::InputInvalid)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(RuntimeError::InputInvalid);
        }
        if metadata.len() > MAX_INPUT_FILE_BYTES {
            return Err(RuntimeError::InputBudgetExceeded);
        }
        let body = fs::read(path).map_err(|_| RuntimeError::InputInvalid)?;
        parse_input(&body)
    }
}

fn parse_input(body: &[u8]) -> Result<EmbeddingRequest, RuntimeError> {
    let mut cursor = 0;
    if take_line(body, &mut cursor)? != INPUT_SCHEMA {
        return Err(RuntimeError::InputInvalid);
    }
    let model_id = header(take_line(body, &mut cursor)?, "model_id")?.to_string();
    let dimension = header(take_line(body, &mut cursor)?, "dimension")?
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or(RuntimeError::InputInvalid)?;
    let count = header(take_line(body, &mut cursor)?, "count")?
        .parse::<usize>()
        .map_err(|_| RuntimeError::InputInvalid)?;
    if count > MAX_INPUTS {
        return Err(RuntimeError::InputBudgetExceeded);
    }
    let mut inputs = Vec::with_capacity(count);
    for _ in 0..count {
        let entry = take_line(body, &mut cursor)?
            .strip_prefix("input=")
            .and_then(|value| value.split_once('\t'))
            .ok_or(RuntimeError::InputInvalid)?;
        let id = entry.0;
        let declared_bytes = entry
            .1
            .parse::<usize>()
            .map_err(|_| RuntimeError::InputInvalid)?;
        if id.is_empty()
            || id.len() > 128
            || id
                .bytes()
                .any(|value| matches!(value, b'\r' | b'\n' | b'\t'))
            || take_line(body, &mut cursor)? != "text:"
        {
            return Err(RuntimeError::InputInvalid);
        }
        if declared_bytes > MAX_TEXT_BYTES {
            return Err(RuntimeError::InputBudgetExceeded);
        }
        let text_end = cursor
            .checked_add(declared_bytes)
            .filter(|end| *end <= body.len())
            .ok_or(RuntimeError::InputInvalid)?;
        let text = std::str::from_utf8(&body[cursor..text_end])
            .map_err(|_| RuntimeError::InputInvalid)?
            .to_string();
        cursor = text_end;
        consume_line_ending(body, &mut cursor)?;
        if take_line(body, &mut cursor)? != BOUNDARY {
            return Err(RuntimeError::InputInvalid);
        }
        inputs.push(EmbeddingRuntimeInput {
            id: id.to_string(),
            text,
        });
    }
    if cursor != body.len() {
        return Err(RuntimeError::InputInvalid);
    }
    Ok(EmbeddingRequest {
        model_id,
        dimension,
        inputs,
    })
}

fn take_line<'a>(body: &'a [u8], cursor: &mut usize) -> Result<&'a str, RuntimeError> {
    let rest = body.get(*cursor..).ok_or(RuntimeError::InputInvalid)?;
    let line_bytes = rest
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or(RuntimeError::InputInvalid)?;
    let line_end = *cursor + line_bytes;
    let line = body
        .get(*cursor..line_end)
        .ok_or(RuntimeError::InputInvalid)?;
    *cursor = line_end + 1;
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    std::str::from_utf8(line).map_err(|_| RuntimeError::InputInvalid)
}

fn consume_line_ending(body: &[u8], cursor: &mut usize) -> Result<(), RuntimeError> {
    let rest = body.get(*cursor..).ok_or(RuntimeError::InputInvalid)?;
    let bytes = if rest.starts_with(b"\r\n") {
        2
    } else if rest.starts_with(b"\n") {
        1
    } else {
        return Err(RuntimeError::InputInvalid);
    };
    *cursor += bytes;
    Ok(())
}

fn header<'a>(line: &'a str, key: &str) -> Result<&'a str, RuntimeError> {
    line.strip_prefix(key)
        .and_then(|value| value.strip_prefix('='))
        .filter(|value| !value.is_empty())
        .ok_or(RuntimeError::InputInvalid)
}

fn format_output(
    request: &EmbeddingRequest,
    vectors: Vec<Vec<f32>>,
) -> Result<String, RuntimeError> {
    if vectors.len() != request.inputs.len() {
        return Err(RuntimeError::OutputInvalid);
    }
    let mut output = format!(
        "{OUTPUT_SCHEMA}\nmodel_id={}\ndimension={}\n",
        request.model_id, request.dimension
    );
    for (input, mut vector) in request.inputs.iter().zip(vectors) {
        if vector.len() != request.dimension || vector.iter().any(|value| !value.is_finite()) {
            return Err(RuntimeError::OutputInvalid);
        }
        vector = normalize_vector(vector)?;
        write!(output, "vector={}\t", input.id).map_err(|_| RuntimeError::OutputInvalid)?;
        for (index, value) in vector.iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            write!(output, "{value:.9}").map_err(|_| RuntimeError::OutputInvalid)?;
        }
        output.push('\n');
        if output.len() > MAX_OUTPUT_BYTES {
            return Err(RuntimeError::OutputInvalid);
        }
    }
    Ok(output)
}

fn normalize_vector(mut vector: Vec<f32>) -> Result<Vec<f32>, RuntimeError> {
    if vector.is_empty() || vector.iter().any(|value| !value.is_finite()) {
        return Err(RuntimeError::OutputInvalid);
    }
    let norm = vector
        .iter()
        .map(|value| f64::from(*value).powi(2))
        .sum::<f64>()
        .sqrt();
    if !norm.is_finite() || norm <= f64::EPSILON {
        return Err(RuntimeError::OutputInvalid);
    }
    for value in &mut vector {
        *value = (f64::from(*value) / norm) as f32;
    }
    Ok(vector)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
