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

fn ffmpeg(arguments: &[&str]) {
    let out = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error"])
        .args(arguments)
        .output()
        .expect("ffmpeg");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// tiff frames, the one image format imfwizard reads without ffmpeg
fn tiff_sequence(root: &Path) -> std::path::PathBuf {
    let frames = root.join("frames");
    std::fs::create_dir_all(&frames).unwrap();
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("testsrc=s={WIDTH}x{HEIGHT}:r=24"),
        "-frames:v",
        &FRAMES.to_string(),
        "-pix_fmt",
        "rgb24",
        &frames.join("frame_%06d.tif").to_string_lossy(),
    ]);
    frames
}

fn testsrc_clip(root: &Path) -> std::path::PathBuf {
    let clip = root.join("source.mp4");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=s=320x180:r=24",
        "-frames:v",
        "12",
        "-pix_fmt",
        "yuv420p",
        &clip.to_string_lossy(),
    ]);
    clip
}

fn video_stream_entry(path: &Path, entry: &str) -> String {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .arg(format!("stream={entry}"))
        .args(["-of", "default=nw=1:nk=1"])
        .arg(path)
        .output()
        .expect("ffprobe");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

// matroska records no frame count in its header, so they are counted on the way past
fn counted_frames(path: &Path) -> u32 {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-count_frames"])
        .args(["-show_entries", "stream=nb_read_frames"])
        .args(["-of", "default=nw=1:nk=1"])
        .arg(path)
        .output()
        .expect("ffprobe");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

/// `/tools` is the `doctor` check over HTTP: the body has to be the real probe,
/// so ffmpeg (which these tests just ran) must come back available.
#[test]
fn tools_reports_the_dependencies_the_doctor_probes() {
    let address = serve();
    let answer = request(address, "GET", "/api/v1/tools", Some(API_KEY));
    assert_eq!(answer.status, 200, "{}", answer.body);

    let body = answer.json();
    let tools = body["tools"].as_array().expect("a tools array");
    let ffmpeg = tools
        .iter()
        .find(|tool| tool["name"] == "ffmpeg")
        .expect("ffmpeg among the probed tools");
    assert_eq!(
        ffmpeg["status"], "available",
        "ffmpeg ran in this test, so the probe must find it: {body}"
    );
    assert!(
        ffmpeg["version"].as_str().is_some_and(|v| !v.is_empty()),
        "the probe must report a version: {ffmpeg}"
    );
    assert_eq!(
        body["available"].as_u64().unwrap_or_default() as usize
            + body["missing"].as_u64().unwrap_or_default() as usize,
        tools.len(),
        "every tool is counted once: {body}"
    );
}

/// `/profiles` serves the delivery presets themselves, values and all.
#[test]
fn profiles_serves_every_delivery_preset_with_its_values() {
    let address = serve();
    let answer = request(address, "GET", "/api/v1/profiles", Some(API_KEY));
    assert_eq!(answer.status, 200, "{}", answer.body);

    let served = answer.json();
    let served = served.as_array().expect("an array of presets");
    let expected = imfwizard_core::profiles::all_profiles();
    assert_eq!(served.len(), expected.len());

    let netflix = served
        .iter()
        .find(|profile| profile["name"] == "Netflix IMF")
        .expect("the Netflix preset");
    assert_eq!(netflix["width"], 3840);
    assert_eq!(netflix["height"], 2160);
    assert_eq!(netflix["bitrate_mbps"], 800.0);
    assert_eq!(netflix["colour_space"], "BT.709 RGB full range");

    let cinema = served
        .iter()
        .find(|profile| profile["name"] == "DCI 2K Theatrical")
        .expect("the DCI 2K preset");
    assert_eq!(cinema["width"], 2048);
    assert_eq!(cinema["bitrate_mbps"], 250.0);
}

/// `/encode` runs the App 2E encoder, so the job leaves one codestream a frame.
#[test]
fn an_encode_job_writes_a_codestream_for_every_frame() {
    let address = serve();
    let directory = TempDir::new().unwrap();
    let frames = tiff_sequence(directory.path());
    let output = directory.path().join("encoded");

    let id = submit(address, "/api/v1/encode", &frames, &output, "encode");
    let job = wait_for_job(address, id);
    assert_eq!(job["state"], "Completed", "{job}");

    let codestreams: Vec<_> = std::fs::read_dir(output.join("j2k"))
        .expect("the j2k output directory")
        .flatten()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "j2c" || extension == "j2k")
        })
        .collect();
    assert_eq!(codestreams.len(), FRAMES);
    for codestream in &codestreams {
        let bytes = std::fs::read(codestream.path()).unwrap();
        assert_eq!(
            &bytes[..4],
            &[0xff, 0x4f, 0xff, 0x51],
            "{:?} does not start with the JPEG 2000 SOC and SIZ markers",
            codestream.path()
        );
    }
}

/// `/transcode` runs ffmpeg, so the job leaves a file the prober can read back.
#[test]
fn a_transcode_job_writes_a_playable_file() {
    let address = serve();
    let directory = TempDir::new().unwrap();
    let clip = testsrc_clip(directory.path());
    let output = directory.path().join("transcoded.mkv");

    let id = submit(address, "/api/v1/transcode", &clip, &output, "transcode");
    let job = wait_for_job(address, id);
    assert_eq!(job["state"], "Completed", "{job}");

    assert!(output.is_file(), "the transcode wrote no file");
    assert_eq!(video_stream_entry(&output, "width"), "320");
    assert_eq!(video_stream_entry(&output, "height"), "180");
    assert_eq!(counted_frames(&output), 12);
}

/// A transcode ffmpeg cannot run fails the job in ffmpeg's own words.
#[test]
fn a_transcode_job_on_a_file_ffmpeg_cannot_read_fails() {
    let address = serve();
    let directory = TempDir::new().unwrap();
    let broken = directory.path().join("broken.mp4");
    std::fs::write(&broken, b"not a container").unwrap();

    let id = submit(
        address,
        "/api/v1/transcode",
        &broken,
        &directory.path().join("out.mkv"),
        "broken",
    );
    let job = wait_for_job(address, id);
    assert_eq!(job["state"], "Failed", "{job}");
    let error = job["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("broken.mp4"),
        "the error must name the input, got {error:?}"
    );
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
