//! Read-only logical authority inspection for a supported encrypted store.

use std::{fs, path::Path};

use rusqlite::OptionalExtension;

use crate::{
    active_store_manifest::{
        owner_regular_file_exists, read_manifest, read_manifest_format_version, MANIFEST_FILE,
    },
    current_store,
    migration_v27::open_encrypted_read_connection,
    migration_v29, schema_v29,
    search_publication::{search_publication_in_connection, SearchPublicationState},
    MetaStoreError, Result, VectorSnapshotMode,
};

/// Bounded, path-free witness for the search authority published by one exact
/// encrypted metadata store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataLogicalAuthority {
    pub generation: String,
    pub visible_epoch: u64,
    pub projection_digest: String,
    pub fulltext_generation: String,
    pub fulltext_document_count: u64,
    pub fulltext_projection_digest: String,
    pub fulltext_logical_content_digest: String,
    pub vector_generation: String,
    pub vector_mode: &'static str,
    pub vector_model_id: Option<String>,
    pub vector_dimension: Option<u32>,
    pub vector_projection_count: u64,
    pub vector_coverage_digest: String,
    pub vector_count: u64,
    pub vector_document_count: u64,
    pub vector_projection_digest: String,
    pub vector_logical_content_digest: String,
}

/// Inspects an exact v29 migration source or exact current authority without
/// acquiring ownership, migrating, repairing, creating files, or exposing
/// paths, keys, or raw corpus data.
pub fn inspect_metadata_logical_authority(data_dir: &Path) -> Result<MetadataLogicalAuthority> {
    let directory_metadata = fs::symlink_metadata(data_dir).map_err(MetaStoreError::io_storage)?;
    if !directory_metadata.file_type().is_dir() || directory_metadata.file_type().is_symlink() {
        return Err(MetaStoreError::storage_invariant());
    }
    let data_dir = fs::canonicalize(data_dir).map_err(MetaStoreError::io_storage)?;
    let manifest_path = data_dir.join(MANIFEST_FILE);
    if !owner_regular_file_exists(&manifest_path)? {
        return Err(MetaStoreError::unsupported_store_schema());
    }
    let manifest_format = read_manifest_format_version(&manifest_path)?;
    let manifest = read_manifest(&manifest_path)?;
    let store_path = data_dir.join(&manifest.file_name);
    if !owner_regular_file_exists(&store_path)? {
        return Err(MetaStoreError::storage_invariant());
    }
    let key = crate::read_metadata_encryption_key_without_repair(
        &crate::metadata_encryption_key_path(&data_dir),
    )?;
    let connection = open_encrypted_read_connection(&store_path, &key)?;
    match (manifest_format, manifest.schema_version) {
        (1, schema_v29::VERSION) => {
            migration_v29::validate_current_v29_connection(&connection, &manifest.store_id_digest)?
        }
        (2, crate::CURRENT_SCHEMA_VERSION) => {
            current_store::validate_current_connection(&connection, &manifest.store_id_digest)?
        }
        _ => return Err(MetaStoreError::unsupported_store_schema()),
    }

    let head = connection
        .query_row(
            "SELECT service_state, generation, visible_epoch
             FROM search_projection_state WHERE state_key = 'default'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .ok_or_else(MetaStoreError::storage_invariant)?;
    let (service_state, generation, visible_epoch) = head;
    let generation = generation.ok_or_else(MetaStoreError::storage_invariant)?;
    if service_state != "ready" {
        return Err(MetaStoreError::storage_invariant());
    }
    let visible_epoch =
        u64::try_from(visible_epoch).map_err(|_| MetaStoreError::storage_invariant())?;
    if visible_epoch == 0 {
        return Err(MetaStoreError::storage_invariant());
    }
    let publication = search_publication_in_connection(&connection, &generation)?
        .ok_or_else(MetaStoreError::storage_invariant)?;
    if publication.state != SearchPublicationState::Ready {
        return Err(MetaStoreError::storage_invariant());
    }
    let fulltext = publication
        .fulltext
        .as_ref()
        .ok_or_else(MetaStoreError::storage_invariant)?;
    let vector = publication
        .vector
        .as_ref()
        .ok_or_else(MetaStoreError::storage_invariant)?;
    let (vector_mode, vector_model_id, vector_dimension) = match vector.mode() {
        VectorSnapshotMode::Disabled => ("disabled", None, None),
        VectorSnapshotMode::Enabled {
            model_id,
            dimension,
        } => ("enabled", Some(model_id.clone()), Some(*dimension)),
    };

    Ok(MetadataLogicalAuthority {
        generation,
        visible_epoch,
        projection_digest: publication.projection_digest.as_str().to_string(),
        fulltext_generation: fulltext.generation().to_string(),
        fulltext_document_count: fulltext.document_count(),
        fulltext_projection_digest: fulltext.projection_digest().as_str().to_string(),
        fulltext_logical_content_digest: fulltext.logical_content_digest().as_str().to_string(),
        vector_generation: vector.generation().to_string(),
        vector_mode,
        vector_model_id,
        vector_dimension,
        vector_projection_count: vector.projection_count(),
        vector_coverage_digest: vector.coverage_digest().as_str().to_string(),
        vector_count: vector.vector_count(),
        vector_document_count: vector.document_count(),
        vector_projection_digest: vector.projection_digest().as_str().to_string(),
        vector_logical_content_digest: vector.logical_content_digest().as_str().to_string(),
    })
}
