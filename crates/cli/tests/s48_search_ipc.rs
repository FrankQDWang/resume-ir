use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use privacy::{ContactHasher, ContactKind};

#[test]
fn search_ipc_submits_authenticated_request_and_renders_redacted_results_without_local_store() {
    let data_dir = temp_path("search-ipc-data");
    let token_file = temp_file("search-ipc-token");
    fs::write(
        &token_file,
        "1212121212121212121212121212121212121212121212121212121212121212\n",
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /search HTTP/1.1"));
        assert!(request.contains(
            "Authorization: Bearer 1212121212121212121212121212121212121212121212121212121212121212"
        ));
        assert!(request.contains("Content-Type: application/json"));
        let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
        let payload: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["query"], "private-query-term");
        assert_eq!(payload["mode"], "fulltext");
        assert_eq!(payload["top_k"], 3);
        assert_eq!(payload["filters"]["degree_min"], "master");
        assert_eq!(
            payload["filters"]["skills_any"],
            serde_json::json!(["java", "rust"])
        );
        assert_eq!(payload["filters"]["years_experience_min"], 5.0);
        assert_eq!(
            payload["filters"]["school_tiers_any"],
            serde_json::json!(["985", "double_first_class"])
        );
        assert_eq!(
            payload["filters"]["schools_any"],
            serde_json::json!(["synthetic institute of technology"])
        );
        assert_eq!(
            payload["filters"]["certificates_any"],
            serde_json::json!(["cka", "pmp"])
        );
        assert_eq!(
            payload["filters"]["date_range_overlaps"],
            serde_json::json!("2021-01/2021-12")
        );
        assert_eq!(
            payload["filters"]["companies_any"],
            serde_json::json!(["synthetic payments"])
        );
        assert_eq!(
            payload["filters"]["titles_any"],
            serde_json::json!(["backend_engineer"])
        );
        assert_eq!(
            payload["filters"]["locations_any"],
            serde_json::json!(["shanghai"])
        );

        let response = serde_json::json!({
            "schema_version": "daemon.search.v1",
            "status": "ok",
            "mode": "fulltext",
            "search_index": "available",
            "result_count": 1,
            "results": [{
                "rank": 1,
                "doc_id": "doc_s48",
                "version_id": "ver_s48",
                "file_name": "candidate@example.test-java.pdf",
                "snippet": "Java engineer candidate@example.test 155-555-0199"
            }]
        })
        .to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("write fake search response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "private-query-term",
            "--ipc",
            &format!("http://{addr}/status"),
            "--ipc-token-file",
            path_str(&token_file),
            "--top-k",
            "3",
            "--degree",
            "master",
            "--skills-any",
            "Rust,Java",
            "--years-experience-min",
            "5",
            "--school-tier",
            "985,双一流",
            "--school",
            "Synthetic Institute of Technology",
            "--certificate",
            "PMP,CKA",
            "--date-range-overlaps",
            "2021-01/2021-12",
            "--company",
            "Synthetic Payments Inc.",
            "--title",
            "Backend Engineer",
            "--location",
            "Shanghai",
        ])
        .output()
        .expect("run resume-cli search --ipc");

    server.join().expect("fake daemon joined");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("results: 1"));
    assert!(stdout.contains("rank: 1"));
    assert!(stdout.contains("doc_id: doc_s48"));
    assert!(stdout.contains("version_id: ver_s48"));
    assert!(stdout.contains("snippet:"));
    assert!(!stdout.contains("private-query-term"));
    assert!(!stdout.contains("candidate@example.test"));
    assert!(!stdout.contains("155-555-0199"));
    assert!(!stdout.contains(path_str(&token_file)));
    assert!(!stdout.contains("12121212"));
    assert!(!data_dir.exists());

    remove_path(&data_dir);
    remove_path(&token_file);
}

#[test]
fn search_ipc_hashes_contact_filters_before_submitting_request() {
    let data_dir = temp_path("search-ipc-contact-data");
    let token_file = temp_file("search-ipc-contact-token");
    fs::write(
        &token_file,
        "3434343434343434343434343434343434343434343434343434343434343434\n",
    )
    .unwrap();
    let hasher = ContactHasher::load_or_create(&data_dir).unwrap();
    let expected_email_hash = hasher
        .hash_contact(ContactKind::Email, "target-contact@example.test")
        .unwrap()
        .as_str()
        .to_string();
    let expected_phone_hash = hasher
        .hash_contact(ContactKind::Phone, "+12125550199")
        .unwrap()
        .as_str()
        .to_string();
    let expected_email_hash_for_request = expected_email_hash.clone();
    let expected_phone_hash_for_request = expected_phone_hash.clone();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /search HTTP/1.1"));
        assert!(request.contains(
            "Authorization: Bearer 3434343434343434343434343434343434343434343434343434343434343434"
        ));
        assert!(!request.contains("target-contact@example.test"));
        assert!(!request.contains("TARGET-CONTACT@example.test"));
        assert!(!request.contains("212-555-0199"));
        assert!(!request.contains("+12125550199"));
        let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
        let payload: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["query"], "private-contact-query");
        let mut actual_contact_hashes = payload["filters"]["contact_hashes_any"]
            .as_array()
            .expect("contact_hashes_any array")
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .expect("contact_hashes_any string")
                    .to_string()
            })
            .collect::<Vec<_>>();
        actual_contact_hashes.sort();
        let mut expected_contact_hashes = vec![
            expected_email_hash_for_request,
            expected_phone_hash_for_request,
        ];
        expected_contact_hashes.sort();
        assert_eq!(actual_contact_hashes, expected_contact_hashes);

        let response = serde_json::json!({
            "schema_version": "daemon.search.v1",
            "status": "ok",
            "mode": "fulltext",
            "search_index": "available",
            "result_count": 0,
            "results": []
        })
        .to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("write fake search response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "private-contact-query",
            "--ipc",
            &format!("http://{addr}/status"),
            "--ipc-token-file",
            path_str(&token_file),
            "--email",
            "TARGET-CONTACT@example.test",
            "--phone",
            "212-555-0199",
            "--top-k",
            "3",
        ])
        .output()
        .expect("run resume-cli contact search --ipc");

    server.join().expect("fake daemon joined");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("results: 0"));
    assert!(!stdout.contains("private-contact-query"));
    assert!(!stdout.contains("target-contact@example.test"));
    assert!(!stdout.contains("TARGET-CONTACT@example.test"));
    assert!(!stdout.contains("212-555-0199"));
    assert!(!stdout.contains(&expected_email_hash));
    assert!(!stdout.contains(&expected_phone_hash));
    assert!(!stdout.contains(path_str(&token_file)));

    remove_path(&data_dir);
    remove_path(&token_file);
}

#[test]
fn search_ipc_auto_discovers_endpoint_and_token_file() {
    let data_dir = temp_path("search-ipc-auto-data");
    let token = "6767676767676767676767676767676767676767676767676767676767676767";
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    write_auto_ipc_files(&data_dir, addr, token);
    let server = thread::spawn(move || {
        let (mut status_stream, _) = accept_with_timeout(&listener);
        let status_request = read_http_request(&mut status_stream);
        assert!(status_request.starts_with("GET /status HTTP/1.1"));
        assert!(!status_request.contains("Authorization:"));
        assert!(!status_request.contains("private-auto-query"));
        write_auto_status_response(&mut status_stream);
        drop(status_stream);

        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /search HTTP/1.1"));
        assert!(request.contains(&format!("Authorization: Bearer {token}")));
        assert!(request.contains("Content-Type: application/json"));
        let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
        let payload: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["query"], "private-auto-query");
        assert_eq!(payload["mode"], "fulltext");
        assert_eq!(payload["top_k"], 3);

        let response = serde_json::json!({
            "schema_version": "daemon.search.v1",
            "status": "ok",
            "mode": "fulltext",
            "search_index": "available",
            "result_count": 1,
            "results": [{
                "rank": 1,
                "doc_id": "doc_s48_auto",
                "version_id": "ver_s48_auto",
                "file_name": "candidate@example.test-auto.pdf",
                "snippet": "Auto query result candidate@example.test 155-555-0199"
            }]
        })
        .to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("write fake search response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "private-auto-query",
            "--ipc",
            "auto",
            "--top-k",
            "3",
        ])
        .output()
        .expect("run resume-cli search --ipc auto");

    server.join().expect("fake daemon joined");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("results: 1"));
    assert!(stdout.contains("doc_id: doc_s48_auto"));
    assert!(!stdout.contains("private-auto-query"));
    assert!(!stdout.contains("candidate@example.test"));
    assert!(!stdout.contains("155-555-0199"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains("ipc.auth"));
    assert!(!stdout.contains("67676767"));
    assert!(!data_dir.join("metadata.sqlite3").exists());

    remove_path(&data_dir);
}

#[test]
fn search_ipc_auto_rejects_stale_manifest_without_sending_token_or_query() {
    let data_dir = temp_path("search-ipc-auto-stale-data");
    let token = "8989898989898989898989898989898989898989898989898989898989898989";
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    write_auto_ipc_files(&data_dir, addr, token);
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("GET /status HTTP/1.1"));
        assert!(!request.contains("Authorization:"));
        assert!(!request.contains(token));
        assert!(!request.contains("private-stale-query"));
        let response = "{\"schema_version\":\"not-daemon.v1\",\"status\":\"ok\"}";
        write!(
            stream,
            "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("write stale status response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "private-stale-query",
            "--ipc",
            "auto",
        ])
        .output()
        .expect("run resume-cli search --ipc auto stale");

    server.join().expect("fake daemon joined");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("daemon ipc auto-discovery is stale"));
    assert!(!stderr.contains("private-stale-query"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains("ipc.auth"));
    assert!(!stderr.contains("89898989"));
    assert!(!data_dir.join("metadata.sqlite3").exists());

    remove_path(&data_dir);
}

#[test]
fn search_ipc_errors_do_not_fallback_to_local_store_or_leak_inputs() {
    let data_dir = temp_path("search-ipc-error-data");
    let token_file = temp_file("search-ipc-error-token");
    fs::write(
        &token_file,
        "3434343434343434343434343434343434343434343434343434343434343434\n",
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /search HTTP/1.1"));
        let response = "{\"schema_version\":\"daemon.error.v1\",\"status\":\"unauthorized\"}";
        write!(
            stream,
            "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("write fake search error response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "secret-query",
            "--ipc",
            &format!("http://{addr}/search"),
            "--ipc-token-file",
            path_str(&token_file),
        ])
        .output()
        .expect("run resume-cli search --ipc against erroring daemon");

    server.join().expect("fake daemon joined");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("daemon search ipc returned an error"));
    assert!(!stderr.contains("secret-query"));
    assert!(!stderr.contains(path_str(&token_file)));
    assert!(!stderr.contains("34343434"));
    assert!(!data_dir.exists());

    remove_path(&data_dir);
    remove_path(&token_file);
}

#[test]
fn search_ipc_rejects_invalid_success_protocol_without_local_store() {
    let data_dir = temp_path("search-ipc-invalid-protocol-data");
    let token_file = temp_valid_token("search-ipc-invalid-protocol-token");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /search HTTP/1.1"));
        let response = "{\"schema_version\":\"daemon.status.v1\",\"status\":\"ok\"}";
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("write invalid search response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "secret-query",
            "--ipc",
            &format!("http://{addr}/search"),
            "--ipc-token-file",
            path_str(&token_file),
        ])
        .output()
        .expect("run resume-cli search --ipc against invalid protocol");

    server.join().expect("fake daemon joined");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("daemon search ipc returned invalid protocol"));
    assert!(!stderr.contains("secret-query"));
    assert!(!stderr.contains(path_str(&token_file)));
    assert!(!data_dir.exists());

    remove_path(&data_dir);
    remove_path(&token_file);
}

#[test]
fn search_ipc_rejects_invalid_json_and_malformed_responses_without_local_store() {
    let invalid_json_data = temp_path("search-ipc-invalid-json-data");
    let invalid_json_token = temp_valid_token("search-ipc-invalid-json-token");
    let invalid_json_listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let invalid_json_addr = invalid_json_listener.local_addr().unwrap();
    let invalid_json_server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&invalid_json_listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /search HTTP/1.1"));
        let response = "not json";
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("write invalid json response");
    });
    let invalid_json = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&invalid_json_data),
            "search",
            "secret-query",
            "--ipc",
            &format!("http://{invalid_json_addr}/search"),
            "--ipc-token-file",
            path_str(&invalid_json_token),
        ])
        .output()
        .expect("run resume-cli search --ipc invalid json");
    invalid_json_server.join().expect("fake daemon joined");
    assert!(!invalid_json.status.success());
    assert!(invalid_json.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&invalid_json.stderr);
    assert!(stderr.contains("daemon search ipc returned invalid json"));
    assert!(!stderr.contains("secret-query"));
    assert!(!invalid_json_data.exists());

    let malformed_data = temp_path("search-ipc-malformed-data");
    let malformed_token = temp_valid_token("search-ipc-malformed-token");
    let malformed_listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let malformed_addr = malformed_listener.local_addr().unwrap();
    let malformed_server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&malformed_listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /search HTTP/1.1"));
        write!(stream, "not an http response").expect("write malformed response");
    });
    let malformed = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&malformed_data),
            "search",
            "secret-query",
            "--ipc",
            &format!("http://{malformed_addr}/search"),
            "--ipc-token-file",
            path_str(&malformed_token),
        ])
        .output()
        .expect("run resume-cli search --ipc malformed response");
    malformed_server.join().expect("fake daemon joined");
    assert!(!malformed.status.success());
    assert!(malformed.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&malformed.stderr);
    assert!(stderr.contains("daemon search ipc response is invalid"));
    assert!(!stderr.contains("secret-query"));
    assert!(!malformed_data.exists());

    remove_path(&invalid_json_data);
    remove_path(&invalid_json_token);
    remove_path(&malformed_data);
    remove_path(&malformed_token);
}

#[test]
fn search_ipc_rejects_unsafe_inputs_and_connect_failures_without_local_store() {
    let data_dir = temp_path("search-ipc-invalid-data");
    let missing_token = temp_path("missing-search-ipc-token");
    let invalid_token = temp_file("invalid-search-ipc-token");
    fs::write(
        &invalid_token,
        "abcd\r\nX-Injected-Header: private-private-private-private-private\n",
    )
    .unwrap();

    let non_loopback = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "secret-query",
            "--ipc",
            "http://192.0.2.1:4000/search",
            "--ipc-token-file",
            path_str(&missing_token),
        ])
        .output()
        .expect("run resume-cli search --ipc non-loopback");
    assert!(!non_loopback.status.success());
    assert_eq!(non_loopback.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&non_loopback.stderr);
    assert!(stderr.contains("loopback"));
    assert!(!stderr.contains("secret-query"));
    assert!(!stderr.contains(path_str(&missing_token)));

    let invalid = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "secret-query",
            "--ipc",
            "http://127.0.0.1:4000/search",
            "--ipc-token-file",
            path_str(&invalid_token),
        ])
        .output()
        .expect("run resume-cli search --ipc invalid token");
    assert!(!invalid.status.success());
    let stderr = String::from_utf8_lossy(&invalid.stderr);
    assert!(stderr.contains("daemon search ipc token is invalid"));
    assert!(!stderr.contains("secret-query"));
    assert!(!stderr.contains(path_str(&invalid_token)));
    assert!(!stderr.contains("Injected"));

    let valid_token = temp_valid_token("search-ipc-valid-token");
    let import_url = unused_loopback_search_url();
    let connect_failure = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "secret-query",
            "--ipc",
            &import_url,
            "--ipc-token-file",
            path_str(&valid_token),
        ])
        .output()
        .expect("run resume-cli search --ipc closed port");
    assert!(!connect_failure.status.success());
    let stderr = String::from_utf8_lossy(&connect_failure.stderr);
    assert!(stderr.contains("unable to connect to daemon search ipc"));
    assert!(!stderr.contains("secret-query"));
    assert!(!data_dir.exists());

    let wrong_path = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "secret-query",
            "--ipc",
            "http://127.0.0.1:4000/private",
            "--ipc-token-file",
            path_str(&valid_token),
        ])
        .output()
        .expect("run resume-cli search --ipc wrong path");
    assert!(!wrong_path.status.success());
    assert_eq!(wrong_path.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&wrong_path.stderr);
    assert!(stderr.contains("resume-cli search"));
    assert!(!stderr.contains("secret-query"));
    assert!(!stderr.contains(path_str(&valid_token)));

    remove_path(&data_dir);
    remove_path(&invalid_token);
    remove_path(&valid_token);
}

fn read_http_request(stream: &mut impl Read) -> String {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 512];
    loop {
        let read = stream.read(&mut buffer).expect("read search request");
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    if let Some(header_end) = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
    {
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        while request.len() < header_end + content_length {
            let read = stream.read(&mut buffer).expect("read search body");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
        }
    }
    String::from_utf8_lossy(&request).into_owned()
}

fn accept_with_timeout(listener: &TcpListener) -> (std::net::TcpStream, std::net::SocketAddr) {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match listener.accept() {
            Ok((stream, addr)) => {
                stream.set_nonblocking(false).unwrap();
                return (stream, addr);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    panic!("resume-cli did not connect to fake daemon");
                }
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => panic!("accept search request: {error}"),
        }
    }
}

fn unused_loopback_search_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind unused loopback port");
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}/search")
}

fn temp_valid_token(label: &str) -> PathBuf {
    let token = temp_file(label);
    fs::write(
        &token,
        "5656565656565656565656565656565656565656565656565656565656565656\n",
    )
    .unwrap();
    token
}

fn write_auto_ipc_files(data_dir: &Path, addr: SocketAddr, token: &str) {
    fs::create_dir_all(data_dir).unwrap();
    fs::write(
        data_dir.join("ipc.endpoints.json"),
        format!(
            "{{\"schema_version\":\"resume-ir.daemon-ipc.v1\",\"status\":\"http://{addr}/status\",\"imports\":\"http://{addr}/imports\",\"search\":\"http://{addr}/search\",\"details\":\"http://{addr}/details\"}}"
        ),
    )
    .unwrap();
    fs::write(data_dir.join("ipc.auth"), format!("{token}\n")).unwrap();
}

fn write_auto_status_response(stream: &mut impl Write) {
    let response = "{\"schema_version\":\"daemon.status.v1\",\"status\":\"ok\",\"index_health\":\"ready\",\"import_tasks_queued\":0,\"import_tasks_cancelled\":0}";
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.len(),
        response
    )
    .expect("write fake status response");
}

fn temp_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("resume-ir-s48-cli-{label}-{unique}"))
}

fn temp_file(label: &str) -> PathBuf {
    let path = temp_path(label);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_path(path: &PathBuf) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_dir_all(path);
}
