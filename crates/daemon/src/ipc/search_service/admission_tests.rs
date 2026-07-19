use std::sync::Arc;

use super::*;

#[test]
fn admission_is_bounded_per_class_and_released_after_all_permit_owners_drop() {
    let admission = Arc::new(AdmissionState::new());
    let first = admission.acquire(ClientClass::CodexValidation).unwrap();
    let first_clone = first.clone();
    let second = admission.acquire(ClientClass::CodexValidation).unwrap();
    assert!(admission.acquire(ClientClass::CodexValidation).is_none());

    drop(first);
    assert!(admission.acquire(ClientClass::CodexValidation).is_none());
    drop(first_clone);
    assert!(admission.acquire(ClientClass::CodexValidation).is_some());
    drop(second);
}
