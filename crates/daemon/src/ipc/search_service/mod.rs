use std::net::TcpStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

mod admission;
mod artifact_fault;
mod batch;
mod cancellation;
mod runtime;
mod wire;

pub(crate) use admission::{BatchAdmissionPermit, ClientClass};
pub(crate) use artifact_fault::{
    artifact_fault_latch, ArtifactFaultReceiver, ArtifactFaultReporter,
};
pub(crate) use batch::BatchWriter;
pub(crate) use batch::{
    overload_body as batch_overload_body, parse_request as parse_batch_request,
};
pub(crate) use wire::{
    error_body, overload_body, parse_cancel_request, parse_request, service_error_body,
    valid_opaque_id, CancelRequest, RequestEnvelope,
};

use crate::search_command::daemon_search_cancelled_output;
use crate::search_contract::{DaemonSearchArgs, SearchDeadline};
use crate::search_runtime_config::SearchRuntimeConfig;

use admission::AdmissionState;
use cancellation::{CancelStatus, CancellationRegistry, RegistryLookup, RequestControl};
use runtime::{
    run_deadline_scheduler, start_search_worker, DeadlineCommand, ScheduledDeadline, SearchQueue,
    SearchTask,
};
use wire::{cancel_response_body, SearchReply};

pub(crate) struct SearchService {
    queue: Arc<SearchQueue>,
    worker: JoinHandle<crate::Result<()>>,
    deadline_sender: Sender<DeadlineCommand>,
    deadline_worker: JoinHandle<()>,
    admission: Arc<AdmissionState>,
    batch_active: Arc<AtomicBool>,
    cancellations: Arc<CancellationRegistry>,
}

impl SearchService {
    pub(crate) fn start(
        data_dir: &Path,
        config: SearchRuntimeConfig,
        artifact_fault_reporter: Option<ArtifactFaultReporter>,
    ) -> crate::Result<Self> {
        let queue = Arc::new(SearchQueue::default());
        let admission = Arc::new(AdmissionState::new());
        let batch_active = Arc::new(AtomicBool::new(false));
        let cancellations = Arc::new(CancellationRegistry::default());
        let (deadline_sender, deadline_receiver) = mpsc::channel::<DeadlineCommand>();
        let deadline_worker = thread::spawn(move || run_deadline_scheduler(deadline_receiver));
        let worker = start_search_worker(
            data_dir.to_path_buf(),
            config,
            Arc::clone(&queue),
            Arc::clone(&cancellations),
            deadline_sender.clone(),
            artifact_fault_reporter,
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
        completion: crate::ipc::ConnectionCompletion,
        envelope: RequestEnvelope,
        args: DaemonSearchArgs,
        query_parse_duration: Duration,
        started_at: Instant,
    ) -> crate::Result<()> {
        self.dispatch_reply(
            SearchReply::Single { stream, completion },
            envelope,
            args,
            query_parse_duration,
            started_at,
        )
    }

    pub(crate) fn dispatch_batch_child(
        &self,
        reply: batch::BatchChildReply,
        envelope: RequestEnvelope,
        args: DaemonSearchArgs,
        query_parse_duration: Duration,
        started_at: Instant,
    ) -> crate::Result<()> {
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
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| BatchAdmissionPermit {
                active: Arc::clone(&self.batch_active),
            })
    }

    pub(crate) fn check_health(&self) -> crate::Result<()> {
        if self.deadline_worker.is_finished() || self.worker.is_finished() {
            return Err(crate::DaemonError::control_plane(
                "query service worker stopped unexpectedly",
            ));
        }
        Ok(())
    }

    fn dispatch_reply(
        &self,
        mut reply: SearchReply,
        envelope: RequestEnvelope,
        args: DaemonSearchArgs,
        query_parse_duration: Duration,
        started_at: Instant,
    ) -> crate::Result<()> {
        let client_class: ClientClass = envelope.client_class;
        let Some(admission_permit) = self.admission.acquire(client_class) else {
            return reply.write_overloaded(&envelope.request_id);
        };
        let deadline = SearchDeadline::new(started_at, envelope.deadline_ms);
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
                crate::DaemonError::control_plane("query deadline monitor is unavailable")
            })?;
        if !self.queue.push(SearchTask {
            reply,
            envelope,
            args,
            visible_epoch: 0,
            query_parse_duration,
            deadline,
            control,
            admission_permit,
        }) {
            return Err(crate::DaemonError::control_plane(
                "query worker is unavailable",
            ));
        }
        Ok(())
    }

    pub(crate) fn cancel(
        &self,
        mut stream: TcpStream,
        request: CancelRequest,
    ) -> crate::Result<()> {
        let status = match self.cancellations.lookup(&request.cancel_token) {
            RegistryLookup::Terminal(status) => status,
            RegistryLookup::Active(control) => {
                control.cancellation.request();
                if let Some(mut task) = self.queue.remove(&control) {
                    let status = if control
                        .completed
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                    {
                        let output = daemon_search_cancelled_output(
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
                    task.admission_permit.release();
                    let _ = self.deadline_sender.send(DeadlineCommand::Wake);
                    status
                } else if control.completed.load(Ordering::Acquire) {
                    CancelStatus::Complete
                } else {
                    CancelStatus::CancelRequested
                }
            }
        };
        let body = cancel_response_body(&request.request_id, status);
        crate::ipc::response::write_http_response(&mut stream, 200, "application/json", &body)
            .map_err(crate::DaemonError::response_sink)
    }

    pub(crate) fn shutdown(self) -> crate::Result<()> {
        let queued = self.queue.close_and_cancel();
        for mut task in queued {
            task.control.cancellation.request();
            let status = if task
                .control
                .completed
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                let output = daemon_search_cancelled_output(
                    &task.envelope.request_id,
                    task.visible_epoch,
                    task.args.mode,
                    task.deadline.elapsed(),
                    task.query_parse_duration,
                );
                let _ = task.reply.write_output(output);
                CancelStatus::Cancelled
            } else {
                CancelStatus::Complete
            };
            self.cancellations
                .complete(task.envelope.cancel_token.as_deref(), status);
            task.admission_permit.release();
        }
        let _ = self.deadline_sender.send(DeadlineCommand::Wake);
        let _ = self.deadline_sender.send(DeadlineCommand::Shutdown);

        let worker_result = self.worker.join();
        let deadline_result = self.deadline_worker.join();
        match (worker_result, deadline_result) {
            (Err(_), _) => Err(crate::DaemonError::control_plane(
                "query worker thread panicked",
            )),
            (_, Err(_)) => Err(crate::DaemonError::control_plane(
                "query deadline monitor panicked",
            )),
            (Ok(Err(error)), Ok(())) => Err(error),
            (Ok(Ok(())), Ok(())) => Ok(()),
        }
    }

    /// Stops accepting work and completes every request that the IPC listener
    /// already admitted. This is used only for a bounded request-limit exit;
    /// it must not turn a test/control-plane limit into request cancellation.
    pub(crate) fn drain_admitted(self) -> crate::Result<()> {
        self.queue.close_for_drain();
        let worker_result = self.worker.join();
        let _ = self.deadline_sender.send(DeadlineCommand::Shutdown);
        let deadline_result = self.deadline_worker.join();
        match (worker_result, deadline_result) {
            (Err(_), _) => Err(crate::DaemonError::control_plane(
                "query worker thread panicked",
            )),
            (_, Err(_)) => Err(crate::DaemonError::control_plane(
                "query deadline monitor panicked",
            )),
            (Ok(Err(error)), Ok(())) => Err(error),
            (Ok(Ok(())), Ok(())) => Ok(()),
        }
    }

    /// Cancels all request work and deliberately detaches the data-plane
    /// threads because the caller is returning a process-fatal control-plane
    /// error. The process supervisor owns the bounded containment deadline;
    /// this path must never wait on an uninterruptible artifact open.
    pub(crate) fn abort_for_process_exit(self) {
        let queued = self.queue.close_and_cancel();
        for task in queued {
            task.control.cancellation.request();
            task.control.completed.store(true, Ordering::Release);
            self.cancellations.complete(
                task.envelope.cancel_token.as_deref(),
                CancelStatus::Cancelled,
            );
            task.admission_permit.release();
        }
        let _ = self.deadline_sender.send(DeadlineCommand::Shutdown);
        // JoinHandle::drop is intentional only in this process-fatal path.
        // Returning the fatal event immediately lets process containment stop
        // any non-cooperative external data-plane operation.
        drop(self.worker);
        drop(self.deadline_worker);
    }
}
