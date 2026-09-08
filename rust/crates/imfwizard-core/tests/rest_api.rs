//! Drives a real REST API server over TCP, the way a render farm would.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::time::{Duration, Instant};

use imfwizard_core::mxf_wrap::synthetic_j2k_codestream;
use imfwizard_core::rest_api::{ApiConfig, bind_server};
use tempfile::TempDir;

const API_KEY: &str = "farm-key-9f3a";
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FRAMES: usize = 4;
const JOB_TIMEOUT: Duration = Duration::from_secs(120);

// a server per test, so pausing one queue cannot reach another
fn serve() -> SocketAddr {
    let config = ApiConfig {
        host: "127.0.0.1".into(),
        port: 0,
        api_key: Some(API_KEY.into()),
    };
    let (server, listener) = bind_server(&config).unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || server.serve_forever(listener).unwrap());
    address
}

struct Answer {
    status: u16,
    body: String,
}

impl Answer {
    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("{e} in the answer body {:?}", self.body))
    }
}

fn request(address: SocketAddr, method: &str, path: &str, key: Option<&str>) -> Answer {
    request_with_body(address, method, path, "", key)
}

fn request_with_body(
    address: SocketAddr,
    method: &str,
    path: &str,
    body: &str,
    key: Option<&str>,
) -> Answer {
    let authorization = match key {
        Some(key) => format!("X-Api-Key: {key}\r\n"),
        None => String::new(),
    };
    let raw = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{authorization}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    send(address, &raw)
}

fn send(address: SocketAddr, raw: &str) -> Answer {
    let mut stream = TcpStream::connect(address).unwrap();
    stream.write_all(raw.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();

    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("no status line in {response:?}"));
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    Answer { status, body }
}

fn submit(address: SocketAddr, endpoint: &str, input: &Path, output: &Path, title: &str) -> u64 {
    let body = serde_json::json!({
        "input": input.to_string_lossy(),
        "output": output.to_string_lossy(),
        "title": title,
    })
    .to_string();
    let answer = request_with_body(address, "POST", endpoint, &body, Some(API_KEY));
    assert_eq!(answer.status, 202, "{}", answer.body);
    answer.json()["id"].as_u64().expect("a job id")
}

// polls the job until the worker has finished with it
fn wait_for_job(address: SocketAddr, id: u64) -> serde_json::Value {
    let deadline = Instant::now() + JOB_TIMEOUT;
    loop {
        let answer = request(address, "GET", &format!("/api/v1/jobs/{id}"), Some(API_KEY));
        assert_eq!(answer.status, 200, "{}", answer.body);
        let job = answer.json();
        match job["state"].as_str().unwrap_or_default() {
            "Queued" | "Running" => assert!(Instant::now() < deadline, "job {id} never finished"),
            _ => return job,
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn codestream_directory(root: &Path) -> std::path::PathBuf {
    let frames = root.join("j2k");
    std::fs::create_dir_all(&frames).unwrap();
    let codestream = synthetic_j2k_codestream(WIDTH, HEIGHT, 12);
    for frame in 0..FRAMES {
        std::fs::write(frames.join(format!("{frame:04}.j2c")), &codestream).unwrap();
    }
    frames
}

#[test]
fn health_answers_without_a_key() {
    let address = serve();
    let answer = request(address, "GET", "/api/v1/health", None);
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert_eq!(answer.json()["status"], "ok");
}

#[test]
fn every_other_path_needs_the_key_in_a_header() {
    let address = serve();
    assert_eq!(
        request(address, "GET", "/api/v1/jobs", None).status,
        401,
        "the job list must not be readable without the key"
    );
    assert_eq!(
        request(address, "GET", "/api/v1/jobs", Some(API_KEY)).status,
        200
    );

    let bearer = format!(
        "GET /api/v1/jobs HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {API_KEY}\r\nConnection: close\r\n\r\n"
    );
    assert_eq!(send(address, &bearer).status, 200);

    assert_eq!(
        request(address, "GET", "/api/v1/jobs", Some("wrong-key")).status,
        401
    );
    // the exemption is the health path itself, not any path holding it
    assert_eq!(
        request(address, "GET", "/api/v1/healthcheck", None).status,
        401
    );
}

#[test]
fn a_validate_job_on_a_missing_directory_fails_naming_it() {
    let address = serve();
    let directory = TempDir::new().unwrap();
    let missing = directory.path().join("no_such_imp");
    let id = submit(
        address,
        "/api/v1/validate",
        &missing,
        Path::new(""),
        "missing",
    );

    let job = wait_for_job(address, id);
    assert_eq!(job["state"], "Failed", "{job}");
    let error = job["error"].as_str().unwrap_or_default();
    assert!(
        error.contains(&missing.to_string_lossy().to_string()),
        "the error must name the directory, got {error:?}"
    );
}

#[test]
fn a_created_imp_validates_through_the_api() {
    let address = serve();
    let directory = TempDir::new().unwrap();
    let frames = codestream_directory(directory.path());
    let imp = directory.path().join("imp");

    let created = submit(address, "/api/v1/create", &frames, &imp, "REST Feature");
    let job = wait_for_job(address, created);
    assert_eq!(job["state"], "Completed", "{job}");

    let validated = submit(address, "/api/v1/validate", &imp, Path::new(""), "check");
    let job = wait_for_job(address, validated);
    assert_eq!(job["state"], "Completed", "{job}");
}

#[test]
fn pause_refuses_a_submission_and_resume_takes_it() {
    let address = serve();
    let directory = TempDir::new().unwrap();
    let missing = directory.path().join("no_such_imp");
    let body = serde_json::json!({ "input": missing.to_string_lossy() }).to_string();

    let paused = request_with_body(address, "POST", "/api/v1/pause", "", Some(API_KEY));
    assert_eq!(paused.status, 200);
    assert_eq!(paused.json()["paused"], true);

    let refused = request_with_body(address, "POST", "/api/v1/validate", &body, Some(API_KEY));
    assert_eq!(refused.status, 503, "{}", refused.body);

    let resumed = request_with_body(address, "POST", "/api/v1/resume", "", Some(API_KEY));
    assert_eq!(resumed.status, 200);
    assert_eq!(resumed.json()["paused"], false);

    let accepted = request_with_body(address, "POST", "/api/v1/validate", &body, Some(API_KEY));
    assert_eq!(accepted.status, 202, "{}", accepted.body);
}

#[test]
fn a_job_waiting_behind_another_can_be_cancelled() {
    let address = serve();
    let directory = TempDir::new().unwrap();
    let frames = codestream_directory(directory.path());

    // the worker takes one job at a time, so the second stays queued while the
    // first wraps and hashes its track file
    let blocking = submit(
        address,
        "/api/v1/create",
        &frames,
        &directory.path().join("imp"),
        "Blocking",
    );
    let waiting = submit(
        address,
        "/api/v1/validate",
        &directory.path().join("no_such_imp"),
        Path::new(""),
        "waiting",
    );

    let holder = request(
        address,
        "GET",
        &format!("/api/v1/jobs/{blocking}"),
        Some(API_KEY),
    );
    let holder_state = holder.json()["state"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        holder_state == "Queued" || holder_state == "Running",
        "the first job was already {holder_state}, so the second was not waiting"
    );

    let cancelled = request(
        address,
        "DELETE",
        &format!("/api/v1/jobs/{waiting}"),
        Some(API_KEY),
    );
    assert_eq!(cancelled.status, 200, "{}", cancelled.body);
    assert_eq!(cancelled.json()["cancelled"], true);

    let answer = request(
        address,
        "GET",
        &format!("/api/v1/jobs/{waiting}"),
        Some(API_KEY),
    );
    assert_eq!(answer.json()["state"], "Cancelled");

    wait_for_job(address, blocking);
    assert_eq!(
        request(
            address,
            "DELETE",
            &format!("/api/v1/jobs/{blocking}"),
            Some(API_KEY)
        )
        .status,
        404,
        "a finished job cannot be cancelled"
    );
    assert_eq!(
        request(address, "DELETE", "/api/v1/jobs/nine", Some(API_KEY)).status,
        400
    );
}

#[test]
fn metrics_count_what_the_job_list_holds() {
    let address = serve();
    let directory = TempDir::new().unwrap();
    let missing = directory.path().join("no_such_imp");

    for _ in 0..3 {
        let id = submit(
            address,
            "/api/v1/validate",
            &missing,
            Path::new(""),
            "count",
        );
        let job = wait_for_job(address, id);
        assert_eq!(job["state"], "Failed", "{job}");
    }

    let answer = request(address, "GET", "/metrics", Some(API_KEY));
    assert_eq!(answer.status, 200);
    assert!(
        answer
            .body
            .contains(r#"imfwizard_jobs_total{state="failed"} 3"#),
        "{}",
        answer.body
    );
    assert!(
        answer
            .body
            .contains(r#"imfwizard_jobs_total{state="completed"} 0"#),
        "{}",
        answer.body
    );
}

#[test]
fn an_unknown_path_is_a_404() {
    let address = serve();
    let answer = request(address, "GET", "/api/v1/jobsXYZ", Some(API_KEY));
    assert_eq!(
        answer.status, 404,
        "a longer path must not reach the job list"
    );
}

#[test]
fn an_unknown_field_is_refused_by_name() {
    let address = serve();
    let body = r#"{"input":"/tmp/imp","imp_dir":"/tmp/other"}"#;
    let answer = request_with_body(address, "POST", "/api/v1/validate", body, Some(API_KEY));
    assert_eq!(answer.status, 400, "{}", answer.body);
    let error = answer.json()["error"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        error.contains("imp_dir"),
        "the error must name the field, got {error:?}"
    );

    let answer = request_with_body(
        address,
        "POST",
        "/api/v1/validate",
        "not json",
        Some(API_KEY),
    );
    assert_eq!(answer.status, 400, "{}", answer.body);
}
