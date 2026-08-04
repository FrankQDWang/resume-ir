use std::{
    env, fs,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    time::Instant,
};

use sha2::{Digest, Sha256};
use tokenizers::Tokenizer;

use super::{
    build_embedding_telemetry, load_tokenizer, ResidentBatchModel, ResidentBatchOutput,
    RuntimeError, RuntimePack, DIMENSION, MAX_INPUTS,
};

const WORKER_ENV: &str = "RESUME_IR_COREML_WORKER_BIN";
const RUNTIME_DIR_ENV: &str = "RESUME_IR_COREML_RUNTIME_DIR";
const MANIFEST_SHA256: &str = "2eb4126da855b69cd9e81f2ecaaaec1b9dea21e37a3efac81271726a2d9d8cb2";
const TOKENS: usize = 512;
const READY_BYTE: u8 = 0xa5;
const PACK_FILES: [(&str, u64, &str); 10] = [
    (
        "e5-b1x512.mlmodelc/analytics/coremldata.bin",
        243,
        "bd65816f677e9e8657902ee40eb94e1d93ee6817571f8fe820a3ca89b08f26e5",
    ),
    (
        "e5-b1x512.mlmodelc/coremldata.bin",
        421,
        "a2c5baceca2dd3a4113757a83ffc8c907ae895e03b943a1678040615a61b59b1",
    ),
    (
        "e5-b1x512.mlmodelc/metadata.json",
        2_418,
        "266e7397874c8385431ecc35da375f09066bbb67287c55e1de131eb1eefa7975",
    ),
    (
        "e5-b1x512.mlmodelc/model.mil",
        131_086,
        "c79a28ea478b4623daf91f48247905de6867b8a0f2bb20645cc33f5ce7a6672d",
    ),
    (
        "e5-b1x512.mlmodelc/weights/weight.bin",
        235_416_192,
        "85f2d1618a9085c27a132955148f72b0223f213e0355f848e53f4da0e2e97d1e",
    ),
    (
        "e5-b4x512.mlmodelc/analytics/coremldata.bin",
        243,
        "97284ff40e122423bd0651ddd2363de243b395f08a15e5dae90e9b3fd56d4a12",
    ),
    (
        "e5-b4x512.mlmodelc/coremldata.bin",
        421,
        "3344bece576df1879f5324537e0fbe963b38e3e87193b37118f30561520bf22e",
    ),
    (
        "e5-b4x512.mlmodelc/metadata.json",
        2_418,
        "cf41fe396deaaf018210eb3d3aef0f73754e8bd45f256af9a35bcd79e97b80f6",
    ),
    (
        "e5-b4x512.mlmodelc/model.mil",
        131_086,
        "9c28b69219428b4fe66d0d0ba289204e46b6fa2fbc182a37abfde7359ce225a7",
    ),
    (
        "e5-b4x512.mlmodelc/weights/weight.bin",
        237_775_488,
        "c288d069a7a39fa013a4f663b3e259f7b806d65389ba82bf938a76c8c39f1d9f",
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CoreMlRole {
    Interactive,
    Bulk,
}

impl CoreMlRole {
    fn worker_batch(self) -> usize {
        match self {
            Self::Interactive => 1,
            Self::Bulk => 4,
        }
    }

    fn model_directory(self) -> &'static str {
        match self {
            Self::Interactive => "e5-b1x512.mlmodelc",
            Self::Bulk => "e5-b4x512.mlmodelc",
        }
    }
}

pub(super) fn requested_role(args: &[String]) -> Result<Option<CoreMlRole>, RuntimeError> {
    let enabled = env::var_os(RUNTIME_DIR_ENV).is_some();
    parse_role(args, enabled)
}

fn parse_role(args: &[String], enabled: bool) -> Result<Option<CoreMlRole>, RuntimeError> {
    if !enabled {
        return Ok(None);
    }
    match args {
        [mode, role]
            if mode == "--resident-embedding-pool-experiment"
                && role == "--resident-embedding-pool-role=interactive" =>
        {
            Ok(Some(CoreMlRole::Interactive))
        }
        [mode, role]
            if mode == "--resident-embedding-pool-experiment"
                && role == "--resident-embedding-pool-role=bulk" =>
        {
            Ok(Some(CoreMlRole::Bulk))
        }
        _ => Err(RuntimeError::EnvironmentInvalid),
    }
}

pub(super) struct CoreMlEmbeddingModel {
    tokenizer: Tokenizer,
    worker: CoreMlWorker,
    role: CoreMlRole,
}

impl CoreMlEmbeddingModel {
    pub(super) fn load(pack: &RuntimePack, role: CoreMlRole) -> Result<Self, RuntimeError> {
        let coreml_pack = CoreMlRuntimePack::load()?;
        Ok(Self {
            tokenizer: load_tokenizer(pack)?,
            worker: CoreMlWorker::start(role, &coreml_pack)?,
            role,
        })
    }
}

struct CoreMlRuntimePack {
    root: PathBuf,
}

impl CoreMlRuntimePack {
    fn load() -> Result<Self, RuntimeError> {
        let root = environment_path(RUNTIME_DIR_ENV)?;
        let canonical = fs::canonicalize(&root).map_err(|_| RuntimeError::EnvironmentInvalid)?;
        if canonical != root {
            return Err(RuntimeError::EnvironmentInvalid);
        }
        regular_directory(&root)?;
        for directory in [
            "e5-b1x512.mlmodelc",
            "e5-b1x512.mlmodelc/analytics",
            "e5-b1x512.mlmodelc/weights",
            "e5-b4x512.mlmodelc",
            "e5-b4x512.mlmodelc/analytics",
            "e5-b4x512.mlmodelc/weights",
        ] {
            regular_directory(&root.join(directory))?;
        }
        validate_file(&root.join("runtime-pack.json"), 2_260, MANIFEST_SHA256)?;
        for (file, bytes, digest) in PACK_FILES {
            validate_file(&root.join(file), bytes, digest)?;
        }
        Ok(Self { root })
    }

    fn model(&self, role: CoreMlRole) -> PathBuf {
        self.root.join(role.model_directory())
    }
}

impl ResidentBatchModel for CoreMlEmbeddingModel {
    fn embed_resident_batch(
        &mut self,
        texts: &[String],
    ) -> Result<ResidentBatchOutput, RuntimeError> {
        let maximum = self.role.worker_batch();
        if texts.is_empty() || texts.len() > maximum || texts.len() > MAX_INPUTS {
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
        let pad_id = self
            .tokenizer
            .get_padding()
            .map(|padding| padding.pad_id)
            .ok_or(RuntimeError::ModelUnavailable)?;
        let capacity = maximum
            .checked_mul(TOKENS)
            .ok_or(RuntimeError::InferenceFailed)?;
        let mut input_ids =
            vec![i32::try_from(pad_id).map_err(|_| RuntimeError::InferenceFailed)?; capacity];
        let mut attention_mask = vec![0_i32; capacity];
        let mut active_token_count = 0_usize;
        for (row, encoding) in encodings.iter().enumerate() {
            let ids = encoding.get_ids();
            let mask = encoding.get_attention_mask();
            if ids.is_empty() || ids.len() > TOKENS || ids.len() != mask.len() {
                return Err(RuntimeError::InferenceFailed);
            }
            let offset = row
                .checked_mul(TOKENS)
                .ok_or(RuntimeError::InferenceFailed)?;
            for (column, (&id, &active)) in ids.iter().zip(mask).enumerate() {
                input_ids[offset + column] =
                    i32::try_from(id).map_err(|_| RuntimeError::InferenceFailed)?;
                attention_mask[offset + column] =
                    i32::try_from(active).map_err(|_| RuntimeError::InferenceFailed)?;
                active_token_count = active_token_count.saturating_add(usize::from(active != 0));
            }
        }
        let tensor = phase_started.elapsed();

        let phase_started = Instant::now();
        let vectors = self
            .worker
            .predict(&input_ids, &attention_mask, texts.len())?;
        let inference = phase_started.elapsed();
        Ok(ResidentBatchOutput {
            vectors,
            telemetry: build_embedding_telemetry(
                texts.len(),
                active_token_count,
                texts.len().saturating_mul(TOKENS),
                [
                    tokenize,
                    tensor,
                    inference,
                    Default::default(),
                    Default::default(),
                ],
                child_started.elapsed(),
            ),
        })
    }
}

struct CoreMlWorker {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: ChildStdout,
    worker_batch: usize,
}

impl CoreMlWorker {
    fn start(role: CoreMlRole, pack: &CoreMlRuntimePack) -> Result<Self, RuntimeError> {
        let worker = regular_executable(environment_path(WORKER_ENV)?)?;
        let model = compiled_model(pack.model(role))?;
        let worker_batch = role.worker_batch();
        let mut child = Command::new(worker)
            .arg(model)
            .arg(worker_batch.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| RuntimeError::RuntimeUnavailable)?;
        let stdin = child.stdin.take().ok_or(RuntimeError::RuntimeUnavailable)?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or(RuntimeError::RuntimeUnavailable)?;
        let mut ready = [0_u8; 1];
        stdout
            .read_exact(&mut ready)
            .map_err(|_| RuntimeError::RuntimeUnavailable)?;
        if ready[0] != READY_BYTE {
            return Err(RuntimeError::RuntimeUnavailable);
        }
        Ok(Self {
            child,
            stdin: Some(stdin),
            stdout,
            worker_batch,
        })
    }

    fn predict(
        &mut self,
        input_ids: &[i32],
        attention_mask: &[i32],
        output_rows: usize,
    ) -> Result<Vec<Vec<f32>>, RuntimeError> {
        let expected = self
            .worker_batch
            .checked_mul(TOKENS)
            .ok_or(RuntimeError::InferenceFailed)?;
        if input_ids.len() != expected
            || attention_mask.len() != expected
            || output_rows == 0
            || output_rows > self.worker_batch
        {
            return Err(RuntimeError::InferenceFailed);
        }
        let stdin = self
            .stdin
            .as_mut()
            .ok_or(RuntimeError::RuntimeUnavailable)?;
        write_i32(stdin, input_ids)?;
        write_i32(stdin, attention_mask)?;
        stdin
            .flush()
            .map_err(|_| RuntimeError::RuntimeUnavailable)?;

        let output_values = self
            .worker_batch
            .checked_mul(DIMENSION)
            .ok_or(RuntimeError::OutputInvalid)?;
        let mut bytes = vec![0_u8; output_values.saturating_mul(4)];
        self.stdout
            .read_exact(&mut bytes)
            .map_err(|_| RuntimeError::InferenceFailed)?;
        decode_vectors(&bytes, output_rows)
    }
}

impl Drop for CoreMlWorker {
    fn drop(&mut self) {
        self.stdin.take();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn write_i32(writer: &mut impl Write, values: &[i32]) -> Result<(), RuntimeError> {
    let mut bytes = Vec::with_capacity(values.len().saturating_mul(4));
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    writer
        .write_all(&bytes)
        .map_err(|_| RuntimeError::RuntimeUnavailable)
}

fn decode_vectors(bytes: &[u8], output_rows: usize) -> Result<Vec<Vec<f32>>, RuntimeError> {
    let output_bytes = output_rows
        .checked_mul(DIMENSION)
        .and_then(|values| values.checked_mul(4))
        .ok_or(RuntimeError::OutputInvalid)?;
    if output_rows == 0 || output_rows > MAX_INPUTS || bytes.len() < output_bytes {
        return Err(RuntimeError::OutputInvalid);
    }
    bytes[..output_bytes]
        .chunks_exact(DIMENSION * 4)
        .map(|row| {
            row.chunks_exact(4)
                .map(|value| {
                    let value = f32::from_le_bytes(
                        value.try_into().map_err(|_| RuntimeError::OutputInvalid)?,
                    );
                    value
                        .is_finite()
                        .then_some(value)
                        .ok_or(RuntimeError::OutputInvalid)
                })
                .collect()
        })
        .collect()
}

fn environment_path(name: &str) -> Result<PathBuf, RuntimeError> {
    let path = PathBuf::from(env::var_os(name).ok_or(RuntimeError::EnvironmentInvalid)?);
    if !path.is_absolute() || path.as_os_str().len() > 4096 {
        return Err(RuntimeError::EnvironmentInvalid);
    }
    Ok(path)
}

fn regular_executable(path: PathBuf) -> Result<PathBuf, RuntimeError> {
    let metadata = fs::symlink_metadata(&path).map_err(|_| RuntimeError::EnvironmentInvalid)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(RuntimeError::EnvironmentInvalid);
    }
    Ok(path)
}

fn regular_directory(path: &Path) -> Result<(), RuntimeError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| RuntimeError::EnvironmentInvalid)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(RuntimeError::EnvironmentInvalid);
    }
    Ok(())
}

fn validate_file(path: &Path, bytes: u64, digest: &str) -> Result<(), RuntimeError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| RuntimeError::EnvironmentInvalid)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != bytes {
        return Err(RuntimeError::EnvironmentInvalid);
    }
    let mut file = File::open(path).map_err(|_| RuntimeError::EnvironmentInvalid)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| RuntimeError::EnvironmentInvalid)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    if format!("{:x}", hasher.finalize()) != digest {
        return Err(RuntimeError::EnvironmentInvalid);
    }
    Ok(())
}

fn compiled_model(path: PathBuf) -> Result<PathBuf, RuntimeError> {
    let metadata = fs::symlink_metadata(&path).map_err(|_| RuntimeError::EnvironmentInvalid)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || path.extension().and_then(|value| value.to_str()) != Some("mlmodelc")
    {
        return Err(RuntimeError::EnvironmentInvalid);
    }
    Ok(path)
}

#[cfg(test)]
#[path = "coreml_product_experiment_tests.rs"]
mod tests;
