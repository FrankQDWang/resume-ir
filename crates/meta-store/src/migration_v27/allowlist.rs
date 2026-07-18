use std::path::Path;

use core_domain::ContentDigest;
use rusqlite::Connection;

use super::open_encrypted_connection;
use crate::{MetaStoreError, Result};

#[derive(Clone)]
struct LegacyDocumentIdentity {
    id: String,
    source_uri: String,
    normalized_path: String,
    file_name: String,
    extension: String,
    byte_size: i64,
    mtime_seconds: i64,
    is_deleted: i64,
    created_at_seconds: i64,
    updated_at_seconds: i64,
}

#[derive(Clone)]
struct LegacyAuthorizedRoot {
    canonical_root_path: String,
    requested_root_path: String,
    root_kind: String,
    root_preset: Option<String>,
    scan_profile: String,
    scan_budget_kind: Option<String>,
    scan_budget_limit: Option<i64>,
    paused: i64,
    updated_at_seconds: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AllowlistInventory {
    document_count: u64,
    authorized_root_count: u64,
    canonical_digest: ContentDigest,
}

impl AllowlistInventory {
    fn from_rows(
        documents: &[LegacyDocumentIdentity],
        roots: &[LegacyAuthorizedRoot],
    ) -> Result<Self> {
        let document_count =
            u64::try_from(documents.len()).map_err(|_| MetaStoreError::storage_invariant())?;
        let authorized_root_count =
            u64::try_from(roots.len()).map_err(|_| MetaStoreError::storage_invariant())?;
        let mut canonical = Vec::new();
        append_inventory_part(&mut canonical, b"resume-ir.metadata-v27-allowlist.v1")?;
        canonical.extend_from_slice(&document_count.to_le_bytes());
        for document in documents {
            append_inventory_part(&mut canonical, document.id.as_bytes())?;
            append_inventory_part(&mut canonical, document.source_uri.as_bytes())?;
            append_inventory_part(&mut canonical, document.normalized_path.as_bytes())?;
            append_inventory_part(&mut canonical, document.file_name.as_bytes())?;
            append_inventory_part(&mut canonical, document.extension.as_bytes())?;
            canonical.extend_from_slice(&document.byte_size.to_le_bytes());
            canonical.extend_from_slice(&document.mtime_seconds.to_le_bytes());
            canonical.extend_from_slice(&document.is_deleted.to_le_bytes());
            canonical.extend_from_slice(&document.created_at_seconds.to_le_bytes());
            canonical.extend_from_slice(&document.updated_at_seconds.to_le_bytes());
        }
        canonical.extend_from_slice(&authorized_root_count.to_le_bytes());
        for root in roots {
            append_inventory_part(&mut canonical, root.canonical_root_path.as_bytes())?;
            append_inventory_part(&mut canonical, root.requested_root_path.as_bytes())?;
            append_inventory_part(&mut canonical, root.root_kind.as_bytes())?;
            append_inventory_optional_part(&mut canonical, root.root_preset.as_deref())?;
            append_inventory_part(&mut canonical, root.scan_profile.as_bytes())?;
            append_inventory_optional_part(&mut canonical, root.scan_budget_kind.as_deref())?;
            append_inventory_optional_i64(&mut canonical, root.scan_budget_limit);
            canonical.extend_from_slice(&root.paused.to_le_bytes());
            canonical.extend_from_slice(&root.updated_at_seconds.to_le_bytes());
        }
        Ok(Self {
            document_count,
            authorized_root_count,
            canonical_digest: ContentDigest::from_bytes(&canonical),
        })
    }
}

fn append_inventory_part(target: &mut Vec<u8>, value: &[u8]) -> Result<()> {
    let length = u64::try_from(value.len()).map_err(|_| MetaStoreError::storage_invariant())?;
    target.extend_from_slice(&length.to_le_bytes());
    target.extend_from_slice(value);
    Ok(())
}

fn append_inventory_optional_part(target: &mut Vec<u8>, value: Option<&str>) -> Result<()> {
    match value {
        Some(value) => {
            target.push(1);
            append_inventory_part(target, value.as_bytes())
        }
        None => {
            target.push(0);
            Ok(())
        }
    }
}

fn append_inventory_optional_i64(target: &mut Vec<u8>, value: Option<i64>) {
    match value {
        Some(value) => {
            target.push(1);
            target.extend_from_slice(&value.to_le_bytes());
        }
        None => target.push(0),
    }
}

/// Copies only stable source identity and import authorization under one
/// source snapshot, returning a canonical inventory witness for target-side
/// count and digest validation.
pub(super) fn copy_allowed_legacy_state(
    source: &mut Connection,
    target_path: &Path,
    key: &[u8],
) -> Result<AllowlistInventory> {
    let source_snapshot = source.transaction().map_err(MetaStoreError::storage)?;
    let documents = {
        let mut statement = source_snapshot
            .prepare(
                "SELECT id, source_uri, normalized_path, file_name, extension, byte_size,
                        mtime_seconds, is_deleted, created_at_seconds, updated_at_seconds
                 FROM document
                 ORDER BY id",
            )
            .map_err(MetaStoreError::storage)?;
        let rows = statement
            .query_map([], |row| {
                Ok(LegacyDocumentIdentity {
                    id: row.get(0)?,
                    source_uri: row.get(1)?,
                    normalized_path: row.get(2)?,
                    file_name: row.get(3)?,
                    extension: row.get(4)?,
                    byte_size: row.get(5)?,
                    mtime_seconds: row.get(6)?,
                    is_deleted: row.get(7)?,
                    created_at_seconds: row.get(8)?,
                    updated_at_seconds: row.get(9)?,
                })
            })
            .map_err(MetaStoreError::storage)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(MetaStoreError::storage)?;
        rows
    };
    let roots = {
        let mut statement = source_snapshot
            .prepare(
                "SELECT scope.canonical_root_path, scope.requested_root_path,
                        scope.root_kind, scope.root_preset, scope.scan_profile,
                        scope.scan_budget_kind, scope.scan_budget_limit,
                        COALESCE(control.paused, 0),
                        MAX(scope.updated_at_seconds, COALESCE(control.updated_at_seconds, 0))
                 FROM import_scan_scope AS scope
                 LEFT JOIN import_root_control AS control
                   ON control.canonical_root_path = scope.canonical_root_path
                 WHERE NOT EXISTS (
                     SELECT 1 FROM import_scan_scope AS newer
                     WHERE newer.canonical_root_path = scope.canonical_root_path
                       AND (
                           newer.updated_at_seconds > scope.updated_at_seconds
                           OR (
                               newer.updated_at_seconds = scope.updated_at_seconds
                               AND newer.rowid > scope.rowid
                           )
                       )
                 )
                 ORDER BY scope.canonical_root_path",
            )
            .map_err(MetaStoreError::storage)?;
        let rows = statement
            .query_map([], |row| {
                Ok(LegacyAuthorizedRoot {
                    canonical_root_path: row.get(0)?,
                    requested_root_path: row.get(1)?,
                    root_kind: row.get(2)?,
                    root_preset: row.get(3)?,
                    scan_profile: row.get(4)?,
                    scan_budget_kind: row.get(5)?,
                    scan_budget_limit: row.get(6)?,
                    paused: row.get(7)?,
                    updated_at_seconds: row.get(8)?,
                })
            })
            .map_err(MetaStoreError::storage)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(MetaStoreError::storage)?;
        rows
    };
    let expected_inventory = AllowlistInventory::from_rows(&documents, &roots)?;
    source_snapshot.commit().map_err(MetaStoreError::storage)?;

    let mut target = open_encrypted_connection(target_path, key)?;
    let transaction = target.transaction().map_err(MetaStoreError::storage)?;
    for document in documents {
        transaction
            .execute(
                "INSERT INTO document (
                    id, source_uri, normalized_path, file_name, extension, byte_size,
                    mtime_seconds, content_hash, text_hash, is_deleted,
                    created_at_seconds, updated_at_seconds, status
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8, ?9, ?10,
                    CASE WHEN ?8 = 1 THEN 'deleted' ELSE 'discovered' END)",
                rusqlite::params![
                    document.id,
                    document.source_uri,
                    document.normalized_path,
                    document.file_name,
                    document.extension,
                    document.byte_size,
                    document.mtime_seconds,
                    document.is_deleted,
                    document.created_at_seconds,
                    document.updated_at_seconds,
                ],
            )
            .map_err(MetaStoreError::storage)?;
    }
    for root in roots {
        transaction
            .execute(
                "INSERT INTO authorized_import_root (
                    canonical_root_path, requested_root_path, root_kind, root_preset,
                    scan_profile, scan_budget_kind, scan_budget_limit, paused,
                    updated_at_seconds
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    root.canonical_root_path,
                    root.requested_root_path,
                    root.root_kind,
                    root.root_preset,
                    root.scan_profile,
                    root.scan_budget_kind,
                    root.scan_budget_limit,
                    root.paused,
                    root.updated_at_seconds,
                ],
            )
            .map_err(MetaStoreError::storage)?;
    }
    transaction.commit().map_err(MetaStoreError::storage)?;
    target
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(MetaStoreError::storage)?;
    Ok(expected_inventory)
}

pub(super) fn validate_allowlist_inventory(
    connection: &Connection,
    expected: &AllowlistInventory,
) -> Result<()> {
    let documents = {
        let mut statement = connection
            .prepare(
                "SELECT id, source_uri, normalized_path, file_name, extension, byte_size,
                        mtime_seconds, is_deleted, created_at_seconds, updated_at_seconds
                 FROM document
                 ORDER BY id",
            )
            .map_err(MetaStoreError::storage)?;
        let rows = statement
            .query_map([], |row| {
                Ok(LegacyDocumentIdentity {
                    id: row.get(0)?,
                    source_uri: row.get(1)?,
                    normalized_path: row.get(2)?,
                    file_name: row.get(3)?,
                    extension: row.get(4)?,
                    byte_size: row.get(5)?,
                    mtime_seconds: row.get(6)?,
                    is_deleted: row.get(7)?,
                    created_at_seconds: row.get(8)?,
                    updated_at_seconds: row.get(9)?,
                })
            })
            .map_err(MetaStoreError::storage)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(MetaStoreError::storage)?;
        rows
    };
    let roots = {
        let mut statement = connection
            .prepare(
                "SELECT canonical_root_path, requested_root_path, root_kind, root_preset,
                        scan_profile, scan_budget_kind, scan_budget_limit, paused,
                        updated_at_seconds
                 FROM authorized_import_root
                 ORDER BY canonical_root_path",
            )
            .map_err(MetaStoreError::storage)?;
        let rows = statement
            .query_map([], |row| {
                Ok(LegacyAuthorizedRoot {
                    canonical_root_path: row.get(0)?,
                    requested_root_path: row.get(1)?,
                    root_kind: row.get(2)?,
                    root_preset: row.get(3)?,
                    scan_profile: row.get(4)?,
                    scan_budget_kind: row.get(5)?,
                    scan_budget_limit: row.get(6)?,
                    paused: row.get(7)?,
                    updated_at_seconds: row.get(8)?,
                })
            })
            .map_err(MetaStoreError::storage)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(MetaStoreError::storage)?;
        rows
    };
    let actual = AllowlistInventory::from_rows(&documents, &roots)?;
    if actual != *expected {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}
