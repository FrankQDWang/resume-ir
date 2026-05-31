//! Query planning and snippet shaping for local full-text search.

use std::fmt;

const DEFAULT_TOP_K: usize = 10;
const DEFAULT_SNIPPET_MAX_CHARS: usize = 80;

/// Caller-controlled full-text search options.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SearchOptions {
    /// Maximum number of ranked results to return.
    pub top_k: usize,
    /// Maximum snippet length in Unicode scalar values.
    pub snippet_max_chars: usize,
    /// Whether deleted-marker documents should be visible.
    pub include_deleted: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            top_k: DEFAULT_TOP_K,
            snippet_max_chars: DEFAULT_SNIPPET_MAX_CHARS,
            include_deleted: false,
        }
    }
}

/// Candidate selected by the retrieval layer before snippet generation.
#[derive(Clone, PartialEq)]
pub struct PlannerCandidate {
    /// One-based rank from the retrieval layer.
    pub rank: usize,
    /// Retrieval score.
    pub score: f32,
    /// Stable document identifier.
    pub doc_id: String,
    /// File name only, never a local path.
    pub file_name: String,
    /// Clean local text used only to generate the final snippet.
    pub clean_text: String,
}

impl fmt::Debug for PlannerCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlannerCandidate")
            .field("rank", &self.rank)
            .field("score", &self.score)
            .field("doc_id", &self.doc_id)
            .field("file_name", &"<redacted>")
            .field("clean_text", &"<redacted>")
            .finish()
    }
}

/// Search hit after snippet planning.
#[derive(Clone, PartialEq)]
pub struct PlannedSearchHit {
    /// One-based rank.
    pub rank: usize,
    /// Stable document identifier.
    pub doc_id: String,
    /// File name only, never a local path.
    pub file_name: String,
    /// Short display snippet.
    pub snippet: String,
}

impl fmt::Debug for PlannedSearchHit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlannedSearchHit")
            .field("rank", &self.rank)
            .field("doc_id", &self.doc_id)
            .field("file_name", &"<redacted>")
            .field("snippet", &"<redacted>")
            .finish()
    }
}

/// Builds display hits and invokes snippet generation only for returned top results.
pub fn plan_snippets_for_top_results<F>(
    candidates: Vec<PlannerCandidate>,
    query: &str,
    options: SearchOptions,
    mut snippet_for: F,
) -> Vec<PlannedSearchHit>
where
    F: FnMut(&str, &str, usize) -> String,
{
    candidates
        .into_iter()
        .take(options.top_k)
        .map(|candidate| PlannedSearchHit {
            rank: candidate.rank,
            doc_id: candidate.doc_id,
            file_name: candidate.file_name,
            snippet: snippet_for(query, &candidate.clean_text, options.snippet_max_chars),
        })
        .collect()
}

/// Generates a short plain-text snippet around the first query term present in `text`.
#[must_use]
pub fn default_snippet(query: &str, text: &str, max_chars: usize) -> String {
    if max_chars == 0 || text.is_empty() {
        return String::new();
    }

    let terms = query
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>();
    let lower_text = text.to_lowercase();
    let first_match = terms
        .iter()
        .filter_map(|term| lower_text.find(&term.to_lowercase()))
        .min()
        .map_or(0, std::convert::identity);
    let match_char = byte_to_char_index(text, first_match);
    let half_window = max_chars / 2;
    let start = match_char.saturating_sub(half_window);
    let end = start.saturating_add(max_chars);

    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

/// Returns the crate name for smoke tests and workspace metadata.
#[must_use]
pub fn crate_name() -> &'static str {
    "search-planner"
}

fn byte_to_char_index(text: &str, byte_offset: usize) -> usize {
    text[..byte_offset.min(text.len())].chars().count()
}
