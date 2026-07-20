use std::fmt;

use import_pipeline::{ImportPipelineError, ImportPipelineErrorClass};
use meta_store::MetaStoreErrorClass;

use crate::ipc;

pub(crate) type Result<T> = std::result::Result<T, DaemonError>;

#[derive(Debug)]
pub(crate) struct DaemonError {
    message: String,
    exit_code: i32,
    kind: DaemonErrorKind,
}

#[derive(Clone, Copy, Debug)]
enum DaemonErrorKind {
    ConfigurationInvalid,
    RuntimeIntegrity,
    LifecycleCancellation,
    RecoverableDependency,
    Import(ImportFailure),
    Store(MetaStoreErrorClass),
    OwnershipConflict,
    ProtocolMismatch,
    ControlPlane,
    ResponseSink(ipc::ResponseSinkError),
}

#[derive(Clone, Copy, Debug)]
struct ImportFailure {
    class: ImportPipelineErrorClass,
    retryable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DaemonFatalClass {
    OwnershipConflict,
    ConfigurationInvalid,
    RuntimeIntegrity,
    ProtocolMismatch,
    ControlPlaneFailure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerRetryClass {
    Maintenance,
    Dependency,
    Storage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerErrorDisposition {
    LifecycleCancellation,
    Retryable(WorkerRetryClass),
    Fatal(DaemonFatalClass),
}

impl DaemonFatalClass {
    fn label(self) -> &'static str {
        match self {
            Self::OwnershipConflict => "ownership_conflict",
            Self::ConfigurationInvalid => "configuration_invalid",
            Self::RuntimeIntegrity => "runtime_integrity",
            Self::ProtocolMismatch => "protocol_mismatch",
            Self::ControlPlaneFailure => "control_plane_failure",
        }
    }

    fn disposition(self) -> &'static str {
        match self {
            Self::ControlPlaneFailure => "restartable",
            Self::OwnershipConflict
            | Self::ConfigurationInvalid
            | Self::RuntimeIntegrity
            | Self::ProtocolMismatch => "blocked",
        }
    }
}

impl DaemonError {
    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 2,
            kind: DaemonErrorKind::ConfigurationInvalid,
        }
    }

    pub(crate) fn user(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 1,
            kind: DaemonErrorKind::RuntimeIntegrity,
        }
    }

    pub(crate) fn store(error: meta_store::MetaStoreError) -> Self {
        Self {
            message: "metadata store operation failed".to_string(),
            exit_code: 1,
            kind: DaemonErrorKind::Store(error.class()),
        }
    }

    pub(crate) fn import(error: ImportPipelineError) -> Self {
        Self {
            message: "import pipeline operation failed".to_string(),
            exit_code: 1,
            kind: DaemonErrorKind::Import(ImportFailure {
                class: error.class(),
                retryable: error.is_retryable(),
            }),
        }
    }

    pub(crate) fn ocr(error: ocr_client::OcrError) -> Self {
        Self {
            message: "ocr service operation failed".to_string(),
            exit_code: 1,
            kind: match error.kind() {
                ocr_client::OcrErrorKind::InvalidRequest => DaemonErrorKind::ConfigurationInvalid,
                ocr_client::OcrErrorKind::Cancelled => DaemonErrorKind::LifecycleCancellation,
                ocr_client::OcrErrorKind::Disabled
                | ocr_client::OcrErrorKind::Timeout
                | ocr_client::OcrErrorKind::WorkerUnavailable
                | ocr_client::OcrErrorKind::LanguageUnavailable
                | ocr_client::OcrErrorKind::EngineFailed => DaemonErrorKind::RecoverableDependency,
            },
        }
    }

    pub(crate) fn embedding(error: embedder::EmbeddingError) -> Self {
        let kind = match error {
            embedder::EmbeddingError::InvalidDimension
            | embedder::EmbeddingError::InvalidRequest
            | embedder::EmbeddingError::BudgetExceeded { .. }
            | embedder::EmbeddingError::TextBudgetExceeded { .. } => {
                DaemonErrorKind::ConfigurationInvalid
            }
            embedder::EmbeddingError::Cancelled => DaemonErrorKind::LifecycleCancellation,
            embedder::EmbeddingError::WorkerUnavailable
            | embedder::EmbeddingError::EngineFailed
            | embedder::EmbeddingError::Overloaded
            | embedder::EmbeddingError::Timeout => DaemonErrorKind::RecoverableDependency,
        };
        Self {
            message: "embedding service operation failed".to_string(),
            exit_code: 1,
            kind,
        }
    }

    pub(crate) fn response_sink(error: ipc::ResponseSinkError) -> Self {
        Self {
            message: error.to_string(),
            exit_code: 1,
            kind: DaemonErrorKind::ResponseSink(error),
        }
    }

    pub(crate) fn control_plane(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 1,
            kind: DaemonErrorKind::ControlPlane,
        }
    }

    pub(crate) fn ownership_conflict() -> Self {
        Self {
            message: "daemon ownership conflict".to_string(),
            exit_code: 1,
            kind: DaemonErrorKind::OwnershipConflict,
        }
    }

    pub(crate) fn runtime_integrity() -> Self {
        Self {
            message: "daemon runtime integrity failure".to_string(),
            exit_code: 1,
            kind: DaemonErrorKind::RuntimeIntegrity,
        }
    }

    pub(crate) fn configuration_invalid(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 1,
            kind: DaemonErrorKind::ConfigurationInvalid,
        }
    }

    pub(crate) fn recoverable_dependency(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 1,
            kind: DaemonErrorKind::RecoverableDependency,
        }
    }

    pub(crate) fn protocol_mismatch() -> Self {
        Self {
            message: "daemon ipc protocol mismatch".to_string(),
            exit_code: 1,
            kind: DaemonErrorKind::ProtocolMismatch,
        }
    }

    pub(crate) fn exit_code(&self) -> i32 {
        self.exit_code
    }

    pub(crate) fn worker_disposition(&self) -> WorkerErrorDisposition {
        match self.kind {
            DaemonErrorKind::LifecycleCancellation => WorkerErrorDisposition::LifecycleCancellation,
            DaemonErrorKind::RecoverableDependency => {
                WorkerErrorDisposition::Retryable(WorkerRetryClass::Dependency)
            }
            DaemonErrorKind::Store(MetaStoreErrorClass::Storage) => {
                WorkerErrorDisposition::Retryable(WorkerRetryClass::Storage)
            }
            DaemonErrorKind::Import(failure) => failure.worker_disposition(),
            _ => WorkerErrorDisposition::Fatal(self.fatal_class()),
        }
    }

    fn fatal_class(&self) -> DaemonFatalClass {
        match self.kind {
            DaemonErrorKind::OwnershipConflict => DaemonFatalClass::OwnershipConflict,
            DaemonErrorKind::ConfigurationInvalid => DaemonFatalClass::ConfigurationInvalid,
            DaemonErrorKind::RuntimeIntegrity => DaemonFatalClass::RuntimeIntegrity,
            DaemonErrorKind::LifecycleCancellation
            | DaemonErrorKind::RecoverableDependency
            | DaemonErrorKind::Store(MetaStoreErrorClass::Storage)
            | DaemonErrorKind::ControlPlane
            | DaemonErrorKind::ResponseSink(_) => DaemonFatalClass::ControlPlaneFailure,
            DaemonErrorKind::Import(failure) => failure.fatal_class(),
            DaemonErrorKind::ProtocolMismatch => DaemonFatalClass::ProtocolMismatch,
            DaemonErrorKind::Store(MetaStoreErrorClass::MigrationOwnershipRequired) => {
                DaemonFatalClass::OwnershipConflict
            }
            DaemonErrorKind::Store(
                MetaStoreErrorClass::WeakPassphrase
                | MetaStoreErrorClass::InvalidBackup
                | MetaStoreErrorClass::Crypto
                | MetaStoreErrorClass::KeyAlreadyExists,
            ) => DaemonFatalClass::ConfigurationInvalid,
            DaemonErrorKind::Store(
                MetaStoreErrorClass::Migration
                | MetaStoreErrorClass::InvalidValue
                | MetaStoreErrorClass::NotFound
                | MetaStoreErrorClass::InvalidTransition
                | MetaStoreErrorClass::ImmutableIdentityConflict
                | MetaStoreErrorClass::StorageInvariant,
            ) => DaemonFatalClass::RuntimeIntegrity,
        }
    }

    pub(crate) fn fatal_event_json(&self) -> String {
        fatal_event_json_for_class(self.fatal_class())
    }

    #[cfg(test)]
    pub(crate) fn test_import_failure(class: ImportPipelineErrorClass, retryable: bool) -> Self {
        Self {
            message: String::new(),
            exit_code: 1,
            kind: DaemonErrorKind::Import(ImportFailure { class, retryable }),
        }
    }

    #[cfg(test)]
    fn test_store_failure(class: MetaStoreErrorClass) -> Self {
        Self {
            message: String::new(),
            exit_code: 1,
            kind: DaemonErrorKind::Store(class),
        }
    }
}

impl ImportFailure {
    fn worker_disposition(self) -> WorkerErrorDisposition {
        use ImportPipelineErrorClass as Class;

        match self.class {
            Class::Cancelled | Class::Interrupted => WorkerErrorDisposition::LifecycleCancellation,
            Class::Repairing if self.retryable => {
                WorkerErrorDisposition::Retryable(WorkerRetryClass::Maintenance)
            }
            Class::Metadata | Class::FullText | Class::VectorStorage if self.retryable => {
                WorkerErrorDisposition::Retryable(WorkerRetryClass::Storage)
            }
            Class::SourceUnavailable | Class::Scan | Class::EmbeddingRuntime | Class::Parser
                if self.retryable =>
            {
                WorkerErrorDisposition::Retryable(WorkerRetryClass::Dependency)
            }
            Class::VectorContract => {
                WorkerErrorDisposition::Fatal(DaemonFatalClass::ConfigurationInvalid)
            }
            Class::ArtifactRetirement | Class::MetadataInvariant | Class::Privacy => {
                WorkerErrorDisposition::Fatal(DaemonFatalClass::RuntimeIntegrity)
            }
            Class::Repairing
            | Class::Metadata
            | Class::SourceUnavailable
            | Class::Scan
            | Class::FullText
            | Class::VectorStorage
            | Class::EmbeddingRuntime
            | Class::Parser => WorkerErrorDisposition::Fatal(DaemonFatalClass::RuntimeIntegrity),
        }
    }

    fn fatal_class(self) -> DaemonFatalClass {
        match self.worker_disposition() {
            WorkerErrorDisposition::Fatal(class) => class,
            WorkerErrorDisposition::LifecycleCancellation
            | WorkerErrorDisposition::Retryable(_) => DaemonFatalClass::ControlPlaneFailure,
        }
    }
}

impl From<&DaemonError> for ipc::RequestFailure {
    fn from(error: &DaemonError) -> Self {
        match error.kind {
            DaemonErrorKind::ResponseSink(error) => Self::ResponseSink(error),
            DaemonErrorKind::ConfigurationInvalid
            | DaemonErrorKind::RuntimeIntegrity
            | DaemonErrorKind::LifecycleCancellation
            | DaemonErrorKind::RecoverableDependency
            | DaemonErrorKind::Import(_)
            | DaemonErrorKind::Store(_)
            | DaemonErrorKind::OwnershipConflict
            | DaemonErrorKind::ProtocolMismatch
            | DaemonErrorKind::ControlPlane => Self::Handler,
        }
    }
}

impl From<DaemonError> for ipc::RequestFailure {
    fn from(error: DaemonError) -> Self {
        Self::from(&error)
    }
}

impl From<DaemonFatalClass> for ipc::DaemonFatalError {
    fn from(class: DaemonFatalClass) -> Self {
        match class {
            DaemonFatalClass::OwnershipConflict => Self::OwnershipConflict,
            DaemonFatalClass::ConfigurationInvalid => Self::ConfigurationInvalid,
            DaemonFatalClass::RuntimeIntegrity => Self::RuntimeIntegrity,
            DaemonFatalClass::ProtocolMismatch => Self::ProtocolMismatch,
            DaemonFatalClass::ControlPlaneFailure => Self::ControlPlaneFailure,
        }
    }
}

impl From<DaemonError> for ipc::DaemonFatalError {
    fn from(error: DaemonError) -> Self {
        error.fatal_class().into()
    }
}

impl From<ipc::DaemonFatalError> for DaemonError {
    fn from(error: ipc::DaemonFatalError) -> Self {
        match error {
            ipc::DaemonFatalError::OwnershipConflict => Self::ownership_conflict(),
            ipc::DaemonFatalError::ConfigurationInvalid => {
                Self::configuration_invalid("daemon configuration is invalid")
            }
            ipc::DaemonFatalError::RuntimeIntegrity => Self::runtime_integrity(),
            ipc::DaemonFatalError::ProtocolMismatch => Self::protocol_mismatch(),
            ipc::DaemonFatalError::ControlPlaneFailure => {
                Self::control_plane("daemon control plane failed")
            }
        }
    }
}

fn fatal_event_json_for_class(class: DaemonFatalClass) -> String {
    serde_json::json!({
        "schema_version": "resume-ir.daemon-fatal.v1",
        "event": "fatal",
        "class": class.label(),
        "disposition": class.disposition(),
    })
    .to_string()
}

impl fmt::Display for DaemonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[cfg(test)]
mod tests {
    use import_pipeline::ImportPipelineErrorClass;
    use meta_store::MetaStoreErrorClass;

    use super::{
        fatal_event_json_for_class, DaemonError, DaemonFatalClass, WorkerErrorDisposition,
        WorkerRetryClass,
    };

    #[test]
    fn fatal_wire_is_closed_bounded_and_contains_no_raw_message() {
        let classes = [
            DaemonFatalClass::OwnershipConflict,
            DaemonFatalClass::ConfigurationInvalid,
            DaemonFatalClass::RuntimeIntegrity,
            DaemonFatalClass::ProtocolMismatch,
            DaemonFatalClass::ControlPlaneFailure,
        ];
        for class in classes {
            let body = fatal_event_json_for_class(class);
            assert!(body.len() <= 1024);
            let value: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(value.as_object().unwrap().len(), 4);
            assert_eq!(value["schema_version"], "resume-ir.daemon-fatal.v1");
            assert_eq!(value["event"], "fatal");
            assert_eq!(value["class"], class.label());
            assert_eq!(value["disposition"], class.disposition());
        }

        let secret = "PRIVATE_PATH_TOKEN_QUERY";
        let event = DaemonError::control_plane(secret).fatal_event_json();
        assert!(!event.contains(secret));
    }

    #[test]
    fn import_failure_mapping_is_closed_and_preserves_retryability() {
        use ImportPipelineErrorClass as Class;

        for (class, retry_class) in [
            (Class::Repairing, WorkerRetryClass::Maintenance),
            (Class::Metadata, WorkerRetryClass::Storage),
            (Class::SourceUnavailable, WorkerRetryClass::Dependency),
            (Class::Scan, WorkerRetryClass::Dependency),
            (Class::FullText, WorkerRetryClass::Storage),
            (Class::VectorStorage, WorkerRetryClass::Storage),
            (Class::EmbeddingRuntime, WorkerRetryClass::Dependency),
            (Class::Parser, WorkerRetryClass::Dependency),
        ] {
            assert_eq!(
                DaemonError::test_import_failure(class, true).worker_disposition(),
                WorkerErrorDisposition::Retryable(retry_class)
            );
            assert_eq!(
                DaemonError::test_import_failure(class, false).worker_disposition(),
                WorkerErrorDisposition::Fatal(DaemonFatalClass::RuntimeIntegrity)
            );
        }

        for class in [Class::Cancelled, Class::Interrupted] {
            assert_eq!(
                DaemonError::test_import_failure(class, true).worker_disposition(),
                WorkerErrorDisposition::LifecycleCancellation
            );
        }
        for class in [
            Class::ArtifactRetirement,
            Class::MetadataInvariant,
            Class::Privacy,
        ] {
            assert_eq!(
                DaemonError::test_import_failure(class, false).worker_disposition(),
                WorkerErrorDisposition::Fatal(DaemonFatalClass::RuntimeIntegrity)
            );
        }
        assert_eq!(
            DaemonError::test_import_failure(Class::VectorContract, false).worker_disposition(),
            WorkerErrorDisposition::Fatal(DaemonFatalClass::ConfigurationInvalid)
        );
    }

    #[test]
    fn fatal_mapping_separates_restartable_dependencies_from_blocked_failures() {
        let restartable = DaemonError::recoverable_dependency("transient dependency");
        let storage = DaemonError::test_store_failure(MetaStoreErrorClass::Storage);
        let invariant = DaemonError::test_store_failure(MetaStoreErrorClass::StorageInvariant);
        let ownership =
            DaemonError::test_store_failure(MetaStoreErrorClass::MigrationOwnershipRequired);

        assert_eq!(
            restartable.fatal_class(),
            DaemonFatalClass::ControlPlaneFailure
        );
        assert_eq!(storage.fatal_class(), DaemonFatalClass::ControlPlaneFailure);
        assert_eq!(invariant.fatal_class(), DaemonFatalClass::RuntimeIntegrity);
        assert_eq!(ownership.fatal_class(), DaemonFatalClass::OwnershipConflict);
        assert_eq!(
            invariant.worker_disposition(),
            WorkerErrorDisposition::Fatal(DaemonFatalClass::RuntimeIntegrity)
        );
    }
}
