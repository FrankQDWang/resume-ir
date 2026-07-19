use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
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
        .finish()
        .expect("stop isolated search service fixture");
}
