use std::collections::BTreeSet;
use std::path::Path;

use index_fulltext::{incremental_snapshot_documents, IndexDocument, IndexSection};
use privacy::redact_contact_values;
use sectionizer::Sectionizer;

use super::{sections_to_index, ImportPipelineError, Result};

#[derive(Default)]
pub(super) struct CurrentImportDocumentCache {
    pub(super) initialized: bool,
    pub(super) base_generation: Option<String>,
    pub(super) documents: Vec<CachedSearchDocument>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CurrentImportCacheMode {
    Retain,
    Consume,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct CachedSearchDocument {
    pub(super) doc_id: String,
    pub(super) resume_version_id: String,
    pub(super) file_name: String,
    pub(super) clean_text: String,
    pub(super) sections: Vec<IndexSection>,
}

impl CachedSearchDocument {
    pub(super) fn from_index_document(document: IndexDocument) -> Self {
        Self {
            doc_id: document.doc_id,
            resume_version_id: document.resume_version_id,
            file_name: redact_contact_values(&document.file_name).into_owned(),
            clean_text: redact_contact_values(&document.clean_text).into_owned(),
            sections: Vec::new(),
        }
    }

    pub(super) fn to_index_document(&self, sectionizer: &Sectionizer) -> IndexDocument {
        IndexDocument {
            doc_id: self.doc_id.clone(),
            resume_version_id: self.resume_version_id.clone(),
            file_name: self.file_name.clone(),
            clean_text: self.clean_text.clone(),
            sections: sections_to_index(sectionizer.sectionize(&self.clean_text)),
        }
    }

    pub(super) fn into_index_document(self, sectionizer: &Sectionizer) -> IndexDocument {
        let sections = sections_to_index(sectionizer.sectionize(&self.clean_text));
        IndexDocument {
            doc_id: self.doc_id,
            resume_version_id: self.resume_version_id,
            file_name: self.file_name,
            clean_text: self.clean_text,
            sections,
        }
    }
}

pub(super) fn ensure_cache_matches_generation(
    index_root: &Path,
    base_generation: Option<&str>,
    cache: &mut CurrentImportDocumentCache,
) -> Result<()> {
    if cache.initialized && cache.base_generation.as_deref() == base_generation {
        return Ok(());
    }

    cache.documents =
        incremental_snapshot_documents(index_root, base_generation, Vec::new(), &BTreeSet::new())
            .map_err(ImportPipelineError::index)?
            .into_iter()
            .map(CachedSearchDocument::from_index_document)
            .collect();
    cache.initialized = true;
    cache.base_generation = base_generation.map(str::to_string);
    Ok(())
}

pub(super) fn apply_document_delta(
    documents: &mut Vec<CachedSearchDocument>,
    replacements: Vec<IndexDocument>,
    removals: &BTreeSet<String>,
) {
    let mut replaced_or_removed_ids = removals.clone();
    for document in &replacements {
        replaced_or_removed_ids.insert(document.doc_id.clone());
    }

    documents.retain(|document| !replaced_or_removed_ids.contains(&document.doc_id));
    documents.extend(
        replacements
            .into_iter()
            .map(CachedSearchDocument::from_index_document),
    );
    documents.sort_by(|left, right| {
        left.doc_id
            .cmp(&right.doc_id)
            .then_with(|| left.resume_version_id.cmp(&right.resume_version_id))
    });
}
