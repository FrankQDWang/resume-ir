use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::ipc::search_service::admission::{AdmissionState, ClientClass};
use crate::ipc::search_service::cancellation::CancellationRegistry;
use crate::ipc::search_service::wire::{RequestEnvelope, DEADLINE_MS_MAX};
use crate::ipc::search_service::SearchService;
use crate::search_contract::{DaemonSearchArgs, DaemonSearchMode};

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

fn fulltext_args() -> DaemonSearchArgs {
    DaemonSearchArgs {
        query: "synthetic".to_string(),
        mode: DaemonSearchMode::FullText,
        top_k: 1,
        filter: meta_store::SearchProjectionFilter::default(),
    }
}

fn queued_search_task(
    request_id: &str,
    admission: &Arc<AdmissionState>,
) -> (TcpStream, SearchTask) {
    let (client, server) = connected_streams();
    let control = Arc::new(RequestControl::new());
    let admission_permit = admission
        .acquire(ClientClass::InteractiveGui)
        .expect("synthetic request admission");
    (
        client,
        SearchTask {
            reply: SearchReply::Single {
                stream: server,
                completion: crate::ipc::ConnectionCompletion::accepted(),
            },
            envelope: interactive_envelope(request_id.to_string()),
            args: fulltext_args(),
            visible_epoch: 0,
            query_parse_duration: Duration::ZERO,
            deadline: crate::search_contract::SearchDeadline::new(Instant::now(), DEADLINE_MS_MAX),
            control,
            admission_permit,
        },
    )
}

#[test]
fn no_request_shutdown_never_opens_query_artifacts() {
    let queue = Arc::new(SearchQueue::default());
    let _ = queue.close_and_cancel();
    let cancellations = Arc::new(CancellationRegistry::default());
    let (deadline_sender, _deadline_receiver) = mpsc::channel();
    let open_count = AtomicUsize::new(0);

    run_search_worker(
        crate::search_runtime_config::SearchRuntimeConfig::new(None, None, None, 100),
        queue,
        cancellations,
        deadline_sender,
        None,
        || {
            open_count.fetch_add(1, AtomicOrdering::SeqCst);
            None
        },
    )
    .unwrap();

    assert_eq!(open_count.load(AtomicOrdering::SeqCst), 0);
}

#[test]
fn shutdown_cancels_active_task_and_never_executes_queued_task() {
    let queue = Arc::new(SearchQueue::default());
    let admission = Arc::new(AdmissionState::new());
    let (_active_client, active) = queued_search_task("active", &admission);
    let active_control = Arc::clone(&active.control);
    let (_queued_client, queued) = queued_search_task("queued", &admission);
    assert!(queue.push(active));
    assert!(queue.push(queued));
    let worker_queue = Arc::clone(&queue);
    let executed = Arc::new(AtomicUsize::new(0));
    let worker_executed = Arc::clone(&executed);
    let (active_entered_sender, active_entered_receiver) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        while let Some(task) = worker_queue.pop() {
            worker_executed.fetch_add(1, AtomicOrdering::SeqCst);
            active_entered_sender.send(()).unwrap();
            while !task.control.cancellation.is_cancelled() {
                thread::yield_now();
            }
            task.control.completed.store(true, AtomicOrdering::Release);
            task.admission_permit.release();
            worker_queue.complete_active(&task.control);
        }
    });

    active_entered_receiver.recv().unwrap();
    let queued = queue.close_and_cancel();
    assert!(active_control.cancellation.is_cancelled());
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].envelope.request_id, "queued");
    for task in queued {
        task.control.cancellation.request();
        task.control.completed.store(true, AtomicOrdering::Release);
        task.admission_permit.release();
    }
    worker.join().unwrap();

    assert_eq!(executed.load(AtomicOrdering::SeqCst), 1);
    let state = queue.state.lock().expect("query queue");
    assert!(state.tasks.is_empty());
    assert!(state.active.is_none());
    drop(state);
    assert_eq!(admission.in_flight(), 0);
}

#[test]
fn request_limit_drain_completes_active_and_queued_tasks_without_cancellation() {
    let queue = Arc::new(SearchQueue::default());
    let admission = Arc::new(AdmissionState::new());
    let (_active_client, active) = queued_search_task("active", &admission);
    let active_control = Arc::clone(&active.control);
    let (_queued_client, queued) = queued_search_task("queued", &admission);
    let queued_control = Arc::clone(&queued.control);
    assert!(queue.push(active));
    assert!(queue.push(queued));

    let worker_queue = Arc::clone(&queue);
    let executed = Arc::new(AtomicUsize::new(0));
    let worker_executed = Arc::clone(&executed);
    let (active_entered_sender, active_entered_receiver) = mpsc::sync_channel(1);
    let (active_release_sender, active_release_receiver) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        while let Some(task) = worker_queue.pop() {
            let execution_index = worker_executed.fetch_add(1, AtomicOrdering::SeqCst);
            if execution_index == 0 {
                active_entered_sender.send(()).unwrap();
                active_release_receiver.recv().unwrap();
            }
            assert!(!task.control.cancellation.is_cancelled());
            task.control.completed.store(true, AtomicOrdering::Release);
            task.admission_permit.release();
            worker_queue.complete_active(&task.control);
        }
    });

    active_entered_receiver.recv().unwrap();
    queue.close_for_drain();
    assert!(!active_control.cancellation.is_cancelled());
    assert!(!queued_control.cancellation.is_cancelled());
    active_release_sender.send(()).unwrap();
    worker.join().unwrap();

    assert_eq!(executed.load(AtomicOrdering::SeqCst), 2);
    let state = queue.state.lock().expect("query queue");
    assert!(state.tasks.is_empty());
    assert!(state.active.is_none());
    assert!(state.closed);
    drop(state);
    assert_eq!(admission.in_flight(), 0);
}

#[test]
fn overload_rejects_only_one_request_and_recovers_after_capacity_is_released() {
    let queue = Arc::new(SearchQueue::default());
    let (deadline_sender, deadline_receiver) = mpsc::channel::<DeadlineCommand>();
    let admission = Arc::new(AdmissionState::new());
    let service = SearchService {
        queue: Arc::clone(&queue),
        worker: thread::spawn(|| Ok(())),
        deadline_sender: deadline_sender.clone(),
        deadline_worker: thread::spawn(move || run_deadline_scheduler(deadline_receiver)),
        admission: Arc::clone(&admission),
        batch_active: Arc::new(AtomicBool::new(false)),
        cancellations: Arc::new(CancellationRegistry::default()),
    };
    let mut accepted_clients = Vec::new();

    for index in 0..ClientClass::InteractiveGui.in_flight_limit() {
        let (client, server) = connected_streams();
        service
            .dispatch(
                server,
                crate::ipc::ConnectionCompletion::accepted(),
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
            crate::ipc::ConnectionCompletion::accepted(),
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
    released.admission_permit.release();
    drop(released);
    deadline_sender
        .send(DeadlineCommand::Wake)
        .expect("wake deadline scheduler after release");

    let (recovered_client, recovered_server) = connected_streams();
    service
        .dispatch(
            recovered_server,
            crate::ipc::ConnectionCompletion::accepted(),
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
        .shutdown()
        .expect("stop isolated search service fixture");
    let queue_state = queue.state.lock().expect("query queue");
    assert!(queue_state.tasks.is_empty());
    assert!(queue_state.active.is_none());
    assert!(queue_state.closed);
    drop(queue_state);
    assert_eq!(admission.in_flight(), 0);
}
