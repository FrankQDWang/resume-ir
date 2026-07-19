use std::{path::Path, str::FromStr};

use core_domain::ContentDigest;
use rusqlite::{Connection, Transaction};
use sha2::{Digest, Sha256};

use crate::{migration_v27::open_encrypted_connection, schema_v27, MetaStoreError, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AllowlistInventory {
    pub(super) document_count: u64,
    pub(super) authorized_root_count: u64,
    pub(super) inherited_visible_epoch: u64,
    pub(super) canonical_digest: ContentDigest,
}

pub(super) fn copy_allowed_source_state(
    source: &Connection,
    source_version: u32,
    target_path: &Path,
    key: &[u8],
) -> Result<AllowlistInventory> {
    let document_count = if source_version == 0 {
        0
    } else {
        count_rows(source, "document")?
    };
    let authorized_root_count = authorized_root_count(source, source_version)?;
    let inherited_visible_epoch = inherited_visible_epoch(source, source_version)?;

    let mut target = open_encrypted_connection(target_path, key)?;
    let target_transaction = target.transaction().map_err(MetaStoreError::storage)?;
    let mut digest = inventory_hasher(
        document_count,
        authorized_root_count,
        inherited_visible_epoch,
    );
    if source_version != 0 {
        copy_documents(source, &target_transaction, &mut digest)?;
    }
    copy_authorized_roots(source, source_version, &target_transaction, &mut digest)?;
    target_transaction
        .execute(
            "INSERT INTO metadata_cow_staging_authority (
                state_key, target_visible_epoch
             ) VALUES ('default', ?1)",
            [u64_to_i64(inherited_visible_epoch)?],
        )
        .map_err(MetaStoreError::storage)?;
    target_transaction
        .execute(
            "UPDATE search_projection_state
             SET visible_epoch = ?1, service_state = 'repairing', generation = NULL,
                 repair_reason = 'migration_rebuild', updated_at_seconds = 0
             WHERE state_key = 'default'",
            [u64_to_i64(inherited_visible_epoch)?],
        )
        .map_err(MetaStoreError::storage)?;
    target_transaction
        .execute(
            "DELETE FROM metadata_cow_staging_authority WHERE state_key = 'default'",
            [],
        )
        .map_err(MetaStoreError::storage)?;
    target_transaction
        .commit()
        .map_err(MetaStoreError::storage)?;

    Ok(AllowlistInventory {
        document_count,
        authorized_root_count,
        inherited_visible_epoch,
        canonical_digest: finish_digest(digest)?,
    })
}

pub(super) fn validate_allowlist_inventory(
    connection: &Connection,
    expected: &AllowlistInventory,
) -> Result<()> {
    let document_count = count_rows(connection, "document")?;
    let authorized_root_count = count_rows(connection, "authorized_import_root")?;
    let inherited_visible_epoch = connection
        .query_row(
            "SELECT visible_epoch FROM search_projection_state WHERE state_key = 'default'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)
        .and_then(i64_to_u64)?;
    let mut digest = inventory_hasher(
        document_count,
        authorized_root_count,
        inherited_visible_epoch,
    );
    hash_target_documents(connection, &mut digest)?;
    hash_target_roots(connection, &mut digest)?;
    let actual = AllowlistInventory {
        document_count,
        authorized_root_count,
        inherited_visible_epoch,
        canonical_digest: finish_digest(digest)?,
    };
    if actual != *expected {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn copy_documents(
    source: &Connection,
    target: &Transaction<'_>,
    digest: &mut Sha256,
) -> Result<()> {
    let mut select = source
        .prepare(
            "SELECT id, source_uri, normalized_path, file_name, extension, byte_size,
                    mtime_seconds, is_deleted, created_at_seconds, updated_at_seconds
             FROM document ORDER BY id",
        )
        .map_err(MetaStoreError::storage)?;
    let mut rows = select.query([]).map_err(MetaStoreError::storage)?;
    let mut insert = target
        .prepare_cached(
            "INSERT INTO document (
                id, source_uri, normalized_path, file_name, extension, byte_size,
                mtime_seconds, content_hash, text_hash, is_deleted,
                created_at_seconds, updated_at_seconds, status
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8, ?9, ?10,
                CASE WHEN ?8 = 1 THEN 'deleted' ELSE 'discovered' END)",
        )
        .map_err(MetaStoreError::storage)?;
    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        let values = (
            row.get::<_, String>(0).map_err(MetaStoreError::storage)?,
            row.get::<_, String>(1).map_err(MetaStoreError::storage)?,
            row.get::<_, String>(2).map_err(MetaStoreError::storage)?,
            row.get::<_, String>(3).map_err(MetaStoreError::storage)?,
            row.get::<_, String>(4).map_err(MetaStoreError::storage)?,
            row.get::<_, i64>(5).map_err(MetaStoreError::storage)?,
            row.get::<_, i64>(6).map_err(MetaStoreError::storage)?,
            row.get::<_, i64>(7).map_err(MetaStoreError::storage)?,
            row.get::<_, i64>(8).map_err(MetaStoreError::storage)?,
            row.get::<_, i64>(9).map_err(MetaStoreError::storage)?,
        );
        hash_document(digest, &values)?;
        insert
            .execute(rusqlite::params![
                values.0, values.1, values.2, values.3, values.4, values.5, values.6, values.7,
                values.8, values.9,
            ])
            .map_err(MetaStoreError::storage)?;
    }
    Ok(())
}

fn copy_authorized_roots(
    source: &Connection,
    source_version: u32,
    target: &Transaction<'_>,
    digest: &mut Sha256,
) -> Result<()> {
    let Some(sql) = source_root_query(source_version) else {
        return Ok(());
    };
    let mut select = source.prepare(sql).map_err(MetaStoreError::storage)?;
    let mut rows = select.query([]).map_err(MetaStoreError::storage)?;
    let mut insert = target
        .prepare_cached(
            "INSERT INTO authorized_import_root (
                canonical_root_path, requested_root_path, root_kind, root_preset,
                scan_profile, scan_budget_kind, scan_budget_limit, paused,
                updated_at_seconds
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .map_err(MetaStoreError::storage)?;
    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        let values = (
            row.get::<_, String>(0).map_err(MetaStoreError::storage)?,
            row.get::<_, String>(1).map_err(MetaStoreError::storage)?,
            row.get::<_, String>(2).map_err(MetaStoreError::storage)?,
            row.get::<_, Option<String>>(3)
                .map_err(MetaStoreError::storage)?,
            row.get::<_, String>(4).map_err(MetaStoreError::storage)?,
            row.get::<_, Option<String>>(5)
                .map_err(MetaStoreError::storage)?,
            row.get::<_, Option<i64>>(6)
                .map_err(MetaStoreError::storage)?,
            row.get::<_, i64>(7).map_err(MetaStoreError::storage)?,
            row.get::<_, i64>(8).map_err(MetaStoreError::storage)?,
        );
        hash_root(digest, &values)?;
        insert
            .execute(rusqlite::params![
                values.0, values.1, values.2, values.3, values.4, values.5, values.6, values.7,
                values.8,
            ])
            .map_err(MetaStoreError::storage)?;
    }
    Ok(())
}

fn hash_target_documents(connection: &Connection, digest: &mut Sha256) -> Result<()> {
    let mut statement = connection
        .prepare(
            "SELECT id, source_uri, normalized_path, file_name, extension, byte_size,
                    mtime_seconds, is_deleted, created_at_seconds, updated_at_seconds
             FROM document ORDER BY id",
        )
        .map_err(MetaStoreError::storage)?;
    let mut rows = statement.query([]).map_err(MetaStoreError::storage)?;
    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        hash_document(
            digest,
            &(
                row.get(0).map_err(MetaStoreError::storage)?,
                row.get(1).map_err(MetaStoreError::storage)?,
                row.get(2).map_err(MetaStoreError::storage)?,
                row.get(3).map_err(MetaStoreError::storage)?,
                row.get(4).map_err(MetaStoreError::storage)?,
                row.get(5).map_err(MetaStoreError::storage)?,
                row.get(6).map_err(MetaStoreError::storage)?,
                row.get(7).map_err(MetaStoreError::storage)?,
                row.get(8).map_err(MetaStoreError::storage)?,
                row.get(9).map_err(MetaStoreError::storage)?,
            ),
        )?;
    }
    Ok(())
}

fn hash_target_roots(connection: &Connection, digest: &mut Sha256) -> Result<()> {
    let mut statement = connection
        .prepare(
            "SELECT canonical_root_path, requested_root_path, root_kind, root_preset,
                    scan_profile, scan_budget_kind, scan_budget_limit, paused,
                    updated_at_seconds
             FROM authorized_import_root ORDER BY canonical_root_path",
        )
        .map_err(MetaStoreError::storage)?;
    let mut rows = statement.query([]).map_err(MetaStoreError::storage)?;
    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        hash_root(
            digest,
            &(
                row.get(0).map_err(MetaStoreError::storage)?,
                row.get(1).map_err(MetaStoreError::storage)?,
                row.get(2).map_err(MetaStoreError::storage)?,
                row.get(3).map_err(MetaStoreError::storage)?,
                row.get(4).map_err(MetaStoreError::storage)?,
                row.get(5).map_err(MetaStoreError::storage)?,
                row.get(6).map_err(MetaStoreError::storage)?,
                row.get(7).map_err(MetaStoreError::storage)?,
                row.get(8).map_err(MetaStoreError::storage)?,
            ),
        )?;
    }
    Ok(())
}

type DocumentRow = (
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    i64,
    i64,
    i64,
);
type RootRow = (
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<i64>,
    i64,
    i64,
);

fn hash_document(digest: &mut Sha256, row: &DocumentRow) -> Result<()> {
    for value in [&row.0, &row.1, &row.2, &row.3, &row.4] {
        update_part(digest, value.as_bytes())?;
    }
    for value in [row.5, row.6, row.7, row.8, row.9] {
        digest.update(value.to_le_bytes());
    }
    Ok(())
}

fn hash_root(digest: &mut Sha256, row: &RootRow) -> Result<()> {
    update_part(digest, row.0.as_bytes())?;
    update_part(digest, row.1.as_bytes())?;
    update_part(digest, row.2.as_bytes())?;
    update_optional_part(digest, row.3.as_deref())?;
    update_part(digest, row.4.as_bytes())?;
    update_optional_part(digest, row.5.as_deref())?;
    update_optional_i64(digest, row.6);
    digest.update(row.7.to_le_bytes());
    digest.update(row.8.to_le_bytes());
    Ok(())
}

fn inventory_hasher(document_count: u64, root_count: u64, visible_epoch: u64) -> Sha256 {
    let mut digest = Sha256::new();
    update_part_infallible(&mut digest, b"resume-ir.metadata-v28-allowlist.v1");
    digest.update(visible_epoch.to_le_bytes());
    digest.update(document_count.to_le_bytes());
    digest.update(root_count.to_le_bytes());
    digest
}

fn update_part(digest: &mut Sha256, value: &[u8]) -> Result<()> {
    let length = u64::try_from(value.len()).map_err(|_| MetaStoreError::storage_invariant())?;
    digest.update(length.to_le_bytes());
    digest.update(value);
    Ok(())
}

fn update_part_infallible(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_le_bytes());
    digest.update(value);
}

fn update_optional_part(digest: &mut Sha256, value: Option<&str>) -> Result<()> {
    if let Some(value) = value {
        digest.update([1]);
        update_part(digest, value.as_bytes())?;
    } else {
        digest.update([0]);
    }
    Ok(())
}

fn update_optional_i64(digest: &mut Sha256, value: Option<i64>) {
    if let Some(value) = value {
        digest.update([1]);
        digest.update(value.to_le_bytes());
    } else {
        digest.update([0]);
    }
}

fn finish_digest(digest: Sha256) -> Result<ContentDigest> {
    ContentDigest::from_str(&format!("sha256:{:x}", digest.finalize()))
        .map_err(|_| MetaStoreError::storage_invariant())
}

fn authorized_root_count(connection: &Connection, source_version: u32) -> Result<u64> {
    let Some(query) = source_root_query(source_version) else {
        return Ok(0);
    };
    connection
        .query_row(&format!("SELECT COUNT(*) FROM ({query})"), [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(MetaStoreError::storage)
        .and_then(i64_to_u64)
}

fn source_root_query(source_version: u32) -> Option<&'static str> {
    if source_version >= schema_v27::VERSION {
        Some(
            "SELECT canonical_root_path, requested_root_path, root_kind, root_preset,
                    scan_profile, scan_budget_kind, scan_budget_limit, paused,
                    updated_at_seconds
             FROM authorized_import_root ORDER BY canonical_root_path",
        )
    } else if source_version >= 26 {
        Some(
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
                   AND (newer.updated_at_seconds > scope.updated_at_seconds
                     OR (newer.updated_at_seconds = scope.updated_at_seconds
                       AND newer.rowid > scope.rowid))
             ) ORDER BY scope.canonical_root_path",
        )
    } else if source_version >= 9 {
        Some(
            "SELECT scope.canonical_root_path, scope.requested_root_path,
                    scope.root_kind, scope.root_preset, scope.scan_profile,
                    scope.scan_budget_kind, scope.scan_budget_limit, 0,
                    scope.updated_at_seconds
             FROM import_scan_scope AS scope
             WHERE NOT EXISTS (
                 SELECT 1 FROM import_scan_scope AS newer
                 WHERE newer.canonical_root_path = scope.canonical_root_path
                   AND (newer.updated_at_seconds > scope.updated_at_seconds
                     OR (newer.updated_at_seconds = scope.updated_at_seconds
                       AND newer.rowid > scope.rowid))
             ) ORDER BY scope.canonical_root_path",
        )
    } else {
        None
    }
}

fn inherited_visible_epoch(connection: &Connection, source_version: u32) -> Result<u64> {
    let value = if source_version >= schema_v27::VERSION {
        connection
            .query_row(
                "SELECT visible_epoch FROM search_projection_state WHERE state_key = 'default'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?
    } else if source_version >= 21 {
        connection
            .query_row(
                "SELECT COALESCE((SELECT visible_epoch FROM index_state
                                  WHERE state_key = 'default'), 0)",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?
    } else {
        0
    };
    i64_to_u64(value)
}

fn count_rows(connection: &Connection, table: &str) -> Result<u64> {
    if !matches!(table, "document" | "authorized_import_root") {
        return Err(MetaStoreError::storage_invariant());
    }
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(MetaStoreError::storage)
        .and_then(i64_to_u64)
}

fn i64_to_u64(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| MetaStoreError::storage_invariant())
}

fn u64_to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| MetaStoreError::storage_invariant())
}
