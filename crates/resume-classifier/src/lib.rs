use std::fmt;

/// Stable epoch for the deterministic precision-first ruleset.
pub const CLASSIFIER_EPOCH: &str = "precision_first_v4";

/// Hard cap for reason codes returned for one document.
pub const MAX_REASON_CODES: usize = 8;

const MIN_RESUME_HEADING_FAMILIES: u8 = 2;

/// Parser and normalized-text states accepted by the classifier core.
///
/// Paths, filenames, extensions, source roles, benchmark labels, and sample
/// identities are intentionally absent so they cannot influence admission.
#[derive(Clone, Copy)]
pub enum ClassifierInput<'a> {
    NormalizedText(&'a str),
    OcrBacklog,
    Failed,
}

impl fmt::Debug for ClassifierInput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NormalizedText(_) => formatter.write_str("NormalizedText(<redacted>)"),
            Self::OcrBacklog => formatter.write_str("OcrBacklog"),
            Self::Failed => formatter.write_str("Failed"),
        }
    }
}

/// Fixed mixed-import classification states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClassificationStatus {
    ResumeCandidate,
    NonResume,
    NeedsReview,
    OcrBacklog,
    Failed,
}

impl ClassificationStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ResumeCandidate => "resume_candidate",
            Self::NonResume => "non_resume",
            Self::NeedsReview => "needs_review",
            Self::OcrBacklog => "ocr_backlog",
            Self::Failed => "failed",
        }
    }
}

/// Bounded, non-content-bearing evidence codes emitted by the classifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReasonCode {
    ProfileHeading,
    ExperienceHeading,
    EducationHeading,
    SkillsHeading,
    CareerHistoryDetail,
    InvoiceHeading,
    InvoiceTerms,
    MeetingHeading,
    MeetingWorkflow,
    ManualHeading,
    ManualInstructions,
    CorroboratedResumeSignals,
    CorroboratedNonResumeSignals,
    ConflictingSignalFamilies,
    InsufficientSignalFamilies,
    EmptyNormalizedText,
    OcrRequired,
    ParserFailed,
}

/// Privacy-safe deterministic classifier output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassificationResult {
    status: ClassificationStatus,
    reason_codes: Vec<ReasonCode>,
    positive_signal_components: u8,
    negative_signal_components: u8,
    classifier_epoch: &'static str,
}

impl ClassificationResult {
    pub fn status(&self) -> ClassificationStatus {
        self.status
    }

    pub fn reason_codes(&self) -> &[ReasonCode] {
        &self.reason_codes
    }

    pub fn positive_signal_components(&self) -> u8 {
        self.positive_signal_components
    }

    pub fn negative_signal_components(&self) -> u8 {
        self.negative_signal_components
    }

    pub fn classifier_epoch(&self) -> &'static str {
        self.classifier_epoch
    }
}

/// Classifies normalized text using deterministic, precision-first rules.
pub fn classify(input: ClassifierInput<'_>) -> ClassificationResult {
    match input {
        ClassifierInput::NormalizedText(text) => classify_normalized_text(text),
        ClassifierInput::OcrBacklog => result(
            ClassificationStatus::OcrBacklog,
            vec![ReasonCode::OcrRequired],
            0,
            0,
        ),
        ClassifierInput::Failed => result(
            ClassificationStatus::Failed,
            vec![ReasonCode::ParserFailed],
            0,
            0,
        ),
    }
}

fn classify_normalized_text(text: &str) -> ClassificationResult {
    if text.trim().is_empty() {
        return result(
            ClassificationStatus::Failed,
            vec![ReasonCode::EmptyNormalizedText],
            0,
            0,
        );
    }

    let headings = normalized_headings(text);
    let normalized_text = text.to_lowercase();

    let experience_heading = has_heading(&headings, EXPERIENCE_HEADINGS);
    let career_history_detail = experience_section_has_history_detail(text);
    let positive = [
        (
            has_heading(&headings, PROFILE_HEADINGS),
            ReasonCode::ProfileHeading,
        ),
        (experience_heading, ReasonCode::ExperienceHeading),
        (
            has_heading(&headings, EDUCATION_HEADINGS),
            ReasonCode::EducationHeading,
        ),
        (
            has_heading(&headings, SKILLS_HEADINGS),
            ReasonCode::SkillsHeading,
        ),
        (career_history_detail, ReasonCode::CareerHistoryDetail),
    ];

    let invoice_heading = has_heading(&headings, INVOICE_HEADINGS);
    let invoice_terms = contains_at_least(
        &normalized_text,
        &[
            "subtotal",
            "payment terms",
            "amount due",
            "invoice number",
            "税额",
            "应付金额",
        ],
        2,
    );
    let meeting_heading = has_heading(&headings, MEETING_HEADINGS);
    let meeting_workflow = contains_at_least(
        &normalized_text,
        &[
            "agenda",
            "action items",
            "meeting minutes",
            "decision to",
            "议程",
            "行动项",
        ],
        2,
    );
    let manual_heading = has_heading(&headings, MANUAL_HEADINGS);
    let manual_instructions = contains_at_least(
        &normalized_text,
        &[
            "maintenance",
            "safety",
            "startup",
            "connect the cable",
            "installation steps",
            "troubleshooting",
            "维护",
            "安全指南",
        ],
        2,
    );
    let negative = [
        (invoice_heading, ReasonCode::InvoiceHeading),
        (invoice_terms, ReasonCode::InvoiceTerms),
        (meeting_heading, ReasonCode::MeetingHeading),
        (meeting_workflow, ReasonCode::MeetingWorkflow),
        (manual_heading, ReasonCode::ManualHeading),
        (manual_instructions, ReasonCode::ManualInstructions),
    ];

    let positive_count = count_matches(&positive);
    let positive_heading_count = positive[..4].iter().filter(|(matched, _)| *matched).count() as u8;
    let negative_count = count_matches(&negative);
    let has_corroborated_resume = positive_heading_count >= MIN_RESUME_HEADING_FAMILIES
        && experience_heading
        && career_history_detail;
    let has_corroborated_negative = (invoice_heading && invoice_terms)
        || (meeting_heading && meeting_workflow)
        || (manual_heading && manual_instructions);

    if has_corroborated_resume && has_corroborated_negative {
        let reasons = collect_reasons(
            ReasonCode::ConflictingSignalFamilies,
            positive.into_iter().chain(negative),
        );
        return result(
            ClassificationStatus::NeedsReview,
            reasons,
            positive_count,
            negative_count,
        );
    }

    if has_corroborated_resume {
        let reasons = collect_reasons(ReasonCode::CorroboratedResumeSignals, positive);
        return result(
            ClassificationStatus::ResumeCandidate,
            reasons,
            positive_count,
            negative_count,
        );
    }

    if has_corroborated_negative {
        let reasons = collect_reasons(ReasonCode::CorroboratedNonResumeSignals, negative);
        return result(
            ClassificationStatus::NonResume,
            reasons,
            positive_count,
            negative_count,
        );
    }

    let reasons = collect_reasons(
        ReasonCode::InsufficientSignalFamilies,
        positive.into_iter().chain(negative),
    );
    result(
        ClassificationStatus::NeedsReview,
        reasons,
        positive_count,
        negative_count,
    )
}

fn result(
    status: ClassificationStatus,
    mut reason_codes: Vec<ReasonCode>,
    positive_signal_components: u8,
    negative_signal_components: u8,
) -> ClassificationResult {
    reason_codes.truncate(MAX_REASON_CODES);
    ClassificationResult {
        status,
        reason_codes,
        positive_signal_components,
        negative_signal_components,
        classifier_epoch: CLASSIFIER_EPOCH,
    }
}

fn collect_reasons(
    primary: ReasonCode,
    signals: impl IntoIterator<Item = (bool, ReasonCode)>,
) -> Vec<ReasonCode> {
    std::iter::once(primary)
        .chain(
            signals
                .into_iter()
                .filter_map(|(matched, reason)| matched.then_some(reason)),
        )
        .take(MAX_REASON_CODES)
        .collect()
}

fn count_matches<const N: usize>(signals: &[(bool, ReasonCode); N]) -> u8 {
    signals.iter().filter(|(matched, _)| *matched).count() as u8
}

fn normalized_headings(text: &str) -> Vec<String> {
    text.lines()
        .map(normalized_heading)
        .filter(|line| !line.is_empty())
        .collect()
}

fn normalized_heading(line: &str) -> String {
    line.trim()
        .trim_end_matches([':', '：'])
        .trim()
        .to_lowercase()
}

fn has_heading(headings: &[String], accepted: &[&str]) -> bool {
    headings
        .iter()
        .any(|heading| accepted.contains(&heading.as_str()))
}

fn contains_at_least(text: &str, phrases: &[&str], minimum: usize) -> bool {
    phrases
        .iter()
        .filter(|phrase| text.contains(**phrase))
        .count()
        >= minimum
}

fn experience_section_has_history_detail(text: &str) -> bool {
    let mut in_experience = false;
    for line in text.lines() {
        let heading = normalized_heading(line);
        if EXPERIENCE_HEADINGS.contains(&heading.as_str()) {
            in_experience = true;
            continue;
        }
        if in_experience && is_section_boundary(line, &heading) {
            break;
        }
        if !in_experience {
            continue;
        }
        if starts_with_history_action(line) {
            return true;
        }
    }
    false
}

fn starts_with_history_action(line: &str) -> bool {
    let normalized = line
        .trim_start_matches(|character: char| {
            character.is_whitespace() || matches!(character, '-' | '*' | '•' | '·')
        })
        .to_lowercase();
    let first_token = normalized
        .split(|character: char| !character.is_alphanumeric())
        .next()
        .unwrap_or_default();
    matches!(
        first_token,
        "achieved"
            | "analyzed"
            | "architected"
            | "automated"
            | "built"
            | "collaborated"
            | "conducted"
            | "coordinated"
            | "created"
            | "delivered"
            | "deployed"
            | "designed"
            | "developed"
            | "engineered"
            | "established"
            | "implemented"
            | "improved"
            | "increased"
            | "launched"
            | "led"
            | "maintained"
            | "managed"
            | "operated"
            | "optimized"
            | "owned"
            | "reduced"
            | "resolved"
            | "streamlined"
            | "supported"
    ) || [
        "主导", "参与", "协助", "完成", "搭建", "推动", "构建", "管理", "设计", "负责", "开发",
        "实现", "优化", "维护", "制定", "提升", "分析", "招聘", "建立", "编写", "测试", "运营",
        "组织", "销售", "监控", "执行", "担任", "获得", "降低", "协调", "培训",
    ]
    .iter()
    .any(|prefix| normalized.starts_with(prefix))
}

fn is_section_boundary(line: &str, heading: &str) -> bool {
    [PROFILE_HEADINGS, EDUCATION_HEADINGS, SKILLS_HEADINGS]
        .iter()
        .any(|accepted| accepted.contains(&heading))
        || (heading.chars().count() <= 40
            && line
                .chars()
                .any(|character| character.is_ascii_alphabetic())
            && !line.chars().any(|character| character.is_ascii_lowercase()))
}

const PROFILE_HEADINGS: &[&str] = &["profile", "summary", "professional summary", "个人简介"];
const EXPERIENCE_HEADINGS: &[&str] = &[
    "experience",
    "work experience",
    "professional experience",
    "工作经历",
];
const EDUCATION_HEADINGS: &[&str] = &["education", "education background", "教育经历", "教育背景"];
const SKILLS_HEADINGS: &[&str] = &["skills", "technical skills", "技能", "专业技能"];
const INVOICE_HEADINGS: &[&str] = &["invoice", "tax invoice", "发票", "账单"];
const MEETING_HEADINGS: &[&str] = &["meeting notes", "meeting minutes", "会议纪要", "会议记录"];
const MANUAL_HEADINGS: &[&str] = &["device manual", "user manual", "设备手册"];

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use serde_json::Value;

    use super::*;

    #[test]
    fn frozen_public_fixture_matches_expected_states_and_metrics() {
        let fixture = public_fixture();
        let samples = fixture["samples"].as_array().expect("samples array");
        let mut counts = [0_usize; 5];
        let mut true_resumes = 0_usize;
        let mut indexed = 0_usize;
        let mut indexed_true_resumes = 0_usize;

        for sample in samples {
            let outcome = sample["parser_outcome"].as_str().expect("parser outcome");
            let content = sample["content"].as_str().expect("content");
            let expected = sample["expected_status"].as_str().expect("expected status");
            let input = match outcome {
                "text_extracted" => ClassifierInput::NormalizedText(content),
                "ocr_required" => ClassifierInput::OcrBacklog,
                "failed" => ClassifierInput::Failed,
                other => panic!("unsupported parser outcome: {other}"),
            };
            let classification = classify(input);

            assert_eq!(classification.status().as_str(), expected);
            assert!(classification.reason_codes().len() <= MAX_REASON_CODES);
            if classification.status() == ClassificationStatus::ResumeCandidate {
                assert!(classification.positive_signal_components() > MIN_RESUME_HEADING_FAMILIES);
                indexed += 1;
            }
            if sample["ground_truth"] == "resume" {
                true_resumes += 1;
                if classification.status() == ClassificationStatus::ResumeCandidate {
                    indexed_true_resumes += 1;
                }
            }

            counts[status_index(classification.status())] += 1;
        }

        assert_eq!(counts, [3, 3, 1, 1, 1]);
        assert_eq!(indexed, 3);
        assert_eq!(indexed_true_resumes, 3);
        assert_eq!(true_resumes, 4);
        assert_eq!(indexed_true_resumes as f64 / indexed as f64, 1.0);
        assert_eq!(indexed_true_resumes as f64 / true_resumes as f64, 0.75);
    }

    #[test]
    fn weak_ambiguous_and_metadata_free_inputs_fail_closed_to_review() {
        for text in [
            "SKILLS\nRust and SQL",
            "Email test@example.invalid and phone 555-0100",
            "Project coordination and analysis.",
            "INVOICE\nA synthetic heading without commercial terms.",
            "Subtotal and payment terms without an invoice heading.",
            "SUMMARY\nWe are hiring.\nSKILLS\nRust.\nEXPERIENCE\nThree years required.",
            "SUMMARY\nWrite a summary.\nEXPERIENCE\nList roles.\nSKILLS\nAdd keywords.",
            "SUMMARY\nWe seek engineers who have built systems.\nEXPERIENCE\nFive years required.",
            "SUMMARY\nTemplate.\nEXPERIENCE\nDescribe projects you developed.\nSKILLS\nAdd keywords.",
            "SUMMARY\nExample profile.\nEXPERIENCE\nSample entry: Built systems.",
            "SUMMARY\nWe are hiring.\nEXPERIENCE\nCandidates who managed teams are preferred.",
            "SUMMARY\nTemplate.\nEXPERIENCE\nExample entry: Led delivery programs.",
            "个人简介\n招聘说明\n工作经历\n候选人需要具备提升效率经验。",
            "个人简介\n模板\n工作经历\n示例：组织跨团队协作。",
            "招聘说明\n工作经历\n执行销售计划并培训团队。",
            "个人简介\n模板\n工作经历\n担任示例角色。\n发票\n税额与应付金额。",
            "An experienced and skillful educational writer.",
        ] {
            let result = classify(ClassifierInput::NormalizedText(text));
            assert_eq!(result.status(), ClassificationStatus::NeedsReview);
        }
    }

    #[test]
    fn generalized_career_action_prefixes_require_corroborated_resume_structure() {
        for line in [
            "提升交付效率。",
            "分析业务数据。",
            "招聘并培养团队。",
            "建立质量体系。",
            "编写自动化工具。",
            "测试核心服务。",
            "运营本地平台。",
            "组织跨团队协作。",
            "销售企业软件。",
            "监控服务质量。",
            "执行交付计划。",
            "担任项目负责人。",
            "获得年度表彰。",
            "降低运营成本。",
            "协调跨部门资源。",
            "培训新成员。",
        ] {
            assert!(starts_with_history_action(line));
            let text = format!("工作经历\n{line}\n教育背景\n示例大学");
            assert_eq!(
                classify(ClassifierInput::NormalizedText(&text)).status(),
                ClassificationStatus::ResumeCandidate
            );
        }
        for text in [
            "SUMMARY\nPlatform engineer\nWORK EXPERIENCE\nLed distributed search delivery.\nEDUCATION\nSynthetic University",
            "个人简介\n平台工程师\n工作经历\n主导分布式检索交付。\n教育背景\n示例大学",
        ] {
            let result = classify(ClassifierInput::NormalizedText(text));
            assert_eq!(result.status(), ClassificationStatus::ResumeCandidate);
            assert!(result.reason_codes().contains(&ReasonCode::CareerHistoryDetail));
        }
    }

    #[test]
    fn resume_threshold_and_heading_boundaries_are_deterministic() {
        for text in [
            "PROFILE\nPlatform engineer.",
            "EXPERIENCE\nBuilt tools.",
            "EDUCATION\nTechnical degree.",
            "SKILLS\nRust and SQL.",
            "EXPERIENCE\nBuilt tools.\nWORK EXPERIENCE\nBuilt more tools.",
        ] {
            assert_eq!(
                classify(ClassifierInput::NormalizedText(text)).status(),
                ClassificationStatus::NeedsReview
            );
        }

        for text in [
            "PROFILE\nEngineer.\nEXPERIENCE\nBuilt tools.",
            "EDUCATION\nDegree.\nEXPERIENCE\nDeveloped tools.",
            "SKILLS\nRust.\nEXPERIENCE\nImplemented services.",
            "  professional summary:  \r\nEngineer.\r\n experience：\r\nBuilt tools.",
            "个人简介\n平台工程师。\n工作经历\n负责检索系统。",
        ] {
            assert_eq!(
                classify(ClassifierInput::NormalizedText(text)).status(),
                ClassificationStatus::ResumeCandidate
            );
        }
    }

    #[test]
    fn conflicting_strong_signal_families_fail_closed_to_review() {
        let result = classify(ClassifierInput::NormalizedText(
            "SUMMARY\nEngineer.\nEXPERIENCE\nBuilt tools.\nINVOICE\nSubtotal. Payment terms.",
        ));

        assert_eq!(result.status(), ClassificationStatus::NeedsReview);
        assert_eq!(
            result.reason_codes().first(),
            Some(&ReasonCode::ConflictingSignalFamilies)
        );
        assert!(result.positive_signal_components() >= 2);
        assert!(result.negative_signal_components() >= 2);
    }

    #[test]
    fn parser_terminal_states_are_fixed() {
        assert_eq!(
            classify(ClassifierInput::OcrBacklog).status(),
            ClassificationStatus::OcrBacklog
        );
        assert_eq!(
            classify(ClassifierInput::Failed).status(),
            ClassificationStatus::Failed
        );
        for text in ["", " \n\t "] {
            assert_eq!(
                classify(ClassifierInput::NormalizedText(text)).status(),
                ClassificationStatus::Failed
            );
        }
    }

    #[test]
    fn debug_output_never_contains_normalized_text() {
        let sentinel = "private-content-sentinel";
        let input = ClassifierInput::NormalizedText(sentinel);
        let result = classify(input);

        assert!(!format!("{input:?}").contains(sentinel));
        assert!(!format!("{result:?}").contains(sentinel));
        assert_eq!(result.classifier_epoch(), CLASSIFIER_EPOCH);
    }

    #[test]
    fn reason_codes_are_bounded_and_repeatable() {
        let text = "SUMMARY\nEngineer.\nEXPERIENCE\nBuilt tools.\nEDUCATION\nDegree.\nSKILLS\nRust.\nINVOICE\nSubtotal and payment terms.\nMEETING NOTES\nAgenda and action items.\nDEVICE MANUAL\nSafety and maintenance before startup.";
        let first = classify(ClassifierInput::NormalizedText(text));
        let second = classify(ClassifierInput::NormalizedText(text));

        assert_eq!(first, second);
        assert_eq!(first.status(), ClassificationStatus::NeedsReview);
        assert_eq!(first.reason_codes().len(), MAX_REASON_CODES);
    }

    fn public_fixture() -> Value {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../perf/fixtures/mixed-import/public-synthetic-benchmark.json");
        serde_json::from_str(&fs::read_to_string(path).expect("read public fixture"))
            .expect("parse public fixture")
    }

    const fn status_index(status: ClassificationStatus) -> usize {
        match status {
            ClassificationStatus::ResumeCandidate => 0,
            ClassificationStatus::NonResume => 1,
            ClassificationStatus::NeedsReview => 2,
            ClassificationStatus::OcrBacklog => 3,
            ClassificationStatus::Failed => 4,
        }
    }
}
