//! Search artifact recovery facade and shared recovery summary.

mod artifact_maintenance;
mod migration_publication;
mod reconciliation;

const RECOVERY_PUBLICATION_LIMIT: usize = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchArtifactRecoverySummary {
    pub interrupted_publications_abandoned: usize,
    pub fulltext_staging_directories_removed: usize,
    pub vector_staging_directories_removed: usize,
    pub fulltext_generations_removed: usize,
    pub vector_generations_removed: usize,
    pub active_generation_rebuilt: bool,
    pub gc_deferred: bool,
    pub gc_partial: bool,
}

pub use migration_publication::finalize_migration_rebuild;
pub use reconciliation::reconcile_search_artifacts;

#[cfg(test)]
pub(super) use migration_publication::{
    finalize_migration_rebuild_with_fault, MigrationPublicationFault,
};
