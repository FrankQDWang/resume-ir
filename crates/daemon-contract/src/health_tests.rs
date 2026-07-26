use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConformanceFixture {
    schema_version: String,
    cases: Vec<ConformanceCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConformanceCase {
    name: String,
    runtime_availability: RuntimeAvailability,
    capabilities: CapabilityMatrix,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeAvailability {
    embedding: bool,
    ocr: bool,
    classifier: bool,
}

fn runtime(available: bool) -> OptionalRuntimeHealth {
    if available {
        OptionalRuntimeHealth::available()
    } else {
        OptionalRuntimeHealth::unavailable(OptionalRuntimeReason::NotConfigured)
    }
}

#[test]
fn all_ready_runtime_combinations_have_one_deterministic_capability_matrix() {
    for bits in 0_u8..8 {
        let runtimes = OptionalRuntimeMatrix {
            embedding: runtime(bits & 1 != 0),
            ocr: runtime(bits & 2 != 0),
            classifier: runtime(bits & 4 != 0),
        };
        let core = CoreHealth::ready();
        let capabilities = CapabilityMatrix::derive(core, runtimes);
        validate_health_contract(StatusState::Ok, core, runtimes, capabilities, None).unwrap();

        assert_eq!(
            capabilities.index_publication,
            if runtimes.embedding.is_available() {
                CapabilityHealth::available()
            } else {
                CapabilityHealth::unavailable(CapabilityReason::EmbeddingUnavailable)
            }
        );
        if !runtimes.classifier.is_available() {
            assert_eq!(
                capabilities.text_import,
                CapabilityHealth::unavailable(CapabilityReason::ClassifierUnavailable)
            );
            assert_eq!(
                capabilities.ocr_import,
                CapabilityHealth::unavailable(CapabilityReason::ClassifierUnavailable)
            );
        }
    }
}

#[test]
fn non_serving_core_states_never_authorize_store_access() {
    for core in [
        CoreHealth::initializing(),
        CoreHealth {
            state: CoreState::Repairing,
            reason: Some(CoreReason::ArtifactUnavailable),
        },
        CoreHealth {
            state: CoreState::Degraded,
            reason: Some(CoreReason::MetadataUnavailable),
        },
        CoreHealth::blocked(CoreReason::RuntimeInvariant),
    ] {
        let runtimes = OptionalRuntimeMatrix::initializing();
        let capabilities = CapabilityMatrix::derive(core, runtimes);
        validate_health_contract(
            core.status(),
            core,
            runtimes,
            capabilities,
            CoreError::for_core(core),
        )
        .unwrap();
        assert!(matches!(
            capabilities.keyword_search.state,
            CapabilityState::Initializing | CapabilityState::Blocked
        ));
        assert_ne!(
            capabilities.keyword_search.state,
            CapabilityState::Available
        );
    }
}

#[test]
fn serde_requires_closed_fields_and_rejects_unknown_fields() {
    let missing_reason = serde_json::json!({"state": "ready"});
    assert!(serde_json::from_value::<CoreHealth>(missing_reason).is_err());
    let unknown = serde_json::json!({"state": "ready", "reason": null, "debug": true});
    assert!(serde_json::from_value::<CoreHealth>(unknown).is_err());
}

#[test]
fn shared_conformance_fixture_covers_every_ready_runtime_combination() {
    let fixture: ConformanceFixture = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/health-combinations-v1.json"
    )))
    .unwrap();
    assert_eq!(
        fixture.schema_version,
        "resume-ir.daemon-health-conformance.v1"
    );
    assert_eq!(fixture.cases.len(), 8);

    for case in fixture.cases {
        let runtimes = OptionalRuntimeMatrix {
            embedding: runtime(case.runtime_availability.embedding),
            ocr: runtime(case.runtime_availability.ocr),
            classifier: runtime(case.runtime_availability.classifier),
        };
        assert_eq!(
            CapabilityMatrix::derive(CoreHealth::ready(), runtimes),
            case.capabilities,
            "shared conformance drift in {}",
            case.name
        );
    }
}
