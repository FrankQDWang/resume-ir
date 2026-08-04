use super::{manifest_model_id, COREML_MODEL_ID, PACK_ID};

#[test]
fn reviewed_pack_identity_remains_the_default() {
    assert_eq!(manifest_model_id(PACK_ID, false), Some(PACK_ID));
    assert_eq!(manifest_model_id(PACK_ID, true), Some(PACK_ID));
}

#[test]
fn coreml_default_uses_reviewed_tokenizer_pack_only_with_its_runtime() {
    assert_eq!(manifest_model_id(COREML_MODEL_ID, false), None);
    assert_eq!(manifest_model_id(COREML_MODEL_ID, true), Some(PACK_ID));
    assert_eq!(manifest_model_id("unreviewed-model", true), None);
}
