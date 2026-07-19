use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::{
    active_store_manifest::owner_regular_file_exists, apply_sqlcipher_key, schema_v27,
    verify_sqlcipher_key, MetaStoreError, Result,
};

pub(crate) fn validate_active_store(path: &Path, key: &[u8], store_id_digest: &str) -> Result<()> {
    if !owner_regular_file_exists(path)? {
        return Err(MetaStoreError::storage_invariant());
    }
    let connection = open_encrypted_connection(path, key)?;
    validate_database_integrity(&connection)?;
    if source_schema_version(&connection)? != schema_v27::VERSION
        || store_identity(&connection)? != store_id_digest
    {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn validate_database_integrity(connection: &Connection) -> Result<()> {
    let integrity = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .map_err(MetaStoreError::storage)?;
    let foreign_key_failures = connection
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(MetaStoreError::storage)?;
    if integrity != "ok" || foreign_key_failures != 0 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

pub(crate) fn open_encrypted_connection(path: &Path, key: &[u8]) -> Result<Connection> {
    let connection = Connection::open(path).map_err(MetaStoreError::storage)?;
    apply_sqlcipher_key(&connection, key)?;
    verify_sqlcipher_key(&connection)?;
    Ok(connection)
}

pub(crate) fn open_encrypted_read_connection(path: &Path, key: &[u8]) -> Result<Connection> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(MetaStoreError::storage)?;
    apply_sqlcipher_key(&connection, key)?;
    verify_sqlcipher_key(&connection)?;
    connection
        .execute_batch("PRAGMA query_only = ON; PRAGMA foreign_keys = ON;")
        .map_err(MetaStoreError::storage)?;
    Ok(connection)
}

pub(crate) fn source_schema_version(connection: &Connection) -> Result<u32> {
    let table_exists = connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'
             )",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?
        == 1;
    if !table_exists {
        return Ok(0);
    }
    let version = connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    u32::try_from(version).map_err(|_| MetaStoreError::invalid_value("schema_migrations.version"))
}

pub(crate) fn store_identity(connection: &Connection) -> Result<String> {
    connection
        .query_row(
            "SELECT store_id_digest FROM metadata_store_identity WHERE state_key = 'default'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .ok_or_else(|| MetaStoreError::invalid_value("metadata_store_identity"))
}
