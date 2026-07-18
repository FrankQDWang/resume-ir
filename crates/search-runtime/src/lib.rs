#![doc = "Composite, generation-pinned search runtime."]

mod coordinator;
mod error;
mod scope;

pub use coordinator::QueryCoordinator;
pub use error::{SearchRuntimeError, SearchRuntimeErrorCode};
pub use scope::{
    FilterSelection, FullTextCandidate, HitLimit, HydratedSearchHit, QueryScope, SelectionLimit,
    SemanticCandidate, SemanticContract, SemanticQueryVector,
};
