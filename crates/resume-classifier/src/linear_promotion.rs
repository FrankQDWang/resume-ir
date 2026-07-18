use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{ClassificationResult, ReasonCode, CLASSIFIER_EPOCH};

const ARTIFACT_SCHEMA: &str = "resume_ir_linear_promotion_v1";
const FEATURE_CONTRACT: &str = "bounded_normalized_text_plus_structure_v1";
pub const PROMOTED_EPOCH_PREFIX: &str = "precision_first_v4_linear_";
const MAX_ARTIFACT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_FEATURES: usize = 250_000;
const MAX_INPUT_CHARS_LIMIT: usize = 32_768;

/// Allowed section structure supplied to local linear promotion inference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromotionSection {
    Profile,
    Contact,
    Education,
    Experience,
    Project,
    Skill,
    Certificate,
    OtherChunk,
}

/// Fail-closed optional local model used only to promote safe-gray reviews.
#[derive(Clone, Default)]
pub struct LinearPromotionPolicy(Option<Arc<LinearModel>>);

impl fmt::Debug for LinearPromotionPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LinearPromotionPolicy")
            .field("enabled", &self.0.is_some())
            .finish()
    }
}

impl LinearPromotionPolicy {
    /// Loads an owner-only local artifact. Every validation failure disables
    /// promotion rather than changing deterministic classifier behavior.
    pub fn load_local(path: &Path) -> Self {
        Self::try_load(path, ArtifactAccess::OwnerOnly).unwrap_or_default()
    }

    /// Loads a reviewed, immutable artifact from an application bundle.
    /// Group/world readability is allowed, but symlinks and writable bundle
    /// artifacts fail closed.
    pub fn load_bundled(path: &Path) -> Self {
        Self::try_load(path, ArtifactAccess::BundledReadOnly).unwrap_or_default()
    }

    pub fn enabled(&self) -> bool {
        self.0.is_some()
    }

    pub fn classifier_epoch(&self) -> Option<&str> {
        self.0.as_ref().map(|model| model.classifier_epoch.as_str())
    }

    pub fn apply(
        &self,
        normalized_text: &str,
        sections: &[PromotionSection],
        mut deterministic: ClassificationResult,
    ) -> ClassificationResult {
        let Some(model) = &self.0 else {
            return deterministic;
        };
        if deterministic.is_conflict_free_safe_gray()
            && model.predict(normalized_text, sections, deterministic.reason_codes())
        {
            deterministic.promote_to_resume_candidate();
        }
        deterministic.set_classifier_epoch(&model.classifier_epoch);
        deterministic
    }

    fn try_load(path: &Path, access: ArtifactAccess) -> Option<Self> {
        let metadata = fs::symlink_metadata(path).ok()?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() == 0
            || metadata.len() > MAX_ARTIFACT_BYTES
        {
            return None;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let forbidden_mode = match access {
                ArtifactAccess::OwnerOnly => 0o077,
                ArtifactAccess::BundledReadOnly => 0o022,
            };
            if metadata.mode() & forbidden_mode != 0 {
                return None;
            }
        }
        let bytes = fs::read(path).ok()?;
        let envelope: ArtifactEnvelope = serde_json::from_slice(&bytes).ok()?;
        let actual = format!("{:x}", Sha256::digest(envelope.model_json.as_bytes()));
        if !constant_time_eq(actual.as_bytes(), envelope.model_sha256.as_bytes()) {
            return None;
        }
        let artifact = serde_json::from_str(&envelope.model_json).ok()?;
        let epoch = format!("{PROMOTED_EPOCH_PREFIX}{}", &actual[..12]);
        let model = LinearModel::from_artifact(artifact, epoch)?;
        Some(Self(Some(Arc::new(model))))
    }
}

#[derive(Clone, Copy)]
enum ArtifactAccess {
    OwnerOnly,
    BundledReadOnly,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct ArtifactEnvelope {
    model_json: String,
    model_sha256: String,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct ArtifactModel {
    schema: String,
    classifier_epoch: String,
    feature_contract: String,
    max_input_chars: usize,
    threshold: f64,
    intercept: f64,
    features: Vec<ArtifactFeature>,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct ArtifactFeature {
    ngram: String,
    idf: f64,
    coefficient: f64,
}

struct LinearModel {
    classifier_epoch: String,
    max_input_chars: usize,
    threshold: f64,
    intercept: f64,
    feature_ids: HashMap<Vec<char>, usize>,
    feature_weights: Vec<(f64, f64)>,
}

impl LinearModel {
    fn from_artifact(artifact: ArtifactModel, classifier_epoch: String) -> Option<Self> {
        if artifact.schema != ARTIFACT_SCHEMA
            || artifact.classifier_epoch != CLASSIFIER_EPOCH
            || artifact.feature_contract != FEATURE_CONTRACT
            || artifact.max_input_chars == 0
            || artifact.max_input_chars > MAX_INPUT_CHARS_LIMIT
            || artifact.features.is_empty()
            || artifact.features.len() > MAX_FEATURES
            || !artifact.threshold.is_finite()
            || artifact.threshold <= 0.0
            || artifact.threshold > 1.0
            || !artifact.intercept.is_finite()
        {
            return None;
        }
        let mut features = BTreeMap::new();
        for feature in artifact.features {
            let char_count = feature.ngram.chars().count();
            if !(3..=5).contains(&char_count)
                || !feature.idf.is_finite()
                || feature.idf <= 0.0
                || !feature.coefficient.is_finite()
                || features
                    .insert(
                        feature.ngram.chars().collect::<Vec<_>>(),
                        (feature.idf, feature.coefficient),
                    )
                    .is_some()
            {
                return None;
            }
        }
        let mut feature_ids = HashMap::with_capacity(features.len());
        let mut feature_weights = Vec::with_capacity(features.len());
        for (feature_id, (ngram, weights)) in features.into_iter().enumerate() {
            feature_ids.insert(ngram, feature_id);
            feature_weights.push(weights);
        }
        Some(Self {
            classifier_epoch,
            max_input_chars: artifact.max_input_chars,
            threshold: artifact.threshold,
            intercept: artifact.intercept,
            feature_ids,
            feature_weights,
        })
    }

    fn predict(
        &self,
        normalized_text: &str,
        sections: &[PromotionSection],
        reasons: &[ReasonCode],
    ) -> bool {
        let feature_text =
            bounded_feature_text(normalized_text, sections, reasons, self.max_input_chars);
        let normalized = collapse_whitespace(&feature_text.to_lowercase());
        let chars = normalized.chars().collect::<Vec<_>>();
        let mut values = BTreeMap::<usize, f64>::new();
        for n in 3..=5 {
            if chars.len() < n {
                continue;
            }
            for window in chars.windows(n) {
                if let Some(feature_id) = self.feature_ids.get(window) {
                    *values.entry(*feature_id).or_default() += 1.0;
                }
            }
        }
        let mut norm_squared = 0.0;
        for (feature_id, count) in &mut values {
            let Some((idf, _)) = self.feature_weights.get(*feature_id) else {
                return false;
            };
            *count = (1.0 + count.ln()) * idf;
            norm_squared += *count * *count;
        }
        let norm = norm_squared.sqrt();
        let score = values
            .into_iter()
            .fold(self.intercept, |score, (feature_id, value)| {
                let coefficient = self
                    .feature_weights
                    .get(feature_id)
                    .map_or(0.0, |(_, coefficient)| *coefficient);
                score
                    + if norm > 0.0 {
                        value / norm * coefficient
                    } else {
                        0.0
                    }
            });
        logistic_probability(score) >= self.threshold
    }
}

fn bounded_feature_text(
    text: &str,
    sections: &[PromotionSection],
    reasons: &[ReasonCode],
    cap: usize,
) -> String {
    let mut tokens = Vec::new();
    for section in sections.iter().take(64) {
        tokens.push(format!("__section_{}__", section_token(*section)));
    }
    for reason in reasons
        .iter()
        .take(crate::MAX_REASON_CODES)
        .filter(|reason| is_safe_primary_reason(**reason))
    {
        tokens.push(format!("__reason_{}__", reason_token(*reason)));
    }
    let structure_cap = 1_024.min(cap / 4);
    let structure = take_chars(&tokens.join(" "), structure_cap);
    let body_cap = cap.saturating_sub(structure.chars().count() + 1).max(1);
    let body = bounded_head_tail(text, body_cap);
    let value = if structure.is_empty() {
        body
    } else {
        format!("{body}\n{structure}")
    };
    take_chars(&value, cap)
}

fn bounded_head_tail(text: &str, cap: usize) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() <= cap {
        return text.to_string();
    }
    let head = cap * 3 / 5;
    let tail = cap - head;
    chars[..head]
        .iter()
        .chain(chars[chars.len() - tail..].iter())
        .collect()
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn take_chars(text: &str, cap: usize) -> String {
    text.chars().take(cap).collect()
}

fn logistic_probability(score: f64) -> f64 {
    if score >= 0.0 {
        1.0 / (1.0 + (-score).exp())
    } else {
        let exp = score.exp();
        exp / (1.0 + exp)
    }
}

fn is_safe_primary_reason(reason: ReasonCode) -> bool {
    matches!(
        reason,
        ReasonCode::ProfileHeading
            | ReasonCode::ExperienceHeading
            | ReasonCode::EducationHeading
            | ReasonCode::SkillsHeading
            | ReasonCode::CareerHistoryDetail
            | ReasonCode::InvoiceHeading
            | ReasonCode::InvoiceTerms
            | ReasonCode::MeetingHeading
            | ReasonCode::MeetingWorkflow
            | ReasonCode::ManualHeading
            | ReasonCode::ManualInstructions
    )
}

fn section_token(section: PromotionSection) -> &'static str {
    match section {
        PromotionSection::Profile => "profile",
        PromotionSection::Contact => "contact",
        PromotionSection::Education => "education",
        PromotionSection::Experience => "experience",
        PromotionSection::Project => "project",
        PromotionSection::Skill => "skill",
        PromotionSection::Certificate => "certificate",
        PromotionSection::OtherChunk => "other_chunk",
    }
}

fn reason_token(reason: ReasonCode) -> &'static str {
    match reason {
        ReasonCode::ProfileHeading => "profileheading",
        ReasonCode::ExperienceHeading => "experienceheading",
        ReasonCode::EducationHeading => "educationheading",
        ReasonCode::SkillsHeading => "skillsheading",
        ReasonCode::CareerHistoryDetail => "careerhistorydetail",
        ReasonCode::InvoiceHeading => "invoiceheading",
        ReasonCode::InvoiceTerms => "invoiceterms",
        ReasonCode::MeetingHeading => "meetingheading",
        ReasonCode::MeetingWorkflow => "meetingworkflow",
        ReasonCode::ManualHeading => "manualheading",
        ReasonCode::ManualInstructions => "manualinstructions",
        ReasonCode::CorroboratedResumeSignals => "corroborated_resume_signals",
        ReasonCode::CorroboratedNonResumeSignals => "corroborated_non_resume_signals",
        ReasonCode::ConflictingSignalFamilies => "conflicting_signal_families",
        ReasonCode::InsufficientSignalFamilies => "insufficient_signal_families",
        ReasonCode::EmptyNormalizedText => "empty_normalized_text",
        ReasonCode::OcrRequired => "ocr_required",
        ReasonCode::ParserFailed => "parser_failed",
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
#[path = "linear_promotion_tests.rs"]
mod tests;
