use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::Duration;

use super::super::wire::DEADLINE_MS_MAX;
use super::{SearchQueue, SearchQueueState};

const GENERATION_CONTROL_TIMEOUT: Duration = Duration::from_millis(DEADLINE_MS_MAX + 5_000);

#[derive(Clone)]
pub(crate) struct GenerationHandoff {
    queue: Arc<SearchQueue>,
    control_timeout: Duration,
}

impl Default for GenerationHandoff {
    fn default() -> Self {
        Self {
            queue: Arc::default(),
            control_timeout: GENERATION_CONTROL_TIMEOUT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GenerationHandoffError {
    Unresponsive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PublicationDisposition {
    Committed,
    Aborted,
}

impl GenerationHandoff {
    #[cfg(test)]
    pub(super) fn with_control_timeout_for_test(control_timeout: Duration) -> Self {
        Self {
            queue: Arc::default(),
            control_timeout,
        }
    }

    pub(crate) fn prepare_runtime(&self) -> Result<bool, GenerationHandoffError> {
        let (reply, response) = mpsc::sync_channel(1);
        match self.queue.prepare_runtime(reply) {
            ControlEnqueue::Completed(result) => Ok(result),
            ControlEnqueue::Queued(id) => {
                self.wait_for_control(id, response, ControlTimeoutDisposition::Withdraw)
            }
            ControlEnqueue::Rejected => Ok(false),
        }
    }

    pub(crate) fn stage(
        &self,
        prepared: search_runtime::PreparedQueryGeneration,
    ) -> Result<bool, GenerationHandoffError> {
        let (reply, response) = mpsc::sync_channel(1);
        let Some(id) = self.queue.stage_generation(prepared, reply) else {
            return Ok(false);
        };
        self.wait_for_control(id, response, ControlTimeoutDisposition::Withdraw)
    }

    pub(crate) fn finish_publication(
        &self,
        disposition: PublicationDisposition,
    ) -> Result<bool, GenerationHandoffError> {
        let (reply, response) = mpsc::sync_channel(1);
        let Some(id) = self.queue.finalize_generation(disposition, reply) else {
            return Ok(false);
        };
        self.wait_for_control(id, response, ControlTimeoutDisposition::Fatal)
    }

    fn wait_for_control(
        &self,
        id: GenerationControlId,
        response: Receiver<bool>,
        timeout_disposition: ControlTimeoutDisposition,
    ) -> Result<bool, GenerationHandoffError> {
        match response.recv_timeout(self.control_timeout) {
            Ok(result) => {
                self.queue.acknowledge_generation_control(id);
                Ok(result)
            }
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                match self
                    .queue
                    .resolve_generation_control_timeout(id, timeout_disposition)
                {
                    ControlTimeoutResolution::Completed(result) => Ok(result),
                    ControlTimeoutResolution::Withdrawn => Ok(false),
                    ControlTimeoutResolution::Fatal => Err(GenerationHandoffError::Unresponsive),
                }
            }
        }
    }

    pub(in crate::ipc::search_service) fn queue(&self) -> Arc<SearchQueue> {
        Arc::clone(&self.queue)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct GenerationControlId(pub(super) u64);

pub(super) struct QueuedGenerationControl {
    pub(super) id: GenerationControlId,
    pub(super) command: GenerationControl,
}

#[derive(Clone, Copy)]
pub(super) struct CompletedGenerationControl {
    pub(super) id: GenerationControlId,
    pub(super) result: bool,
}

pub(super) enum GenerationControl {
    PrepareRuntime {
        reply: SyncSender<bool>,
    },
    Install {
        prepared: Box<search_runtime::PreparedQueryGeneration>,
        reply: SyncSender<bool>,
    },
    Finalize {
        disposition: PublicationDisposition,
        reply: SyncSender<bool>,
    },
}

enum ControlEnqueue {
    Completed(bool),
    Queued(GenerationControlId),
    Rejected,
}

#[derive(Clone, Copy)]
enum ControlTimeoutDisposition {
    Withdraw,
    Fatal,
}

enum ControlTimeoutResolution {
    Completed(bool),
    Withdrawn,
    Fatal,
}

impl SearchQueue {
    fn prepare_runtime(&self, reply: SyncSender<bool>) -> ControlEnqueue {
        let mut state = self.state.lock().expect("query queue");
        if state.closed || !state.publication_enabled || state.control_protocol_failed {
            return ControlEnqueue::Rejected;
        }
        if state.runtime_prepared {
            return ControlEnqueue::Completed(true);
        }
        let Some(id) = reserve_generation_control(&mut state) else {
            return ControlEnqueue::Rejected;
        };
        state.generation_control = Some(QueuedGenerationControl {
            id,
            command: GenerationControl::PrepareRuntime { reply },
        });
        self.ready.notify_one();
        ControlEnqueue::Queued(id)
    }

    fn stage_generation(
        &self,
        prepared: search_runtime::PreparedQueryGeneration,
        reply: SyncSender<bool>,
    ) -> Option<GenerationControlId> {
        let mut state = self.state.lock().expect("query queue");
        if state.closed
            || !state.publication_enabled
            || !state.runtime_prepared
            || state.control_protocol_failed
            || state.publication_commit_in_flight
        {
            return None;
        }
        let id = reserve_generation_control(&mut state)?;
        state.publication_commit_in_flight = true;
        state.generation_control = Some(QueuedGenerationControl {
            id,
            command: GenerationControl::Install {
                prepared: Box::new(prepared),
                reply,
            },
        });
        self.ready.notify_one();
        Some(id)
    }

    fn finalize_generation(
        &self,
        disposition: PublicationDisposition,
        reply: SyncSender<bool>,
    ) -> Option<GenerationControlId> {
        let mut state = self.state.lock().expect("query queue");
        if state.closed || state.control_protocol_failed || !state.publication_commit_in_flight {
            return None;
        }
        let id = reserve_generation_control(&mut state)?;
        state.generation_control = Some(QueuedGenerationControl {
            id,
            command: GenerationControl::Finalize { disposition, reply },
        });
        self.ready.notify_one();
        Some(id)
    }

    pub(super) fn complete_generation_control(
        &self,
        id: GenerationControlId,
        result: bool,
        completes_publication: bool,
    ) {
        let mut state = self.state.lock().expect("query queue");
        if state.active_generation_control == Some(id) {
            state.active_generation_control = None;
        }
        if completes_publication {
            state.publication_commit_in_flight = false;
        }
        if !state.control_protocol_failed {
            state.completed_generation_control = Some(CompletedGenerationControl { id, result });
        }
        self.ready.notify_all();
    }

    pub(super) fn acknowledge_generation_control(&self, id: GenerationControlId) {
        let mut state = self.state.lock().expect("query queue");
        if state
            .completed_generation_control
            .is_some_and(|completed| completed.id == id)
        {
            state.completed_generation_control = None;
        }
        self.ready.notify_all();
    }

    fn resolve_generation_control_timeout(
        &self,
        id: GenerationControlId,
        disposition: ControlTimeoutDisposition,
    ) -> ControlTimeoutResolution {
        let mut state = self.state.lock().expect("query queue");
        if state
            .completed_generation_control
            .is_some_and(|completed| completed.id == id)
        {
            let completed = state
                .completed_generation_control
                .take()
                .expect("matching completed generation control");
            return ControlTimeoutResolution::Completed(completed.result);
        }
        let queued_matches = state
            .generation_control
            .as_ref()
            .is_some_and(|control| control.id == id);
        if queued_matches && matches!(disposition, ControlTimeoutDisposition::Withdraw) {
            let control = state
                .generation_control
                .take()
                .expect("matching queued generation control");
            if matches!(control.command, GenerationControl::Install { .. }) {
                state.publication_commit_in_flight = false;
            }
            self.ready.notify_all();
            return ControlTimeoutResolution::Withdrawn;
        }
        if queued_matches {
            state.generation_control = None;
        }
        state.control_protocol_failed = true;
        state.closed = true;
        if let Some(active) = state.active.as_ref() {
            active.cancellation.request();
        }
        self.ready.notify_all();
        ControlTimeoutResolution::Fatal
    }

    pub(super) fn control_protocol_failed(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| state.control_protocol_failed)
    }
}

fn reserve_generation_control(state: &mut SearchQueueState) -> Option<GenerationControlId> {
    if state.generation_control.is_some()
        || state.active_generation_control.is_some()
        || state.completed_generation_control.is_some()
    {
        return None;
    }
    state.next_generation_control_id = state.next_generation_control_id.wrapping_add(1).max(1);
    Some(GenerationControlId(state.next_generation_control_id))
}

pub(super) fn reject_generation_control(control: QueuedGenerationControl) {
    match control.command {
        GenerationControl::PrepareRuntime { reply }
        | GenerationControl::Install { reply, .. }
        | GenerationControl::Finalize { reply, .. } => {
            let _ = reply.send(false);
        }
    }
}
