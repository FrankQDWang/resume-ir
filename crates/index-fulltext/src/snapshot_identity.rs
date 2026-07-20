use super::*;

struct ValidatedSnapshotContents {
    identity_pairs: Vec<(String, String)>,
    logical_content_digest: ContentDigest,
}

pub(super) struct ExactSnapshotValidation {
    pub(super) projection_digest: SearchProjectionDigest,
    pub(super) logical_content_digest: ContentDigest,
    pub(super) identity_pairs: Vec<(String, String)>,
}

impl FullTextIndex {
    /// Returns the complete exact document/version mapping stored in this
    /// immutable snapshot, sorted by document identity. Exact-open validation
    /// populates this cache once; callers never rescan Tantivy for the mapping.
    pub fn exact_identity_pairs(&self) -> Result<&[(String, String)]> {
        self.exact_identity_pairs.as_deref().ok_or_else(|| {
            FullTextError::internal("full-text exact identity projection is unavailable")
        })
    }

    fn validated_snapshot_contents(&self) -> Result<ValidatedSnapshotContents> {
        self.reload()?;
        let searcher = self.reader.searcher();
        let actual_count = usize::try_from(searcher.num_docs())
            .map_err(|_| FullTextError::internal("full-text document count overflow"))?;

        let mut document_ids = BTreeSet::new();
        let mut resume_version_ids = BTreeSet::new();
        let mut records = Vec::with_capacity(actual_count);
        for segment_reader in searcher.segment_readers() {
            let store_reader = segment_reader
                .get_store_reader(10)
                .map_err(FullTextError::io)?;
            for stored in store_reader.iter::<TantivyDocument>(segment_reader.alive_bitset()) {
                let stored = stored.map_err(FullTextError::tantivy)?;
                let document_id = required_text_value(&stored, self.fields.doc_id, "document id")?;
                let resume_version_id = required_text_value(
                    &stored,
                    self.fields.resume_version_id,
                    "resume version id",
                )?;
                validate_stable_id(&document_id, "doc_", "document")?;
                validate_stable_id(&resume_version_id, "ver_", "resume version")?;
                if !document_ids.insert(document_id.clone())
                    || !resume_version_ids.insert(resume_version_id.clone())
                {
                    return Err(FullTextError::internal(
                        "full-text snapshot identity mapping is not one-to-one",
                    ));
                }
                let file_name = required_text_value(&stored, self.fields.file_name, "file name")?;
                let clean_text =
                    required_text_value(&stored, self.fields.clean_text, "clean text")?;
                records.push((document_id, resume_version_id, file_name, clean_text));
            }
        }
        if records.len() != actual_count {
            return Err(FullTextError::internal(
                "full-text snapshot stored document count mismatch",
            ));
        }
        records.sort_unstable_by(|left, right| {
            left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1))
        });
        let logical_content_digest = logical_content_digest(&records);
        let identity_pairs = records
            .into_iter()
            .map(|(document_id, resume_version_id, _, _)| (document_id, resume_version_id))
            .collect();
        Ok(ValidatedSnapshotContents {
            identity_pairs,
            logical_content_digest,
        })
    }

    pub(super) fn validate_exact_contents(
        &self,
        expected_document_count: usize,
    ) -> Result<ExactSnapshotValidation> {
        let contents = self.validated_snapshot_contents()?;
        if contents.identity_pairs.len() != expected_document_count {
            return Err(FullTextError::internal(
                "full-text snapshot document count mismatch",
            ));
        }
        let projection_digest = SearchProjectionDigest::from_pairs(
            contents
                .identity_pairs
                .iter()
                .map(|(document, version)| (document.as_str(), version.as_str())),
        )
        .map_err(|_| FullTextError::internal("full-text snapshot identity mapping invalid"))?;
        Ok(ExactSnapshotValidation {
            projection_digest,
            logical_content_digest: contents.logical_content_digest,
            identity_pairs: contents.identity_pairs,
        })
    }
}

fn logical_content_digest(records: &[(String, String, String, String)]) -> ContentDigest {
    let mut canonical = Vec::with_capacity(64 + records.len() * 4 * 71);
    canonical.extend_from_slice(b"resume-ir.fulltext.logical-content.v3");
    canonical.extend_from_slice(&(records.len() as u64).to_le_bytes());
    for (document_id, resume_version_id, file_name, clean_text) in records {
        for value in [document_id, resume_version_id, file_name, clean_text] {
            canonical.extend_from_slice(
                ContentDigest::from_bytes(value.as_bytes())
                    .as_str()
                    .as_bytes(),
            );
        }
    }
    ContentDigest::from_bytes(&canonical)
}
