#![doc = "Composite, generation-pinned search runtime."]

mod coordinator;
mod error;
mod scope;

pub use coordinator::{PreparedQueryGeneration, QueryCoordinator, SearchArtifactFaultKey};
pub use error::{SearchRuntimeError, SearchRuntimeErrorCode};
pub use scope::{
    FilterSelection, FullTextCandidate, HitLimit, HydratedSearchHit, LexicalQueryScope, QueryScope,
    SelectionLimit, SemanticCandidate, SemanticContract, SemanticQueryVector,
};
