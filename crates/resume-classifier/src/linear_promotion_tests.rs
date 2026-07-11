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

#[test]
fn valid_synthetic_artifact_promotes_only_safe_gray() {
    let artifact = write_artifact(synthetic_model());
    let policy = LinearPromotionPolicy::load_local(artifact.path());
    let text = "PROFILE\nPlatform engineer with Rust and distributed systems experience.\nINVOICE";
    let gray = classify(ClassifierInput::NormalizedText(text));
    let promoted = policy.apply(text, &[PromotionSection::Profile], gray);
    assert_eq!(promoted.status(), ClassificationStatus::ResumeCandidate);
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
