use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use resume_classifier::{LinearPromotionPolicy, CLASSIFIER_EPOCH, PROMOTED_EPOCH_PREFIX};

use super::AdmissionDecision;

const MODEL_JSON: &str = r#"{"schema":"resume_ir_linear_promotion_v1","classifier_epoch":"precision_first_v4","feature_contract":"bounded_normalized_text_plus_structure_v1","max_input_chars":128,"threshold":0.7,"intercept":0.0,"features":[{"ngram":"pla","idf":1.0,"coefficient":1.0}]}"#;
const MODEL_SHA256: &str = "b0196c68ad7a8a9212d6421cb8d5e435c87ffa80c62b0cdd1d2a92b9c0e50d4e";
static NEXT_ARTIFACT_ID: AtomicU64 = AtomicU64::new(0);

struct LocalArtifact(PathBuf);

impl LocalArtifact {
    fn synthetic() -> Self {
        let path = std::env::temp_dir().join(format!(
            "resume-ir-classification-{}-{}.json",
            std::process::id(),
            NEXT_ARTIFACT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let envelope = serde_json::json!({
            "model_json": MODEL_JSON,
            "model_sha256": MODEL_SHA256,
        });
        fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for LocalArtifact {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
fn terminal_triage_states_use_enabled_model_epoch() {
    let artifact = LocalArtifact::synthetic();
    let policy = LinearPromotionPolicy::load_local(artifact.path());
    let epoch = policy.classifier_epoch().unwrap();
    let later_version = AdmissionDecision::after_sectionization("PROFILE\nEngineer.", &[], &policy);

    assert!(epoch.starts_with(PROMOTED_EPOCH_PREFIX));
    assert_eq!(
        AdmissionDecision::ocr_backlog(&policy).0.classifier_epoch(),
        epoch
    );
    assert_eq!(
        AdmissionDecision::failed(&policy).0.classifier_epoch(),
        epoch
    );
    assert_eq!(later_version.0.classifier_epoch(), epoch);
}

#[test]
fn terminal_triage_states_retain_deterministic_epoch_when_policy_is_disabled() {
    let policy = LinearPromotionPolicy::default();
    let later_version = AdmissionDecision::after_sectionization("PROFILE\nEngineer.", &[], &policy);

    assert_eq!(
        AdmissionDecision::ocr_backlog(&policy).0.classifier_epoch(),
        CLASSIFIER_EPOCH
    );
    assert_eq!(
        AdmissionDecision::failed(&policy).0.classifier_epoch(),
        CLASSIFIER_EPOCH
    );
    assert_eq!(later_version.0.classifier_epoch(), CLASSIFIER_EPOCH);
}
