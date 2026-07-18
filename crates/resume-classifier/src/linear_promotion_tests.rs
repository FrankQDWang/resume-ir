use std::fs;

use sha2::{Digest, Sha256};

use super::*;
use crate::{classify, ClassificationStatus, ClassifierInput};

fn write_artifact(model: ArtifactModel) -> tempfile::NamedTempFile {
    let model_json = serde_json::to_string(&model).unwrap();
    let digest = format!("{:x}", Sha256::digest(model_json.as_bytes()));
    let envelope = ArtifactEnvelope {
        model_json,
        model_sha256: digest,
    };
    let file = tempfile::NamedTempFile::new().unwrap();
    fs::write(file.path(), serde_json::to_vec(&envelope).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(file.path(), fs::Permissions::from_mode(0o600)).unwrap();
    }
    file
}

fn synthetic_model() -> ArtifactModel {
    ArtifactModel {
        schema: ARTIFACT_SCHEMA.to_string(),
        classifier_epoch: CLASSIFIER_EPOCH.to_string(),
        feature_contract: FEATURE_CONTRACT.to_string(),
        max_input_chars: 128,
        threshold: 0.7,
        intercept: 0.0,
        features: vec![ArtifactFeature {
            ngram: "pla".to_string(),
            idf: 1.0,
            coefficient: 1.0,
        }],
    }
}

fn equivalence_model() -> ArtifactModel {
    ArtifactModel {
        schema: ARTIFACT_SCHEMA.to_string(),
        classifier_epoch: CLASSIFIER_EPOCH.to_string(),
        feature_contract: FEATURE_CONTRACT.to_string(),
        max_input_chars: 96,
        threshold: 0.51,
        intercept: -0.35,
        features: vec![
            ArtifactFeature {
                ngram: "pla".to_string(),
                idf: 1.2,
                coefficient: 0.8,
            },
            ArtifactFeature {
                ngram: "工程师".to_string(),
                idf: 1.7,
                coefficient: 1.1,
            },
            ArtifactFeature {
                ngram: "rust".to_string(),
                idf: 1.4,
                coefficient: 0.6,
            },
            ArtifactFeature {
                ngram: "__sec".to_string(),
                idf: 0.9,
                coefficient: 0.4,
            },
            ArtifactFeature {
                ngram: "发票与".to_string(),
                idf: 1.3,
                coefficient: -1.4,
            },
        ],
    }
}

fn reference_predict(
    artifact: &ArtifactModel,
    normalized_text: &str,
    sections: &[PromotionSection],
    reasons: &[ReasonCode],
) -> bool {
    let feature_text =
        bounded_feature_text(normalized_text, sections, reasons, artifact.max_input_chars);
    let normalized = collapse_whitespace(&feature_text.to_lowercase());
    let chars = normalized.chars().collect::<Vec<_>>();
    let features = artifact
        .features
        .iter()
        .map(|feature| (feature.ngram.as_str(), (feature.idf, feature.coefficient)))
        .collect::<BTreeMap<_, _>>();
    let mut values = BTreeMap::<String, f64>::new();
    for n in 3..=5 {
        for window in chars.windows(n) {
            let ngram = window.iter().collect::<String>();
            if features.contains_key(ngram.as_str()) {
                *values.entry(ngram).or_default() += 1.0;
            }
        }
    }
    let mut norm_squared = 0.0;
    for (ngram, count) in &mut values {
        let (idf, _) = features[ngram.as_str()];
        *count = (1.0 + count.ln()) * idf;
        norm_squared += *count * *count;
    }
    let norm = norm_squared.sqrt();
    let score = values
        .into_iter()
        .fold(artifact.intercept, |score, (ngram, value)| {
            let coefficient = features[ngram.as_str()].1;
            score
                + if norm > 0.0 {
                    value / norm * coefficient
                } else {
                    0.0
                }
        });
    logistic_probability(score) >= artifact.threshold
}

#[test]
fn valid_synthetic_artifact_promotes_only_safe_gray() {
    let artifact = write_artifact(synthetic_model());
    let policy = LinearPromotionPolicy::load_local(artifact.path());
    let model_epoch = policy.classifier_epoch().unwrap().to_string();
    let text = "PROFILE\nPlatform engineer with Rust and distributed systems experience.\nINVOICE";
    let gray = classify(ClassifierInput::NormalizedText(text));
    let promoted = policy.apply(text, &[PromotionSection::Profile], gray);
    assert_eq!(promoted.status(), ClassificationStatus::ResumeCandidate);
    assert_eq!(promoted.classifier_epoch(), model_epoch);
}

#[test]
fn enabled_model_stamps_unchanged_non_gray_result_with_model_epoch() {
    let artifact = write_artifact(synthetic_model());
    let policy = LinearPromotionPolicy::load_local(artifact.path());
    let model_epoch = policy.classifier_epoch().unwrap().to_string();
    let text = "PROFILE\nEngineer.\nEXPERIENCE\nBuilt tools.";
    let baseline = classify(ClassifierInput::NormalizedText(text));
    assert_eq!(baseline.status(), ClassificationStatus::ResumeCandidate);

    let classified = policy.apply(text, &[PromotionSection::Profile], baseline);

    assert_eq!(classified.status(), ClassificationStatus::ResumeCandidate);
    assert_eq!(classified.classifier_epoch(), model_epoch);
}

#[test]
fn enabled_model_stamps_gray_result_even_when_not_promoted() {
    let artifact = write_artifact(synthetic_model());
    let policy = LinearPromotionPolicy::load_local(artifact.path());
    let model_epoch = policy.classifier_epoch().unwrap().to_string();
    let text = "PROFILE\nEngineer.";
    let baseline = classify(ClassifierInput::NormalizedText(text));
    assert_eq!(baseline.status(), ClassificationStatus::NeedsReview);

    let classified = policy.apply(text, &[PromotionSection::Profile], baseline);

    assert_eq!(classified.status(), ClassificationStatus::NeedsReview);
    assert_eq!(classified.classifier_epoch(), model_epoch);
}

#[test]
fn disabled_policy_retains_deterministic_classifier_epoch() {
    let policy = LinearPromotionPolicy::default();
    let text = "PROFILE\nEngineer.";
    let baseline = classify(ClassifierInput::NormalizedText(text));

    let classified = policy.apply(text, &[PromotionSection::Profile], baseline);

    assert_eq!(classified.status(), ClassificationStatus::NeedsReview);
    assert_eq!(classified.classifier_epoch(), CLASSIFIER_EPOCH);
}

#[test]
fn deterministic_hard_vetoes_cannot_be_promoted() {
    let artifact = write_artifact(synthetic_model());
    let policy = LinearPromotionPolicy::load_local(artifact.path());
    for text in [
        "PROFILE\nPlatform engineer.\nINVOICE\nSubtotal and payment terms.",
        "SUMMARY\nPlatform engineer.\nEXPERIENCE\nBuilt tools.\nINVOICE\nSubtotal and payment terms.",
        "DEVICE MANUAL\nPlatform startup, safety, and maintenance instructions.",
    ] {
        let baseline = classify(ClassifierInput::NormalizedText(text));
        let expected = baseline.status();
        assert_eq!(
            policy.apply(text, &[PromotionSection::Profile], baseline).status(),
            expected
        );
    }
}

#[test]
fn inference_is_bounded_and_repeatable() {
    let policy = LinearPromotionPolicy::load_local(write_artifact(synthetic_model()).path());
    let text = format!("{}pla{}", "x".repeat(100_000), "y".repeat(100_000));
    let baseline = classify(ClassifierInput::NormalizedText("PROFILE\nEngineer."));
    let first = policy.apply(&text, &[PromotionSection::Profile], baseline.clone());
    let second = policy.apply(&text, &[PromotionSection::Profile], baseline);
    assert_eq!(first, second);
}

#[test]
fn borrowed_ngram_lookup_matches_string_allocating_reference() {
    let artifact = equivalence_model();
    let model = LinearModel::from_artifact(equivalence_model(), "test-epoch".to_string()).unwrap();
    let cases = [
        (
            "Platform engineer using Rust",
            vec![PromotionSection::Profile],
        ),
        ("平台工程师 Rust Rust", vec![PromotionSection::Experience]),
        ("发票与付款说明", vec![PromotionSection::OtherChunk]),
        ("mixed 工程师 and platform", vec![PromotionSection::Skill]),
        ("x", vec![]),
        (
            &"Pla 工程师 Rust 发票 ".repeat(20),
            vec![PromotionSection::Profile],
        ),
    ];
    let reasons = [ReasonCode::ProfileHeading, ReasonCode::InvoiceHeading];
    for (text, sections) in cases {
        assert_eq!(
            model.predict(text, &sections, &reasons),
            reference_predict(&artifact, text, &sections, &reasons),
            "reference drift for synthetic case"
        );
    }
}

#[test]
fn missing_corrupt_incompatible_checksum_and_permissions_fail_closed() {
    let corrupt = tempfile::NamedTempFile::new().unwrap();
    fs::write(corrupt.path(), b"not-json").unwrap();
    assert!(!LinearPromotionPolicy::load_local(corrupt.path()).enabled());

    let mut incompatible = synthetic_model();
    incompatible.feature_contract = "other".to_string();
    assert!(!LinearPromotionPolicy::load_local(write_artifact(incompatible).path()).enabled());

    let drifted = write_artifact(synthetic_model());
    let mut bytes = fs::read(drifted.path()).unwrap();
    let position = bytes.iter().position(|byte| *byte == b'p').unwrap();
    bytes[position] = b'q';
    fs::write(drifted.path(), bytes).unwrap();
    assert!(!LinearPromotionPolicy::load_local(drifted.path()).enabled());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let exposed = write_artifact(synthetic_model());
        fs::set_permissions(exposed.path(), fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!LinearPromotionPolicy::load_local(exposed.path()).enabled());
    }
}

#[cfg(unix)]
#[test]
fn bundled_artifact_allows_readability_but_rejects_writes_and_symlinks() {
    use std::os::unix::fs::PermissionsExt;

    let bundled = write_artifact(synthetic_model());
    fs::set_permissions(bundled.path(), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(LinearPromotionPolicy::load_bundled(bundled.path()).enabled());
    assert!(!LinearPromotionPolicy::load_local(bundled.path()).enabled());

    fs::set_permissions(bundled.path(), fs::Permissions::from_mode(0o664)).unwrap();
    assert!(!LinearPromotionPolicy::load_bundled(bundled.path()).enabled());

    fs::set_permissions(bundled.path(), fs::Permissions::from_mode(0o644)).unwrap();
    let root = tempfile::tempdir().unwrap();
    let link = root.path().join("model.json");
    std::os::unix::fs::symlink(bundled.path(), &link).unwrap();
    assert!(!LinearPromotionPolicy::load_bundled(&link).enabled());
}
