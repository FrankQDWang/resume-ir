pub fn crate_name() -> &'static str {
    "index-fulltext"
}

use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, TantivyDocument, Value, FAST, STORED, STRING, TEXT};
use tantivy::{Index, IndexReader, IndexWriter};

const WRITER_HEAP_BYTES: usize = 50_000_000;
const DEFAULT_LIMIT: usize = 10;
const MAX_LIMIT: usize = 100;

#[derive(Clone, PartialEq, Eq)]
pub struct IndexDocument {
    pub doc_id: String,
    pub version_id: String,
    pub file_name: String,
    pub clean_text: String,
    pub sections: Vec<IndexSection>,
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
            .field("section_count", &self.sections.len())
            .field("is_deleted", &self.is_deleted)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct IndexSection {
    pub section_type: String,
    pub text: String,
}

impl fmt::Debug for IndexSection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IndexSection")
            .field("section_type", &self.section_type)
            .field("text", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SearchQuery {
    text: String,
    limit: usize,
}

impl fmt::Debug for SearchQuery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchQuery")
            .field("text", &"<redacted>")
            .field("limit", &self.limit)
            .finish()
    }
}

impl SearchQuery {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            limit: DEFAULT_LIMIT,
        }
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit.clamp(1, MAX_LIMIT);
        self
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn limit(&self) -> usize {
        self.limit
    }
}

#[derive(Clone, PartialEq)]
pub struct SearchHit {
    pub rank: usize,
    pub score: f32,
    pub doc_id: String,
    pub version_id: String,
    pub file_name: String,
    pub snippet: String,
}

impl fmt::Debug for SearchHit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchHit")
            .field("rank", &self.rank)
            .field("score", &self.score)
            .field("doc_id", &self.doc_id)
            .field("version_id", &self.version_id)
            .field("file_name", &"<redacted>")
            .field("snippet", &"<redacted>")
            .finish()
    }
}

pub struct FullTextIndex {
    index: Index,
    reader: IndexReader,
    writer: Option<Mutex<IndexWriter>>,
    fields: IndexFields,
}

impl FullTextIndex {
    pub fn open(index_dir: &Path) -> Result<Self> {
        let index = Index::open_in_dir(index_dir).map_err(FullTextError::tantivy)?;
        let schema = index.schema();
        let fields = IndexFields::from_schema(&schema)?;
        let reader = index.reader().map_err(FullTextError::tantivy)?;

        Ok(Self {
            index,
            reader,
            writer: None,
            fields,
        })
    }

    pub fn open_or_create(index_dir: &Path) -> Result<Self> {
        fs::create_dir_all(index_dir).map_err(FullTextError::io)?;
        let schema = build_schema();
        let index = if index_dir.join("meta.json").exists() {
            Index::open_in_dir(index_dir).map_err(FullTextError::tantivy)?
        } else {
            Index::create_in_dir(index_dir, schema).map_err(FullTextError::tantivy)?
        };

        let schema = index.schema();
        let fields = IndexFields::from_schema(&schema)?;
        let reader = index.reader().map_err(FullTextError::tantivy)?;
        let writer = index
            .writer(WRITER_HEAP_BYTES)
            .map_err(FullTextError::tantivy)?;

        Ok(Self {
            index,
            reader,
            writer: Some(Mutex::new(writer)),
            fields,
        })
    }

    pub fn replace_documents<I>(&self, documents: I) -> Result<()>
    where
        I: IntoIterator<Item = IndexDocument>,
    {
        let writer = self
            .writer
            .as_ref()
            .ok_or_else(|| FullTextError::internal("index opened read-only"))?
            .lock()
            .map_err(|_| FullTextError::internal("index writer lock poisoned"))?;
        writer
            .delete_all_documents()
            .map_err(FullTextError::tantivy)?;

        for document in documents {
            if document.is_deleted {
                continue;
            }

            let section_text = document
                .sections
                .iter()
                .map(|section| section.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");

            let mut tantivy_document = TantivyDocument::default();
            tantivy_document.add_text(self.fields.doc_id, &document.doc_id);
            tantivy_document.add_text(self.fields.version_id, &document.version_id);
            tantivy_document.add_text(self.fields.file_name, &document.file_name);
            tantivy_document.add_text(self.fields.clean_text, &document.clean_text);
            tantivy_document.add_text(self.fields.all_sections, &section_text);
            tantivy_document.add_bool(self.fields.is_deleted, false);
            for section in &document.sections {
                tantivy_document.add_text(self.fields.section_type, &section.section_type);
                tantivy_document.add_text(self.fields.section_text, &section.text);
            }
            writer
                .add_document(tantivy_document)
                .map_err(FullTextError::tantivy)?;
        }

        Ok(())
    }

    pub fn commit(&self) -> Result<()> {
        let mut writer = self
            .writer
            .as_ref()
            .ok_or_else(|| FullTextError::internal("index opened read-only"))?
            .lock()
            .map_err(|_| FullTextError::internal("index writer lock poisoned"))?;
        writer.commit().map_err(FullTextError::tantivy)?;
        Ok(())
    }

    pub fn reload(&self) -> Result<()> {
        self.reader.reload().map_err(FullTextError::tantivy)
    }

    pub fn search(&self, query: SearchQuery) -> Result<Vec<SearchHit>> {
        self.reload()?;
        let searcher = self.reader.searcher();
        let query_parser = QueryParser::for_index(
            &self.index,
            vec![
                self.fields.file_name,
                self.fields.clean_text,
                self.fields.section_text,
                self.fields.all_sections,
            ],
        );
        if query.text().trim().is_empty() {
            return Ok(Vec::new());
        }

        let (parsed_query, _parse_errors) = query_parser.parse_query_lenient(query.text());
        let candidate_limit = query.limit();
        let top_docs = searcher
            .search(
                &parsed_query,
                &TopDocs::with_limit(candidate_limit).order_by_score(),
            )
            .map_err(FullTextError::tantivy)?;

        let mut hits = Vec::new();
        let mut seen_doc_ids = std::collections::BTreeSet::new();
        for (score, address) in top_docs {
            let stored = searcher
                .doc::<TantivyDocument>(address)
                .map_err(FullTextError::tantivy)?;
            if bool_value(&stored, self.fields.is_deleted).unwrap_or(false) {
                continue;
            }

            let Some(doc_id) = text_value(&stored, self.fields.doc_id) else {
                continue;
            };
            if !seen_doc_ids.insert(doc_id.clone()) {
                continue;
            }

            let clean_text = text_value(&stored, self.fields.clean_text).unwrap_or_default();
            hits.push(SearchHit {
                rank: hits.len() + 1,
                score,
                doc_id,
                version_id: text_value(&stored, self.fields.version_id).unwrap_or_default(),
                file_name: text_value(&stored, self.fields.file_name).unwrap_or_default(),
                snippet: build_snippet(&clean_text, query.text()),
            });

            if hits.len() == query.limit() {
                break;
            }
        }

        Ok(hits)
    }
}

#[derive(Clone, Copy)]
struct IndexFields {
    doc_id: Field,
    version_id: Field,
    file_name: Field,
    clean_text: Field,
    section_type: Field,
    section_text: Field,
    all_sections: Field,
    is_deleted: Field,
}

impl IndexFields {
    fn from_schema(schema: &Schema) -> Result<Self> {
        Ok(Self {
            doc_id: schema.get_field("doc_id").map_err(FullTextError::tantivy)?,
            version_id: schema
                .get_field("version_id")
                .map_err(FullTextError::tantivy)?,
            file_name: schema
                .get_field("file_name")
                .map_err(FullTextError::tantivy)?,
            clean_text: schema
                .get_field("clean_text")
                .map_err(FullTextError::tantivy)?,
            section_type: schema
                .get_field("section_type")
                .map_err(FullTextError::tantivy)?,
            section_text: schema
                .get_field("section_text")
                .map_err(FullTextError::tantivy)?,
            all_sections: schema
                .get_field("all_sections")
                .map_err(FullTextError::tantivy)?,
            is_deleted: schema
                .get_field("is_deleted")
                .map_err(FullTextError::tantivy)?,
        })
    }
}

fn build_schema() -> Schema {
    let mut builder = Schema::builder();
    builder.add_text_field("doc_id", STRING | STORED | FAST);
    builder.add_text_field("version_id", STRING | STORED | FAST);
    builder.add_text_field("file_name", TEXT | STORED);
    builder.add_text_field("clean_text", TEXT | STORED);
    builder.add_text_field("section_type", STRING | STORED | FAST);
    builder.add_text_field("section_text", TEXT | STORED);
    builder.add_text_field("all_sections", TEXT | STORED);
    builder.add_bool_field("is_deleted", STORED | FAST);
    builder.build()
}

fn text_value(document: &TantivyDocument, field: Field) -> Option<String> {
    document
        .get_first(field)
        .and_then(|value| value.as_value().as_str())
        .map(str::to_string)
}

fn bool_value(document: &TantivyDocument, field: Field) -> Option<bool> {
    document
        .get_first(field)
        .and_then(|value| value.as_value().as_bool())
}

fn build_snippet(text: &str, query: &str) -> String {
    let terms = query.split_whitespace().collect::<Vec<_>>();
    let lower_text = text.to_ascii_lowercase();
    let first_match = terms
        .iter()
        .filter(|term| !term.is_empty())
        .find_map(|term| lower_text.find(&term.to_ascii_lowercase()))
        .unwrap_or(0);

    let start = nearest_char_boundary_before(text, first_match.saturating_sub(40));
    let end = nearest_char_boundary_after(text, (first_match + 80).min(text.len()));
    text[start..end].trim().to_string()
}

fn nearest_char_boundary_before(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn nearest_char_boundary_after(text: &str, mut index: usize) -> usize {
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

pub type Result<T> = std::result::Result<T, FullTextError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FullTextError {
    Io { diagnostic: String },
    Tantivy { diagnostic: String },
    Internal { diagnostic: String },
}

impl FullTextError {
    fn io(error: std::io::Error) -> Self {
        Self::Io {
            diagnostic: error.to_string(),
        }
    }

    fn tantivy(error: tantivy::TantivyError) -> Self {
        Self::Tantivy {
            diagnostic: error.to_string(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            diagnostic: message.into(),
        }
    }
}

impl fmt::Display for FullTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FullTextError::Io { .. } => formatter.write_str("full-text index IO error"),
            FullTextError::Tantivy { .. } => {
                formatter.write_str("full-text index operation failed")
            }
            FullTextError::Internal { .. } => formatter.write_str("full-text index internal error"),
        }
    }
}

impl std::error::Error for FullTextError {}
