/// REST API for IMF Wizard.
///
/// Provides HTTP endpoints for IMP creation, validation, encoding,
/// transcoding, and job management via the integrated job queue.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::job_queue::{Job, JobQueue, JobState, JobType};
use crate::tools;

/// API server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    pub host: String,
    pub port: u16,
    pub api_key: Option<String>,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 8081,
            api_key: None,
        }
    }
}

/// Start the REST API server.
///
/// Endpoints:
/// - `GET  /api/v1/health`      — health check
/// - `POST /api/v1/create`      — submit IMP creation job
/// - `POST /api/v1/validate`    — submit validation job
/// - `POST /api/v1/encode`      — submit encoding job
/// - `POST /api/v1/transcode`   — submit transcode job
/// - `GET  /api/v1/jobs`        — list all jobs
/// - `GET  /api/v1/jobs/<id>`   — job status
/// - `DELETE /api/v1/jobs/<id>` — cancel job
/// - `GET  /api/v1/profiles`    — list delivery presets
/// - `GET  /api/v1/tools`       — dependency check
/// - `POST /api/v1/pause`       — pause job queue
/// - `POST /api/v1/resume`      — resume job queue
/// - `GET  /metrics`            — Prometheus metrics
pub fn start_server(config: &ApiConfig) -> Result<(), String> {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    let addr = format!("{}:{}", config.host, config.port);
    let listener =
        TcpListener::bind(&addr).map_err(|e| format!("Failed to bind to {addr}: {e}"))?;

    tracing::info!("IMF Wizard REST API listening on {addr}");

    let queue = JobQueue::new();
    let paused = Arc::new(AtomicBool::new(false));

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to accept connection: {e}");
                continue;
            }
        };

        let mut buf = [0u8; 65536];
        let n = match stream.read(&mut buf) {
            Ok(n) => n,
            Err(_) => continue,
        };

        let request = String::from_utf8_lossy(&buf[..n]);
        let first_line = request.lines().next().unwrap_or("");
        let body = request
            .split("\r\n\r\n")
            .nth(1)
            .or_else(|| request.split("\n\n").nth(1))
            .unwrap_or("");

        // API key check
        if let Some(ref key) = config.api_key {
            let has_key = request.contains(&format!("X-Api-Key: {key}"))
                || request.contains(&format!("Authorization: Bearer {key}"));
            if !has_key && !first_line.contains("/api/v1/health") {
                let resp = json_response("401 Unauthorized", r#"{"error":"unauthorized"}"#);
                let _ = stream.write_all(resp.as_bytes());
                continue;
            }
        }

        let (status, response_body) = route(first_line, body, &queue, &paused);

        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\n\r\n{response_body}",
            response_body.len()
        );
        let _ = stream.write_all(response.as_bytes());
    }

    Ok(())
}

fn route(
    first_line: &str,
    body: &str,
    queue: &JobQueue,
    paused: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> (&'static str, String) {
    use std::sync::atomic::Ordering;

    // GET /api/v1/health
    if first_line.starts_with("GET /api/v1/health") || first_line.starts_with("GET /health") {
        return ("200 OK", r#"{"status":"ok","version":"1.0.0"}"#.into());
    }

    // GET /api/v1/tools
    if first_line.starts_with("GET /api/v1/tools") {
        let result = tools::check_all_tools();
        let json = serde_json::to_string(&result).unwrap_or_else(|_| "{}".into());
        return ("200 OK", json);
    }

    // GET /api/v1/profiles
    if first_line.starts_with("GET /api/v1/profiles") {
        let profiles = crate::profiles::all_profiles();
        let json = serde_json::to_string(&profiles).unwrap_or_else(|_| "[]".into());
        return ("200 OK", json);
    }

    // GET /api/v1/jobs/<id>
    if first_line.starts_with("GET /api/v1/jobs/") {
        let id_str = first_line
            .strip_prefix("GET /api/v1/jobs/")
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("");
        if let Ok(id) = id_str.parse::<u64>() {
            if let Some(job) = queue.get(id) {
                let json = serde_json::to_string(&job).unwrap_or_else(|_| "{}".into());
                return ("200 OK", json);
            }
            return ("404 Not Found", r#"{"error":"job not found"}"#.into());
        }
        return ("400 Bad Request", r#"{"error":"invalid job id"}"#.into());
    }

    // DELETE /api/v1/jobs/<id>
    if first_line.starts_with("DELETE /api/v1/jobs/") {
        let id_str = first_line
            .strip_prefix("DELETE /api/v1/jobs/")
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("");
        if let Ok(id) = id_str.parse::<u64>() {
            if queue.cancel(id) {
                return ("200 OK", r#"{"cancelled":true}"#.into());
            }
            return (
                "404 Not Found",
                r#"{"error":"job not found or not cancellable"}"#.into(),
            );
        }
        return ("400 Bad Request", r#"{"error":"invalid job id"}"#.into());
    }

    // GET /api/v1/jobs
    if first_line.starts_with("GET /api/v1/jobs") {
        let jobs = queue.list();
        let json = serde_json::to_string(&jobs).unwrap_or_else(|_| "[]".into());
        return ("200 OK", json);
    }

    // POST /api/v1/create
    if first_line.starts_with("POST /api/v1/create") {
        return submit_job(body, JobType::Create, queue, paused);
    }

    // POST /api/v1/validate
    if first_line.starts_with("POST /api/v1/validate") {
        return submit_job(body, JobType::Validate, queue, paused);
    }

    // POST /api/v1/encode
    if first_line.starts_with("POST /api/v1/encode") {
        return submit_job(body, JobType::Encode, queue, paused);
    }

    // POST /api/v1/transcode
    if first_line.starts_with("POST /api/v1/transcode") {
        return submit_job(body, JobType::Transcode, queue, paused);
    }

    // POST /api/v1/pause
    if first_line.starts_with("POST /api/v1/pause") {
        paused.store(true, Ordering::Relaxed);
        return ("200 OK", r#"{"paused":true}"#.into());
    }

    // POST /api/v1/resume
    if first_line.starts_with("POST /api/v1/resume") {
        paused.store(false, Ordering::Relaxed);
        return ("200 OK", r#"{"paused":false}"#.into());
    }

    // GET /metrics
    if first_line.starts_with("GET /metrics") {
        let jobs = queue.list();
        let queued = jobs.iter().filter(|j| j.state == JobState::Queued).count();
        let running = jobs.iter().filter(|j| j.state == JobState::Running).count();
        let completed = jobs
            .iter()
            .filter(|j| j.state == JobState::Completed)
            .count();
        let failed = jobs.iter().filter(|j| j.state == JobState::Failed).count();
        let metrics = format!(
            "# HELP imfwizard_jobs_total Total jobs by state\n\
             # TYPE imfwizard_jobs_total gauge\n\
             imfwizard_jobs_total{{state=\"queued\"}} {queued}\n\
             imfwizard_jobs_total{{state=\"running\"}} {running}\n\
             imfwizard_jobs_total{{state=\"completed\"}} {completed}\n\
             imfwizard_jobs_total{{state=\"failed\"}} {failed}\n"
        );
        return ("200 OK", metrics);
    }

    ("404 Not Found", r#"{"error":"not found"}"#.into())
}

fn submit_job(
    body: &str,
    job_type: JobType,
    queue: &JobQueue,
    paused: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> (&'static str, String) {
    use std::sync::atomic::Ordering;

    if paused.load(Ordering::Relaxed) {
        return (
            "503 Service Unavailable",
            r#"{"error":"queue is paused"}"#.into(),
        );
    }

    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let input = parsed["input"]
        .as_str()
        .or_else(|| parsed["imp_dir"].as_str())
        .unwrap_or("")
        .to_string();
    let output = parsed["output"]
        .as_str()
        .or_else(|| parsed["output_dir"].as_str())
        .unwrap_or("")
        .to_string();
    let description = parsed["title"]
        .as_str()
        .or_else(|| parsed["description"].as_str())
        .unwrap_or("")
        .to_string();

    let job = Job {
        job_type,
        description,
        input: PathBuf::from(&input),
        output: PathBuf::from(&output),
        ..Default::default()
    };

    let id = queue.submit(job);
    (
        "202 Accepted",
        format!(r#"{{"id":{id},"status":"queued"}}"#),
    )
}

fn json_response(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}
