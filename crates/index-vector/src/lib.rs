pub fn crate_name() -> &'static str {
    "index-vector"
}

use fs4::fs_std::FileExt;
use hnsw_rs::prelude::{DistCosine, Hnsw};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as FmtWrite;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

const SNAPSHOT_FILE: &str = "vector.snapshot";
const SNAPSHOT_TMP_FILE: &str = "vector.snapshot.tmp";
const SNAPSHOT_LAST_GOOD_FILE: &str = "vector.snapshot.last-good";
const SNAPSHOT_LAST_GOOD_TMP_FILE: &str = "vector.snapshot.last-good.tmp";
const SNAPSHOT_KEY_FILE: &str = "vector.snapshot.key-v1";
const SNAPSHOT_LOCK_FILE: &str = "vector.lock";
const SNAPSHOT_HEADER_ENCRYPTED_V1: &str = "resume-ir-vector-index-encrypted-v1";
const SNAPSHOT_PLAINTEXT_HEADER_V1: &str = "resume-ir-vector-index-plaintext-v1";
const SNAPSHOT_KEY_LEN: usize = 32;
const SNAPSHOT_NONCE_LEN: usize = 24;
const HNSW_MAX_CONNECTIONS: usize = 24;
const HNSW_MAX_LAYERS: usize = 16;
const HNSW_EF_CONSTRUCTION: usize = 200;
const HNSW_EF_SEARCH: usize = 64;

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
    last_good_path: PathBuf,
    last_good_temp_path: PathBuf,
    key_path: PathBuf,
    lock_path: PathBuf,
    state: Mutex<IndexState>,
    ann_backend: Mutex<HnswSearchBackend>,
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
        let last_good_path = root.join(SNAPSHOT_LAST_GOOD_FILE);
        let last_good_temp_path = root.join(SNAPSHOT_LAST_GOOD_TMP_FILE);
        let key_path = root.join(SNAPSHOT_KEY_FILE);
        let lock_path = root.join(SNAPSHOT_LOCK_FILE);
        let state =
            read_snapshot_with_recovery(&snapshot_path, &last_good_path, &key_path, dimension)?
                .unwrap_or_default();
        let ann_backend = HnswSearchBackend::build(&state);

        Ok(Self {
            dimension,
            snapshot_path,
            temp_path,
            last_good_path,
            last_good_temp_path,
            key_path,
            lock_path,
            state: Mutex::new(state),
            ann_backend: Mutex::new(ann_backend),
        })
    }

    fn persist_state(&self, state: &IndexState) -> Result<(), VectorIndexError> {
        write_snapshot(&self.temp_path, &self.key_path, self.dimension, state)?;
        self.refresh_last_good_snapshot()?;
        fs::rename(&self.temp_path, &self.snapshot_path).map_err(|_| VectorIndexError::Storage)?;
        Ok(())
    }

    fn refresh_last_good_snapshot(&self) -> Result<(), VectorIndexError> {
        if !self.snapshot_path.exists() {
            return Ok(());
        }

        match read_snapshot(&self.snapshot_path, &self.key_path, self.dimension) {
            Ok(_) => {}
            Err(VectorIndexError::CorruptSnapshot) => return Ok(()),
            Err(error) => return Err(error),
        }

        let snapshot_bytes =
            fs::read(&self.snapshot_path).map_err(|_| VectorIndexError::Storage)?;
        let mut file = create_private_file(&self.last_good_temp_path)?;
        file.write_all(&snapshot_bytes)
            .map_err(|_| VectorIndexError::Storage)?;
        file.sync_all().map_err(|_| VectorIndexError::Storage)?;
        fs::rename(&self.last_good_temp_path, &self.last_good_path)
            .map_err(|_| VectorIndexError::Storage)?;
        Ok(())
    }

    pub fn purge_doc_ids(&self, doc_ids: &BTreeSet<String>) -> Result<usize, VectorIndexError> {
        if doc_ids.is_empty() {
            return Ok(0);
        }

        self.mutate_latest_state(|state| {
            let vector_ids = state
                .vectors
                .values()
                .filter(|vector| doc_ids.contains(vector.doc_id()))
                .map(|vector| vector.vector_id().to_string())
                .collect::<Vec<_>>();
            let removed = vector_ids.len();
            for vector_id in vector_ids {
                state.vectors.remove(&vector_id);
                state.deleted.remove(&vector_id);
            }
            Ok(removed)
        })
    }

    fn mutate_latest_state<T>(
        &self,
        mutate: impl FnOnce(&mut IndexState) -> Result<T, VectorIndexError>,
    ) -> Result<T, VectorIndexError> {
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.lock_path)
            .map_err(|_| VectorIndexError::Storage)?;
        lock_file
            .lock_exclusive()
            .map_err(|_| VectorIndexError::Storage)?;
        let result = self.mutate_latest_state_locked(mutate);
        lock_file.unlock().map_err(|_| VectorIndexError::Storage)?;
        result
    }

    fn mutate_latest_state_locked<T>(
        &self,
        mutate: impl FnOnce(&mut IndexState) -> Result<T, VectorIndexError>,
    ) -> Result<T, VectorIndexError> {
        let mut latest = self.read_latest_state()?;
        let output = mutate(&mut latest)?;
        self.persist_state(&latest)?;
        {
            let mut state = self.state.lock().map_err(|_| VectorIndexError::Poisoned)?;
            *state = latest;
            self.rebuild_ann_backend(&state)?;
        }
        Ok(output)
    }

    fn read_latest_state(&self) -> Result<IndexState, VectorIndexError> {
        Ok(read_snapshot_with_recovery(
            &self.snapshot_path,
            &self.last_good_path,
            &self.key_path,
            self.dimension,
        )?
        .unwrap_or_default())
    }

    fn rebuild_ann_backend(&self, state: &IndexState) -> Result<(), VectorIndexError> {
        let mut ann_backend = self
            .ann_backend
            .lock()
            .map_err(|_| VectorIndexError::Poisoned)?;
        *ann_backend = HnswSearchBackend::build(state);
        Ok(())
    }
}

pub fn inspect_persistent_vector_snapshot(
    root: impl AsRef<Path>,
) -> PersistentVectorSnapshotInspection {
    let root = root.as_ref();
    let snapshot_path = root.join(SNAPSHOT_FILE);
    let last_good_path = root.join(SNAPSHOT_LAST_GOOD_FILE);
    let key_path = root.join(SNAPSHOT_KEY_FILE);
    if !snapshot_path.exists() {
        if last_good_path.exists() {
            return inspect_recovered_vector_snapshot(&last_good_path, &key_path);
        }
        return PersistentVectorSnapshotInspection {
            state: PersistentVectorSnapshotState::Missing,
            snapshot: None,
        };
    }

    match read_snapshot_unchecked_dimension(&snapshot_path, &key_path) {
        Ok((dimension, state)) => PersistentVectorSnapshotInspection {
            state: PersistentVectorSnapshotState::Ready,
            snapshot: Some(snapshot_from_state(
                &state,
                dimension,
                VectorSearchBackend::HnswAnn,
            )),
        },
        Err(VectorIndexError::Storage) => PersistentVectorSnapshotInspection {
            state: PersistentVectorSnapshotState::Unreadable,
            snapshot: None,
        },
        Err(_) => inspect_recovered_vector_snapshot(&last_good_path, &key_path),
    }
}

fn inspect_recovered_vector_snapshot(
    last_good_path: &Path,
    key_path: &Path,
) -> PersistentVectorSnapshotInspection {
    if !last_good_path.exists() {
        return PersistentVectorSnapshotInspection {
            state: PersistentVectorSnapshotState::Corrupt,
            snapshot: None,
        };
    }

    match read_snapshot_unchecked_dimension(last_good_path, key_path) {
        Ok((dimension, state)) => PersistentVectorSnapshotInspection {
            state: PersistentVectorSnapshotState::Recovered,
            snapshot: Some(snapshot_from_state(
                &state,
                dimension,
                VectorSearchBackend::HnswAnn,
            )),
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
    Recovered,
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

        self.mutate_latest_state(|state| {
            for vector in vectors {
                state.deleted.remove(vector.vector_id());
                state.vectors.insert(vector.vector_id().to_string(), vector);
            }
            Ok(())
        })
    }

    fn mark_deleted(&self, vector_ids: &[&str]) -> Result<(), VectorIndexError> {
        self.mutate_latest_state(|state| {
            for vector_id in vector_ids {
                state.deleted.insert((*vector_id).to_string());
            }
            Ok(())
        })
    }

    fn knn(&self, query: QueryVector, k: usize) -> Result<Vec<VectorHit>, VectorIndexError> {
        validate_dimension(self.dimension, query.values())?;
        let ann_backend = self
            .ann_backend
            .lock()
            .map_err(|_| VectorIndexError::Poisoned)?;
        Ok(ann_backend.knn(query.values(), k, None))
    }

    fn knn_for_model(
        &self,
        query: QueryVector,
        k: usize,
        model_id: &str,
    ) -> Result<Vec<VectorHit>, VectorIndexError> {
        validate_dimension(self.dimension, query.values())?;
        validate_model_id(model_id)?;
        let ann_backend = self
            .ann_backend
            .lock()
            .map_err(|_| VectorIndexError::Poisoned)?;
        Ok(ann_backend.knn(query.values(), k, Some(model_id)))
    }

    fn snapshot(&self) -> Result<VectorSnapshot, VectorIndexError> {
        let state = self.state.lock().map_err(|_| VectorIndexError::Poisoned)?;
        Ok(snapshot_from_state(
            &state,
            self.dimension,
            VectorSearchBackend::HnswAnn,
        ))
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
        Ok(snapshot_from_state(
            &state,
            self.dimension,
            VectorSearchBackend::LinearScan,
        ))
    }
}

#[derive(Default, Debug)]
struct IndexState {
    vectors: BTreeMap<String, VectorDocument>,
    deleted: BTreeSet<String>,
}

struct HnswSearchBackend {
    all: Option<HnswShard>,
    by_model: BTreeMap<String, HnswShard>,
}

impl HnswSearchBackend {
    fn build(state: &IndexState) -> Self {
        let active_documents = state
            .vectors
            .values()
            .filter(|vector| !state.deleted.contains(vector.vector_id()))
            .cloned()
            .collect::<Vec<_>>();
        let mut by_model_documents = BTreeMap::<String, Vec<VectorDocument>>::new();
        for vector in &active_documents {
            if let Some(model_id) = effective_model_id(vector) {
                by_model_documents
                    .entry(model_id.to_string())
                    .or_default()
                    .push(vector.clone());
            }
        }
        let by_model = by_model_documents
            .into_iter()
            .filter_map(|(model_id, documents)| {
                HnswShard::build(documents).map(|shard| (model_id, shard))
            })
            .collect::<BTreeMap<_, _>>();

        Self {
            all: HnswShard::build(active_documents),
            by_model,
        }
    }

    fn knn(&self, query: &[f32], k: usize, model_id: Option<&str>) -> Vec<VectorHit> {
        if k == 0 {
            return Vec::new();
        }
        match model_id {
            Some(model_id) => self
                .by_model
                .get(model_id)
                .map(|shard| shard.knn(query, k))
                .unwrap_or_default(),
            None => self
                .all
                .as_ref()
                .map(|shard| shard.knn(query, k))
                .unwrap_or_default(),
        }
    }
}

struct HnswShard {
    index: Hnsw<'static, f32, DistCosine>,
    documents: Vec<VectorDocument>,
}

impl HnswShard {
    fn build(documents: Vec<VectorDocument>) -> Option<Self> {
        if documents.is_empty() {
            return None;
        }
        let max_layer = HNSW_MAX_LAYERS
            .min((documents.len() as f32).ln().trunc() as usize)
            .max(1);
        let mut index = Hnsw::<f32, DistCosine>::new(
            HNSW_MAX_CONNECTIONS,
            documents.len(),
            max_layer,
            HNSW_EF_CONSTRUCTION,
            DistCosine {},
        );
        for (external_id, document) in documents.iter().enumerate() {
            index.insert((document.values(), external_id));
        }
        index.set_searching_mode(true);

        Some(Self { index, documents })
    }

    fn knn(&self, query: &[f32], k: usize) -> Vec<VectorHit> {
        let candidate_count = k.min(self.documents.len());
        if candidate_count == 0 {
            return Vec::new();
        }
        let ef_search = HNSW_EF_SEARCH.max(candidate_count);
        let mut hits = self
            .index
            .search(query, candidate_count, ef_search)
            .into_iter()
            .filter_map(|neighbour| self.documents.get(neighbour.d_id))
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
    search_backend: VectorSearchBackend,
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

    pub fn search_backend(self) -> VectorSearchBackend {
        self.search_backend
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorSearchBackend {
    LinearScan,
    HnswAnn,
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
    effective_model_id(vector) == Some(model_id)
}

fn effective_model_id(vector: &VectorDocument) -> Option<&str> {
    vector
        .model_id()
        .or_else(|| legacy_vector_model_id(vector.vector_id()))
}

fn legacy_vector_model_id(vector_id: &str) -> Option<&str> {
    vector_id
        .split_once(':')
        .map(|(model_id, _)| model_id)
        .filter(|model_id| !model_id.is_empty())
}

fn snapshot_from_state(
    state: &IndexState,
    dimension: usize,
    search_backend: VectorSearchBackend,
) -> VectorSnapshot {
    VectorSnapshot {
        vector_count: state.vectors.len(),
        deleted_count: state.deleted.len(),
        dimension,
        search_backend,
    }
}

fn read_snapshot(
    path: &Path,
    key_path: &Path,
    expected_dimension: usize,
) -> Result<IndexState, VectorIndexError> {
    let (actual_dimension, state) = read_snapshot_unchecked_dimension(path, key_path)?;
    if actual_dimension != expected_dimension {
        return Err(VectorIndexError::InvalidDimension {
            expected: expected_dimension,
            actual: actual_dimension,
        });
    }

    Ok(state)
}

fn read_snapshot_with_recovery(
    snapshot_path: &Path,
    last_good_path: &Path,
    key_path: &Path,
    expected_dimension: usize,
) -> Result<Option<IndexState>, VectorIndexError> {
    if snapshot_path.exists() {
        return match read_snapshot(snapshot_path, key_path, expected_dimension) {
            Ok(state) => Ok(Some(state)),
            Err(VectorIndexError::CorruptSnapshot) => {
                if !last_good_path.exists() {
                    return Err(VectorIndexError::CorruptSnapshot);
                }
                read_snapshot(last_good_path, key_path, expected_dimension).map(Some)
            }
            Err(error) => Err(error),
        };
    }

    read_last_good_snapshot(last_good_path, key_path, expected_dimension)
}

fn read_last_good_snapshot(
    last_good_path: &Path,
    key_path: &Path,
    expected_dimension: usize,
) -> Result<Option<IndexState>, VectorIndexError> {
    if !last_good_path.exists() {
        return Ok(None);
    }

    read_snapshot(last_good_path, key_path, expected_dimension).map(Some)
}

fn read_snapshot_unchecked_dimension(
    path: &Path,
    key_path: &Path,
) -> Result<(usize, IndexState), VectorIndexError> {
    let file = File::open(path).map_err(|_| VectorIndexError::Storage)?;
    let mut lines = BufReader::new(file).lines();
    let header = lines
        .next()
        .ok_or(VectorIndexError::CorruptSnapshot)?
        .map_err(|_| VectorIndexError::Storage)?;
    if header != SNAPSHOT_HEADER_ENCRYPTED_V1 {
        return Err(VectorIndexError::CorruptSnapshot);
    }
    let nonce_hex = lines
        .next()
        .ok_or(VectorIndexError::CorruptSnapshot)?
        .map_err(|_| VectorIndexError::Storage)?;
    let ciphertext_hex = lines
        .next()
        .ok_or(VectorIndexError::CorruptSnapshot)?
        .map_err(|_| VectorIndexError::Storage)?;
    if lines.next().is_some() {
        return Err(VectorIndexError::CorruptSnapshot);
    }
    let nonce = decode_fixed_hex::<SNAPSHOT_NONCE_LEN>(&nonce_hex)?;
    let ciphertext = decode_hex(&ciphertext_hex)?;
    let key = read_snapshot_key(key_path)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let plaintext = cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: SNAPSHOT_HEADER_ENCRYPTED_V1.as_bytes(),
            },
        )
        .map_err(|_| VectorIndexError::CorruptSnapshot)?;

    parse_snapshot_plaintext(&plaintext)
}

fn parse_snapshot_plaintext(bytes: &[u8]) -> Result<(usize, IndexState), VectorIndexError> {
    let mut lines = BufReader::new(bytes).lines();
    let header = lines
        .next()
        .ok_or(VectorIndexError::CorruptSnapshot)?
        .map_err(|_| VectorIndexError::Storage)?;
    let mut header_parts = header.split('\t');
    if header_parts.next() != Some(SNAPSHOT_PLAINTEXT_HEADER_V1) {
        return Err(VectorIndexError::CorruptSnapshot);
    }
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
                let model_id =
                    decode_field(parts.next().ok_or(VectorIndexError::CorruptSnapshot)?)?;
                let values = decode_values(parts.next().ok_or(VectorIndexError::CorruptSnapshot)?)?;
                if parts.next().is_some() {
                    return Err(VectorIndexError::CorruptSnapshot);
                }
                let model_id = if model_id.is_empty() {
                    None
                } else {
                    validate_model_id(&model_id)?;
                    Some(model_id)
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

fn write_snapshot(
    path: &Path,
    key_path: &Path,
    dimension: usize,
    state: &IndexState,
) -> Result<(), VectorIndexError> {
    let plaintext = snapshot_plaintext(dimension, state)?;
    let key = load_or_create_snapshot_key(key_path)?;
    let nonce = random_nonce()?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext.as_bytes(),
                aad: SNAPSHOT_HEADER_ENCRYPTED_V1.as_bytes(),
            },
        )
        .map_err(|_| VectorIndexError::Storage)?;

    let mut file = create_private_file(path)?;
    writeln!(file, "{SNAPSHOT_HEADER_ENCRYPTED_V1}").map_err(|_| VectorIndexError::Storage)?;
    writeln!(file, "{}", encode_hex(&nonce)).map_err(|_| VectorIndexError::Storage)?;
    writeln!(file, "{}", encode_hex(&ciphertext)).map_err(|_| VectorIndexError::Storage)?;
    file.sync_all().map_err(|_| VectorIndexError::Storage)?;
    Ok(())
}

fn snapshot_plaintext(dimension: usize, state: &IndexState) -> Result<String, VectorIndexError> {
    let mut output = String::new();
    writeln!(
        output,
        "{SNAPSHOT_PLAINTEXT_HEADER_V1}\tdimension\t{dimension}"
    )
    .map_err(|_| VectorIndexError::Storage)?;
    for vector in state.vectors.values() {
        writeln!(
            output,
            "V\t{}\t{}\t{}\t{}",
            encode_field(vector.vector_id()),
            encode_field(vector.doc_id()),
            encode_field(vector.model_id().unwrap_or("")),
            encode_values(vector.values())
        )
        .map_err(|_| VectorIndexError::Storage)?;
    }
    for vector_id in &state.deleted {
        writeln!(output, "D\t{}", encode_field(vector_id))
            .map_err(|_| VectorIndexError::Storage)?;
    }
    Ok(output)
}

fn load_or_create_snapshot_key(
    key_path: &Path,
) -> Result<[u8; SNAPSHOT_KEY_LEN], VectorIndexError> {
    match read_snapshot_key(key_path) {
        Ok(key) => Ok(key),
        Err(VectorIndexError::Storage) if !key_path.exists() => {
            let key = random_key()?;
            write_private_file(key_path, encode_hex(&key).as_bytes())?;
            Ok(key)
        }
        Err(error) => Err(error),
    }
}

fn read_snapshot_key(key_path: &Path) -> Result<[u8; SNAPSHOT_KEY_LEN], VectorIndexError> {
    let value = fs::read_to_string(key_path).map_err(|_| VectorIndexError::Storage)?;
    decode_fixed_hex::<SNAPSHOT_KEY_LEN>(value.trim())
}

fn random_key() -> Result<[u8; SNAPSHOT_KEY_LEN], VectorIndexError> {
    let mut key = [0_u8; SNAPSHOT_KEY_LEN];
    getrandom::getrandom(&mut key).map_err(|_| VectorIndexError::Storage)?;
    Ok(key)
}

fn random_nonce() -> Result<[u8; SNAPSHOT_NONCE_LEN], VectorIndexError> {
    let mut nonce = [0_u8; SNAPSHOT_NONCE_LEN];
    getrandom::getrandom(&mut nonce).map_err(|_| VectorIndexError::Storage)?;
    Ok(nonce)
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), VectorIndexError> {
    let mut file = create_private_file(path)?;
    file.write_all(bytes)
        .map_err(|_| VectorIndexError::Storage)?;
    file.write_all(b"\n")
        .map_err(|_| VectorIndexError::Storage)?;
    file.sync_all().map_err(|_| VectorIndexError::Storage)?;
    restrict_private_file_permissions(path)?;
    Ok(())
}

fn create_private_file(path: &Path) -> Result<File, VectorIndexError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| VectorIndexError::Storage)?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|_| VectorIndexError::Storage)?;
    restrict_private_file_permissions(path)?;
    Ok(file)
}

#[cfg(unix)]
fn restrict_private_file_permissions(path: &Path) -> Result<(), VectorIndexError> {
    let mut permissions = fs::metadata(path)
        .map_err(|_| VectorIndexError::Storage)?
        .permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions).map_err(|_| VectorIndexError::Storage)
}

#[cfg(not(unix))]
fn restrict_private_file_permissions(_path: &Path) -> Result<(), VectorIndexError> {
    Ok(())
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn decode_fixed_hex<const N: usize>(value: &str) -> Result<[u8; N], VectorIndexError> {
    let bytes = decode_hex(value)?;
    bytes
        .try_into()
        .map_err(|_| VectorIndexError::CorruptSnapshot)
}

fn decode_hex(value: &str) -> Result<Vec<u8>, VectorIndexError> {
    if !value.len().is_multiple_of(2) {
        return Err(VectorIndexError::CorruptSnapshot);
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    let mut index = 0;
    while index < value.len() {
        let byte = u8::from_str_radix(&value[index..index + 2], 16)
            .map_err(|_| VectorIndexError::CorruptSnapshot)?;
        bytes.push(byte);
        index += 2;
    }
    Ok(bytes)
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
