use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use embedder::{EmbeddingError, EmbeddingPriority, ResidentEmbeddingTelemetryObserver};
use embedding_protocol::EmbeddingTelemetry;

use crate::daemon_error::{DaemonError, Result};

const OBSERVER_ENV: &str = "RESUME_IR_RESIDENT_EMBEDDING_POOL_OBSERVER";
const MAX_INTERACTIVE_SAMPLES: usize = 512;

pub(super) struct ResidentPoolObserver {
    path: PathBuf,
    state: Mutex<ObserverState>,
    writer: Mutex<()>,
}

#[derive(Clone, Default)]
struct ObserverState {
    bulk_calls: u64,
    bulk_inputs: u64,
    bulk_active_tokens: u64,
    bulk_nonconforming: u64,
    interactive_calls: u64,
    interactive_inputs: u64,
    interactive_active_tokens: u64,
    interactive_nonconforming: u64,
    interactive_queue_wait_us: VecDeque<u64>,
}

impl ResidentPoolObserver {
    pub(super) fn from_environment(experiment_active: bool) -> Result<Option<Arc<Self>>> {
        let Some(path) = std::env::var_os(OBSERVER_ENV).map(PathBuf::from) else {
            return Ok(None);
        };
        if !experiment_active {
            return Err(DaemonError::configuration_invalid(
                "resident embedding pool observer requires an explicit experiment arm",
            ));
        }
        if !path.is_absolute() || path.parent().is_none_or(|parent| !parent.is_dir()) {
            return Err(DaemonError::configuration_invalid(
                "resident embedding pool observer path is invalid",
            ));
        }
        Ok(Some(Arc::new(Self::new(path))))
    }

    fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            state: Mutex::new(ObserverState::default()),
            writer: Mutex::new(()),
        }
    }

    pub(super) fn record_bulk(&self, telemetry: &EmbeddingTelemetry) -> std::io::Result<()> {
        let _writer = self
            .writer
            .lock()
            .map_err(|_| std::io::Error::other("resident pool observer writer unavailable"))?;
        let snapshot = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| std::io::Error::other("resident pool observer state unavailable"))?;
            state.bulk_calls = state.bulk_calls.saturating_add(1);
            state.bulk_inputs = state.bulk_inputs.saturating_add(telemetry.input_count);
            state.bulk_active_tokens = state
                .bulk_active_tokens
                .saturating_add(telemetry.active_token_count);
            if telemetry.input_count != 4 || telemetry.active_token_count != 4 * 512 {
                state.bulk_nonconforming = state.bulk_nonconforming.saturating_add(1);
            }
            state.clone()
        };
        persist(&self.path, &snapshot)
    }
}

impl ResidentEmbeddingTelemetryObserver for ResidentPoolObserver {
    fn observe(
        &self,
        priority: EmbeddingPriority,
        telemetry: &EmbeddingTelemetry,
    ) -> std::result::Result<(), EmbeddingError> {
        if priority != EmbeddingPriority::Interactive {
            return Err(EmbeddingError::InvalidRequest);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| EmbeddingError::EngineFailed)?;
        state.interactive_calls = state.interactive_calls.saturating_add(1);
        state.interactive_inputs = state
            .interactive_inputs
            .saturating_add(telemetry.input_count);
        state.interactive_active_tokens = state
            .interactive_active_tokens
            .saturating_add(telemetry.active_token_count);
        if telemetry.input_count != 1 || telemetry.active_token_count != 32 {
            state.interactive_nonconforming = state.interactive_nonconforming.saturating_add(1);
        }
        state
            .interactive_queue_wait_us
            .push_back(telemetry.queue_wait_us);
        if state.interactive_queue_wait_us.len() > MAX_INTERACTIVE_SAMPLES {
            state.interactive_queue_wait_us.pop_front();
        }
        Ok(())
    }
}

fn persist(path: &Path, state: &ObserverState) -> std::io::Result<()> {
    let retained = u64::try_from(state.interactive_queue_wait_us.len()).unwrap_or(u64::MAX);
    let first_retained_sequence = if retained == 0 {
        0
    } else {
        state
            .interactive_calls
            .saturating_sub(retained)
            .saturating_add(1)
    };
    let body = serde_json::json!({
        "schema_version": "resume-ir.resident-embedding-pool-observer.v1",
        "bulk": {
            "completed_calls": state.bulk_calls,
            "completed_inputs": state.bulk_inputs,
            "active_token_count": state.bulk_active_tokens,
            "nonconforming_calls": state.bulk_nonconforming,
        },
        "interactive": {
            "completed_calls": state.interactive_calls,
            "completed_inputs": state.interactive_inputs,
            "active_token_count": state.interactive_active_tokens,
            "nonconforming_calls": state.interactive_nonconforming,
            "first_retained_sequence": first_retained_sequence,
            "queue_wait_us": state.interactive_queue_wait_us,
        },
    });
    let staging = path.with_extension("next");
    let mut output = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&staging)?;
    serde_json::to_writer(&mut output, &body).map_err(std::io::Error::other)?;
    output.write_all(b"\n")?;
    drop(output);
    fs::rename(staging, path)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;

    use embedder::ResidentEmbeddingTelemetryObserver as _;
    use embedding_protocol::EmbeddingTelemetry;

    use super::ResidentPoolObserver;

    #[test]
    fn observer_publishes_bounded_content_free_bulk_and_interactive_telemetry() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("observer.json");
        let observer = ResidentPoolObserver::new(&path);
        observer
            .observe(
                embedder::EmbeddingPriority::Interactive,
                &EmbeddingTelemetry {
                    input_count: 1,
                    active_token_count: 32,
                    queue_wait_us: 7,
                    ..Default::default()
                },
            )
            .unwrap();
        observer
            .record_bulk(&EmbeddingTelemetry {
                input_count: 4,
                active_token_count: 2_048,
                ..Default::default()
            })
            .unwrap();

        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "schema_version": "resume-ir.resident-embedding-pool-observer.v1",
                "bulk": {
                    "completed_calls": 1,
                    "completed_inputs": 4,
                    "active_token_count": 2_048,
                    "nonconforming_calls": 0,
                },
                "interactive": {
                    "completed_calls": 1,
                    "completed_inputs": 1,
                    "active_token_count": 32,
                    "nonconforming_calls": 0,
                    "first_retained_sequence": 1,
                    "queue_wait_us": [7],
                },
            })
        );
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
