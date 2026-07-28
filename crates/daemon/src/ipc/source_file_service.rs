use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use meta_store::{ReadMetaStore, SearchSelection};

use super::{response, ConnectionCompletion, ConnectionOutcome, RequestFailure};
use crate::source_file_authority::{open_verified_with_cancellation, SourceFileError};
use crate::source_preview::{PreviewError, PreviewLeaseStore, MAX_RANGE_BYTES};

const QUEUE_CAPACITY: usize = 16;
const CREATE_RESPONSE_SCHEMA: &str = "resume-ir.source-preview.v1";
const RANGE_RESPONSE_SCHEMA: &str = "resume-ir.source-preview-range.v1";
const CLOSE_RESPONSE_SCHEMA: &str = "resume-ir.source-preview-close.v1";
const REVEAL_RESPONSE_SCHEMA: &str = "resume-ir.source-reveal-target.v1";

pub(crate) struct SourceFileService {
    sender: SyncSender<Task>,
    worker: JoinHandle<()>,
    cancellation: Arc<AtomicBool>,
}

enum Operation {
    CreatePreview {
        request_id: String,
        selection: SearchSelection,
    },
    ReadPreviewRange {
        request_id: String,
        lease_id: String,
        offset: u64,
        length: usize,
    },
    ClosePreview {
        request_id: String,
        lease_id: String,
    },
    ResolveReveal {
        request_id: String,
        selection: SearchSelection,
    },
}

struct Task {
    stream: TcpStream,
    completion: ConnectionCompletion,
    operation: Operation,
}

impl SourceFileService {
    pub(crate) fn start(store: ReadMetaStore) -> crate::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<Task>(QUEUE_CAPACITY);
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker_cancellation = Arc::clone(&cancellation);
        let worker = thread::Builder::new()
            .name("resume-source-files".to_string())
            .spawn(move || {
                let mut leases = PreviewLeaseStore::new();
                while let Ok(task) = receiver.recv() {
                    task.execute(&store, &mut leases, &worker_cancellation);
                }
            })
            .map_err(|_| {
                crate::DaemonError::control_plane("source file service worker could not start")
            })?;
        Ok(Self {
            sender,
            worker,
            cancellation,
        })
    }

    pub(crate) fn create_preview(
        &self,
        stream: TcpStream,
        completion: ConnectionCompletion,
        request_id: String,
        selection: SearchSelection,
    ) {
        self.dispatch(Task {
            stream,
            completion,
            operation: Operation::CreatePreview {
                request_id,
                selection,
            },
        });
    }

    pub(crate) fn read_preview_range(
        &self,
        stream: TcpStream,
        completion: ConnectionCompletion,
        request_id: String,
        lease_id: String,
        offset: u64,
        length: usize,
    ) {
        self.dispatch(Task {
            stream,
            completion,
            operation: Operation::ReadPreviewRange {
                request_id,
                lease_id,
                offset,
                length,
            },
        });
    }

    pub(crate) fn close_preview(
        &self,
        stream: TcpStream,
        completion: ConnectionCompletion,
        request_id: String,
        lease_id: String,
    ) {
        self.dispatch(Task {
            stream,
            completion,
            operation: Operation::ClosePreview {
                request_id,
                lease_id,
            },
        });
    }

    pub(crate) fn resolve_reveal(
        &self,
        stream: TcpStream,
        completion: ConnectionCompletion,
        request_id: String,
        selection: SearchSelection,
    ) {
        self.dispatch(Task {
            stream,
            completion,
            operation: Operation::ResolveReveal {
                request_id,
                selection,
            },
        });
    }

    pub(crate) fn check_health(&self) -> crate::Result<()> {
        if self.worker.is_finished() {
            return Err(crate::DaemonError::control_plane(
                "source file service worker stopped unexpectedly",
            ));
        }
        Ok(())
    }

    pub(crate) fn drain_admitted(self) -> crate::Result<()> {
        let Self {
            sender,
            worker,
            cancellation: _,
        } = self;
        drop(sender);
        join_worker(worker)
    }

    pub(crate) fn shutdown(self) -> crate::Result<()> {
        self.cancellation.store(true, Ordering::Release);
        let Self {
            sender,
            worker,
            cancellation: _,
        } = self;
        drop(sender);
        join_worker(worker)
    }

    pub(crate) fn abort_for_process_exit(self) {
        self.cancellation.store(true, Ordering::Release);
        drop(self.sender);
        drop(self.worker);
    }

    fn dispatch(&self, task: Task) {
        match self.sender.try_send(task) {
            Ok(()) => {}
            Err(TrySendError::Full(task) | TrySendError::Disconnected(task)) => {
                task.respond_unavailable();
            }
        }
    }
}

fn join_worker(worker: JoinHandle<()>) -> crate::Result<()> {
    worker
        .join()
        .map_err(|_| crate::DaemonError::control_plane("source file service worker panicked"))
}

impl Task {
    fn execute(
        mut self,
        store: &ReadMetaStore,
        leases: &mut PreviewLeaseStore,
        cancellation: &AtomicBool,
    ) {
        let result = match self.operation {
            Operation::CreatePreview {
                request_id,
                selection,
            } => create_preview(
                &mut self.stream,
                store,
                leases,
                cancellation,
                request_id,
                selection,
            ),
            Operation::ReadPreviewRange {
                request_id,
                lease_id,
                offset,
                length,
            } => read_preview_range(
                &mut self.stream,
                store,
                leases,
                request_id,
                &lease_id,
                offset,
                length,
            ),
            Operation::ClosePreview {
                request_id,
                lease_id,
            } => close_preview(&mut self.stream, leases, request_id, &lease_id),
            Operation::ResolveReveal {
                request_id,
                selection,
            } => resolve_reveal(&mut self.stream, store, cancellation, request_id, selection),
        };
        self.completion
            .finish(ConnectionOutcome::from_request_result(result));
    }

    fn respond_unavailable(mut self) {
        let request_id = match &self.operation {
            Operation::CreatePreview { request_id, .. }
            | Operation::ReadPreviewRange { request_id, .. }
            | Operation::ClosePreview { request_id, .. }
            | Operation::ResolveReveal { request_id, .. } => request_id,
        };
        let result = write_error(
            &mut self.stream,
            Some(request_id),
            503,
            "METADATA_UNAVAILABLE",
            "retry",
        );
        self.completion
            .finish(ConnectionOutcome::from_request_result(result));
    }
}

fn create_preview(
    stream: &mut TcpStream,
    store: &ReadMetaStore,
    leases: &mut PreviewLeaseStore,
    cancellation: &AtomicBool,
    request_id: String,
    selection: SearchSelection,
) -> Result<(), RequestFailure> {
    if let Err(failure) = leases.ensure_capacity() {
        return write_preview_error(stream, Some(&request_id), failure);
    }
    let verified = open_verified_with_cancellation(store, &selection, cancellation)
        .map_err(PreviewError::Source);
    let created = match verified.and_then(|verified| leases.create_verified(selection, verified)) {
        Ok(created) => created,
        Err(failure) => return write_preview_error(stream, Some(&request_id), failure),
    };
    write_json(
        stream,
        200,
        &serde_json::json!({
            "schema_version": CREATE_RESPONSE_SCHEMA,
            "request_id": request_id,
            "status": "ok",
            "lease_id": created.lease_id,
            "byte_size": created.byte_size,
            "expires_in_ms": created.expires_in_ms,
            "range_bytes": MAX_RANGE_BYTES,
        })
        .to_string(),
    )
}

fn read_preview_range(
    stream: &mut TcpStream,
    store: &ReadMetaStore,
    leases: &mut PreviewLeaseStore,
    request_id: String,
    lease_id: &str,
    offset: u64,
    length: usize,
) -> Result<(), RequestFailure> {
    let range = match leases.read_range(store, lease_id, offset, length) {
        Ok(range) => range,
        Err(failure) => return write_preview_error(stream, Some(&request_id), failure),
    };
    write_json(
        stream,
        200,
        &serde_json::json!({
            "schema_version": RANGE_RESPONSE_SCHEMA,
            "request_id": request_id,
            "status": "ok",
            "offset": range.offset,
            "bytes_read": range.bytes_read,
            "total_bytes": range.total_bytes,
            "data_base64": range.base64_data,
        })
        .to_string(),
    )
}

fn close_preview(
    stream: &mut TcpStream,
    leases: &mut PreviewLeaseStore,
    request_id: String,
    lease_id: &str,
) -> Result<(), RequestFailure> {
    let closed = leases.close(lease_id);
    write_json(
        stream,
        200,
        &serde_json::json!({
            "schema_version": CLOSE_RESPONSE_SCHEMA,
            "request_id": request_id,
            "status": "ok",
            "closed": closed,
        })
        .to_string(),
    )
}

fn resolve_reveal(
    stream: &mut TcpStream,
    store: &ReadMetaStore,
    cancellation: &AtomicBool,
    request_id: String,
    selection: SearchSelection,
) -> Result<(), RequestFailure> {
    let verified = match open_verified_with_cancellation(store, &selection, cancellation) {
        Ok(verified) => verified,
        Err(failure) => return write_source_error(stream, Some(&request_id), failure),
    };
    let byte_size = verified.byte_size();
    let content_hash = verified.content_hash().to_string();
    let (_, path, _) = verified.into_parts();
    let Some(path) = path.to_str() else {
        return write_error(
            stream,
            Some(&request_id),
            422,
            "SOURCE_UNSUPPORTED",
            "select_supported_view",
        );
    };
    write_json(
        stream,
        200,
        &serde_json::json!({
            "schema_version": REVEAL_RESPONSE_SCHEMA,
            "request_id": request_id,
            "status": "ok",
            "path": path,
            "byte_size": byte_size,
            "content_hash": content_hash,
        })
        .to_string(),
    )
}

fn write_preview_error(
    stream: &mut TcpStream,
    request_id: Option<&str>,
    failure: PreviewError,
) -> Result<(), RequestFailure> {
    match failure {
        PreviewError::Source(failure) => write_source_error(stream, request_id, failure),
        PreviewError::LeaseInvalid => {
            write_error(stream, request_id, 410, "PREVIEW_EXPIRED", "reopen_preview")
        }
        PreviewError::RangeInvalid => {
            write_error(stream, request_id, 416, "INVALID_RANGE", "correct_request")
        }
        PreviewError::Capacity => write_error(stream, request_id, 429, "PREVIEW_CAPACITY", "retry"),
        PreviewError::Io => write_error(stream, request_id, 503, "METADATA_UNAVAILABLE", "retry"),
    }
}

fn write_source_error(
    stream: &mut TcpStream,
    request_id: Option<&str>,
    failure: SourceFileError,
) -> Result<(), RequestFailure> {
    let (status, code, action) = match failure {
        SourceFileError::StaleSelection => (409, "STALE_SELECTION", "refresh_search"),
        SourceFileError::NotFound | SourceFileError::SourceMissing => {
            (404, "SOURCE_UNAVAILABLE", "rescan_source")
        }
        SourceFileError::SourceChanged => (409, "SOURCE_CHANGED", "rescan_source"),
        SourceFileError::UnsafePath | SourceFileError::UnsupportedFormat => {
            (422, "SOURCE_UNSUPPORTED", "select_supported_view")
        }
        SourceFileError::MetadataUnavailable | SourceFileError::Cancelled | SourceFileError::Io => {
            (503, "METADATA_UNAVAILABLE", "retry")
        }
    };
    write_error(stream, request_id, status, code, action)
}

fn write_error(
    stream: &mut TcpStream,
    request_id: Option<&str>,
    status: u16,
    code: &str,
    action: &str,
) -> Result<(), RequestFailure> {
    write_json(
        stream,
        status,
        &response::unified_error_body(request_id, code, action),
    )
}

fn write_json(stream: &mut TcpStream, status: u16, body: &str) -> Result<(), RequestFailure> {
    response::write_http_response(stream, status, "application/json", body)
        .map_err(RequestFailure::ResponseSink)
}
