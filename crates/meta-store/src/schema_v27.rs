pub(super) const VERSION: u32 = 27;

mod bootstrap;
mod immutable_storage;
mod privacy_maintenance;
mod search_publication;

/// v27 is a hard correctness cut. Legacy parse-derived identities cannot be
/// upgraded because v26 allowed one ID to be overwritten with different text.
/// Source documents and import authorization remain; all derived authority is
/// rebuilt from source revisions before search can become ready again.
///
/// These pure-Rust fragments execute in order inside one SQLite transaction;
/// no build-time or runtime resource lookup is part of the schema contract.
pub(super) const SCHEMA_PARTS: &[&str] = &[
    bootstrap::SCHEMA,
    immutable_storage::SCHEMA,
    privacy_maintenance::SCHEMA,
    search_publication::SCHEMA,
];
