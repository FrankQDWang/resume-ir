use meta_store::{
    ContactHash, ContentDigest, Document, EntityMention, ImmutableIngestStage, OwnedMetaStore,
    Result, ResumeVersion, ResumeVersionClassification, ResumeVersionId, SourceRevision,
    SourceRevisionTriage,
};

/// Immutable parse-derived data staged before an index generation is published.
///
/// Staging is deliberately separate from `ActiveSearchProjection`: partially
/// staged data cannot become query-visible until the publication CAS succeeds.
pub(super) struct StagedResume<'a> {
    pub document: &'a Document,
    pub source_revision: &'a SourceRevision,
    pub derived: StagedDerivedData<'a>,
}

pub(super) enum StagedDerivedData<'a> {
    SourceTriage(&'a SourceRevisionTriage),
    ClassifiedVersion {
        version: &'a ResumeVersion,
        classification: &'a ResumeVersionClassification,
        mentions: &'a [EntityMention],
        email_hash: Option<&'a ContactHash>,
        phone_hash: Option<&'a ContactHash>,
    },
}

pub(super) fn source_revision(document: &Document, bytes: &[u8]) -> SourceRevision {
    SourceRevision::for_content(
        document.id.clone(),
        ContentDigest::from_bytes(bytes),
        bytes.len() as u64,
    )
}

pub(super) fn resume_version(
    document: &Document,
    source_revision: &SourceRevision,
    clean_text: String,
    parse_version: &str,
    schema_version: &str,
    language_set: Vec<String>,
    page_count: Option<u32>,
    quality_score: Option<f32>,
) -> ResumeVersion {
    let normalized_text_hash = ContentDigest::from_bytes(clean_text.as_bytes());
    let id = ResumeVersionId::from_content_identity(
        &document.id,
        &source_revision.id,
        &normalized_text_hash,
        parse_version,
        schema_version,
    );
    ResumeVersion {
        id,
        document_id: document.id.clone(),
        source_revision_id: source_revision.id.clone(),
        normalized_text_hash,
        parse_version: parse_version.to_string(),
        schema_version: schema_version.to_string(),
        language_set,
        page_count,
        raw_text: None,
        clean_text: Some(clean_text),
        quality_score,
    }
}

pub(super) fn stage(store: &OwnedMetaStore, staged: StagedResume<'_>) -> Result<()> {
    let stage = match staged.derived {
        StagedDerivedData::SourceTriage(triage) => ImmutableIngestStage::SourceTriage {
            document: staged.document,
            source_revision: staged.source_revision,
            triage,
        },
        StagedDerivedData::ClassifiedVersion {
            version,
            classification,
            mentions,
            email_hash,
            phone_hash,
        } => ImmutableIngestStage::ClassifiedResume {
            document: staged.document,
            source_revision: staged.source_revision,
            version,
            classification,
            mentions,
            email_hash,
            phone_hash,
        },
    };
    store.stage_immutable_ingest(stage)
}
