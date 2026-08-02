use std::collections::BTreeMap;
use std::fs;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use core_domain::{SearchSelection, SectionType};
use meta_store::{
    BoundedFilterSelection, ReadMetaStore, SearchMetadataTransactionError, SearchProjectionFilter,
    SearchTextBytePageRequest, SearchTextBytePageResolution, MAX_BOUNDED_FILTER_SELECTION,
    MAX_SEARCH_TEXT_BYTE_PAGE_BYTES,
};
use sectionizer::Sectionizer;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokenizers::Tokenizer;

const REPORT_SCHEMA: &str = "resume-ir.embedding-input-observation.v1";
const PACK_ID: &str = "intfloat-multilingual-e5-small-qint8-r1";
const MODEL_ID: &str = PACK_ID;
const UPSTREAM_REVISION: &str = "614241f622f53c4eeff9890bdc4f31cfecc418b3";
const PASSAGE_PREFIX: &str = "passage: ";
const MAX_TEXT_BYTES: u64 = 65_536;
const MAX_REPORT_BYTES: usize = 64 * 1024;
const MIN_DOCUMENTS: u64 = 1_000;
const MIN_SATURATION_RATIO: f64 = 0.25;
const MIN_PRIORITY_LOSS_RATIO: f64 = 0.10;
const MIN_WORK_REDUCTION_RATIO: f64 = 0.10;
const BUDGETS: [usize; 3] = [512, 384, 256];
const FAMILIES: [Family; 9] = [
    Family::Profile,
    Family::Experience,
    Family::Skill,
    Family::Project,
    Family::Education,
    Family::Certificate,
    Family::Contact,
    Family::Other,
    Family::Unassigned,
];

pub fn run_embedding_input_observation(
    data_dir: &Path,
    runtime_dir: &Path,
) -> Result<EmbeddingInputObservationReport, EmbeddingInputObservationError> {
    let tokenizer = load_production_tokenizer(runtime_dir)?;
    let store = ReadMetaStore::open_data_dir(data_dir)
        .map_err(|_| EmbeddingInputObservationError::StoreUnavailable)?;
    let accumulator = match store.with_search_metadata_snapshot(|snapshot| {
        let selected = snapshot
            .bounded_filter_selection(
                &SearchProjectionFilter::default(),
                NonZeroUsize::new(MAX_BOUNDED_FILTER_SELECTION)
                    .ok_or(EmbeddingInputObservationError::ObservationInvalid)?,
            )
            .map_err(|_| EmbeddingInputObservationError::ObservationInvalid)?;
        let projections = match selected {
            BoundedFilterSelection::Selected(projections) => projections,
            BoundedFilterSelection::TooLarge { .. } => {
                return Err(EmbeddingInputObservationError::ObservationInvalid);
            }
        };
        let mut accumulator = Accumulator::new(projections.len())?;
        for projection in projections {
            let selection = SearchSelection {
                document_id: projection.document_id,
                resume_version_id: projection.resume_version_id,
                visible_epoch: snapshot.head().visible_epoch,
            };
            let first_request = SearchTextBytePageRequest::new(
                selection.clone(),
                0,
                MAX_SEARCH_TEXT_BYTE_PAGE_BYTES,
            )
            .map_err(|_| EmbeddingInputObservationError::ObservationInvalid)?;
            let first = current_page(
                snapshot
                    .clean_text_byte_page(&first_request)
                    .map_err(|_| EmbeddingInputObservationError::ObservationInvalid)?,
            )?;
            if first.total_bytes > MAX_TEXT_BYTES {
                accumulator.exclude_oversize()?;
                continue;
            }
            let mut text = first.text;
            let mut offset = first.next_offset_bytes;
            while offset < first.total_bytes {
                let request = SearchTextBytePageRequest::new(
                    selection.clone(),
                    offset,
                    MAX_SEARCH_TEXT_BYTE_PAGE_BYTES,
                )
                .map_err(|_| EmbeddingInputObservationError::ObservationInvalid)?;
                let page = current_page(
                    snapshot
                        .clean_text_byte_page(&request)
                        .map_err(|_| EmbeddingInputObservationError::ObservationInvalid)?,
                )?;
                if page.offset_bytes != offset || page.total_bytes != first.total_bytes {
                    return Err(EmbeddingInputObservationError::ObservationInvalid);
                }
                text.push_str(&page.text);
                offset = page.next_offset_bytes;
            }
            let text = text.trim();
            if text.is_empty() {
                accumulator.fail_document()?;
                continue;
            }
            match observe_document(&tokenizer, text) {
                Ok(observation) => accumulator.observe(observation)?,
                Err(EmbeddingInputObservationError::TokenizationFailed) => {
                    accumulator.fail_document()?;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(accumulator)
    }) {
        Ok(accumulator) => accumulator,
        Err(SearchMetadataTransactionError::Operation(error)) => return Err(error),
        Err(
            SearchMetadataTransactionError::Unavailable(_)
            | SearchMetadataTransactionError::Store(_),
        ) => {
            return Err(EmbeddingInputObservationError::StoreUnavailable);
        }
    };
    accumulator.finish()
}

fn current_page(
    resolution: SearchTextBytePageResolution,
) -> Result<meta_store::SearchTextBytePage, EmbeddingInputObservationError> {
    match resolution {
        SearchTextBytePageResolution::Current(page) => Ok(page),
        SearchTextBytePageResolution::Stale
        | SearchTextBytePageResolution::NotFound
        | SearchTextBytePageResolution::InvalidOffset => {
            Err(EmbeddingInputObservationError::ObservationInvalid)
        }
    }
}

fn observe_document(
    tokenizer: &Tokenizer,
    text: &str,
) -> Result<DocumentObservation, EmbeddingInputObservationError> {
    let prefixed = format!("{PASSAGE_PREFIX}{text}");
    let encoding = tokenizer
        .encode(prefixed, true)
        .map_err(|_| EmbeddingInputObservationError::TokenizationFailed)?;
    let active_tokens = encoding.len();
    if active_tokens == 0 {
        return Err(EmbeddingInputObservationError::TokenizationFailed);
    }
    let spans = section_spans(text)?;
    let mut families = BTreeMap::<Family, FamilyTokens>::new();
    for (index, &(start, end)) in encoding.get_offsets().iter().enumerate() {
        if start == end || end <= PASSAGE_PREFIX.len() {
            continue;
        }
        let clean_start = start.saturating_sub(PASSAGE_PREFIX.len()).min(text.len());
        let clean_end = end.saturating_sub(PASSAGE_PREFIX.len()).min(text.len());
        let family = spans
            .iter()
            .find(|span| clean_start < span.end && clean_end > span.start)
            .map_or(Family::Unassigned, |span| span.family);
        let tokens = families.entry(family).or_default();
        tokens.total = tokens
            .total
            .checked_add(1)
            .ok_or(EmbeddingInputObservationError::ObservationInvalid)?;
        for (budget_index, budget) in BUDGETS.iter().enumerate() {
            if index < *budget {
                tokens.retained[budget_index] = tokens.retained[budget_index]
                    .checked_add(1)
                    .ok_or(EmbeddingInputObservationError::ObservationInvalid)?;
            }
        }
    }
    Ok(DocumentObservation {
        active_tokens,
        families,
    })
}

fn section_spans(text: &str) -> Result<Vec<FamilySpan>, EmbeddingInputObservationError> {
    let mut spans = Sectionizer::default()
        .sectionize(text)
        .into_iter()
        .map(|section| {
            Ok(FamilySpan {
                start: char_to_byte(text, section.char_start)?,
                end: char_to_byte(text, section.char_end)?,
                family: Family::from_section(&section.section_type),
            })
        })
        .collect::<Result<Vec<_>, EmbeddingInputObservationError>>()?;
    spans.sort_by_key(|span| (span.start, span.end));
    let mut previous_end = 0;
    for span in &spans {
        if span.start >= span.end || span.end > text.len() || span.start < previous_end {
            return Err(EmbeddingInputObservationError::SectionAttributionUnstable);
        }
        previous_end = span.end;
    }
    Ok(spans)
}

fn char_to_byte(text: &str, char_offset: usize) -> Result<usize, EmbeddingInputObservationError> {
    if char_offset == text.chars().count() {
        return Ok(text.len());
    }
    text.char_indices()
        .nth(char_offset)
        .map(|(byte_offset, _)| byte_offset)
        .ok_or(EmbeddingInputObservationError::SectionAttributionUnstable)
}

fn load_production_tokenizer(
    runtime_dir: &Path,
) -> Result<Tokenizer, EmbeddingInputObservationError> {
    let root = canonical_direct_dir(runtime_dir)?;
    let tokenizer_path = validated_asset(
        &root.join("tokenizer.json"),
        17_082_730,
        "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
    )?;
    let mut tokenizer = Tokenizer::from_file(tokenizer_path)
        .map_err(|_| EmbeddingInputObservationError::RuntimePackInvalid)?;
    tokenizer.with_padding(None);
    tokenizer
        .with_truncation(None)
        .map_err(|_| EmbeddingInputObservationError::RuntimePackInvalid)?;
    Ok(tokenizer)
}

fn canonical_direct_dir(path: &Path) -> Result<PathBuf, EmbeddingInputObservationError> {
    if !path.is_absolute() || path.as_os_str().len() > 4_096 {
        return Err(EmbeddingInputObservationError::RuntimePackInvalid);
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| EmbeddingInputObservationError::RuntimePackInvalid)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(EmbeddingInputObservationError::RuntimePackInvalid);
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| EmbeddingInputObservationError::RuntimePackInvalid)?;
    Ok(canonical)
}

fn validated_asset(
    path: &Path,
    expected_bytes: u64,
    expected_sha256: &str,
) -> Result<PathBuf, EmbeddingInputObservationError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| EmbeddingInputObservationError::RuntimePackInvalid)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() != expected_bytes
        || sha256_file(path)? != expected_sha256
    {
        return Err(EmbeddingInputObservationError::RuntimePackInvalid);
    }
    Ok(path.to_path_buf())
}

fn sha256_file(path: &Path) -> Result<String, EmbeddingInputObservationError> {
    let bytes = fs::read(path).map_err(|_| EmbeddingInputObservationError::RuntimePackInvalid)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[derive(Clone, Copy, Debug, Default)]
struct FamilyTokens {
    total: u64,
    retained: [u64; 3],
}

struct DocumentObservation {
    active_tokens: usize,
    families: BTreeMap<Family, FamilyTokens>,
}

#[derive(Clone, Copy, Debug)]
struct FamilySpan {
    start: usize,
    end: usize,
    family: Family,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Family {
    Profile,
    Experience,
    Skill,
    Project,
    Education,
    Certificate,
    Contact,
    Other,
    Unassigned,
}

impl Family {
    fn from_section(section: &SectionType) -> Self {
        match section {
            SectionType::Profile => Self::Profile,
            SectionType::Experience => Self::Experience,
            SectionType::Skill => Self::Skill,
            SectionType::Project => Self::Project,
            SectionType::Education => Self::Education,
            SectionType::Certificate => Self::Certificate,
            SectionType::Contact => Self::Contact,
            SectionType::Other(_) => Self::Other,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Experience => "experience",
            Self::Skill => "skill",
            Self::Project => "project",
            Self::Education => "education",
            Self::Certificate => "certificate",
            Self::Contact => "contact",
            Self::Other => "other",
            Self::Unassigned => "unassigned",
        }
    }

    const fn is_priority(self) -> bool {
        matches!(self, Self::Experience | Self::Skill | Self::Project)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct CoverageBucket {
    complete_loss: u64,
    partial_below_half: u64,
    partial_at_least_half: u64,
    complete_retained: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct FamilyCoverage {
    present_documents: u64,
    budgets: [CoverageBucket; 3],
}

struct Accumulator {
    selected_documents: u64,
    observed_documents: u64,
    excluded_oversize_documents: u64,
    failed_documents: u64,
    token_histogram: BTreeMap<usize, u64>,
    work: [u64; 3],
    families: BTreeMap<Family, FamilyCoverage>,
    priority_present_documents: u64,
    priority_low_coverage_documents_512: u64,
}

impl Accumulator {
    fn new(selected_documents: usize) -> Result<Self, EmbeddingInputObservationError> {
        Ok(Self {
            selected_documents: u64::try_from(selected_documents)
                .map_err(|_| EmbeddingInputObservationError::ObservationInvalid)?,
            observed_documents: 0,
            excluded_oversize_documents: 0,
            failed_documents: 0,
            token_histogram: BTreeMap::new(),
            work: [0; 3],
            families: FAMILIES
                .into_iter()
                .map(|family| (family, FamilyCoverage::default()))
                .collect(),
            priority_present_documents: 0,
            priority_low_coverage_documents_512: 0,
        })
    }

    fn exclude_oversize(&mut self) -> Result<(), EmbeddingInputObservationError> {
        checked_increment(&mut self.excluded_oversize_documents)
    }

    fn fail_document(&mut self) -> Result<(), EmbeddingInputObservationError> {
        checked_increment(&mut self.failed_documents)
    }

    fn observe(
        &mut self,
        observation: DocumentObservation,
    ) -> Result<(), EmbeddingInputObservationError> {
        checked_increment(&mut self.observed_documents)?;
        checked_increment(
            self.token_histogram
                .entry(observation.active_tokens)
                .or_default(),
        )?;
        for (index, budget) in BUDGETS.iter().enumerate() {
            self.work[index] = self.work[index]
                .checked_add(
                    u64::try_from(observation.active_tokens.min(*budget))
                        .map_err(|_| EmbeddingInputObservationError::ObservationInvalid)?,
                )
                .ok_or(EmbeddingInputObservationError::ObservationInvalid)?;
        }
        let mut priority_present = false;
        let mut priority_low_coverage = false;
        for family in FAMILIES {
            let tokens = observation
                .families
                .get(&family)
                .copied()
                .unwrap_or_default();
            if tokens.total == 0 {
                continue;
            }
            let coverage = self
                .families
                .get_mut(&family)
                .ok_or(EmbeddingInputObservationError::ObservationInvalid)?;
            checked_increment(&mut coverage.present_documents)?;
            for (index, retained) in tokens.retained.iter().enumerate() {
                let bucket = &mut coverage.budgets[index];
                if *retained == 0 {
                    checked_increment(&mut bucket.complete_loss)?;
                } else if retained.saturating_mul(2) < tokens.total {
                    checked_increment(&mut bucket.partial_below_half)?;
                } else if *retained < tokens.total {
                    checked_increment(&mut bucket.partial_at_least_half)?;
                } else {
                    checked_increment(&mut bucket.complete_retained)?;
                }
            }
            if family.is_priority() {
                priority_present = true;
                priority_low_coverage |= tokens.retained[0].saturating_mul(2) < tokens.total;
            }
        }
        if priority_present {
            checked_increment(&mut self.priority_present_documents)?;
        }
        if priority_low_coverage {
            checked_increment(&mut self.priority_low_coverage_documents_512)?;
        }
        Ok(())
    }

    fn finish(self) -> Result<EmbeddingInputObservationReport, EmbeddingInputObservationError> {
        if self.observed_documents + self.excluded_oversize_documents + self.failed_documents
            != self.selected_documents
            || self.token_histogram.values().sum::<u64>() != self.observed_documents
        {
            return Err(EmbeddingInputObservationError::ObservationInvalid);
        }
        let saturated_512 = self
            .token_histogram
            .range(513..)
            .map(|(_, count)| *count)
            .sum::<u64>();
        let saturation_ratio = ratio(saturated_512, self.observed_documents);
        let priority_loss_ratio = ratio(
            self.priority_low_coverage_documents_512,
            self.priority_present_documents,
        );
        let work_reduction_384 = reduction(self.work[0], self.work[1]);
        let triggers = [
            self.observed_documents >= MIN_DOCUMENTS,
            saturation_ratio >= MIN_SATURATION_RATIO,
            self.priority_present_documents > 0 && priority_loss_ratio >= MIN_PRIORITY_LOSS_RATIO,
            work_reduction_384 >= MIN_WORK_REDUCTION_RATIO,
        ];
        let value = json!({
            "schema_version": REPORT_SCHEMA,
            "artifact_id": "embedding-input-observation-issue-312",
            "scope": "private local clean-text token observation; bounded redacted aggregate only",
            "production_identity": {
                "runtime_pack_id": PACK_ID,
                "model_id": MODEL_ID,
                "upstream_revision": UPSTREAM_REVISION,
                "tokenizer_sha256": "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
                "prefix": "passage",
                "truncation": "right"
            },
            "documents": {
                "selected": self.selected_documents,
                "observed": self.observed_documents,
                "excluded_oversize": self.excluded_oversize_documents,
                "failed": self.failed_documents
            },
            "pre_truncation_active_tokens": {
                "p50": quantile(&self.token_histogram, self.observed_documents, 50),
                "p75": quantile(&self.token_histogram, self.observed_documents, 75),
                "p90": quantile(&self.token_histogram, self.observed_documents, 90),
                "p95": quantile(&self.token_histogram, self.observed_documents, 95),
                "p99": quantile(&self.token_histogram, self.observed_documents, 99),
                "buckets": token_buckets(&self.token_histogram),
                "exceed_256": count_above(&self.token_histogram, 256),
                "exceed_384": count_above(&self.token_histogram, 384),
                "exceed_512": saturated_512,
                "exceed_512_ratio": saturation_ratio
            },
            "aggregate_active_token_work": {
                "budget_512": self.work[0],
                "budget_384": self.work[1],
                "budget_256": self.work[2],
                "reduction_384_vs_512": work_reduction_384,
                "reduction_256_vs_512": reduction(self.work[0], self.work[2])
            },
            "section_coverage": family_json(&self.families),
            "priority_coverage_512": {
                "documents_present": self.priority_present_documents,
                "documents_below_half": self.priority_low_coverage_documents_512,
                "below_half_ratio": priority_loss_ratio
            },
            "triggers": {
                "minimum_documents": triggers[0],
                "saturation_over_512": triggers[1],
                "priority_loss_at_512": triggers[2],
                "work_reduction_at_384": triggers[3],
                "all": triggers.into_iter().all(|trigger| trigger)
            },
            "decision": if triggers.into_iter().all(|trigger| trigger) { "l1_eligible" } else { "lost" },
            "privacy": {
                "contains_raw_text": false,
                "contains_token_ids": false,
                "contains_per_document_rows": false,
                "contains_paths": false,
                "contains_names": false,
                "contains_direct_raw_hashes": false
            },
            "claims": ["observation_only", "no_product_speedup", "no_quality_claim", "no_release_claim"]
        });
        let json = serde_json::to_string(&value)
            .map_err(|_| EmbeddingInputObservationError::ObservationInvalid)?;
        if json.len() > MAX_REPORT_BYTES || !json.is_ascii() {
            return Err(EmbeddingInputObservationError::ObservationInvalid);
        }
        Ok(EmbeddingInputObservationReport { json })
    }
}

fn checked_increment(value: &mut u64) -> Result<(), EmbeddingInputObservationError> {
    *value = value
        .checked_add(1)
        .ok_or(EmbeddingInputObservationError::ObservationInvalid)?;
    Ok(())
}

fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn reduction(control: u64, candidate: u64) -> f64 {
    if control == 0 {
        0.0
    } else {
        control.saturating_sub(candidate) as f64 / control as f64
    }
}

fn count_above(histogram: &BTreeMap<usize, u64>, boundary: usize) -> u64 {
    histogram
        .range((boundary + 1)..)
        .map(|(_, count)| *count)
        .sum()
}

fn quantile(histogram: &BTreeMap<usize, u64>, count: u64, percentile: u64) -> usize {
    if count == 0 {
        return 0;
    }
    let rank = count.saturating_mul(percentile).div_ceil(100).max(1);
    let mut cumulative = 0_u64;
    for (tokens, bucket_count) in histogram {
        cumulative = cumulative.saturating_add(*bucket_count);
        if cumulative >= rank {
            return *tokens;
        }
    }
    0
}

fn token_buckets(histogram: &BTreeMap<usize, u64>) -> Value {
    let ranges = [
        ("le_256", 0..=256),
        ("257_384", 257..=384),
        ("385_512", 385..=512),
        ("513_768", 513..=768),
        ("769_1024", 769..=1024),
        ("gt_1024", 1025..=usize::MAX),
    ];
    Value::Object(
        ranges
            .into_iter()
            .map(|(label, range)| {
                let count = histogram.range(range).map(|(_, count)| *count).sum::<u64>();
                (label.to_string(), Value::from(count))
            })
            .collect(),
    )
}

fn family_json(families: &BTreeMap<Family, FamilyCoverage>) -> Value {
    Value::Object(
        FAMILIES
            .into_iter()
            .map(|family| {
                let coverage = families.get(&family).copied().unwrap_or_default();
                let budgets = BUDGETS
                    .into_iter()
                    .enumerate()
                    .map(|(index, budget)| {
                        let bucket = coverage.budgets[index];
                        (
                            budget.to_string(),
                            json!({
                                "complete_loss": bucket.complete_loss,
                                "partial_below_half": bucket.partial_below_half,
                                "partial_at_least_half": bucket.partial_at_least_half,
                                "complete_retained": bucket.complete_retained
                            }),
                        )
                    })
                    .collect();
                (
                    family.label().to_string(),
                    json!({
                        "present_documents": coverage.present_documents,
                        "budgets": Value::Object(budgets)
                    }),
                )
            })
            .collect(),
    )
}

pub struct EmbeddingInputObservationReport {
    json: String,
}

impl EmbeddingInputObservationReport {
    pub fn to_redacted_json(&self) -> &str {
        &self.json
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmbeddingInputObservationError {
    RuntimePackInvalid,
    StoreUnavailable,
    TokenizationFailed,
    SectionAttributionUnstable,
    ObservationInvalid,
}

impl std::fmt::Display for EmbeddingInputObservationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::RuntimePackInvalid => "embedding observation runtime pack is invalid",
            Self::StoreUnavailable => "embedding observation store is unavailable",
            Self::TokenizationFailed => "embedding observation tokenization failed",
            Self::SectionAttributionUnstable => {
                "embedding observation section attribution is unstable"
            }
            Self::ObservationInvalid => "embedding observation result is invalid",
        })
    }
}

impl std::error::Error for EmbeddingInputObservationError {}

#[cfg(test)]
#[path = "embedding_input_observation_tests.rs"]
mod tests;
