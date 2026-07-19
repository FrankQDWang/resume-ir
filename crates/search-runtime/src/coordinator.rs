use std::path::{Path, PathBuf};

use core_domain::{ActiveSearchProjection, ContentDigest};
use index_fulltext::{FullTextIndex, SnapshotReadLease};
use index_vector::{
    VectorModelContract, VectorSnapshotReadLease, VectorSnapshotReader, VectorSnapshotRoot,
};
use meta_store::{
    MetaStoreError, MetaStoreErrorClass, ReadMetaStore, SearchMetadataSnapshot,
    SearchMetadataTransactionError, SearchPublicationRecord, VectorSnapshotMode,
};

use crate::error::SearchRuntimeError;
use crate::scope::QueryScope;

pub struct QueryCoordinator {
    store: ReadMetaStore,
    fulltext_root: PathBuf,
    vector_root: VectorSnapshotRoot,
    cache: Option<ValidatedGeneration>,
}

struct ValidatedGeneration {
    key: CacheKey,
    fulltext: FullTextIndex,
    vector: VectorSnapshotReader,
}

#[derive(PartialEq, Eq)]
struct CacheKey {
    generation: String,
    publication_fingerprint: ContentDigest,
}

impl QueryCoordinator {
    pub fn open(data_dir: &Path) -> Result<Self, SearchRuntimeError> {
        let store = ReadMetaStore::open_data_dir(data_dir).map_err(map_store_error)?;
        let fulltext_root = data_dir.join("search-index");
        let vector_root = VectorSnapshotRoot::new(data_dir.join("vector-index"))
            .map_err(|_| SearchRuntimeError::unavailable())?;
        Ok(Self {
            store,
            fulltext_root,
            vector_root,
            cache: None,
        })
    }

    pub fn with_query<T>(
        &mut self,
        operation: impl for<'query> FnOnce(QueryScope<'query>) -> Result<T, SearchRuntimeError>,
    ) -> Result<T, SearchRuntimeError> {
        let fulltext_lease = SnapshotReadLease::acquire(&self.fulltext_root)
            .map_err(|_| SearchRuntimeError::unavailable())?
            .ok_or_else(SearchRuntimeError::unavailable)?;
        let mut vector_lease = Some(
            self.vector_root
                .acquire_read_lease()
                .map_err(|_| SearchRuntimeError::unavailable())?,
        );
        let fulltext_root = &self.fulltext_root;
        let vector_root = &self.vector_root;
        let cache = &mut self.cache;
        self.store
            .with_search_metadata_snapshot(|snapshot| {
                let key = cache_key(snapshot.head().publication.clone())?;
                if cache.as_ref().is_none_or(|cached| cached.key != key) {
                    *cache = None;
                    let validated = validate_generation(
                        snapshot,
                        fulltext_root,
                        vector_root,
                        fulltext_lease,
                        vector_lease
                            .take()
                            .ok_or_else(SearchRuntimeError::integrity)?,
                        key,
                    )?;
                    *cache = Some(validated);
                } else {
                    drop(fulltext_lease);
                    drop(vector_lease.take());
                }
                let validated = cache.as_ref().ok_or_else(SearchRuntimeError::integrity)?;
                operation(QueryScope::new(
                    snapshot,
                    &validated.fulltext,
                    &validated.vector,
                ))
            })
            .map_err(map_transaction_error)
    }
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

fn validate_generation(
    snapshot: &SearchMetadataSnapshot<'_>,
    fulltext_root: &Path,
    vector_root: &VectorSnapshotRoot,
    fulltext_lease: SnapshotReadLease,
    vector_lease: VectorSnapshotReadLease,
    key: CacheKey,
) -> Result<ValidatedGeneration, SearchRuntimeError> {
    let publication = &snapshot.head().publication;
    let fulltext_descriptor = publication
        .fulltext
        .as_ref()
        .ok_or_else(SearchRuntimeError::integrity)?;
    let vector_descriptor = publication
        .vector
        .as_ref()
        .ok_or_else(SearchRuntimeError::integrity)?;
    if snapshot.head().generation != key.generation
        || fulltext_descriptor.generation() != key.generation
        || vector_descriptor.generation() != key.generation
    {
        return Err(SearchRuntimeError::integrity());
    }
    let projections = snapshot
        .validated_active_projections()
        .map_err(|_| SearchRuntimeError::integrity())?;
    let fulltext =
        FullTextIndex::open_snapshot_with_lease(fulltext_root, &key.generation, fulltext_lease)
            .map_err(|_| SearchRuntimeError::integrity())?
            .ok_or_else(SearchRuntimeError::unavailable)?;
    validate_fulltext(&fulltext, fulltext_descriptor, &projections)?;

    let vector_contract = vector_contract(vector_descriptor.mode())?;
    let vector = vector_root
        .open_generation_with_lease(&key.generation, &vector_contract, vector_lease)
        .map_err(|_| SearchRuntimeError::integrity())?;
    validate_vector(&vector, vector_descriptor, &projections, &vector_contract)?;
    Ok(ValidatedGeneration {
        key,
        fulltext,
        vector,
    })
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
