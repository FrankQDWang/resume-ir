use std::sync::Mutex;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{FAST, Field, INDEXED, STORED, STRING, Schema, TEXT, TantivyDocument, Value};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, doc};

pub type FullTextResult<T> = tantivy::Result<T>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexDocument {
    pub doc_id: String,
    pub version_id: String,
    pub file_name: String,
    pub clean_text: String,
    pub section_type: String,
    pub is_deleted: bool,
}

impl IndexDocument {
    #[must_use]
    pub fn searchable(
        doc_id: impl Into<String>,
        version_id: impl Into<String>,
        file_name: impl Into<String>,
        clean_text: impl Into<String>,
        section_type: impl Into<String>,
    ) -> Self {
        Self {
            doc_id: doc_id.into(),
            version_id: version_id.into(),
            file_name: file_name.into(),
            clean_text: clean_text.into(),
            section_type: section_type.into(),
            is_deleted: false,
        }
    }

    #[must_use]
    pub fn deleted(
        doc_id: impl Into<String>,
        version_id: impl Into<String>,
        file_name: impl Into<String>,
        clean_text: impl Into<String>,
        section_type: impl Into<String>,
    ) -> Self {
        Self {
            is_deleted: true,
            ..Self::searchable(doc_id, version_id, file_name, clean_text, section_type)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchHit {
    pub rank: usize,
    pub doc_id: String,
    pub version_id: String,
    pub file_name: String,
    pub snippet: String,
}

#[derive(Clone, Copy)]
struct FullTextFields {
    doc_id: Field,
    version_id: Field,
    file_name: Field,
    clean_text: Field,
    section_type: Field,
    is_deleted: Field,
}

pub struct FullTextIndex {
    index: Index,
    reader: IndexReader,
    writer: Mutex<IndexWriter>,
    fields: FullTextFields,
}

impl FullTextIndex {
    pub fn create_in_memory() -> tantivy::Result<Self> {
        let mut builder = Schema::builder();
        let fields = FullTextFields {
            doc_id: builder.add_text_field("doc_id", STRING | STORED),
            version_id: builder.add_text_field("version_id", STRING | STORED),
            file_name: builder.add_text_field("file_name", TEXT | STORED),
            clean_text: builder.add_text_field("clean_text", TEXT | STORED),
            section_type: builder.add_text_field("section_type", STRING | STORED),
            is_deleted: builder.add_u64_field("is_deleted", INDEXED | STORED | FAST),
        };
        let schema = builder.build();
        let index = Index::create_in_ram(schema);
        let writer = Mutex::new(index.writer(50_000_000)?);
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;

        Ok(Self {
            index,
            reader,
            writer,
            fields,
        })
    }

    pub fn index_batch(&self, docs: Vec<IndexDocument>) -> tantivy::Result<()> {
        let writer = self
            .writer
            .lock()
            .map_err(|_| tantivy::TantivyError::Poisoned)?;
        for document in docs {
            writer.add_document(doc!(
                self.fields.doc_id => document.doc_id,
                self.fields.version_id => document.version_id,
                self.fields.file_name => document.file_name,
                self.fields.clean_text => document.clean_text,
                self.fields.section_type => document.section_type,
                self.fields.is_deleted => u64::from(document.is_deleted),
            ))?;
        }
        Ok(())
    }

    pub fn commit(&self) -> tantivy::Result<()> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| tantivy::TantivyError::Poisoned)?;
        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    pub fn search(&self, query: &str, top_k: usize) -> tantivy::Result<Vec<SearchHit>> {
        self.reader.reload()?;
        let searcher = self.reader.searcher();
        let query_parser = QueryParser::for_index(
            &self.index,
            vec![self.fields.file_name, self.fields.clean_text],
        );
        let parsed_query = query_parser.parse_query(query)?;
        let candidate_limit = top_k.saturating_mul(4).max(top_k).max(1);
        let top_docs = searcher.search(&parsed_query, &TopDocs::with_limit(candidate_limit))?;
        let mut hits = Vec::new();

        for (_score, address) in top_docs {
            let document = searcher.doc::<TantivyDocument>(address)?;
            if get_u64(&document, self.fields.is_deleted) == Some(1) {
                continue;
            }
            let clean_text = get_text(&document, self.fields.clean_text);
            hits.push(SearchHit {
                rank: hits.len() + 1,
                doc_id: get_text(&document, self.fields.doc_id),
                version_id: get_text(&document, self.fields.version_id),
                file_name: get_text(&document, self.fields.file_name),
                snippet: make_snippet(&clean_text, query),
            });
            if hits.len() == top_k {
                break;
            }
        }

        Ok(hits)
    }
}

fn get_text(document: &TantivyDocument, field: Field) -> String {
    document
        .get_first(field)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_owned()
}

fn get_u64(document: &TantivyDocument, field: Field) -> Option<u64> {
    document.get_first(field).and_then(|value| value.as_u64())
}

fn make_snippet(text: &str, query: &str) -> String {
    let first_term = query.split_whitespace().next().unwrap_or_default();
    let Some(start) = text
        .to_ascii_lowercase()
        .find(&first_term.to_ascii_lowercase())
    else {
        return text.chars().take(80).collect();
    };
    let snippet_start = start.saturating_sub(24);
    let snippet_end = (start + first_term.len() + 56).min(text.len());
    text[snippet_start..snippet_end].to_owned()
}

#[must_use]
pub fn crate_name() -> &'static str {
    "index-fulltext"
}
