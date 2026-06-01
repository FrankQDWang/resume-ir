pub fn crate_name() -> &'static str {
    "index-vector"
}

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const SNAPSHOT_FILE: &str = "vector.snapshot";
const SNAPSHOT_TMP_FILE: &str = "vector.snapshot.tmp";
const SNAPSHOT_HEADER_V1: &str = "resume-ir-vector-index-v1";
const SNAPSHOT_HEADER_V2: &str = "resume-ir-vector-index-v2";

pub trait VectorIndex {
    fn upsert(&self, vectors: Vec<VectorDocument>) -> Result<(), VectorIndexError>;
    fn mark_deleted(&self, vector_ids: &[&str]) -> Result<(), VectorIndexError>;
    fn knn(&self, query: QueryVector, k: usize) -> Result<Vec<VectorHit>, VectorIndexError>;
    fn knn_for_model(
        &self,
        query: QueryVector,
        k: usize,
        model_id: &str,
    ) -> Result<Vec<VectorHit>, VectorIndexError>;
    fn snapshot(&self) -> Result<VectorSnapshot, VectorIndexError>;
}

#[derive(Debug)]
pub struct InMemoryVectorIndex {
    dimension: usize,
    state: Mutex<IndexState>,
}

impl InMemoryVectorIndex {
    pub fn new(dimension: usize) -> Self {
        Self {
            dimension,
            state: Mutex::new(IndexState::default()),
        }
    }
}

pub struct PersistentVectorIndex {
    dimension: usize,
    snapshot_path: PathBuf,
    temp_path: PathBuf,
    state: Mutex<IndexState>,
}

impl PersistentVectorIndex {
    pub fn open(root: impl AsRef<Path>, dimension: usize) -> Result<Self, VectorIndexError> {
        if dimension == 0 {
            return Err(VectorIndexError::InvalidDimension {
                expected: 1,
                actual: 0,
            });
        }

        let root = root.as_ref();
        fs::create_dir_all(root).map_err(|_| VectorIndexError::Storage)?;
        let snapshot_path = root.join(SNAPSHOT_FILE);
        let temp_path = root.join(SNAPSHOT_TMP_FILE);
        let state = if snapshot_path.exists() {
            read_snapshot(&snapshot_path, dimension)?
        } else {
            IndexState::default()
        };

        Ok(Self {
            dimension,
            snapshot_path,
            temp_path,
            state: Mutex::new(state),
        })
    }

    fn persist_state(&self, state: &IndexState) -> Result<(), VectorIndexError> {
        write_snapshot(&self.temp_path, self.dimension, state)?;
        fs::rename(&self.temp_path, &self.snapshot_path).map_err(|_| VectorIndexError::Storage)?;
        Ok(())
    }
}

pub fn inspect_persistent_vector_snapshot(
    root: impl AsRef<Path>,
) -> PersistentVectorSnapshotInspection {
    let snapshot_path = root.as_ref().join(SNAPSHOT_FILE);
    if !snapshot_path.exists() {
        return PersistentVectorSnapshotInspection {
            state: PersistentVectorSnapshotState::Missing,
            snapshot: None,
        };
    }

    match read_snapshot_unchecked_dimension(&snapshot_path) {
        Ok((dimension, state)) => PersistentVectorSnapshotInspection {
            state: PersistentVectorSnapshotState::Ready,
            snapshot: Some(snapshot_from_state(&state, dimension)),
        },
        Err(VectorIndexError::Storage) => PersistentVectorSnapshotInspection {
            state: PersistentVectorSnapshotState::Unreadable,
            snapshot: None,
        },
        Err(_) => PersistentVectorSnapshotInspection {
            state: PersistentVectorSnapshotState::Corrupt,
            snapshot: None,
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PersistentVectorSnapshotInspection {
    state: PersistentVectorSnapshotState,
    snapshot: Option<VectorSnapshot>,
}

impl PersistentVectorSnapshotInspection {
    pub fn state(self) -> PersistentVectorSnapshotState {
        self.state
    }

    pub fn snapshot(self) -> Option<VectorSnapshot> {
        self.snapshot
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersistentVectorSnapshotState {
    Missing,
    Ready,
    Corrupt,
    Unreadable,
}

impl fmt::Debug for PersistentVectorIndex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PersistentVectorIndex")
            .field("snapshot_path", &"<redacted>")
            .field("dimension", &self.dimension)
            .finish_non_exhaustive()
    }
}

impl VectorIndex for PersistentVectorIndex {
    fn upsert(&self, vectors: Vec<VectorDocument>) -> Result<(), VectorIndexError> {
        for vector in &vectors {
            validate_dimension(self.dimension, vector.values())?;
        }

        let mut state = self.state.lock().map_err(|_| VectorIndexError::Poisoned)?;
        for vector in vectors {
            state.deleted.remove(vector.vector_id());
            state.vectors.insert(vector.vector_id().to_string(), vector);
        }
        self.persist_state(&state)
    }

    fn mark_deleted(&self, vector_ids: &[&str]) -> Result<(), VectorIndexError> {
        let mut state = self.state.lock().map_err(|_| VectorIndexError::Poisoned)?;
        for vector_id in vector_ids {
            state.deleted.insert((*vector_id).to_string());
        }
        self.persist_state(&state)
    }

    fn knn(&self, query: QueryVector, k: usize) -> Result<Vec<VectorHit>, VectorIndexError> {
        validate_dimension(self.dimension, query.values())?;
        let state = self.state.lock().map_err(|_| VectorIndexError::Poisoned)?;
        Ok(knn_from_state(&state, query.values(), k, None))
    }

    fn knn_for_model(
        &self,
        query: QueryVector,
        k: usize,
        model_id: &str,
    ) -> Result<Vec<VectorHit>, VectorIndexError> {
        validate_dimension(self.dimension, query.values())?;
        validate_model_id(model_id)?;
        let state = self.state.lock().map_err(|_| VectorIndexError::Poisoned)?;
        Ok(knn_from_state(&state, query.values(), k, Some(model_id)))
    }

    fn snapshot(&self) -> Result<VectorSnapshot, VectorIndexError> {
        let state = self.state.lock().map_err(|_| VectorIndexError::Poisoned)?;
        Ok(snapshot_from_state(&state, self.dimension))
    }
}

impl VectorIndex for InMemoryVectorIndex {
    fn upsert(&self, vectors: Vec<VectorDocument>) -> Result<(), VectorIndexError> {
        let mut state = self.state.lock().map_err(|_| VectorIndexError::Poisoned)?;
        for vector in vectors {
            validate_dimension(self.dimension, vector.values())?;
            state.deleted.remove(vector.vector_id());
            state.vectors.insert(vector.vector_id().to_string(), vector);
        }
        Ok(())
    }

    fn mark_deleted(&self, vector_ids: &[&str]) -> Result<(), VectorIndexError> {
        let mut state = self.state.lock().map_err(|_| VectorIndexError::Poisoned)?;
        for vector_id in vector_ids {
            state.deleted.insert((*vector_id).to_string());
        }
        Ok(())
    }

    fn knn(&self, query: QueryVector, k: usize) -> Result<Vec<VectorHit>, VectorIndexError> {
        validate_dimension(self.dimension, query.values())?;
        let state = self.state.lock().map_err(|_| VectorIndexError::Poisoned)?;
        Ok(knn_from_state(&state, query.values(), k, None))
    }

    fn knn_for_model(
        &self,
        query: QueryVector,
        k: usize,
        model_id: &str,
    ) -> Result<Vec<VectorHit>, VectorIndexError> {
        validate_dimension(self.dimension, query.values())?;
        validate_model_id(model_id)?;
        let state = self.state.lock().map_err(|_| VectorIndexError::Poisoned)?;
        Ok(knn_from_state(&state, query.values(), k, Some(model_id)))
    }

    fn snapshot(&self) -> Result<VectorSnapshot, VectorIndexError> {
        let state = self.state.lock().map_err(|_| VectorIndexError::Poisoned)?;
        Ok(snapshot_from_state(&state, self.dimension))
    }
}

#[derive(Default, Debug)]
struct IndexState {
    vectors: BTreeMap<String, VectorDocument>,
    deleted: BTreeSet<String>,
}

#[derive(Clone, PartialEq)]
pub struct VectorDocument {
    vector_id: String,
    doc_id: String,
    model_id: Option<String>,
    values: Vec<f32>,
}

impl VectorDocument {
    pub fn new(
        vector_id: impl Into<String>,
        doc_id: impl Into<String>,
        values: Vec<f32>,
    ) -> Result<Self, VectorIndexError> {
        if values.is_empty() {
            return Err(VectorIndexError::InvalidDimension {
                expected: 1,
                actual: 0,
            });
        }

        Ok(Self {
            vector_id: vector_id.into(),
            doc_id: doc_id.into(),
            model_id: None,
            values,
        })
    }

    pub fn new_for_model(
        model_id: impl Into<String>,
        vector_id: impl Into<String>,
        doc_id: impl Into<String>,
        values: Vec<f32>,
    ) -> Result<Self, VectorIndexError> {
        let model_id = model_id.into();
        validate_model_id(&model_id)?;
        let mut document = Self::new(vector_id, doc_id, values)?;
        document.model_id = Some(model_id);
        Ok(document)
    }

    pub fn vector_id(&self) -> &str {
        &self.vector_id
    }

    pub fn doc_id(&self) -> &str {
        &self.doc_id
    }

    pub fn model_id(&self) -> Option<&str> {
        self.model_id.as_deref()
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }
}

impl fmt::Debug for VectorDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VectorDocument")
            .field("vector_id", &self.vector_id)
            .field("doc_id", &self.doc_id)
            .field("model_id", &self.model_id.as_deref().unwrap_or("<legacy>"))
            .field("dimension", &self.values.len())
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct QueryVector {
    values: Vec<f32>,
}

impl QueryVector {
    pub fn new(values: Vec<f32>) -> Result<Self, VectorIndexError> {
        if values.is_empty() {
            return Err(VectorIndexError::InvalidDimension {
                expected: 1,
                actual: 0,
            });
        }

        Ok(Self { values })
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }
}

impl fmt::Debug for QueryVector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QueryVector")
            .field("dimension", &self.values.len())
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct VectorHit {
    vector_id: String,
    doc_id: String,
    score: f32,
}

impl VectorHit {
    fn new(vector_id: String, doc_id: String, score: f32) -> Self {
        Self {
            vector_id,
            doc_id,
            score,
        }
    }

    pub fn vector_id(&self) -> &str {
        &self.vector_id
    }

    pub fn doc_id(&self) -> &str {
        &self.doc_id
    }

    pub fn score(&self) -> f32 {
        self.score
    }
}

impl fmt::Debug for VectorHit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VectorHit")
            .field("vector_id", &self.vector_id)
            .field("doc_id", &self.doc_id)
            .field("score", &self.score)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VectorSnapshot {
    vector_count: usize,
    deleted_count: usize,
    dimension: usize,
}

impl VectorSnapshot {
    pub fn vector_count(self) -> usize {
        self.vector_count
    }

    pub fn deleted_count(self) -> usize {
        self.deleted_count
    }

    pub fn dimension(self) -> usize {
        self.dimension
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VectorIndexError {
    InvalidDimension { expected: usize, actual: usize },
    InvalidModelId,
    Poisoned,
    Storage,
    CorruptSnapshot,
}

impl fmt::Display for VectorIndexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimension { expected, actual } => write!(
                formatter,
                "vector dimension must be {expected}, got {actual}"
            ),
            Self::InvalidModelId => formatter.write_str("vector model id is invalid"),
            Self::Poisoned => formatter.write_str("vector index state is unavailable"),
            Self::Storage => formatter.write_str("vector index storage is unavailable"),
            Self::CorruptSnapshot => formatter.write_str("vector index snapshot is corrupt"),
        }
    }
}

impl std::error::Error for VectorIndexError {}

fn validate_dimension(expected: usize, values: &[f32]) -> Result<(), VectorIndexError> {
    if values.len() == expected {
        Ok(())
    } else {
        Err(VectorIndexError::InvalidDimension {
            expected,
            actual: values.len(),
        })
    }
}

fn validate_model_id(model_id: &str) -> Result<(), VectorIndexError> {
    if model_id.trim().is_empty()
        || model_id.contains('\n')
        || model_id.contains('\r')
        || model_id.contains('\t')
    {
        Err(VectorIndexError::InvalidModelId)
    } else {
        Ok(())
    }
}

fn knn_from_state(
    state: &IndexState,
    query: &[f32],
    k: usize,
    model_id: Option<&str>,
) -> Vec<VectorHit> {
    let mut hits = state
        .vectors
        .values()
        .filter(|vector| !state.deleted.contains(vector.vector_id()))
        .filter(|vector| {
            model_id
                .map(|model_id| vector_matches_model(vector, model_id))
                .unwrap_or(true)
        })
        .map(|vector| {
            VectorHit::new(
                vector.vector_id().to_string(),
                vector.doc_id().to_string(),
                cosine_similarity(query, vector.values()),
            )
        })
        .collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        right
            .score()
            .partial_cmp(&left.score())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.doc_id().cmp(right.doc_id()))
    });
    hits.truncate(k);
    hits
}

fn vector_matches_model(vector: &VectorDocument, model_id: &str) -> bool {
    match vector.model_id() {
        Some(vector_model_id) => vector_model_id == model_id,
        None => legacy_vector_model_id(vector.vector_id()) == Some(model_id),
    }
}

fn legacy_vector_model_id(vector_id: &str) -> Option<&str> {
    vector_id
        .split_once(':')
        .map(|(model_id, _)| model_id)
        .filter(|model_id| !model_id.is_empty())
}

fn snapshot_from_state(state: &IndexState, dimension: usize) -> VectorSnapshot {
    VectorSnapshot {
        vector_count: state.vectors.len(),
        deleted_count: state.deleted.len(),
        dimension,
    }
}

fn read_snapshot(path: &Path, expected_dimension: usize) -> Result<IndexState, VectorIndexError> {
    let (actual_dimension, state) = read_snapshot_unchecked_dimension(path)?;
    if actual_dimension != expected_dimension {
        return Err(VectorIndexError::InvalidDimension {
            expected: expected_dimension,
            actual: actual_dimension,
        });
    }

    Ok(state)
}

fn read_snapshot_unchecked_dimension(path: &Path) -> Result<(usize, IndexState), VectorIndexError> {
    let file = File::open(path).map_err(|_| VectorIndexError::Storage)?;
    let mut lines = BufReader::new(file).lines();
    let header = lines
        .next()
        .ok_or(VectorIndexError::CorruptSnapshot)?
        .map_err(|_| VectorIndexError::Storage)?;
    let mut header_parts = header.split('\t');
    let snapshot_version = match header_parts.next() {
        Some(SNAPSHOT_HEADER_V1) => SnapshotVersion::V1,
        Some(SNAPSHOT_HEADER_V2) => SnapshotVersion::V2,
        _ => return Err(VectorIndexError::CorruptSnapshot),
    };
    if header_parts.next() != Some("dimension") {
        return Err(VectorIndexError::CorruptSnapshot);
    }
    let actual_dimension = header_parts
        .next()
        .ok_or(VectorIndexError::CorruptSnapshot)?
        .parse::<usize>()
        .map_err(|_| VectorIndexError::CorruptSnapshot)?;
    if header_parts.next().is_some() {
        return Err(VectorIndexError::CorruptSnapshot);
    }

    let mut state = IndexState::default();
    for line in lines {
        let line = line.map_err(|_| VectorIndexError::Storage)?;
        let mut parts = line.split('\t');
        match parts.next() {
            Some("V") => {
                let vector_id =
                    decode_field(parts.next().ok_or(VectorIndexError::CorruptSnapshot)?)?;
                let doc_id = decode_field(parts.next().ok_or(VectorIndexError::CorruptSnapshot)?)?;
                let first_payload =
                    decode_field(parts.next().ok_or(VectorIndexError::CorruptSnapshot)?)?;
                let (model_id, values) = match snapshot_version {
                    SnapshotVersion::V1 => {
                        if parts.next().is_some() {
                            return Err(VectorIndexError::CorruptSnapshot);
                        }
                        (None, decode_values(&first_payload)?)
                    }
                    SnapshotVersion::V2 => {
                        let values =
                            decode_values(parts.next().ok_or(VectorIndexError::CorruptSnapshot)?)?;
                        if parts.next().is_some() {
                            return Err(VectorIndexError::CorruptSnapshot);
                        }
                        if first_payload.is_empty() {
                            (None, values)
                        } else {
                            validate_model_id(&first_payload)?;
                            (Some(first_payload), values)
                        }
                    }
                };
                validate_dimension(actual_dimension, &values)?;
                let mut document = VectorDocument::new(vector_id.clone(), doc_id, values)?;
                document.model_id = model_id;
                state.vectors.insert(vector_id, document);
            }
            Some("D") => {
                let vector_id =
                    decode_field(parts.next().ok_or(VectorIndexError::CorruptSnapshot)?)?;
                if parts.next().is_some() {
                    return Err(VectorIndexError::CorruptSnapshot);
                }
                state.deleted.insert(vector_id);
            }
            _ => return Err(VectorIndexError::CorruptSnapshot),
        }
    }

    Ok((actual_dimension, state))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SnapshotVersion {
    V1,
    V2,
}

fn write_snapshot(
    path: &Path,
    dimension: usize,
    state: &IndexState,
) -> Result<(), VectorIndexError> {
    let mut file = File::create(path).map_err(|_| VectorIndexError::Storage)?;
    writeln!(file, "{SNAPSHOT_HEADER_V2}\tdimension\t{dimension}")
        .map_err(|_| VectorIndexError::Storage)?;
    for vector in state.vectors.values() {
        writeln!(
            file,
            "V\t{}\t{}\t{}\t{}",
            encode_field(vector.vector_id()),
            encode_field(vector.doc_id()),
            encode_field(vector.model_id().unwrap_or("")),
            encode_values(vector.values())
        )
        .map_err(|_| VectorIndexError::Storage)?;
    }
    for vector_id in &state.deleted {
        writeln!(file, "D\t{}", encode_field(vector_id)).map_err(|_| VectorIndexError::Storage)?;
    }
    file.sync_all().map_err(|_| VectorIndexError::Storage)?;
    Ok(())
}

fn encode_values(values: &[f32]) -> String {
    values
        .iter()
        .map(|value| format!("{:08x}", value.to_bits()))
        .collect::<Vec<_>>()
        .join(",")
}

fn decode_values(value: &str) -> Result<Vec<f32>, VectorIndexError> {
    if value.is_empty() {
        return Err(VectorIndexError::CorruptSnapshot);
    }
    value
        .split(',')
        .map(|part| {
            if part.len() != 8 {
                return Err(VectorIndexError::CorruptSnapshot);
            }
            let bits =
                u32::from_str_radix(part, 16).map_err(|_| VectorIndexError::CorruptSnapshot)?;
            Ok(f32::from_bits(bits))
        })
        .collect()
}

fn encode_field(value: &str) -> String {
    let mut output = String::new();
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' => {
                output.push(char::from(*byte));
            }
            _ => output.push_str(&format!("%{byte:02X}")),
        }
    }
    output
}

fn decode_field(value: &str) -> Result<String, VectorIndexError> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let Some(hex) = value.get(index + 1..index + 3) else {
                return Err(VectorIndexError::CorruptSnapshot);
            };
            let byte =
                u8::from_str_radix(hex, 16).map_err(|_| VectorIndexError::CorruptSnapshot)?;
            output.push(byte);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).map_err(|_| VectorIndexError::CorruptSnapshot)
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    let dot = left
        .iter()
        .zip(right.iter())
        .map(|(left, right)| left * right)
        .sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm * right_norm)
    }
}
