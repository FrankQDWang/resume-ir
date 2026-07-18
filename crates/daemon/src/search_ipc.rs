use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::net::TcpStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const REQUEST_SCHEMA_VERSION: &str = "resume-ir.ipc-request.v3";
const RESPONSE_SCHEMA_VERSION: &str = "resume-ir.search-response.v3";
const REQUEST_ID_MAX_BYTES: usize = 128;
const CANCEL_TOKEN_MAX_BYTES: usize = 128;
const CANCEL_HISTORY_LIMIT: usize = 128;
const DEADLINE_MS_MAX: u64 = 60_000;
const TOTAL_IN_FLIGHT_LIMIT: usize = 16;
const DEADLINE_SCHEDULER_POLL: Duration = Duration::from_millis(10);

pub(crate) struct RequestEnvelope {
    pub(crate) request_id: String,
    pub(crate) deadline_ms: u64,
    pub(crate) payload: serde_json::Value,
    cancel_token: Option<String>,
    client_class: ClientClass,
}

impl RequestEnvelope {
    pub(crate) fn cancel_token(&self) -> Option<&str> {
        self.cancel_token.as_deref()
    }

    pub(crate) fn client_class(&self) -> ClientClass {
        self.client_class
    }
}

pub(crate) struct CancelRequest {
    request_id: String,
    cancel_token: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CancelStatus {
    Cancelled,
    CancelRequested,
    Complete,
}

impl CancelStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::CancelRequested => "cancel_requested",
            Self::Complete => "complete",
        }
    }
}

pub(crate) struct RequestCancellation {
    requested: AtomicBool,
}

impl RequestCancellation {
    fn new() -> Self {
        Self {
            requested: AtomicBool::new(false),
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.requested.load(AtomicOrdering::Acquire)
    }

    fn request(&self) {
        self.requested.store(true, AtomicOrdering::Release);
    }
}

struct RequestControl {
    completed: AtomicBool,
    cancellation: RequestCancellation,
}

impl RequestControl {
    fn new() -> Self {
        Self {
            completed: AtomicBool::new(false),
            cancellation: RequestCancellation::new(),
        }
    }
}

#[derive(Default)]
struct CancellationRegistry {
    state: Mutex<CancellationRegistryState>,
}

#[derive(Default)]
struct CancellationRegistryState {
    active: HashMap<String, Arc<RequestControl>>,
    terminal: HashMap<String, CancelStatus>,
    terminal_order: VecDeque<String>,
}

impl CancellationRegistry {
    fn register(&self, token: &str, control: Arc<RequestControl>) -> bool {
        let mut state = self.state.lock().expect("query cancellation registry");
        if state.active.contains_key(token) || state.terminal.contains_key(token) {
            return false;
        }
        state.active.insert(token.to_string(), control);
        true
    }

    fn lookup(&self, token: &str) -> RegistryLookup {
        let state = self.state.lock().expect("query cancellation registry");
        if let Some(control) = state.active.get(token) {
            return RegistryLookup::Active(Arc::clone(control));
        }
        RegistryLookup::Terminal(
            state
                .terminal
                .get(token)
                .copied()
                .unwrap_or(CancelStatus::Complete),
        )
    }

    fn complete(&self, token: Option<&str>, status: CancelStatus) {
        let Some(token) = token else {
            return;
        };
        let mut state = self.state.lock().expect("query cancellation registry");
        state.active.remove(token);
        state.terminal.insert(token.to_string(), status);
        state.terminal_order.push_back(token.to_string());
        while state.terminal_order.len() > CANCEL_HISTORY_LIMIT {
            if let Some(expired) = state.terminal_order.pop_front() {
                state.terminal.remove(&expired);
            }
        }
    }
}

enum RegistryLookup {
    Active(Arc<RequestControl>),
    Terminal(CancelStatus),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClientClass {
    InteractiveGui,
    CodexValidation,
    Benchmark,
    Background,
}

impl ClientClass {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "interactive_gui" => Some(Self::InteractiveGui),
            "codex_validation" => Some(Self::CodexValidation),
            "benchmark" => Some(Self::Benchmark),
            "background" => Some(Self::Background),
            _ => None,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::InteractiveGui => 0,
            Self::CodexValidation => 1,
            Self::Benchmark => 2,
            Self::Background => 3,
        }
    }

    fn in_flight_limit(self) -> usize {
        match self {
            Self::InteractiveGui => 8,
            Self::CodexValidation => 2,
            Self::Benchmark => 8,
            Self::Background => 4,
        }
    }
}

struct AdmissionState {
    total: AtomicUsize,
    by_class: [AtomicUsize; 4],
}

impl AdmissionState {
    fn new() -> Self {
        Self {
            total: AtomicUsize::new(0),
            by_class: std::array::from_fn(|_| AtomicUsize::new(0)),
        }
    }

    fn acquire(self: &Arc<Self>, class: ClientClass) -> Option<AdmissionPermit> {
        let class_counter = &self.by_class[class.index()];
        class_counter
            .fetch_update(AtomicOrdering::AcqRel, AtomicOrdering::Acquire, |current| {
                (current < class.in_flight_limit()).then_some(current + 1)
            })
            .ok()?;
        if self
            .total
            .fetch_update(AtomicOrdering::AcqRel, AtomicOrdering::Acquire, |current| {
                (current < TOTAL_IN_FLIGHT_LIMIT).then_some(current + 1)
            })
            .is_err()
        {
            class_counter.fetch_sub(1, AtomicOrdering::AcqRel);
            return None;
        }
        Some(AdmissionPermit {
            _inner: Arc::new(AdmissionPermitInner {
                state: Arc::clone(self),
                class,
                released: AtomicBool::new(false),
            }),
        })
    }
}

#[derive(Clone)]
struct AdmissionPermit {
    _inner: Arc<AdmissionPermitInner>,
}

struct AdmissionPermitInner {
    state: Arc<AdmissionState>,
    class: ClientClass,
    released: AtomicBool,
}

impl AdmissionPermit {
    fn release(&self) {
        if self._inner.released.swap(true, AtomicOrdering::AcqRel) {
            return;
        }
        self._inner.state.total.fetch_sub(1, AtomicOrdering::AcqRel);
        self._inner.state.by_class[self._inner.class.index()].fetch_sub(1, AtomicOrdering::AcqRel);
    }
}

impl Drop for AdmissionPermitInner {
    fn drop(&mut self) {
        if self.released.swap(true, AtomicOrdering::AcqRel) {
            return;
        }
        self.state.total.fetch_sub(1, AtomicOrdering::AcqRel);
        self.state.by_class[self.class.index()].fetch_sub(1, AtomicOrdering::AcqRel);
    }
}

#[derive(Clone, Copy)]
pub(crate) struct RequestDeadline {
    started_at: Instant,
    expires_at: Instant,
}

impl RequestDeadline {
    fn new(started_at: Instant, deadline_ms: u64) -> Self {
        Self {
            started_at,
            expires_at: started_at + Duration::from_millis(deadline_ms),
        }
    }

    pub(crate) fn expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }

    pub(crate) fn remaining_ms(&self) -> Option<u64> {
        let remaining = self.expires_at.checked_duration_since(Instant::now())?;
        u64::try_from(remaining.as_millis().max(1)).ok()
    }

    pub(crate) fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }
}

pub(crate) struct SearchService {
    queue: Arc<SearchQueue>,
    worker: JoinHandle<super::Result<()>>,
    deadline_sender: Sender<DeadlineCommand>,
    deadline_worker: JoinHandle<()>,
    admission: Arc<AdmissionState>,
    batch_active: Arc<AtomicBool>,
    cancellations: Arc<CancellationRegistry>,
}

struct SearchTask {
    reply: SearchReply,
    envelope: RequestEnvelope,
    args: super::DaemonSearchArgs,
    visible_epoch: u64,
    query_parse_duration: Duration,
    deadline: RequestDeadline,
    control: Arc<RequestControl>,
    _admission_permit: AdmissionPermit,
}

#[derive(Default)]
struct SearchQueue {
    state: Mutex<SearchQueueState>,
    ready: Condvar,
}

#[derive(Default)]
struct SearchQueueState {
    tasks: VecDeque<SearchTask>,
    shutdown: bool,
}

impl SearchQueue {
    fn push(&self, task: SearchTask) -> bool {
        let mut state = self.state.lock().expect("query queue");
        if state.shutdown {
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
                return Some(task);
            }
            if state.shutdown {
                return None;
            }
            state = self.ready.wait(state).expect("query queue");
        }
    }

    fn remove(&self, control: &Arc<RequestControl>) -> Option<SearchTask> {
        let mut state = self.state.lock().expect("query queue");
        let index = state
            .tasks
            .iter()
            .position(|task| Arc::ptr_eq(&task.control, control))?;
        state.tasks.remove(index)
    }

    fn shutdown(&self) {
        let mut state = self.state.lock().expect("query queue");
        state.shutdown = true;
        self.ready.notify_all();
    }
}

enum DeadlineCommand {
    Schedule(ScheduledDeadline),
    Wake,
    Shutdown,
}

struct ScheduledDeadline {
    sequence: u64,
    reply: SearchReply,
    request_id: String,
    visible_epoch: u64,
    mode: super::DaemonSearchMode,
    query_parse_duration: Duration,
    deadline: RequestDeadline,
    control: Arc<RequestControl>,
    _admission_permit: AdmissionPermit,
}

enum SearchReply {
    Single(TcpStream),
    Batch(super::search_batch::BatchChildReply),
}

impl SearchReply {
    fn try_clone(&self) -> super::Result<Self> {
        match self {
            Self::Single(stream) => stream.try_clone().map(Self::Single).map_err(|_| {
                super::DaemonError::recoverable_dependency("unable to monitor query deadline")
            }),
            Self::Batch(reply) => Ok(Self::Batch(reply.clone())),
        }
    }

    fn write_output(&mut self, output: super::DaemonSearchOutput) -> super::Result<()> {
        match self {
            Self::Single(stream) => super::write_search_http_response(stream, output),
            Self::Batch(reply) => {
                reply.complete(200, &output.body);
                Ok(())
            }
        }
    }

    fn write_error(
        &mut self,
        request_id: &str,
        status_code: u16,
        code: &str,
        message: &str,
    ) -> super::Result<()> {
        let body = error_body(request_id, code, message);
        match self {
            Self::Single(stream) => {
                super::write_http_response(stream, status_code, "application/json", &body)
            }
            Self::Batch(reply) => {
                reply.complete(status_code, &body);
                Ok(())
            }
        }
    }

    fn write_overloaded(&mut self, request_id: &str) -> super::Result<()> {
        let body = overload_body(request_id);
        match self {
            Self::Single(stream) => {
                super::write_http_response(stream, 503, "application/json", &body)
            }
            Self::Batch(reply) => {
                reply.complete(503, &body);
                Ok(())
            }
        }
    }
}

pub(crate) struct BatchAdmissionPermit {
    active: Arc<AtomicBool>,
}

impl Drop for BatchAdmissionPermit {
    fn drop(&mut self) {
        self.active.store(false, AtomicOrdering::Release);
    }
}

impl PartialEq for ScheduledDeadline {
    fn eq(&self, other: &Self) -> bool {
        self.deadline.expires_at == other.deadline.expires_at && self.sequence == other.sequence
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
            .expires_at
            .cmp(&self.deadline.expires_at)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

impl SearchService {
    pub(crate) fn start(data_dir: &Path, options: &super::RunOptions) -> super::Result<Self> {
        let store = super::open_store(data_dir)?;
        let queue = Arc::new(SearchQueue::default());
        let admission = Arc::new(AdmissionState::new());
        let batch_active = Arc::new(AtomicBool::new(false));
        let cancellations = Arc::new(CancellationRegistry::default());
        let (deadline_sender, deadline_receiver) = mpsc::channel::<DeadlineCommand>();
        let deadline_worker = thread::spawn(move || run_deadline_scheduler(deadline_receiver));
        let worker = start_search_worker(
            data_dir.to_path_buf(),
            store,
            options.clone(),
            Arc::clone(&queue),
            Arc::clone(&cancellations),
            deadline_sender.clone(),
        );
        Ok(Self {
            queue,
            worker,
            deadline_sender,
            deadline_worker,
            admission,
            batch_active,
            cancellations,
        })
    }

    pub(crate) fn dispatch(
        &self,
        stream: TcpStream,
        envelope: RequestEnvelope,
        args: super::DaemonSearchArgs,
        query_parse_duration: Duration,
        started_at: Instant,
    ) -> super::Result<()> {
        self.dispatch_reply(
            SearchReply::Single(stream),
            envelope,
            args,
            query_parse_duration,
            started_at,
        )
    }

    pub(crate) fn dispatch_batch_child(
        &self,
        reply: super::search_batch::BatchChildReply,
        envelope: RequestEnvelope,
        args: super::DaemonSearchArgs,
        query_parse_duration: Duration,
        started_at: Instant,
    ) -> super::Result<()> {
        self.dispatch_reply(
            SearchReply::Batch(reply),
            envelope,
            args,
            query_parse_duration,
            started_at,
        )
    }

    pub(crate) fn acquire_batch(&self) -> Option<BatchAdmissionPermit> {
        self.batch_active
            .compare_exchange(false, true, AtomicOrdering::AcqRel, AtomicOrdering::Acquire)
            .ok()
            .map(|_| BatchAdmissionPermit {
                active: Arc::clone(&self.batch_active),
            })
    }

    pub(crate) fn check_health(&self) -> super::Result<()> {
        if self.deadline_worker.is_finished() || self.worker.is_finished() {
            return Err(super::DaemonError::control_plane(
                "query service worker stopped unexpectedly",
            ));
        }
        Ok(())
    }

    fn dispatch_reply(
        &self,
        mut reply: SearchReply,
        envelope: RequestEnvelope,
        args: super::DaemonSearchArgs,
        query_parse_duration: Duration,
        started_at: Instant,
    ) -> super::Result<()> {
        let Some(admission_permit) = self.admission.acquire(envelope.client_class) else {
            return reply.write_overloaded(&envelope.request_id);
        };
        let deadline = RequestDeadline::new(started_at, envelope.deadline_ms);
        let control = Arc::new(RequestControl::new());
        if envelope
            .cancel_token
            .as_deref()
            .is_some_and(|token| !self.cancellations.register(token, Arc::clone(&control)))
        {
            return reply.write_error(
                &envelope.request_id,
                409,
                "CONFLICT",
                "cancel_token is already registered",
            );
        }
        let deadline_reply = reply.try_clone()?;
        self.deadline_sender
            .send(DeadlineCommand::Schedule(ScheduledDeadline {
                sequence: 0,
                reply: deadline_reply,
                request_id: envelope.request_id.clone(),
                visible_epoch: 0,
                mode: args.mode,
                query_parse_duration,
                deadline,
                control: Arc::clone(&control),
                _admission_permit: admission_permit.clone(),
            }))
            .map_err(|_| {
                super::DaemonError::control_plane("query deadline monitor is unavailable")
            })?;
        if !self.queue.push(SearchTask {
            reply,
            envelope,
            args,
            visible_epoch: 0,
            query_parse_duration,
            deadline,
            control,
            _admission_permit: admission_permit,
        }) {
            return Err(super::DaemonError::control_plane(
                "query worker is unavailable",
            ));
        }
        Ok(())
    }

    pub(crate) fn cancel(
        &self,
        mut stream: TcpStream,
        request: CancelRequest,
    ) -> super::Result<()> {
        let status = match self.cancellations.lookup(&request.cancel_token) {
            RegistryLookup::Terminal(status) => status,
            RegistryLookup::Active(control) => {
                control.cancellation.request();
                if let Some(mut task) = self.queue.remove(&control) {
                    let status = if control
                        .completed
                        .compare_exchange(
                            false,
                            true,
                            AtomicOrdering::AcqRel,
                            AtomicOrdering::Acquire,
                        )
                        .is_ok()
                    {
                        let output = super::daemon_search_cancelled_output(
                            &task.envelope.request_id,
                            task.visible_epoch,
                            task.args.mode,
                            task.deadline.elapsed(),
                            task.query_parse_duration,
                        );
                        let _ = task.reply.write_output(output);
                        self.cancellations.complete(
                            task.envelope.cancel_token.as_deref(),
                            CancelStatus::Cancelled,
                        );
                        CancelStatus::Cancelled
                    } else {
                        self.cancellations.complete(
                            task.envelope.cancel_token.as_deref(),
                            CancelStatus::Complete,
                        );
                        CancelStatus::Complete
                    };
                    task._admission_permit.release();
                    let _ = self.deadline_sender.send(DeadlineCommand::Wake);
                    status
                } else if control.completed.load(AtomicOrdering::Acquire) {
                    CancelStatus::Complete
                } else {
                    CancelStatus::CancelRequested
                }
            }
        };
        let body = cancel_response_body(&request.request_id, status);
        super::write_http_response(&mut stream, 200, "application/json", &body)
    }

    pub(crate) fn finish(self) -> super::Result<()> {
        self.queue.shutdown();
        let worker_result = self
            .worker
            .join()
            .map_err(|_| super::DaemonError::control_plane("query worker thread panicked"))?;
        let _ = self.deadline_sender.send(DeadlineCommand::Shutdown);
        self.deadline_worker
            .join()
            .map_err(|_| super::DaemonError::control_plane("query deadline monitor panicked"))?;
        worker_result
    }
}

fn start_search_worker(
    data_dir: std::path::PathBuf,
    store: super::MetaStore,
    options: super::RunOptions,
    queue: Arc<SearchQueue>,
    cancellations: Arc<CancellationRegistry>,
    deadline_waker: Sender<DeadlineCommand>,
) -> JoinHandle<super::Result<()>> {
    thread::spawn(move || {
        let mut query_runtime = super::query_runtime::DaemonQueryRuntime::open(&data_dir).ok();
        while let Some(mut task) = queue.pop() {
            if task.control.completed.load(AtomicOrdering::Acquire) {
                cancellations.complete(
                    task.envelope.cancel_token.as_deref(),
                    CancelStatus::Complete,
                );
                continue;
            }
            let execution = super::DaemonSearchExecution {
                request_id: &task.envelope.request_id,
                args: &task.args,
                query_parse_duration: task.query_parse_duration,
                deadline: &task.deadline,
                cancellation: &task.control.cancellation,
            };
            if query_runtime.is_none() {
                query_runtime = super::query_runtime::DaemonQueryRuntime::open(&data_dir).ok();
            }
            let result = match query_runtime.as_mut() {
                Some(query_runtime) => {
                    super::execute_search_command(&store, &execution, &options, query_runtime)
                }
                None => Err(super::IpcCommandError::ServiceUnavailable(
                    "QUERY_SERVICE_UNAVAILABLE",
                )),
            };
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
                continue;
            }
            let terminal_status = match &result {
                Ok(output) if output.cancelled => CancelStatus::Cancelled,
                _ => CancelStatus::Complete,
            };
            let write_result = match result {
                Ok(output) => task.reply.write_output(output),
                Err(super::IpcCommandError::BadRequest(message)) => {
                    task.reply
                        .write_error(&task.envelope.request_id, 400, "BAD_REQUEST", message)
                }
                Err(super::IpcCommandError::Conflict(message)) => {
                    task.reply
                        .write_error(&task.envelope.request_id, 409, "CONFLICT", message)
                }
                Err(super::IpcCommandError::NotFound(message)) => {
                    task.reply
                        .write_error(&task.envelope.request_id, 404, "NOT_FOUND", message)
                }
                Err(super::IpcCommandError::TooLarge(message)) => task.reply.write_error(
                    &task.envelope.request_id,
                    413,
                    "LIMIT_EXCEEDED",
                    message,
                ),
                Err(super::IpcCommandError::ServiceUnavailable(code)) => task.reply.write_error(
                    &task.envelope.request_id,
                    503,
                    code,
                    "query service is unavailable",
                ),
                Err(super::IpcCommandError::Internal(_)) => task.reply.write_error(
                    &task.envelope.request_id,
                    503,
                    "QUERY_SERVICE_UNAVAILABLE",
                    "query service is unavailable",
                ),
            };
            let _ = write_result;
            cancellations.complete(task.envelope.cancel_token.as_deref(), terminal_status);
            task._admission_permit.release();
            let _ = deadline_waker.send(DeadlineCommand::Wake);
        }
        Ok(())
    })
}

fn run_deadline_scheduler(receiver: Receiver<DeadlineCommand>) {
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
                .expires_at
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
                scheduled.retain(|task| !task.control.completed.load(AtomicOrdering::Acquire))
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
        .is_some_and(|task| task.deadline.expires_at <= now)
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
        let mut stage_timing = super::QueryStageTiming::default();
        stage_timing.record_duration(super::QueryStage::QueryParse, task.query_parse_duration);
        let output = super::daemon_search_deadline_output(
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

pub(crate) struct SearchResponse {
    pub(crate) request_id: String,
    pub(crate) status: &'static str,
    pub(crate) visible_epoch: u64,
    pub(crate) query_mode: &'static str,
    pub(crate) partial_reasons: Vec<&'static str>,
    pub(crate) latency_ms: f64,
    pub(crate) stage_latency_ms: serde_json::Value,
    pub(crate) search_index: &'static str,
    pub(crate) results: Vec<serde_json::Value>,
}

pub(crate) fn parse_request(body: &[u8]) -> Result<RequestEnvelope, &'static str> {
    let value = serde_json::from_slice::<serde_json::Value>(body).map_err(|_| "invalid json")?;
    let object = value
        .as_object()
        .ok_or("search request must be an object")?;
    const ALLOWED_FIELDS: &[&str] = &[
        "schema_version",
        "request_id",
        "client_capability",
        "deadline_ms",
        "cancel_token",
        "payload",
    ];
    if object
        .keys()
        .any(|field| !ALLOWED_FIELDS.contains(&field.as_str()))
    {
        return Err("search request contains an unknown field");
    }
    if value
        .get("schema_version")
        .and_then(serde_json::Value::as_str)
        != Some(REQUEST_SCHEMA_VERSION)
    {
        return Err("search request schema_version is invalid");
    }
    let request_id = value
        .get("request_id")
        .and_then(serde_json::Value::as_str)
        .filter(|request_id| valid_opaque_id(request_id))
        .ok_or("request_id is invalid")?
        .to_string();
    let client_capability = value
        .get("client_capability")
        .and_then(serde_json::Value::as_str)
        .ok_or("client_capability is invalid")?;
    let client_class =
        ClientClass::parse(client_capability).ok_or("client_capability is invalid")?;
    let deadline_ms = value
        .get("deadline_ms")
        .and_then(serde_json::Value::as_u64)
        .filter(|deadline_ms| (1..=DEADLINE_MS_MAX).contains(deadline_ms))
        .ok_or("deadline_ms is invalid")?;
    let payload = value
        .get("payload")
        .filter(|payload| payload.is_object())
        .cloned()
        .ok_or("payload must be an object")?;
    let cancel_token = value
        .get("cancel_token")
        .map(|value| {
            value
                .as_str()
                .filter(|token| valid_cancel_token(token))
                .map(str::to_string)
                .ok_or("cancel_token is invalid")
        })
        .transpose()?;
    Ok(RequestEnvelope {
        request_id,
        deadline_ms,
        payload,
        cancel_token,
        client_class,
    })
}

pub(crate) fn parse_cancel_request(body: &[u8]) -> Result<CancelRequest, &'static str> {
    let value = serde_json::from_slice::<serde_json::Value>(body).map_err(|_| "invalid json")?;
    let object = value
        .as_object()
        .ok_or("search cancel request must be an object")?;
    const ALLOWED_FIELDS: &[&str] = &["schema_version", "request_id", "cancel_token"];
    if object
        .keys()
        .any(|field| !ALLOWED_FIELDS.contains(&field.as_str()))
    {
        return Err("search cancel request contains an unknown field");
    }
    if value
        .get("schema_version")
        .and_then(serde_json::Value::as_str)
        != Some("resume-ir.search-cancel-request.v1")
    {
        return Err("search cancel schema_version is invalid");
    }
    let request_id = value
        .get("request_id")
        .and_then(serde_json::Value::as_str)
        .filter(|request_id| valid_opaque_id(request_id))
        .ok_or("request_id is invalid")?
        .to_string();
    let cancel_token = value
        .get("cancel_token")
        .and_then(serde_json::Value::as_str)
        .filter(|token| valid_cancel_token(token))
        .ok_or("cancel_token is invalid")?
        .to_string();
    Ok(CancelRequest {
        request_id,
        cancel_token,
    })
}

pub(crate) fn response_body(response: SearchResponse) -> String {
    let result_count = response.results.len();
    serde_json::json!({
        "schema_version": RESPONSE_SCHEMA_VERSION,
        "request_id": response.request_id,
        "status": response.status,
        "visible_epoch": response.visible_epoch,
        "query_mode": response.query_mode,
        "partial": !response.partial_reasons.is_empty(),
        "partial_reasons": response.partial_reasons,
        "latency_ms": response.latency_ms,
        "stage_latency_ms": response.stage_latency_ms,
        "search_index": response.search_index,
        "result_count": result_count,
        "results": response.results,
    })
    .to_string()
}

pub(crate) fn error_body(request_id: &str, code: &str, _message: &str) -> String {
    serde_json::json!({
        "schema_version": "resume-ir.error.v1",
        "request_id": request_id,
        "status": "error",
        "error": {
            "code": code,
            "action": error_action(code),
        },
    })
    .to_string()
}

pub(crate) fn overload_body(request_id: &str) -> String {
    serde_json::json!({
        "schema_version": "resume-ir.error.v1",
        "request_id": request_id,
        "status": "error",
        "error": {
            "code": "OVERLOADED",
            "action": "retry",
            "retry_after_ms": 250,
        },
    })
    .to_string()
}

fn error_action(code: &str) -> &'static str {
    match code {
        "BAD_REQUEST" => "correct_request",
        "CONFLICT" => "retry",
        "NOT_FOUND" => "refresh_search",
        "LIMIT_EXCEEDED" => "reduce_page_size",
        "SEMANTIC_DISABLED" => "select_supported_mode",
        "REPAIRING" => "wait_for_repair",
        "QUERY_SERVICE_UNAVAILABLE" => "retry",
        _ => "retry",
    }
}

fn cancel_response_body(request_id: &str, status: CancelStatus) -> String {
    serde_json::json!({
        "schema_version": "resume-ir.search-cancel-response.v1",
        "request_id": request_id,
        "status": status.label(),
    })
    .to_string()
}

pub(crate) fn valid_opaque_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= REQUEST_ID_MAX_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_cancel_token(value: &str) -> bool {
    value.len() <= CANCEL_TOKEN_MAX_BYTES && valid_opaque_id(value)
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::net::TcpListener;

    use super::*;

    fn connected_streams() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind search service fixture");
        let client =
            TcpStream::connect(listener.local_addr().unwrap()).expect("connect search fixture");
        let (server, _) = listener.accept().expect("accept search fixture");
        (client, server)
    }

    fn interactive_envelope(request_id: String) -> RequestEnvelope {
        RequestEnvelope {
            request_id,
            deadline_ms: DEADLINE_MS_MAX,
            payload: serde_json::json!({}),
            cancel_token: None,
            client_class: ClientClass::InteractiveGui,
        }
    }

    fn fulltext_args() -> crate::DaemonSearchArgs {
        crate::DaemonSearchArgs {
            query: "synthetic".to_string(),
            mode: crate::DaemonSearchMode::FullText,
            top_k: 1,
            filter: meta_store::SearchProjectionFilter::default(),
        }
    }

    #[test]
    fn request_envelope_requires_bounded_identity_capability_deadline_and_payload() {
        let request = parse_request(
            br#"{"schema_version":"resume-ir.ipc-request.v3","request_id":"request-1","client_capability":"interactive_gui","deadline_ms":200,"payload":{"query":"private"}}"#,
        )
        .unwrap();
        assert_eq!(request.request_id, "request-1");
        assert_eq!(request.payload["query"], "private");
        assert!(request.cancel_token.is_none());

        let cancellable = parse_request(
            br#"{"schema_version":"resume-ir.ipc-request.v3","request_id":"request-2","client_capability":"interactive_gui","deadline_ms":200,"cancel_token":"cancel-2","payload":{}}"#,
        )
        .unwrap();
        assert_eq!(cancellable.cancel_token.as_deref(), Some("cancel-2"));

        for invalid in [
            serde_json::json!({"schema_version":"legacy","request_id":"request-1","client_capability":"interactive_gui","deadline_ms":200,"payload":{}}),
            serde_json::json!({"schema_version":REQUEST_SCHEMA_VERSION,"request_id":"bad id","client_capability":"interactive_gui","deadline_ms":200,"payload":{}}),
            serde_json::json!({"schema_version":REQUEST_SCHEMA_VERSION,"request_id":"request-1","client_capability":"untrusted","deadline_ms":200,"payload":{}}),
            serde_json::json!({"schema_version":REQUEST_SCHEMA_VERSION,"request_id":"request-1","client_capability":"interactive_gui","deadline_ms":0,"payload":{}}),
            serde_json::json!({"schema_version":REQUEST_SCHEMA_VERSION,"request_id":"request-1","client_capability":"interactive_gui","deadline_ms":200,"payload":[]}),
            serde_json::json!({"schema_version":REQUEST_SCHEMA_VERSION,"request_id":"request-1","client_capability":"interactive_gui","deadline_ms":200,"cancel_token":"private cancel token","payload":{}}),
            serde_json::json!({"schema_version":REQUEST_SCHEMA_VERSION,"request_id":"request-1","client_capability":"interactive_gui","deadline_ms":200,"payload":{},"legacy_alias":true}),
        ] {
            assert!(parse_request(invalid.to_string().as_bytes()).is_err());
        }
    }

    #[test]
    fn cancel_request_requires_bounded_identity() {
        let request = parse_cancel_request(
            br#"{"schema_version":"resume-ir.search-cancel-request.v1","request_id":"cancel-command-1","cancel_token":"cancel-token-1"}"#,
        )
        .unwrap();
        assert_eq!(request.request_id, "cancel-command-1");
        assert_eq!(request.cancel_token, "cancel-token-1");
        for invalid in [
            serde_json::json!({"schema_version":"legacy","request_id":"cancel-command-1","cancel_token":"cancel-token-1"}),
            serde_json::json!({"schema_version":"resume-ir.search-cancel-request.v1","request_id":"bad id","cancel_token":"cancel-token-1"}),
            serde_json::json!({"schema_version":"resume-ir.search-cancel-request.v1","request_id":"cancel-command-1","cancel_token":"bad token"}),
            serde_json::json!({"schema_version":"resume-ir.search-cancel-request.v1","request_id":"cancel-command-1","cancel_token":"cancel-token-1","legacy_alias":true}),
        ] {
            assert!(parse_cancel_request(invalid.to_string().as_bytes()).is_err());
        }
    }

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

    #[test]
    fn overload_rejects_only_one_request_and_recovers_after_capacity_is_released() {
        let queue = Arc::new(SearchQueue::default());
        let (deadline_sender, deadline_receiver) = mpsc::channel::<DeadlineCommand>();
        let service = SearchService {
            queue: Arc::clone(&queue),
            worker: thread::spawn(|| Ok(())),
            deadline_sender: deadline_sender.clone(),
            deadline_worker: thread::spawn(move || run_deadline_scheduler(deadline_receiver)),
            admission: Arc::new(AdmissionState::new()),
            batch_active: Arc::new(AtomicBool::new(false)),
            cancellations: Arc::new(CancellationRegistry::default()),
        };
        let mut accepted_clients = Vec::new();

        for index in 0..ClientClass::InteractiveGui.in_flight_limit() {
            let (client, server) = connected_streams();
            service
                .dispatch(
                    server,
                    interactive_envelope(format!("accepted-{index}")),
                    fulltext_args(),
                    Duration::ZERO,
                    Instant::now(),
                )
                .expect("request within capacity is admitted");
            accepted_clients.push(client);
        }

        let (mut overloaded_client, overloaded_server) = connected_streams();
        overloaded_client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("bound overload response read");
        service
            .dispatch(
                overloaded_server,
                interactive_envelope("overloaded".to_string()),
                fulltext_args(),
                Duration::ZERO,
                Instant::now(),
            )
            .expect("overload is a request-scoped response");
        let mut overloaded_response = String::new();
        overloaded_client
            .read_to_string(&mut overloaded_response)
            .expect("read bounded overload response");
        assert!(overloaded_response.starts_with("HTTP/1.1 503 Service Unavailable"));
        assert!(overloaded_response.contains(r#""request_id":"overloaded""#));
        assert!(overloaded_response.contains(r#""code":"OVERLOADED""#));

        let released = queue
            .state
            .lock()
            .expect("query queue")
            .tasks
            .pop_front()
            .expect("queued admitted request");
        released
            .control
            .completed
            .store(true, AtomicOrdering::Release);
        released._admission_permit.release();
        drop(released);
        deadline_sender
            .send(DeadlineCommand::Wake)
            .expect("wake deadline scheduler after release");

        let (recovered_client, recovered_server) = connected_streams();
        service
            .dispatch(
                recovered_server,
                interactive_envelope("recovered".to_string()),
                fulltext_args(),
                Duration::ZERO,
                Instant::now(),
            )
            .expect("capacity release admits the next request");
        accepted_clients.push(recovered_client);
        assert_eq!(
            queue.state.lock().expect("query queue").tasks.len(),
            ClientClass::InteractiveGui.in_flight_limit()
        );

        service
            .finish()
            .expect("stop isolated search service fixture");
    }
}
