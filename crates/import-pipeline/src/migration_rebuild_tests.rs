use meta_store::{
    ContentDigest, SearchProjectionDigest, SearchProjectionServiceState, SearchProjectionState,
    SearchPublicationRecord, SearchPublicationState, SearchRepairReason, UnixTimestamp,
};

use super::{evaluate_ocr_preclaim_state, OcrPreclaimDecision, OcrPreclaimNotReady};

#[test]
fn ocr_preclaim_requires_the_complete_ready_publication_identity() {
    let now = UnixTimestamp::from_unix_seconds(1_800_410_000);
    let generation = "synthetic-ready-generation".to_string();
    let projection_digest = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
    let publication = SearchPublicationRecord {
        generation: generation.clone(),
        base_generation: None,
        expected_visible_epoch: 0,
        classifier_epoch: "synthetic-classifier".to_string(),
        projection_digest,
        publication_fingerprint: Some(ContentDigest::from_bytes(b"synthetic-publication")),
        state: SearchPublicationState::Ready,
        fulltext: None,
        vector: None,
        created_at: now,
        updated_at: now,
    };
    let exact_ready = SearchProjectionState {
        service_state: SearchProjectionServiceState::Ready,
        generation: Some(generation.clone()),
        visible_epoch: 0,
        repair_reason: None,
        publication: Some(Box::new(publication.clone())),
        updated_at: now,
    };
    assert_eq!(
        evaluate_ocr_preclaim_state(&exact_ready),
        OcrPreclaimDecision::Ready
    );

    for incomplete in [
        SearchProjectionState {
            generation: None,
            publication: None,
            ..exact_ready.clone()
        },
        SearchProjectionState {
            generation: Some(generation),
            publication: None,
            ..exact_ready.clone()
        },
        SearchProjectionState {
            repair_reason: Some(SearchRepairReason::ArtifactUnavailable),
            ..exact_ready
        },
    ] {
        assert_eq!(
            evaluate_ocr_preclaim_state(&incomplete),
            OcrPreclaimDecision::NotReady(OcrPreclaimNotReady::IncompletePublication)
        );
    }
}

#[test]
fn ocr_preclaim_returns_closed_not_ready_states_for_repair_lifecycle() {
    let now = UnixTimestamp::from_unix_seconds(1_800_410_100);
    let repairing = SearchProjectionState {
        service_state: SearchProjectionServiceState::Repairing,
        generation: None,
        visible_epoch: 0,
        repair_reason: Some(SearchRepairReason::MigrationRebuild),
        publication: None,
        updated_at: now,
    };
    assert_eq!(
        evaluate_ocr_preclaim_state(&repairing),
        OcrPreclaimDecision::NotReady(OcrPreclaimNotReady::Repairing)
    );

    let blocked = SearchProjectionState {
        service_state: SearchProjectionServiceState::RepairBlocked,
        repair_reason: Some(SearchRepairReason::RuntimeInvariant),
        ..repairing
    };
    assert_eq!(
        evaluate_ocr_preclaim_state(&blocked),
        OcrPreclaimDecision::NotReady(OcrPreclaimNotReady::RepairBlocked)
    );
}
