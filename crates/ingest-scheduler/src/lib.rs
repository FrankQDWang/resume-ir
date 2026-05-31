//! Deterministic ingest scheduling primitives for the S12 OCR-required path.

use ocr_client::OcrCacheKey;
use std::fmt;

/// Routing state used when ingestion determines a page requires OCR.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OcrRoutingState {
    /// The document or page cannot become searchable until OCR runs later.
    OcrRequired,
}

/// OCR task priority used by deterministic claiming.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum OcrTaskPriority {
    /// Low-priority background OCR work.
    Low,
    /// Normal-priority background OCR work.
    Normal,
    /// High-priority background OCR work.
    High,
}

/// Deterministic scheduler tick used for retry tests and deferred work.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct QueueTick(u64);

impl QueueTick {
    /// Creates a deterministic scheduler tick.
    #[must_use]
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the numeric tick value.
    #[must_use]
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

/// Mutable lifecycle state for an OCR-required task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OcrTaskState {
    /// Task is waiting to be claimed.
    Queued,
    /// Task has been claimed by a worker.
    Running,
    /// Task has been deferred until a deterministic retry tick.
    Deferred {
        /// Tick at which the task may return to the queued state.
        retry_after: QueueTick,
    },
    /// Task was cancelled and will not be claimed.
    Cancelled,
}

/// Resource class for OCR work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OcrResourceClass {
    /// Heavy OCR work must run only in background worker paths.
    BackgroundOnly,
}

/// Policy used when claiming OCR tasks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OcrClaimPolicy {
    allow_background_ocr: bool,
    max_render_dpi: u16,
}

impl OcrClaimPolicy {
    /// Claim policy for query paths, which must not run heavy OCR.
    #[must_use]
    pub fn query_path() -> Self {
        Self {
            allow_background_ocr: false,
            max_render_dpi: 0,
        }
    }

    /// Claim policy for background OCR workers with a maximum render DPI.
    #[must_use]
    pub fn background(max_render_dpi: u16) -> Self {
        Self {
            allow_background_ocr: true,
            max_render_dpi,
        }
    }

    fn allows(&self, task: &OcrTask) -> bool {
        matches!(task.resource_class, OcrResourceClass::BackgroundOnly)
            && self.allow_background_ocr
            && task.cache_key.render_dpi() <= self.max_render_dpi
    }
}

/// Stable in-memory OCR task identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OcrTaskId(u64);

impl OcrTaskId {
    /// Returns the numeric task identifier.
    #[must_use]
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

/// One OCR-required page task.
#[derive(Clone, Eq, PartialEq)]
pub struct OcrTask {
    task_id: OcrTaskId,
    doc_id: String,
    cache_key: OcrCacheKey,
    priority: OcrTaskPriority,
    resource_class: OcrResourceClass,
    state: OcrTaskState,
    attempts: u32,
    sequence: u64,
    routing_state: OcrRoutingState,
}

impl OcrTask {
    /// Returns the stable task identifier.
    #[must_use]
    pub fn task_id(&self) -> OcrTaskId {
        self.task_id
    }

    /// Returns the caller-provided document identifier.
    #[must_use]
    pub fn doc_id(&self) -> &str {
        &self.doc_id
    }

    /// Returns the OCR cache key for this page.
    #[must_use]
    pub fn cache_key(&self) -> &OcrCacheKey {
        &self.cache_key
    }

    /// Returns task priority.
    #[must_use]
    pub fn priority(&self) -> OcrTaskPriority {
        self.priority
    }

    /// Returns task resource class.
    #[must_use]
    pub fn resource_class(&self) -> OcrResourceClass {
        self.resource_class
    }

    /// Returns lifecycle state.
    #[must_use]
    pub fn state(&self) -> OcrTaskState {
        self.state
    }

    /// Returns how many times the task has been claimed.
    #[must_use]
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Returns the OCR routing state.
    #[must_use]
    pub fn routing_state(&self) -> OcrRoutingState {
        self.routing_state
    }
}

impl fmt::Debug for OcrTask {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrTask")
            .field("task_id", &self.task_id)
            .field("doc_id", &"[redacted local document id]")
            .field("cache_key", &"[redacted OCR cache key]")
            .field("page_number", &self.cache_key.page_number())
            .field("render_dpi", &self.cache_key.render_dpi())
            .field("priority", &self.priority)
            .field("resource_class", &self.resource_class)
            .field("state", &self.state)
            .field("attempts", &self.attempts)
            .field("routing_state", &self.routing_state)
            .finish()
    }
}

/// Deterministic in-memory queue for OCR-required pages.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct InMemoryOcrQueue {
    tasks: Vec<OcrTask>,
    next_task_id: u64,
    next_sequence: u64,
}

impl InMemoryOcrQueue {
    /// Creates an empty OCR-required queue.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tasks: Vec::new(),
            next_task_id: 1,
            next_sequence: 0,
        }
    }

    /// Enqueues a page that ingestion classified as OCR-required.
    pub fn enqueue_ocr_required(
        &mut self,
        doc_id: impl Into<String>,
        cache_key: OcrCacheKey,
        priority: OcrTaskPriority,
    ) -> OcrTaskId {
        let task_id = OcrTaskId(self.next_task_id);
        self.next_task_id = self.next_task_id.saturating_add(1);
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);

        self.tasks.push(OcrTask {
            task_id,
            doc_id: doc_id.into(),
            cache_key,
            priority,
            resource_class: OcrResourceClass::BackgroundOnly,
            state: OcrTaskState::Queued,
            attempts: 0,
            sequence,
            routing_state: OcrRoutingState::OcrRequired,
        });

        task_id
    }

    /// Returns the number of queued OCR-required tasks.
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.tasks
            .iter()
            .filter(|task| task.state == OcrTaskState::Queued)
            .count()
    }

    /// Returns the routing state for a task when it exists.
    #[must_use]
    pub fn routing_state(&self, task_id: OcrTaskId) -> Option<OcrRoutingState> {
        self.task(task_id).map(OcrTask::routing_state)
    }

    /// Returns the lifecycle state for a task when it exists.
    #[must_use]
    pub fn task_state(&self, task_id: OcrTaskId) -> Option<OcrTaskState> {
        self.task(task_id).map(OcrTask::state)
    }

    /// Claims the highest-priority queued task allowed by the resource policy.
    pub fn claim_next(&mut self, policy: &OcrClaimPolicy) -> Option<OcrTask> {
        let index = self.best_claim_index(policy)?;
        let task = &mut self.tasks[index];
        task.state = OcrTaskState::Running;
        task.attempts = task.attempts.saturating_add(1);
        Some(task.clone())
    }

    /// Defers a running task until a deterministic retry tick.
    pub fn defer(&mut self, task_id: OcrTaskId, retry_after: QueueTick) -> bool {
        let Some(task) = self.task_mut(task_id) else {
            return false;
        };
        if task.state != OcrTaskState::Running {
            return false;
        }
        task.state = OcrTaskState::Deferred { retry_after };
        true
    }

    /// Moves deferred tasks whose retry tick has arrived back to queued state.
    pub fn release_ready_deferred(&mut self, now: QueueTick) -> usize {
        let mut released = 0;
        for task in &mut self.tasks {
            if let OcrTaskState::Deferred { retry_after } = task.state {
                if retry_after <= now {
                    task.state = OcrTaskState::Queued;
                    released += 1;
                }
            }
        }
        released
    }

    /// Cancels a task so it cannot be claimed.
    pub fn cancel(&mut self, task_id: OcrTaskId) -> bool {
        let Some(task) = self.task_mut(task_id) else {
            return false;
        };
        task.state = OcrTaskState::Cancelled;
        true
    }

    fn best_claim_index(&self, policy: &OcrClaimPolicy) -> Option<usize> {
        let mut best_index = None;
        for (index, task) in self.tasks.iter().enumerate() {
            if task.state != OcrTaskState::Queued || !policy.allows(task) {
                continue;
            }

            match best_index {
                None => best_index = Some(index),
                Some(current_index) => {
                    let current = &self.tasks[current_index];
                    if task.priority > current.priority
                        || (task.priority == current.priority && task.sequence < current.sequence)
                    {
                        best_index = Some(index);
                    }
                }
            }
        }
        best_index
    }

    fn task(&self, task_id: OcrTaskId) -> Option<&OcrTask> {
        self.tasks.iter().find(|task| task.task_id == task_id)
    }

    fn task_mut(&mut self, task_id: OcrTaskId) -> Option<&mut OcrTask> {
        self.tasks.iter_mut().find(|task| task.task_id == task_id)
    }
}

impl fmt::Debug for InMemoryOcrQueue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let queued = self
            .tasks
            .iter()
            .filter(|task| task.state == OcrTaskState::Queued)
            .count();
        let running = self
            .tasks
            .iter()
            .filter(|task| task.state == OcrTaskState::Running)
            .count();
        let deferred = self
            .tasks
            .iter()
            .filter(|task| matches!(task.state, OcrTaskState::Deferred { .. }))
            .count();
        let cancelled = self
            .tasks
            .iter()
            .filter(|task| task.state == OcrTaskState::Cancelled)
            .count();

        formatter
            .debug_struct("InMemoryOcrQueue")
            .field("task_count", &self.tasks.len())
            .field("queued", &queued)
            .field("running", &running)
            .field("deferred", &deferred)
            .field("cancelled", &cancelled)
            .finish()
    }
}
