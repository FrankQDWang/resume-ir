//! Tantivy-backed full-text indexing for local resume search.

use search_planner::{
    default_snippet, plan_snippets_for_top_results, PlannerCandidate, SearchOptions,
};
use std::fmt;
use std::fs;
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::directory::error::OpenDirectoryError;
use tantivy::directory::MmapDirectory;
use tantivy::query::QueryParserError;
use tantivy::query::{BooleanQuery, Query, QueryParser, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, Schema, TantivyDocument, TextFieldIndexing, TextOptions, Value, FAST,
    INDEXED, STORED, STRING,
};
use tantivy::tokenizer::{LowerCaser, NgramTokenizer, TextAnalyzer};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, Term};
use thiserror::Error;

const CLEAN_TEXT_TOKENIZER: &str = "resume_cjk_ngram";
const WRITER_MEMORY_BYTES: usize = 50_000_000;

/// Full-text index operation result.
pub type Result<T> = std::result::Result<T, FullTextError>;

/// Full-text index operation error.
#[derive(Error)]
pub enum FullTextError {
    /// No Tantivy index metadata exists at the requested index directory.
    #[error("full-text index is not available")]
    MissingIndex,
    /// Filesystem operation failed.
    #[error("full-text index filesystem operation failed")]
    Io(#[from] std::io::Error),
    /// Tantivy directory operation failed.
    #[error("full-text index directory operation failed")]
    Directory(#[from] OpenDirectoryError),
    /// Query parsing failed.
    #[error("full-text search query could not be parsed")]
    QueryParser(#[from] QueryParserError),
    /// A stored index document is missing required fields.
    #[error("full-text index document is malformed")]
    MalformedDocument,
    /// Tantivy operation failed.
    #[error("full-text index operation failed")]
    Tantivy(#[from] tantivy::TantivyError),
}

impl fmt::Debug for FullTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingIndex => formatter.write_str("FullTextError::MissingIndex"),
            Self::Io(_) => formatter.write_str("FullTextError::Io([redacted filesystem error])"),
            Self::Directory(_) => {
                formatter.write_str("FullTextError::Directory([redacted directory error])")
            }
            Self::QueryParser(_) => {
                formatter.write_str("FullTextError::QueryParser([redacted query error])")
            }
            Self::MalformedDocument => formatter.write_str("FullTextError::MalformedDocument"),
            Self::Tantivy(_) => {
                formatter.write_str("FullTextError::Tantivy([redacted index error])")
            }
        }
    }
}

/// Local document payload accepted by the full-text writer.
#[derive(Clone, PartialEq)]
pub struct IndexDocument {
    /// Stable document identifier.
    pub doc_id: String,
    /// Stable parsed version identifier.
    pub version_id: String,
    /// File name only, never a local path.
    pub file_name: String,
    /// Clean local text to index and store for snippet generation.
    pub clean_text: String,
    /// Semantic section type label.
    pub section_type: String,
    /// Whether this document is a deleted marker hidden from default search.
    pub is_deleted: bool,
}

impl fmt::Debug for IndexDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IndexDocument")
            .field("doc_id", &self.doc_id)
            .field("version_id", &self.version_id)
            .field("file_name", &"<redacted>")
            .field("clean_text", &"<redacted>")
            .field("section_type", &self.section_type)
            .field("is_deleted", &self.is_deleted)
            .finish()
    }
}

/// Ranked full-text search hit.
#[derive(Clone, PartialEq)]
pub struct SearchHit {
    /// One-based result rank.
    pub rank: usize,
    /// Stable document identifier.
    pub doc_id: String,
    /// File name only, never a local path.
    pub file_name: String,
    /// Short display snippet.
    pub snippet: String,
}

impl fmt::Debug for SearchHit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchHit")
            .field("rank", &self.rank)
            .field("doc_id", &self.doc_id)
            .field("file_name", &"<redacted>")
            .field("snippet", &"<redacted>")
            .finish()
    }
}

/// Field handles for the Tantivy resume schema.
#[derive(Clone, Copy)]
pub struct ResumeSchemaFields {
    doc_id: Field,
    version_id: Field,
    file_name: Field,
    clean_text: Field,
    section_type: Field,
    is_deleted: Field,
}

/// Owns Tantivy writes. Keep separate from readers so search can reload independently.
pub struct FullTextIndexWriter {
    fields: ResumeSchemaFields,
    writer: IndexWriter,
}

/// Owns Tantivy reads and query parsing.
pub struct FullTextIndexReader {
    index: Index,
    fields: ResumeSchemaFields,
    reader: IndexReader,
}

impl FullTextIndexWriter {
    /// Opens an existing index or creates one at `index_dir`.
    pub fn open_or_create(index_dir: impl AsRef<Path>) -> Result<Self> {
        fs::create_dir_all(index_dir.as_ref())?;
        let schema = resume_schema();
        let directory = MmapDirectory::open(index_dir.as_ref())?;
        let mut index = Index::open_or_create(directory, schema)?;
        register_tokenizers(&mut index)?;
        let fields = schema_fields(index.schema())?;
        let writer = index.writer(WRITER_MEMORY_BYTES)?;
        Ok(Self { fields, writer })
    }

    /// Adds or replaces the current searchable version for one document.
    pub fn add_document(&mut self, document: IndexDocument) -> Result<()> {
        self.writer
            .delete_term(Term::from_field_text(self.fields.doc_id, &document.doc_id));
        self.writer.delete_term(Term::from_field_text(
            self.fields.version_id,
            &document.version_id,
        ));
        let mut tantivy_document = TantivyDocument::default();
        tantivy_document.add_text(self.fields.doc_id, &document.doc_id);
        tantivy_document.add_text(self.fields.version_id, &document.version_id);
        tantivy_document.add_text(self.fields.file_name, &document.file_name);
        tantivy_document.add_text(self.fields.clean_text, &document.clean_text);
        tantivy_document.add_text(self.fields.section_type, &document.section_type);
        tantivy_document.add_bool(self.fields.is_deleted, document.is_deleted);
        self.writer.add_document(tantivy_document)?;
        Ok(())
    }

    /// Removes all indexed versions for one document.
    pub fn delete_document(&mut self, doc_id: &str) {
        self.writer
            .delete_term(Term::from_field_text(self.fields.doc_id, doc_id));
    }

    /// Commits pending writes.
    pub fn commit(&mut self) -> Result<()> {
        self.writer.commit()?;
        Ok(())
    }
}

impl FullTextIndexReader {
    /// Opens an existing full-text index.
    pub fn open_existing(index_dir: impl AsRef<Path>) -> Result<Self> {
        if !index_dir.as_ref().join("meta.json").is_file() {
            return Err(FullTextError::MissingIndex);
        }
        let mut index = Index::open_in_dir(index_dir)?;
        register_tokenizers(&mut index)?;
        let fields = schema_fields(index.schema())?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        Ok(Self {
            index,
            fields,
            reader,
        })
    }

    /// Searches the current index and reloads the reader before every query.
    pub fn search(&self, query_text: &str, options: SearchOptions) -> Result<Vec<SearchHit>> {
        if options.top_k == 0 {
            return Ok(Vec::new());
        }
        self.reader.reload()?;
        let parser = QueryParser::for_index(&self.index, vec![self.fields.clean_text]);
        let text_query = parser.parse_query(query_text)?;
        let query: Box<dyn Query> = if options.include_deleted {
            text_query
        } else {
            Box::new(BooleanQuery::intersection(vec![
                text_query,
                Box::new(TermQuery::new(
                    Term::from_field_bool(self.fields.is_deleted, false),
                    IndexRecordOption::Basic,
                )),
            ]))
        };
        let searcher = self.reader.searcher();
        let top_docs = searcher.search(
            &*query,
            &TopDocs::with_limit(options.top_k).order_by_score(),
        )?;
        let mut candidates = Vec::with_capacity(top_docs.len());

        for (zero_based_rank, (score, address)) in top_docs.into_iter().enumerate() {
            let document = searcher.doc::<TantivyDocument>(address)?;
            candidates.push(PlannerCandidate {
                rank: zero_based_rank + 1,
                score,
                doc_id: required_stored_text(&document, self.fields.doc_id)?,
                file_name: required_stored_text(&document, self.fields.file_name)?,
                clean_text: required_stored_text(&document, self.fields.clean_text)?,
            });
        }

        Ok(
            plan_snippets_for_top_results(candidates, query_text, options, default_snippet)
                .into_iter()
                .map(|hit| SearchHit {
                    rank: hit.rank,
                    doc_id: hit.doc_id,
                    file_name: hit.file_name,
                    snippet: hit.snippet,
                })
                .collect(),
        )
    }
}

/// Returns the crate name for smoke tests and workspace metadata.
#[must_use]
pub fn crate_name() -> &'static str {
    "index-fulltext"
}

fn resume_schema() -> Schema {
    let mut builder = Schema::builder();
    let clean_text_options = TextOptions::default().set_stored().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(CLEAN_TEXT_TOKENIZER)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    builder.add_text_field("doc_id", STRING | STORED | FAST);
    builder.add_text_field("version_id", STRING | STORED | FAST);
    builder.add_text_field("file_name", STRING | STORED | FAST);
    builder.add_text_field("clean_text", clean_text_options);
    builder.add_text_field("section_type", STRING | STORED | FAST);
    builder.add_bool_field("is_deleted", INDEXED | STORED | FAST);
    builder.build()
}

fn schema_fields(schema: Schema) -> Result<ResumeSchemaFields> {
    Ok(ResumeSchemaFields {
        doc_id: schema.get_field("doc_id")?,
        version_id: schema.get_field("version_id")?,
        file_name: schema.get_field("file_name")?,
        clean_text: schema.get_field("clean_text")?,
        section_type: schema.get_field("section_type")?,
        is_deleted: schema.get_field("is_deleted")?,
    })
}

fn register_tokenizers(index: &mut Index) -> Result<()> {
    let tokenizer = TextAnalyzer::builder(NgramTokenizer::all_ngrams(1, 24)?)
        .filter(LowerCaser)
        .build();
    index.tokenizers().register(CLEAN_TEXT_TOKENIZER, tokenizer);
    Ok(())
}

fn stored_text(document: &TantivyDocument, field: Field) -> Option<String> {
    document
        .get_first(field)
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
}

fn required_stored_text(document: &TantivyDocument, field: Field) -> Result<String> {
    let value = stored_text(document, field).ok_or(FullTextError::MalformedDocument)?;
    if value.is_empty() {
        return Err(FullTextError::MalformedDocument);
    }
    Ok(value)
}
