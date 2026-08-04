use super::{manifest_model_id, COREML_MODEL_ID, PACK_ID};

#[test]
fn reviewed_pack_identity_remains_the_default() {
    assert_eq!(manifest_model_id(PACK_ID), Some(PACK_ID));
}

#[test]
fn coreml_default_owns_a_distinct_tokenizer_pack_identity() {
    assert_eq!(manifest_model_id(COREML_MODEL_ID), Some(COREML_MODEL_ID));
    assert_eq!(manifest_model_id("unreviewed-model"), None);
}
