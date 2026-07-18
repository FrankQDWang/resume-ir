use std::fmt;
use std::io::Read;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use embedding_protocol::{
    read_frame, write_frame, EmbedRequest, ResidentInput, ResidentResponse, MAX_REQUEST_BYTES,
    MAX_RESPONSE_BYTES,
};
use process_containment::OwnedLeafChild;

use super::{
    spawn_output_reader, EmbeddingBudget, EmbeddingError, EmbeddingInput, EmbeddingVector,
    LocalEmbeddingCommandSpec,
};

const INTERACTIVE_QUEUE_CAPACITY: usize = 8;
const BACKGROUND_QUEUE_CAPACITY: usize = 4;
const SUPERVISOR_POLL: Duration = Duration::from_millis(10);
const STDERR_CAP: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbeddingPriority {
    Interactive,
    Background,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ResidentEmbeddingStatus {
    Starting = 0,
    Ready = 1,
    Restarting = 2,
    Unavailable = 3,
    Shutdown = 4,
}

impl ResidentEmbeddingStatus {
    fn from_raw(value: u8) -> Self {
        match value {
            0 => Self::Starting,
            1 => Self::Ready,
            2 => Self::Restarting,
            3 => Self::Unavailable,
            _ => Self::Shutdown,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ResidentEmbeddingSpec {
    command: LocalEmbeddingCommandSpec,
    intra_threads: usize,
}

impl ResidentEmbeddingSpec {
    pub fn new(command: LocalEmbeddingCommandSpec) -> Self {
        Self {
            command,
            intra_threads: 1,
        }
    }

    pub fn with_intra_threads(mut self, intra_threads: usize) -> Result<Self, EmbeddingError> {
        if !(1..=3).contains(&intra_threads) {
            return Err(EmbeddingError::InvalidRequest);
        }
        self.intra_threads = intra_threads;
        Ok(self)
    }
}

impl fmt::Debug for ResidentEmbeddingSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResidentEmbeddingSpec")
            .field("command", &self.command)
            .field("intra_threads", &self.intra_threads)
            .finish()
    }
}

pub struct ResidentEmbeddingOwner {
    client: ResidentEmbeddingClient,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl ResidentEmbeddingOwner {
    pub fn start(spec: ResidentEmbeddingSpec) -> Result<Self, EmbeddingError> {
        let model_id: Arc<str> = Arc::from(spec.command.model_id.as_str());
        let dimension = spec.command.dimension;
        let (interactive_sender, interactive_receiver) =
            mpsc::sync_channel(INTERACTIVE_QUEUE_CAPACITY);
        let (background_sender, background_receiver) =
            mpsc::sync_channel(BACKGROUND_QUEUE_CAPACITY);
        let shutdown = Arc::new(AtomicBool::new(false));
        let status = Arc::new(AtomicU8::new(ResidentEmbeddingStatus::Starting as u8));
        let worker_shutdown = Arc::clone(&shutdown);
        let worker_status = Arc::clone(&status);
        let worker = thread::Builder::new()
            .name("embedding-supervisor".to_string())
            .spawn(move || {
                Supervisor::new(
                    spec,
                    interactive_receiver,
                    background_receiver,
                    worker_shutdown,
                    worker_status,
                )
                .run();
            })
            .map_err(|_| EmbeddingError::WorkerUnavailable)?;
        let client = ResidentEmbeddingClient {
            interactive_sender,
            background_sender,
            status,
            model_id,
            dimension,
        };
        Ok(Self {
            client,
            shutdown,
            worker: Some(worker),
        })
    }

    pub fn client(&self) -> ResidentEmbeddingClient {
        self.client.clone()
    }
}

impl Drop for ResidentEmbeddingOwner {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Clone)]
pub struct ResidentEmbeddingClient {
    interactive_sender: SyncSender<EmbeddingTask>,
    background_sender: SyncSender<EmbeddingTask>,
    status: Arc<AtomicU8>,
    model_id: Arc<str>,
    dimension: usize,
}

impl ResidentEmbeddingClient {
    pub fn status(&self) -> ResidentEmbeddingStatus {
        ResidentEmbeddingStatus::from_raw(self.status.load(Ordering::Acquire))
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    pub fn dimension(&self) -> usize {
        self.dimension
    }

    pub fn embed_batch_with_cancel(
        &self,
        priority: EmbeddingPriority,
        inputs: &[EmbeddingInput],
        budget: EmbeddingBudget,
        timeout_ms: u64,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<Vec<EmbeddingVector>, EmbeddingError> {
        super::validate_embedding_inputs(inputs, budget)?;
        if inputs.is_empty() || inputs.len() > embedding_protocol::MAX_INPUTS || timeout_ms == 0 {
            return Err(EmbeddingError::InvalidRequest);
        }
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(timeout_ms))
            .ok_or(EmbeddingError::InvalidRequest)?;
        let cancellation = Arc::new(AtomicU8::new(0));
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        let task = EmbeddingTask {
            inputs: inputs.to_vec(),
            deadline,
            cancellation: Arc::clone(&cancellation),
            response_sender,
        };
        let sender = match priority {
            EmbeddingPriority::Interactive => &self.interactive_sender,
            EmbeddingPriority::Background => &self.background_sender,
        };
        match sender.try_send(task) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(EmbeddingError::Overloaded),
            Err(TrySendError::Disconnected(_)) => return Err(EmbeddingError::WorkerUnavailable),
        }

        loop {
            if is_cancelled() {
                let _ = cancellation.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
            } else if Instant::now() >= deadline {
                let _ = cancellation.compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire);
            }
            match response_receiver.recv_timeout(SUPERVISOR_POLL) {
                Ok(result) => return result,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(EmbeddingError::WorkerUnavailable)
                }
            }
        }
    }
}

impl fmt::Debug for ResidentEmbeddingClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResidentEmbeddingClient")
            .field("status", &self.status())
            .field("model_id", &self.model_id)
            .field("dimension", &self.dimension)
            .finish()
    }
}

struct EmbeddingTask {
    inputs: Vec<EmbeddingInput>,
    deadline: Instant,
    cancellation: Arc<AtomicU8>,
    response_sender: SyncSender<Result<Vec<EmbeddingVector>, EmbeddingError>>,
}

struct Supervisor {
    spec: ResidentEmbeddingSpec,
    interactive_receiver: Receiver<EmbeddingTask>,
    background_receiver: Receiver<EmbeddingTask>,
    shutdown: Arc<AtomicBool>,
    status: Arc<AtomicU8>,
    next_request_id: AtomicU64,
}

impl Supervisor {
    fn new(
        spec: ResidentEmbeddingSpec,
        interactive_receiver: Receiver<EmbeddingTask>,
        background_receiver: Receiver<EmbeddingTask>,
        shutdown: Arc<AtomicBool>,
        status: Arc<AtomicU8>,
    ) -> Self {
        Self {
            spec,
            interactive_receiver,
            background_receiver,
            shutdown,
            status,
            next_request_id: AtomicU64::new(1),
        }
    }

    fn run(self) {
        let mut child = self.spawn_child().ok();
        if child.is_none() {
            self.set_status(ResidentEmbeddingStatus::Unavailable);
        }
        while !self.shutdown.load(Ordering::Acquire) {
            let Some(task) = self.next_task() else {
                continue;
            };
            let cancelled = cancellation_error(&task);
            if let Some(error) = cancelled {
                let _ = task.response_sender.send(Err(error));
                continue;
            }
            if child.is_none() {
                self.set_status(ResidentEmbeddingStatus::Restarting);
                child = self.spawn_child().ok();
            }
            let result = match child.as_mut() {
                Some(runtime) => self.execute_task(runtime, &task),
                None => Err(EmbeddingError::WorkerUnavailable),
            };
            if result.is_err() {
                if let Some(mut runtime) = child.take() {
                    runtime.terminate();
                }
                self.set_status(ResidentEmbeddingStatus::Restarting);
            }
            let _ = task.response_sender.send(result);
        }
        if let Some(mut runtime) = child {
            runtime.terminate();
        }
        self.set_status(ResidentEmbeddingStatus::Shutdown);
    }

    fn next_task(&self) -> Option<EmbeddingTask> {
        match self.interactive_receiver.try_recv() {
            Ok(task) => return Some(task),
            Err(TryRecvError::Disconnected) | Err(TryRecvError::Empty) => {}
        }
        match self.background_receiver.try_recv() {
            Ok(task) => return Some(task),
            Err(TryRecvError::Disconnected) | Err(TryRecvError::Empty) => {}
        }
        match self.interactive_receiver.recv_timeout(SUPERVISOR_POLL) {
            Ok(task) => Some(task),
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => None,
        }
    }

    fn spawn_child(&self) -> Result<ResidentChild, EmbeddingError> {
        let mut command = Command::new(&self.spec.command.program);
        command
            .args(&self.spec.command.args)
            .arg("--resident")
            .env("RESUME_IR_EMBEDDING_MODEL_ID", &self.spec.command.model_id)
            .env(
                "RESUME_IR_EMBEDDING_DIMENSION",
                self.spec.command.dimension.to_string(),
            )
            .env(
                "RESUME_IR_EMBEDDING_INTRA_THREADS",
                self.spec.intra_threads.to_string(),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child =
            OwnedLeafChild::spawn(&mut command).map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
                    EmbeddingError::WorkerUnavailable
                }
                _ => EmbeddingError::EngineFailed,
            })?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.take_stdin(), child.take_stdout(), child.take_stderr())
        else {
            child.terminate();
            return Err(EmbeddingError::EngineFailed);
        };
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        let response_reader = thread::spawn(move || run_response_reader(stdout, response_sender));
        let stderr_reader = spawn_output_reader(stderr, STDERR_CAP);
        let mut runtime = ResidentChild {
            child,
            stdin,
            response_receiver,
            response_reader: Some(response_reader),
            stderr_reader: Some(stderr_reader),
        };
        let deadline = Instant::now() + Duration::from_millis(self.spec.command.timeout_ms);
        loop {
            if self.shutdown.load(Ordering::Acquire) || Instant::now() >= deadline {
                runtime.terminate();
                return Err(EmbeddingError::WorkerUnavailable);
            }
            match runtime.response_receiver.recv_timeout(SUPERVISOR_POLL) {
                Ok(Ok(response)) => {
                    if response
                        .validate_ready(&self.spec.command.model_id, self.spec.command.dimension)
                        .is_err()
                    {
                        runtime.terminate();
                        return Err(EmbeddingError::EngineFailed);
                    }
                    self.set_status(ResidentEmbeddingStatus::Ready);
                    return Ok(runtime);
                }
                Ok(Err(error)) => {
                    runtime.terminate();
                    return Err(error);
                }
                Err(RecvTimeoutError::Disconnected) => {
                    runtime.terminate();
                    return Err(EmbeddingError::EngineFailed);
                }
                Err(RecvTimeoutError::Timeout) => {
                    if runtime.child.try_wait().ok().flatten().is_some() {
                        runtime.terminate();
                        return Err(EmbeddingError::EngineFailed);
                    }
                }
            }
        }
    }

    fn execute_task(
        &self,
        runtime: &mut ResidentChild,
        task: &EmbeddingTask,
    ) -> Result<Vec<EmbeddingVector>, EmbeddingError> {
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let request = EmbedRequest::new(
            request_id,
            &self.spec.command.model_id,
            self.spec.command.dimension,
            task.inputs
                .iter()
                .map(|input| ResidentInput {
                    role: input.role(),
                    text: input.text().to_string(),
                })
                .collect(),
        );
        request
            .validate()
            .map_err(|_| EmbeddingError::InvalidRequest)?;
        write_frame(&mut runtime.stdin, &request, MAX_REQUEST_BYTES)
            .map_err(|_| EmbeddingError::EngineFailed)?;
        loop {
            if self.shutdown.load(Ordering::Acquire) {
                return Err(EmbeddingError::WorkerUnavailable);
            }
            if let Some(error) = cancellation_error(task) {
                return Err(error);
            }
            match runtime.response_receiver.recv_timeout(SUPERVISOR_POLL) {
                Ok(Ok(response)) => {
                    response
                        .validate_result(request_id, task.inputs.len(), self.spec.command.dimension)
                        .map_err(|_| EmbeddingError::EngineFailed)?;
                    let ResidentResponse::Result { vectors, .. } = response else {
                        return Err(EmbeddingError::EngineFailed);
                    };
                    return task
                        .inputs
                        .iter()
                        .zip(vectors)
                        .map(|(input, values)| {
                            EmbeddingVector::new(input.id(), &self.spec.command.model_id, values)
                        })
                        .collect();
                }
                Ok(Err(error)) => return Err(error),
                Err(RecvTimeoutError::Disconnected) => return Err(EmbeddingError::EngineFailed),
                Err(RecvTimeoutError::Timeout) => {
                    if runtime.child.try_wait().ok().flatten().is_some() {
                        return Err(EmbeddingError::EngineFailed);
                    }
                }
            }
        }
    }

    fn set_status(&self, status: ResidentEmbeddingStatus) {
        self.status.store(status as u8, Ordering::Release);
    }
}

fn cancellation_error(task: &EmbeddingTask) -> Option<EmbeddingError> {
    match task.cancellation.load(Ordering::Acquire) {
        1 => Some(EmbeddingError::Cancelled),
        2 => Some(EmbeddingError::Timeout),
        _ if Instant::now() >= task.deadline => {
            task.cancellation.store(2, Ordering::Release);
            Some(EmbeddingError::Timeout)
        }
        _ => None,
    }
}

struct ResidentChild {
    child: OwnedLeafChild,
    stdin: ChildStdin,
    response_receiver: Receiver<Result<ResidentResponse, EmbeddingError>>,
    response_reader: Option<JoinHandle<()>>,
    stderr_reader: Option<JoinHandle<Result<Vec<u8>, EmbeddingError>>>,
}

impl ResidentChild {
    fn terminate(&mut self) {
        self.child.terminate();
        if let Some(reader) = self.response_reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
}

fn run_response_reader(
    mut stdout: impl Read,
    sender: SyncSender<Result<ResidentResponse, EmbeddingError>>,
) {
    loop {
        let response = match read_frame::<ResidentResponse>(&mut stdout, MAX_RESPONSE_BYTES) {
            Ok(Some(response)) => Ok(response),
            Ok(None) | Err(_) => Err(EmbeddingError::EngineFailed),
        };
        let failed = response.is_err();
        if sender.try_send(response).is_err() || failed {
            return;
        }
    }
}
