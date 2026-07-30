use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use core_domain::{ActiveSearchProjection, ContentDigest};
use index_fulltext::{FullTextIndex, SnapshotReadLease};
use index_vector::{
    VectorModelContract, VectorSnapshotReadLease, VectorSnapshotReader, VectorSnapshotRoot,
};
use meta_store::{
    MetaStoreError, MetaStoreErrorClass, ReadMetaStore, SearchMetadataSnapshot,
    SearchMetadataTransactionError, SearchPublicationRecord, SearchPublicationState,
    VectorSnapshotMode,
};

use crate::error::SearchRuntimeError;
use crate::scope::{LexicalQueryScope, QueryScope};

pub struct QueryCoordinator {
    data_dir: PathBuf,
    store: ReadMetaStore,
    fulltext_root: PathBuf,
    vector_root: VectorSnapshotRoot,
    fulltext_cache: Option<ValidatedFullTextGeneration>,
    vector_cache: Option<ValidatedVectorGeneration>,
    pending_generation: Option<PreparedQueryGeneration>,
    pending_artifact_fault: Option<SearchArtifactFaultKey>,
}

/// Exact generation-pinned query readers prepared before publication becomes
/// visible. Construction performs the same deep artifact and projection
/// validation as a cold query; installation never makes the generation visible
/// without a matching metadata publication key.
pub struct PreparedQueryGeneration {
    data_dir: PathBuf,
    fulltext: ValidatedFullTextGeneration,
    vector: ValidatedVectorGeneration,
}

/// Exact immutable publication identity whose payload failed deep validation.
#[derive(Clone, PartialEq, Eq)]
pub struct SearchArtifactFaultKey {
    generation: String,
    publication_fingerprint: ContentDigest,
}

impl SearchArtifactFaultKey {
    pub fn generation(&self) -> &str {
        &self.generation
    }

    pub fn publication_fingerprint(&self) -> &ContentDigest {
        &self.publication_fingerprint
    }
}

impl fmt::Debug for SearchArtifactFaultKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchArtifactFaultKey")
            .field("generation", &"<redacted>")
            .field("publication_fingerprint", &self.publication_fingerprint)
            .finish()
    }
}

struct ValidatedFullTextGeneration {
    key: CacheKey,
    fulltext: FullTextIndex,
}

struct ValidatedVectorGeneration {
    key: CacheKey,
    vector: VectorSnapshotReader,
}

#[derive(Clone, PartialEq, Eq)]
struct CacheKey {
    generation: String,
    publication_fingerprint: ContentDigest,
}

impl QueryCoordinator {
    pub fn open(data_dir: &Path) -> Result<Self, SearchRuntimeError> {
        let data_dir = fs::canonicalize(data_dir).map_err(|_| SearchRuntimeError::unavailable())?;
        let store = ReadMetaStore::open_data_dir(&data_dir).map_err(map_store_error)?;
        let fulltext_root = data_dir.join("search-index");
        let vector_root = VectorSnapshotRoot::new(data_dir.join("vector-index"))
            .map_err(|_| SearchRuntimeError::unavailable())?;
        Ok(Self {
            data_dir,
            store,
            fulltext_root,
            vector_root,
            fulltext_cache: None,
            vector_cache: None,
            pending_generation: None,
            pending_artifact_fault: None,
        })
    }

    pub fn prepare_current_generation(&mut self) -> Result<(), SearchRuntimeError> {
        let (publication, projections) = self
            .store
            .with_search_metadata_snapshot(|snapshot| {
                Ok::<_, SearchRuntimeError>((
                    snapshot.head().publication.clone(),
                    snapshot
                        .validated_active_projections()
                        .map_err(|_| SearchRuntimeError::integrity())?,
                ))
            })
            .map_err(map_transaction_error)?;
        let prepared = PreparedQueryGeneration::open(&self.data_dir, &publication, &projections)?;
        self.fulltext_cache = Some(prepared.fulltext);
        self.vector_cache = Some(prepared.vector);
        self.pending_generation = None;
        Ok(())
    }

    pub fn install_prepared_generation(
        &mut self,
        prepared: PreparedQueryGeneration,
    ) -> Result<(), SearchRuntimeError> {
        if prepared.data_dir != self.data_dir {
            return Err(SearchRuntimeError::integrity());
        }
        if self.pending_generation.is_some() {
            return Err(SearchRuntimeError::unavailable());
        }
        self.pending_generation = Some(prepared);
        Ok(())
    }

    /// Atomically replaces the active readers with the exact pending
    /// generation after its metadata publication is committed.
    pub fn activate_prepared_generation(&mut self) -> Result<(), SearchRuntimeError> {
        let key = self
            .store
            .with_search_metadata_snapshot(|snapshot| {
                cache_key(snapshot.head().publication.clone())
            })
            .map_err(map_transaction_error)?;
        if let Some(prepared) = self.pending_generation.take() {
            if prepared.fulltext.key != key || prepared.vector.key != key {
                return Err(SearchRuntimeError::integrity());
            }
            self.fulltext_cache = Some(prepared.fulltext);
            self.vector_cache = Some(prepared.vector);
            return Ok(());
        }
        let active_matches = self
            .fulltext_cache
            .as_ref()
            .is_some_and(|cached| cached.key == key)
            && self
                .vector_cache
                .as_ref()
                .is_some_and(|cached| cached.key == key);
        active_matches
            .then_some(())
            .ok_or_else(SearchRuntimeError::unavailable)
    }

    /// Drops a publication that did not commit, releasing its complete reader
    /// pair before another generation may be prepared.
    pub fn discard_prepared_generation(&mut self) {
        self.pending_generation = None;
    }

    /// Counts distinct immutable search generations whose readers are resident.
    ///
    /// This is a bounded resource invariant, not a corpus or query diagnostic:
    /// callers can prove reader ownership without exposing generation identity.
    pub fn resident_generation_count(&self) -> usize {
        let mut keys = Vec::<&CacheKey>::new();
        if let Some(cached) = self.fulltext_cache.as_ref() {
            keys.push(&cached.key);
        }
        if let Some(cached) = self.vector_cache.as_ref() {
            keys.push(&cached.key);
        }
        if let Some(prepared) = self.pending_generation.as_ref() {
            keys.push(&prepared.fulltext.key);
        }
        keys.sort_by(|left, right| left.generation.cmp(&right.generation));
        keys.dedup_by(|left, right| *left == *right);
        keys.len()
    }

    /// Takes the most recent exact publication fault produced while opening
    /// immutable payloads. The fault remains pending across metadata and
    /// request failures until taken or replaced by a later artifact fault.
    pub fn take_artifact_fault(&mut self) -> Option<SearchArtifactFaultKey> {
        self.pending_artifact_fault.take()
    }

    pub fn with_query<T>(
        &mut self,
        operation: impl for<'query> FnOnce(QueryScope<'query>) -> Result<T, SearchRuntimeError>,
    ) -> Result<T, SearchRuntimeError> {
        let mut fulltext_lease = Some(
            SnapshotReadLease::acquire(&self.fulltext_root)
                .map_err(|_| SearchRuntimeError::unavailable())?
                .ok_or_else(SearchRuntimeError::unavailable)?,
        );
        let mut vector_lease = Some(
            self.vector_root
                .acquire_read_lease()
                .map_err(|_| SearchRuntimeError::unavailable())?,
        );
        let fulltext_root = &self.fulltext_root;
        let vector_root = &self.vector_root;
        let fulltext_cache = &mut self.fulltext_cache;
        let vector_cache = &mut self.vector_cache;
        let pending_generation = &mut self.pending_generation;
        let pending_artifact_fault = &mut self.pending_artifact_fault;
        self.store
            .with_search_metadata_snapshot(|snapshot| {
                let key = cache_key(snapshot.head().publication.clone())?;
                adopt_pending_generation(&key, pending_generation, fulltext_cache, vector_cache)?;
                let fulltext_changed = fulltext_cache
                    .as_ref()
                    .is_none_or(|cached| cached.key != key);
                let vector_changed = vector_cache.as_ref().is_none_or(|cached| cached.key != key);
                let projections = if fulltext_changed || vector_changed {
                    Some(
                        snapshot
                            .validated_active_projections()
                            .map_err(|_| SearchRuntimeError::integrity())?,
                    )
                } else {
                    None
                };
                if fulltext_changed || vector_changed {
                    *fulltext_cache = None;
                    *vector_cache = None;
                }
                if fulltext_changed {
                    let validated = match validate_fulltext_generation(
                        snapshot,
                        fulltext_root,
                        fulltext_lease
                            .take()
                            .ok_or_else(SearchRuntimeError::integrity)?,
                        key.clone(),
                        projections
                            .as_deref()
                            .ok_or_else(SearchRuntimeError::integrity)?,
                    ) {
                        Ok(validated) => validated,
                        Err(error) => {
                            return Err(record_generation_error(error, pending_artifact_fault))
                        }
                    };
                    *fulltext_cache = Some(validated);
                } else {
                    drop(fulltext_lease.take());
                }
                if vector_changed {
                    let validated = match validate_vector_generation(
                        snapshot,
                        vector_root,
                        vector_lease
                            .take()
                            .ok_or_else(SearchRuntimeError::integrity)?,
                        key,
                        projections
                            .as_deref()
                            .ok_or_else(SearchRuntimeError::integrity)?,
                    ) {
                        Ok(validated) => validated,
                        Err(error) => {
                            return Err(record_generation_error(error, pending_artifact_fault))
                        }
                    };
                    *vector_cache = Some(validated);
                } else {
                    drop(vector_lease.take());
                }
                let fulltext = fulltext_cache
                    .as_ref()
                    .ok_or_else(SearchRuntimeError::integrity)?;
                let vector = vector_cache
                    .as_ref()
                    .ok_or_else(SearchRuntimeError::integrity)?;
                operation(QueryScope::new(
                    snapshot,
                    &fulltext.fulltext,
                    &vector.vector,
                ))
            })
            .map_err(map_transaction_error)
    }

    pub fn with_lexical_query<T>(
        &mut self,
        operation: impl for<'query> FnOnce(LexicalQueryScope<'query>) -> Result<T, SearchRuntimeError>,
    ) -> Result<T, SearchRuntimeError> {
        let mut fulltext_lease = Some(
            SnapshotReadLease::acquire(&self.fulltext_root)
                .map_err(|_| SearchRuntimeError::unavailable())?
                .ok_or_else(SearchRuntimeError::unavailable)?,
        );
        let fulltext_root = &self.fulltext_root;
        let fulltext_cache = &mut self.fulltext_cache;
        let vector_cache = &mut self.vector_cache;
        let pending_generation = &mut self.pending_generation;
        let pending_artifact_fault = &mut self.pending_artifact_fault;
        self.store
            .with_search_metadata_snapshot(|snapshot| {
                let key = cache_key(snapshot.head().publication.clone())?;
                adopt_pending_generation(&key, pending_generation, fulltext_cache, vector_cache)?;
                if fulltext_cache
                    .as_ref()
                    .is_none_or(|cached| cached.key != key)
                {
                    *fulltext_cache = None;
                    *vector_cache = None;
                    let projections = snapshot
                        .validated_active_projections()
                        .map_err(|_| SearchRuntimeError::integrity())?;
                    let validated = match validate_fulltext_generation(
                        snapshot,
                        fulltext_root,
                        fulltext_lease
                            .take()
                            .ok_or_else(SearchRuntimeError::integrity)?,
                        key,
                        &projections,
                    ) {
                        Ok(validated) => validated,
                        Err(error) => {
                            return Err(record_generation_error(error, pending_artifact_fault))
                        }
                    };
                    *fulltext_cache = Some(validated);
                } else {
                    drop(fulltext_lease.take());
                }
                let fulltext = fulltext_cache
                    .as_ref()
                    .ok_or_else(SearchRuntimeError::integrity)?;
                operation(LexicalQueryScope::new(snapshot, &fulltext.fulltext))
            })
            .map_err(map_transaction_error)
    }
}

impl PreparedQueryGeneration {
    pub fn generation(&self) -> &str {
        &self.fulltext.key.generation
    }

    pub fn open(
        data_dir: &Path,
        publication: &SearchPublicationRecord,
        projections: &[ActiveSearchProjection],
    ) -> Result<Self, SearchRuntimeError> {
        if !matches!(
            publication.state,
            SearchPublicationState::Validated | SearchPublicationState::Ready
        ) {
            return Err(SearchRuntimeError::unavailable());
        }
        let data_dir = fs::canonicalize(data_dir).map_err(|_| SearchRuntimeError::unavailable())?;
        let key = cache_key(publication.clone())?;
        let fulltext_descriptor = publication
            .fulltext
            .as_ref()
            .ok_or_else(SearchRuntimeError::integrity)?;
        let vector_descriptor = publication
            .vector
            .as_ref()
            .ok_or_else(SearchRuntimeError::integrity)?;
        let fulltext_root = data_dir.join("search-index");
        let fulltext_lease = SnapshotReadLease::acquire(&fulltext_root)
            .map_err(|_| SearchRuntimeError::unavailable())?
            .ok_or_else(SearchRuntimeError::unavailable)?;
        let vector_root = VectorSnapshotRoot::new(data_dir.join("vector-index"))
            .map_err(|_| SearchRuntimeError::unavailable())?;
        let vector_lease = vector_root
            .acquire_read_lease()
            .map_err(|_| SearchRuntimeError::unavailable())?;
        let fulltext = open_fulltext_generation(
            &fulltext_root,
            fulltext_lease,
            key.clone(),
            fulltext_descriptor,
            projections,
        )
        .map_err(generation_validation_error)?;
        let vector = open_vector_generation(
            &vector_root,
            vector_lease,
            key,
            vector_descriptor,
            projections,
        )
        .map_err(generation_validation_error)?;
        Ok(Self {
            data_dir,
            fulltext,
            vector,
        })
    }
}

fn adopt_pending_generation(
    key: &CacheKey,
    pending: &mut Option<PreparedQueryGeneration>,
    fulltext: &mut Option<ValidatedFullTextGeneration>,
    vector: &mut Option<ValidatedVectorGeneration>,
) -> Result<(), SearchRuntimeError> {
    let active_matches = fulltext.as_ref().is_some_and(|cached| cached.key == *key)
        && vector.as_ref().is_some_and(|cached| cached.key == *key);
    if active_matches {
        return Ok(());
    }
    let prepared_matches = pending
        .as_ref()
        .is_some_and(|prepared| prepared.fulltext.key == *key && prepared.vector.key == *key);
    if prepared_matches {
        let prepared = pending.take().expect("matching pending generation exists");
        *fulltext = Some(prepared.fulltext);
        *vector = Some(prepared.vector);
        return Ok(());
    }
    if pending.is_some() {
        return Err(SearchRuntimeError::unavailable());
    }
    Ok(())
}

enum GenerationValidationError {
    Metadata(SearchRuntimeError),
    Artifact {
        key: CacheKey,
        error: SearchRuntimeError,
    },
}

fn cache_key(publication: SearchPublicationRecord) -> Result<CacheKey, SearchRuntimeError> {
    let publication_fingerprint = publication
        .publication_fingerprint
        .ok_or_else(SearchRuntimeError::integrity)?;
    Ok(CacheKey {
        generation: publication.generation,
        publication_fingerprint,
    })
}

fn record_generation_error(
    error: GenerationValidationError,
    pending_artifact_fault: &mut Option<SearchArtifactFaultKey>,
) -> SearchRuntimeError {
    match error {
        GenerationValidationError::Metadata(error) => error,
        GenerationValidationError::Artifact { key, error } => {
            *pending_artifact_fault = Some(SearchArtifactFaultKey {
                generation: key.generation,
                publication_fingerprint: key.publication_fingerprint,
            });
            error
        }
    }
}

fn generation_validation_error(error: GenerationValidationError) -> SearchRuntimeError {
    match error {
        GenerationValidationError::Metadata(error)
        | GenerationValidationError::Artifact { error, .. } => error,
    }
}

fn validate_fulltext_generation(
    snapshot: &SearchMetadataSnapshot<'_>,
    fulltext_root: &Path,
    fulltext_lease: SnapshotReadLease,
    key: CacheKey,
    projections: &[ActiveSearchProjection],
) -> Result<ValidatedFullTextGeneration, GenerationValidationError> {
    let publication = &snapshot.head().publication;
    let fulltext_descriptor = publication
        .fulltext
        .as_ref()
        .ok_or_else(|| GenerationValidationError::Metadata(SearchRuntimeError::integrity()))?;
    if snapshot.head().generation != key.generation
        || fulltext_descriptor.generation() != key.generation
    {
        return Err(GenerationValidationError::Metadata(
            SearchRuntimeError::integrity(),
        ));
    }
    open_fulltext_generation(
        fulltext_root,
        fulltext_lease,
        key,
        fulltext_descriptor,
        projections,
    )
}

fn open_fulltext_generation(
    fulltext_root: &Path,
    fulltext_lease: SnapshotReadLease,
    key: CacheKey,
    fulltext_descriptor: &meta_store::FullTextSnapshotDescriptor,
    projections: &[ActiveSearchProjection],
) -> Result<ValidatedFullTextGeneration, GenerationValidationError> {
    let fulltext =
        FullTextIndex::open_snapshot_with_lease(fulltext_root, &key.generation, fulltext_lease)
            .map_err(|_| GenerationValidationError::Artifact {
                key: key.clone(),
                error: SearchRuntimeError::integrity(),
            })?
            .ok_or_else(|| GenerationValidationError::Artifact {
                key: key.clone(),
                error: SearchRuntimeError::unavailable(),
            })?;
    validate_fulltext(&fulltext, fulltext_descriptor, projections).map_err(|error| {
        GenerationValidationError::Artifact {
            key: key.clone(),
            error,
        }
    })?;
    Ok(ValidatedFullTextGeneration { key, fulltext })
}

fn validate_vector_generation(
    snapshot: &SearchMetadataSnapshot<'_>,
    vector_root: &VectorSnapshotRoot,
    vector_lease: VectorSnapshotReadLease,
    key: CacheKey,
    projections: &[ActiveSearchProjection],
) -> Result<ValidatedVectorGeneration, GenerationValidationError> {
    let vector_descriptor = snapshot
        .head()
        .publication
        .vector
        .as_ref()
        .ok_or_else(|| GenerationValidationError::Metadata(SearchRuntimeError::integrity()))?;
    if snapshot.head().generation != key.generation
        || vector_descriptor.generation() != key.generation
    {
        return Err(GenerationValidationError::Metadata(
            SearchRuntimeError::integrity(),
        ));
    }
    open_vector_generation(
        vector_root,
        vector_lease,
        key,
        vector_descriptor,
        projections,
    )
}

fn open_vector_generation(
    vector_root: &VectorSnapshotRoot,
    vector_lease: VectorSnapshotReadLease,
    key: CacheKey,
    vector_descriptor: &meta_store::VectorSnapshotDescriptor,
    projections: &[ActiveSearchProjection],
) -> Result<ValidatedVectorGeneration, GenerationValidationError> {
    let vector_contract =
        vector_contract(vector_descriptor.mode()).map_err(GenerationValidationError::Metadata)?;
    let vector = vector_root
        .open_generation_with_lease(&key.generation, &vector_contract, vector_lease)
        .map_err(|_| GenerationValidationError::Artifact {
            key: key.clone(),
            error: SearchRuntimeError::integrity(),
        })?;
    validate_vector(&vector, vector_descriptor, projections, &vector_contract).map_err(
        |error| GenerationValidationError::Artifact {
            key: key.clone(),
            error,
        },
    )?;
    Ok(ValidatedVectorGeneration { key, vector })
}

fn validate_fulltext(
    fulltext: &FullTextIndex,
    descriptor: &meta_store::FullTextSnapshotDescriptor,
    projections: &[ActiveSearchProjection],
) -> Result<(), SearchRuntimeError> {
    let metadata = fulltext
        .snapshot_metadata()
        .ok_or_else(SearchRuntimeError::integrity)?;
    let count =
        u64::try_from(metadata.document_count()).map_err(|_| SearchRuntimeError::integrity())?;
    let identities = fulltext
        .exact_identity_pairs()
        .map_err(|_| SearchRuntimeError::integrity())?;
    let identities_match = identities.len() == projections.len()
        && identities
            .iter()
            .zip(projections)
            .all(|(identity, projection)| {
                identity.0 == projection.document_id.as_str()
                    && identity.1 == projection.resume_version_id.as_str()
            });
    if metadata.generation() != descriptor.generation()
        || count != descriptor.document_count()
        || metadata.projection_digest() != descriptor.projection_digest()
        || metadata.logical_content_digest() != descriptor.logical_content_digest()
        || !identities_match
    {
        return Err(SearchRuntimeError::integrity());
    }
    Ok(())
}

fn validate_vector(
    vector: &VectorSnapshotReader,
    descriptor: &meta_store::VectorSnapshotDescriptor,
    projections: &[ActiveSearchProjection],
    contract: &VectorModelContract,
) -> Result<(), SearchRuntimeError> {
    let summary = vector.summary();
    let projection_count =
        u64::try_from(summary.projection_count()).map_err(|_| SearchRuntimeError::integrity())?;
    let vector_count =
        u64::try_from(summary.vector_count()).map_err(|_| SearchRuntimeError::integrity())?;
    let document_count = u64::try_from(summary.vector_document_count())
        .map_err(|_| SearchRuntimeError::integrity())?;
    if summary.generation() != descriptor.generation()
        || summary.model_contract() != contract
        || projection_count != descriptor.projection_count()
        || vector_count != descriptor.vector_count()
        || document_count != descriptor.document_count()
        || summary.projection_digest() != descriptor.projection_digest()
        || summary.coverage_digest() != descriptor.coverage_digest()
        || summary.logical_content_digest() != descriptor.logical_content_digest()
        || vector.exact_projection() != projections
    {
        return Err(SearchRuntimeError::integrity());
    }
    Ok(())
}

fn vector_contract(mode: &VectorSnapshotMode) -> Result<VectorModelContract, SearchRuntimeError> {
    match mode {
        VectorSnapshotMode::Disabled => Ok(VectorModelContract::Disabled),
        VectorSnapshotMode::Enabled {
            model_id,
            dimension,
        } => VectorModelContract::enabled(
            model_id.clone(),
            usize::try_from(*dimension).map_err(|_| SearchRuntimeError::integrity())?,
        )
        .map_err(|_| SearchRuntimeError::integrity()),
    }
}

fn map_store_error(error: MetaStoreError) -> SearchRuntimeError {
    match error.class() {
        MetaStoreErrorClass::Storage
        | MetaStoreErrorClass::Migration
        | MetaStoreErrorClass::MigrationOwnershipRequired
        | MetaStoreErrorClass::UnsupportedStoreSchema
        | MetaStoreErrorClass::Crypto
        | MetaStoreErrorClass::WeakPassphrase
        | MetaStoreErrorClass::InvalidBackup
        | MetaStoreErrorClass::KeyAlreadyExists => SearchRuntimeError::unavailable(),
        MetaStoreErrorClass::InvalidValue
        | MetaStoreErrorClass::NotFound
        | MetaStoreErrorClass::InvalidTransition
        | MetaStoreErrorClass::ImmutableIdentityConflict
        | MetaStoreErrorClass::StorageInvariant => SearchRuntimeError::integrity(),
    }
}

fn map_transaction_error(
    error: SearchMetadataTransactionError<SearchRuntimeError>,
) -> SearchRuntimeError {
    if let Some(operation) = error.operation_error() {
        return *operation;
    }
    if let Some(store) = error.store_error() {
        return map_store_error(store.clone());
    }
    SearchRuntimeError::unavailable()
}
