use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::command_failure::CommandFailure;
use crate::query_timing::{QueryStage, QueryStageTiming};
use crate::search_command::{
    daemon_search_deadline_output, execute_search_command, DaemonSearchExecution,
    SearchCommandCompletion,
};
use crate::search_contract::{DaemonSearchArgs, DaemonSearchMode, SearchDeadline};
use crate::search_runtime_config::SearchRuntimeConfig;

use super::admission::AdmissionPermit;
use super::artifact_fault::ArtifactFaultReporter;
use super::cancellation::{CancelStatus, CancellationRegistry, RequestControl};
use super::wire::{RequestEnvelope, SearchReply};

const DEADLINE_SCHEDULER_POLL: Duration = Duration::from_millis(10);

pub(super) struct SearchTask {
    pub(super) reply: SearchReply,
    pub(super) envelope: RequestEnvelope,
    pub(super) args: DaemonSearchArgs,
    pub(super) visible_epoch: u64,
    pub(super) query_parse_duration: Duration,
    pub(super) deadline: SearchDeadline,
    pub(super) control: Arc<RequestControl>,
    pub(super) admission_permit: AdmissionPermit,
}

#[derive(Default)]
pub(super) struct SearchQueue {
    state: Mutex<SearchQueueState>,
    ready: Condvar,
}

#[derive(Default)]
struct SearchQueueState {
    tasks: VecDeque<SearchTask>,
    active: Option<Arc<RequestControl>>,
    closed: bool,
}

impl SearchQueue {
    pub(super) fn push(&self, task: SearchTask) -> bool {
        let mut state = self.state.lock().expect("query queue");
        if state.closed {
            return false;
        }
        state.tasks.push_back(task);
        self.ready.notify_one();
        true
    }

    fn pop(&self) -> Option<SearchTask> {
        let mut state = self.state.lock().expect("query queue");
        loop {
            if let Some(task) = state.tasks.pop_front() {
                state.active = Some(Arc::clone(&task.control));
                return Some(task);
            }
            if state.closed {
                return None;
            }
            state = self.ready.wait(state).expect("query queue");
        }
    }

    pub(super) fn remove(&self, control: &Arc<RequestControl>) -> Option<SearchTask> {
        let mut state = self.state.lock().expect("query queue");
        let index = state
            .tasks
            .iter()
            .position(|task| Arc::ptr_eq(&task.control, control))?;
        state.tasks.remove(index)
    }

    fn complete_active(&self, control: &Arc<RequestControl>) {
        let mut state = self.state.lock().expect("query queue");
        if state
            .active
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, control))
        {
            state.active = None;
        }
    }

    pub(super) fn close_for_drain(&self) {
        let mut state = self.state.lock().expect("query queue");
        state.closed = true;
        self.ready.notify_all();
    }

    pub(super) fn close_and_cancel(&self) -> Vec<SearchTask> {
        let mut state = self.state.lock().expect("query queue");
        state.closed = true;
        if let Some(active) = state.active.as_ref() {
            active.cancellation.request();
        }
        let queued = state.tasks.drain(..).collect();
        self.ready.notify_all();
        queued
    }
}

pub(super) enum DeadlineCommand {
    Schedule(ScheduledDeadline),
    Wake,
    Shutdown,
}

pub(super) struct ScheduledDeadline {
    pub(super) sequence: u64,
    pub(super) reply: SearchReply,
    pub(super) request_id: String,
    pub(super) visible_epoch: u64,
    pub(super) mode: DaemonSearchMode,
    pub(super) query_parse_duration: Duration,
    pub(super) deadline: SearchDeadline,
    pub(super) control: Arc<RequestControl>,
    pub(super) _admission_permit: AdmissionPermit,
}

impl PartialEq for ScheduledDeadline {
    fn eq(&self, other: &Self) -> bool {
        self.deadline.expires_at() == other.deadline.expires_at() && self.sequence == other.sequence
    }
}

impl Eq for ScheduledDeadline {}

impl PartialOrd for ScheduledDeadline {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledDeadline {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .deadline
            .expires_at()
            .cmp(&self.deadline.expires_at())
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

pub(super) fn start_search_worker(
    data_dir: PathBuf,
    config: SearchRuntimeConfig,
    queue: Arc<SearchQueue>,
    cancellations: Arc<CancellationRegistry>,
    deadline_waker: Sender<DeadlineCommand>,
    artifact_fault_reporter: Option<ArtifactFaultReporter>,
) -> JoinHandle<crate::Result<()>> {
    thread::spawn(move || {
        run_search_worker(
            config,
            queue,
            cancellations,
            deadline_waker,
            artifact_fault_reporter,
            || crate::query_runtime::DaemonQueryRuntime::open(&data_dir).ok(),
        )
    })
}

fn run_search_worker(
    config: SearchRuntimeConfig,
    queue: Arc<SearchQueue>,
    cancellations: Arc<CancellationRegistry>,
    deadline_waker: Sender<DeadlineCommand>,
    artifact_fault_reporter: Option<ArtifactFaultReporter>,
    mut open_runtime: impl FnMut() -> Option<crate::query_runtime::DaemonQueryRuntime>,
) -> crate::Result<()> {
    // Keep control-plane startup and no-request shutdown independent from
    // artifact opening. The first admitted search owns lazy data-plane
    // initialization and retains the existing service-unavailable result.
    let mut query_runtime = None;
    while let Some(mut task) = queue.pop() {
        if task.control.completed.load(AtomicOrdering::Acquire) {
            cancellations.complete(
                task.envelope.cancel_token.as_deref(),
                CancelStatus::Complete,
            );
            queue.complete_active(&task.control);
            continue;
        }
        let execution = DaemonSearchExecution {
            request_id: &task.envelope.request_id,
            args: &task.args,
            query_parse_duration: task.query_parse_duration,
            deadline: &task.deadline,
            cancellation: &task.control.cancellation,
        };
        if query_runtime.is_none() {
            query_runtime = open_runtime();
        }
        let result = match query_runtime.as_mut() {
            Some(query_runtime) => execute_search_command(&execution, &config, query_runtime),
            None => Err(CommandFailure::ServiceUnavailable(
                "QUERY_SERVICE_UNAVAILABLE",
            )),
        };
        if let Some(fault) = query_runtime
            .as_mut()
            .and_then(crate::query_runtime::DaemonQueryRuntime::take_artifact_fault)
        {
            if let Some(reporter) = artifact_fault_reporter.as_ref() {
                reporter.report(fault);
            }
        }
        if task
            .control
            .completed
            .compare_exchange(false, true, AtomicOrdering::AcqRel, AtomicOrdering::Acquire)
            .is_err()
        {
            cancellations.complete(
                task.envelope.cancel_token.as_deref(),
                CancelStatus::Complete,
            );
            queue.complete_active(&task.control);
            continue;
        }
        let terminal_status = match &result {
            Ok(output) if output.completion == SearchCommandCompletion::Cancelled => {
                CancelStatus::Cancelled
            }
            _ => CancelStatus::Complete,
        };
        let write_result = match result {
            Ok(output) => task.reply.write_output(output),
            Err(CommandFailure::BadRequest(message)) => {
                task.reply
                    .write_error(&task.envelope.request_id, 400, "BAD_REQUEST", message)
            }
            Err(CommandFailure::Conflict(message)) => {
                task.reply
                    .write_error(&task.envelope.request_id, 409, "CONFLICT", message)
            }
            Err(CommandFailure::NotFound(message)) => {
                task.reply
                    .write_error(&task.envelope.request_id, 404, "NOT_FOUND", message)
            }
            Err(CommandFailure::TooLarge(message)) => {
                task.reply
                    .write_error(&task.envelope.request_id, 413, "LIMIT_EXCEEDED", message)
            }
            Err(CommandFailure::ServiceUnavailable(code)) => task.reply.write_error(
                &task.envelope.request_id,
                503,
                code,
                "query service is unavailable",
            ),
            Err(CommandFailure::Internal) => task.reply.write_error(
                &task.envelope.request_id,
                503,
                "QUERY_SERVICE_UNAVAILABLE",
                "query service is unavailable",
            ),
        };
        let _ = write_result;
        cancellations.complete(task.envelope.cancel_token.as_deref(), terminal_status);
        task.admission_permit.release();
        queue.complete_active(&task.control);
        let _ = deadline_waker.send(DeadlineCommand::Wake);
    }
    Ok(())
}

pub(super) fn run_deadline_scheduler(receiver: Receiver<DeadlineCommand>) {
    let mut scheduled = BinaryHeap::<ScheduledDeadline>::new();
    let mut next_sequence = 0_u64;
    loop {
        expire_ready_deadlines(&mut scheduled);
        while scheduled
            .peek()
            .is_some_and(|task| task.control.completed.load(AtomicOrdering::Acquire))
        {
            scheduled.pop();
        }

        let command = if let Some(next) = scheduled.peek() {
            let remaining = next
                .deadline
                .expires_at()
                .saturating_duration_since(Instant::now())
                .min(DEADLINE_SCHEDULER_POLL);
            match receiver.recv_timeout(remaining) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match receiver.recv() {
                Ok(command) => Some(command),
                Err(_) => return,
            }
        };

        match command {
            Some(DeadlineCommand::Schedule(mut task)) => {
                task.sequence = next_sequence;
                next_sequence = next_sequence.wrapping_add(1);
                scheduled.push(task);
            }
            Some(DeadlineCommand::Wake) => {
                scheduled.retain(|task| !task.control.completed.load(AtomicOrdering::Acquire));
            }
            Some(DeadlineCommand::Shutdown) => return,
            None => expire_ready_deadlines(&mut scheduled),
        }
    }
}

fn expire_ready_deadlines(scheduled: &mut BinaryHeap<ScheduledDeadline>) {
    let now = Instant::now();
    while scheduled
        .peek()
        .is_some_and(|task| task.deadline.expires_at() <= now)
    {
        let Some(mut task) = scheduled.pop() else {
            break;
        };
        if task
            .control
            .completed
            .compare_exchange(false, true, AtomicOrdering::AcqRel, AtomicOrdering::Acquire)
            .is_err()
        {
            continue;
        }
        let mut stage_timing = QueryStageTiming::default();
        stage_timing.record_duration(QueryStage::QueryParse, task.query_parse_duration);
        let output = daemon_search_deadline_output(
            &task.request_id,
            task.visible_epoch,
            task.mode,
            task.deadline.elapsed(),
            stage_timing,
            Vec::new(),
        );
        let _ = task.reply.write_output(output);
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
