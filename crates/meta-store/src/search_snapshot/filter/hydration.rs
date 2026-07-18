use std::num::NonZeroUsize;

use super::{
    ExactHitHydration, ExactHitHydrationFailure, ExactHitHydrationFailureKind, SearchHitMetadata,
    SearchHitMetadataLimit, MAX_EXACT_HIT_HYDRATION,
};
use crate::{
    search_snapshot::{
        selection::{
            bounded_document_by_id, bounded_entity_mentions_for_version,
            candidate_id_for_current_version, resolve_selection, BoundedDocument, BoundedMentions,
        },
        SearchMetadataSnapshot,
    },
    ActiveSearchProjection, Document, EntityMention, MetaStoreError, Result, SearchSelection,
    SearchSelectionResolution,
};

const MAX_HYDRATED_MENTIONS: usize = 2_048;
const MAX_HYDRATED_STRING_BYTES: usize = 2 * 1024 * 1024;

impl SearchMetadataSnapshot<'_> {
    /// Hydrates only exact `(document, version)` identities from this snapshot.
    /// Input order is preserved; there is no document-to-latest fallback.
    pub fn hydrate_exact_hits(
        &self,
        identities: &[ActiveSearchProjection],
        cap: NonZeroUsize,
    ) -> Result<ExactHitHydration> {
        let cap = cap.get();
        if cap > MAX_EXACT_HIT_HYDRATION {
            return Err(MetaStoreError::invalid_value("search_hit_hydration.cap"));
        }
        if identities.len() > cap {
            return Ok(hydration_limit(None, SearchHitMetadataLimit::InputCount));
        }

        let mut hydrated = Vec::with_capacity(identities.len());
        let mut total_mentions = 0usize;
        let mut total_string_bytes = 0usize;
        for (position, identity) in identities.iter().enumerate() {
            let selection = SearchSelection {
                document_id: identity.document_id.clone(),
                resume_version_id: identity.resume_version_id.clone(),
                visible_epoch: self.head.visible_epoch,
            };
            match resolve_selection(self.connection, &self.head.generation, &selection)? {
                SearchSelectionResolution::Stale => {
                    return Ok(hydration_failure(
                        position,
                        ExactHitHydrationFailureKind::Stale,
                    ));
                }
                SearchSelectionResolution::NotFound => {
                    return Ok(hydration_failure(
                        position,
                        ExactHitHydrationFailureKind::NotFound,
                    ));
                }
                SearchSelectionResolution::Current { .. } => {}
            }

            let document = match bounded_document_by_id(self.connection, &identity.document_id)? {
                BoundedDocument::Document(document) => *document,
                BoundedDocument::LimitExceeded => {
                    return Ok(hydration_limit(
                        Some(position),
                        SearchHitMetadataLimit::DocumentMetadata,
                    ));
                }
            };
            let mentions = match bounded_entity_mentions_for_version(
                self.connection,
                &identity.resume_version_id,
            )? {
                BoundedMentions::Mentions(mentions) => mentions,
                BoundedMentions::LimitExceeded => {
                    return Ok(hydration_limit(
                        Some(position),
                        SearchHitMetadataLimit::MentionsPerHit,
                    ));
                }
            };
            total_mentions = total_mentions
                .checked_add(mentions.len())
                .ok_or_else(MetaStoreError::storage_invariant)?;
            if total_mentions > MAX_HYDRATED_MENTIONS {
                return Ok(hydration_limit(
                    Some(position),
                    SearchHitMetadataLimit::TotalMentions,
                ));
            }
            total_string_bytes = total_string_bytes
                .checked_add(document_string_bytes(&document))
                .and_then(|total| total.checked_add(mention_string_bytes(&mentions)))
                .ok_or_else(MetaStoreError::storage_invariant)?;
            if total_string_bytes > MAX_HYDRATED_STRING_BYTES {
                return Ok(hydration_limit(
                    Some(position),
                    SearchHitMetadataLimit::TotalStringBytes,
                ));
            }
            let candidate_id =
                candidate_id_for_current_version(self.connection, &identity.resume_version_id)?;
            hydrated.push(SearchHitMetadata {
                projection: identity.clone(),
                document,
                candidate_id,
                mentions,
            });
        }
        Ok(ExactHitHydration::Hydrated(hydrated))
    }
}

fn hydration_failure(position: usize, kind: ExactHitHydrationFailureKind) -> ExactHitHydration {
    ExactHitHydration::Failed(ExactHitHydrationFailure {
        position: Some(position),
        kind,
    })
}

fn hydration_limit(position: Option<usize>, limit: SearchHitMetadataLimit) -> ExactHitHydration {
    ExactHitHydration::Failed(ExactHitHydrationFailure {
        position,
        kind: ExactHitHydrationFailureKind::LimitExceeded(limit),
    })
}

fn document_string_bytes(document: &Document) -> usize {
    document.source_uri.len()
        + document.normalized_path.len()
        + document.file_name.len()
        + match &document.extension {
            crate::FileExtension::Other(value) => value.len(),
            _ => 0,
        }
        + document.content_hash.as_ref().map_or(0, String::len)
        + document.text_hash.as_ref().map_or(0, String::len)
}

fn mention_string_bytes(mentions: &[EntityMention]) -> usize {
    mentions
        .iter()
        .map(|mention| {
            mention.raw_value.len()
                + mention.normalized_value.as_ref().map_or(0, String::len)
                + mention.extractor.len()
        })
        .sum()
}
