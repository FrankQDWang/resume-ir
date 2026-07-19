use std::{fmt, str::FromStr};

use core_domain::{ContentDigest, DocumentId, ResumeVersionId, SourceRevisionId};

use crate::{ImportTaskId, MetaStoreError, Result, UnixTimestamp};

const CONTRACT_FIELD_MAX_BYTES: usize = 64;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImportProcessingContractId(ContentDigest);

impl ImportProcessingContractId {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for ImportProcessingContractId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ImportProcessingContractId(<redacted>)")
    }
}

impl FromStr for ImportProcessingContractId {
    type Err = MetaStoreError;

    fn from_str(value: &str) -> Result<Self> {
        value
            .parse::<ContentDigest>()
            .map(Self)
            .map_err(|_| MetaStoreError::invalid_value("import_processing_contract.id"))
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ImportProcessingContract {
    id: ImportProcessingContractId,
    primary_parse_version: String,
    ocr_parse_version: String,
    derived_schema_version: String,
    classifier_epoch: String,
}

impl ImportProcessingContract {
    pub fn new(
        primary_parse_version: impl Into<String>,
        ocr_parse_version: impl Into<String>,
        derived_schema_version: impl Into<String>,
        classifier_epoch: impl Into<String>,
    ) -> Result<Self> {
        let primary_parse_version = primary_parse_version.into();
        let ocr_parse_version = ocr_parse_version.into();
        let derived_schema_version = derived_schema_version.into();
        let classifier_epoch = classifier_epoch.into();
        validate_contract_field(
            &primary_parse_version,
            "import_processing_contract.primary_parse_version",
        )?;
        validate_contract_field(
            &ocr_parse_version,
            "import_processing_contract.ocr_parse_version",
        )?;
        validate_contract_field(
            &derived_schema_version,
            "import_processing_contract.derived_schema_version",
        )?;
        if crate::CurrentClassifierEpoch::parse(&classifier_epoch).is_none() {
            return Err(MetaStoreError::invalid_value("import_processing_contract"));
        }
        let id = contract_id(
            &primary_parse_version,
            &ocr_parse_version,
            &derived_schema_version,
            &classifier_epoch,
        )?;
        Ok(Self {
            id,
            primary_parse_version,
            ocr_parse_version,
            derived_schema_version,
            classifier_epoch,
        })
    }

    pub fn id(&self) -> &ImportProcessingContractId {
        &self.id
    }

    pub fn primary_parse_version(&self) -> &str {
        &self.primary_parse_version
    }

    pub fn ocr_parse_version(&self) -> &str {
        &self.ocr_parse_version
    }

    pub fn derived_schema_version(&self) -> &str {
        &self.derived_schema_version
    }

    pub fn classifier_epoch(&self) -> &str {
        &self.classifier_epoch
    }

    pub(super) fn from_stored_parts(
        id: &str,
        primary_parse_version: String,
        ocr_parse_version: String,
        derived_schema_version: String,
        classifier_epoch: String,
    ) -> Result<Self> {
        let contract = Self::new(
            primary_parse_version,
            ocr_parse_version,
            derived_schema_version,
            classifier_epoch,
        )?;
        if contract.id.as_str() != id {
            return Err(MetaStoreError::storage_invariant());
        }
        Ok(contract)
    }
}

impl fmt::Debug for ImportProcessingContract {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportProcessingContract")
            .field("id", &self.id)
            .field("primary_parse_version", &"<redacted>")
            .field("ocr_parse_version", &"<redacted>")
            .field("derived_schema_version", &"<redacted>")
            .field("classifier_epoch", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportSourceDispositionKind {
    Searchable,
    Excluded,
    OcrBacklog,
    Failed,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ImportTaskSourceDisposition {
    pub source_ordinal: u64,
    pub document_id: DocumentId,
    pub source_revision_id: SourceRevisionId,
    pub resume_version_id: Option<ResumeVersionId>,
    pub kind: ImportSourceDispositionKind,
}

pub const IMPORT_SOURCE_DISPOSITION_BATCH_LIMIT: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportTaskDispositionBatchOutcome {
    pub inserted: usize,
    pub already_present: usize,
}

impl fmt::Debug for ImportTaskSourceDisposition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportTaskSourceDisposition")
            .field("source_ordinal", &self.source_ordinal)
            .field("document_id", &"<redacted>")
            .field("source_revision_id", &"<redacted>")
            .field(
                "resume_version_id",
                &self.resume_version_id.as_ref().map(|_| "<redacted>"),
            )
            .field("kind", &self.kind)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ImportTaskCompletion {
    pub import_task_id: ImportTaskId,
    pub processing_contract_id: ImportProcessingContractId,
    pub source_disposition_count: u64,
    pub source_manifest_digest: ContentDigest,
    pub completed_at: UnixTimestamp,
}

impl fmt::Debug for ImportTaskCompletion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportTaskCompletion")
            .field("import_task_id", &"<redacted>")
            .field("processing_contract_id", &self.processing_contract_id)
            .field("source_disposition_count", &self.source_disposition_count)
            .field("source_manifest_digest", &"<redacted>")
            .field("completed_at", &self.completed_at)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationRebuildContractActivation {
    Activated,
    AlreadyActive,
    Superseded,
    RunningTaskConflict,
}

fn validate_contract_field(value: &str, field: &'static str) -> Result<()> {
    if value.is_empty()
        || value.len() > CONTRACT_FIELD_MAX_BYTES
        || value
            .bytes()
            .any(|byte| matches!(byte, 0 | b'\n' | b'\r' | b'\t'))
    {
        return Err(MetaStoreError::invalid_value(field));
    }
    Ok(())
}

fn contract_id(
    primary_parse_version: &str,
    ocr_parse_version: &str,
    derived_schema_version: &str,
    classifier_epoch: &str,
) -> Result<ImportProcessingContractId> {
    let mut canonical = Vec::new();
    append_part(&mut canonical, b"resume-ir.import-processing-contract.v1")?;
    append_part(&mut canonical, primary_parse_version.as_bytes())?;
    append_part(&mut canonical, ocr_parse_version.as_bytes())?;
    append_part(&mut canonical, derived_schema_version.as_bytes())?;
    append_part(&mut canonical, classifier_epoch.as_bytes())?;
    Ok(ImportProcessingContractId(ContentDigest::from_bytes(
        &canonical,
    )))
}

pub(super) fn append_part(target: &mut Vec<u8>, value: &[u8]) -> Result<()> {
    let length = u64::try_from(value.len()).map_err(|_| MetaStoreError::storage_invariant())?;
    target.extend_from_slice(&length.to_le_bytes());
    target.extend_from_slice(value);
    Ok(())
}

pub(super) fn disposition_to_storage(value: ImportSourceDispositionKind) -> &'static str {
    match value {
        ImportSourceDispositionKind::Searchable => "searchable",
        ImportSourceDispositionKind::Excluded => "excluded",
        ImportSourceDispositionKind::OcrBacklog => "ocr_backlog",
        ImportSourceDispositionKind::Failed => "failed",
    }
}
