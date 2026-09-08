/// REST API for IMF Wizard.
///
/// Provides HTTP endpoints for IMP creation, validation, encoding,
/// transcoding, and job management via the integrated job queue.
use serde::{Deserialize, Serialize};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use postkit::rest_api::{Request, RestServer, RouteResponse};

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

// the only paths an API key is not required on
const HEALTH_PATHS: [&str; 2] = ["/api/v1/health", "/health"];

const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// What a job submission body carries. Anything else in the JSON object is a
/// 400 naming the field, so a misspelt key cannot become a silently empty path.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct JobRequest {
    #[serde(default)]
    input: String,
    #[serde(default)]
    output: String,
    #[serde(default)]
    title: String,
}

/// Bind the API without serving it, so a caller can read the address it got
/// when the port was 0. The background worker is already running.
pub fn bind_server(config: &ApiConfig) -> Result<(RestServer, TcpListener), String> {
    let server = build_server(config);
    let listener = server
        .bind()
        .map_err(|e| format!("Failed to bind to {}: {e}", server.bind_address))?;
    Ok((server, listener))
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
/// - `POST /api/v1/pause`       — refuse new submissions
/// - `POST /api/v1/resume`      — accept submissions again
/// - `GET  /metrics`            — Prometheus metrics
pub fn start_server(config: &ApiConfig) -> Result<(), String> {
    let (server, listener) = bind_server(config)?;
    tracing::info!(
        "IMF Wizard REST API listening on {}",
        listener
            .local_addr()
            .map(|address| address.to_string())
            .unwrap_or_else(|_| server.bind_address.clone())
    );
    server
        .serve_forever(listener)
        .map_err(|e| format!("REST API server failed: {e}"))
}

fn build_server(config: &ApiConfig) -> RestServer {
    let mut server = RestServer::new(&format!("{}:{}", config.host, config.port));
    if let Some(key) = &config.api_key {
        server.require_api_key(key, &HEALTH_PATHS);
    }

    let queue = JobQueue::new();
    let paused = Arc::new(AtomicBool::new(false));

    // Background worker runs submitted jobs. The queue is in-memory, so jobs
    // live only for the lifetime of this server process.
    let _worker_stop = crate::executor::spawn_worker(&queue);

    for path in HEALTH_PATHS {
        server.route(
            "GET",
            path,
            Box::new(|_request| {
                (
                    200,
                    format!(
                        r#"{{"status":"ok","version":"{}"}}"#,
                        env!("CARGO_PKG_VERSION")
                    ),
                )
            }),
        );
    }

    server.route(
        "GET",
        "/api/v1/tools",
        Box::new(|_request| {
            let result = tools::check_all_tools();
            (
                200,
                serde_json::to_string(&result).unwrap_or_else(|_| "{}".into()),
            )
        }),
    );

    server.route(
        "GET",
        "/api/v1/profiles",
        Box::new(|_request| {
            let profiles = crate::profiles::all_profiles();
            (
                200,
                serde_json::to_string(&profiles).unwrap_or_else(|_| "[]".into()),
            )
        }),
    );

    let jobs = queue.clone();
    server.route(
        "GET",
        "/api/v1/jobs",
        Box::new(move |_request| {
            (
                200,
                serde_json::to_string(&jobs.list()).unwrap_or_else(|_| "[]".into()),
            )
        }),
    );

    let jobs = queue.clone();
    server.route_with_parameter(
        "GET",
        "/api/v1/jobs/",
        Box::new(move |_request, id| {
            let Ok(id) = id.parse::<u64>() else {
                return (400, r#"{"error":"invalid job id"}"#.into());
            };
            match jobs.get(id) {
                Some(job) => (
                    200,
                    serde_json::to_string(&job).unwrap_or_else(|_| "{}".into()),
                ),
                None => (404, r#"{"error":"job not found"}"#.into()),
            }
        }),
    );

    let jobs = queue.clone();
    server.route_with_parameter(
        "DELETE",
        "/api/v1/jobs/",
        Box::new(move |_request, id| {
            let Ok(id) = id.parse::<u64>() else {
                return (400, r#"{"error":"invalid job id"}"#.into());
            };
            if jobs.cancel(id) {
                return (200, r#"{"cancelled":true}"#.into());
            }
            (
                404,
                r#"{"error":"job not found or not cancellable"}"#.into(),
            )
        }),
    );

    for (path, job_type) in [
        ("/api/v1/create", JobType::Create),
        ("/api/v1/validate", JobType::Validate),
        ("/api/v1/encode", JobType::Encode),
        ("/api/v1/transcode", JobType::Transcode),
    ] {
        let jobs = queue.clone();
        let paused = paused.clone();
        server.route(
            "POST",
            path,
            Box::new(move |request| submit_job(request, job_type, &jobs, &paused)),
        );
    }

    for (path, wanted) in [("/api/v1/pause", true), ("/api/v1/resume", false)] {
        let paused = paused.clone();
        server.route(
            "POST",
            path,
            Box::new(move |_request| {
                paused.store(wanted, Ordering::Relaxed);
                (200, format!(r#"{{"paused":{wanted}}}"#))
            }),
        );
    }

    let jobs = queue.clone();
    server.route_with_content_type(
        "GET",
        "/metrics",
        Box::new(move |_request| RouteResponse {
            status: 200,
            content_type: PROMETHEUS_CONTENT_TYPE,
            body: metrics(&jobs),
        }),
    );

    server
}

fn metrics(queue: &JobQueue) -> String {
    let jobs = queue.list();
    let count = |state: JobState| jobs.iter().filter(|job| job.state == state).count();
    let queued = count(JobState::Queued);
    let running = count(JobState::Running);
    let completed = count(JobState::Completed);
    let failed = count(JobState::Failed);
    format!(
        "# HELP imfwizard_jobs_total Total jobs by state\n\
         # TYPE imfwizard_jobs_total gauge\n\
         imfwizard_jobs_total{{state=\"queued\"}} {queued}\n\
         imfwizard_jobs_total{{state=\"running\"}} {running}\n\
         imfwizard_jobs_total{{state=\"completed\"}} {completed}\n\
         imfwizard_jobs_total{{state=\"failed\"}} {failed}\n"
    )
}

fn submit_job(
    request: &Request,
    job_type: JobType,
    queue: &JobQueue,
    paused: &AtomicBool,
) -> (u16, String) {
    if paused.load(Ordering::Relaxed) {
        return (503, r#"{"error":"queue is paused"}"#.into());
    }

    let parsed: JobRequest = match serde_json::from_str(&request.body) {
        Ok(parsed) => parsed,
        Err(e) => {
            return (
                400,
                serde_json::json!({ "error": e.to_string() }).to_string(),
            );
        }
    };

    let id = queue.submit(Job {
        job_type,
        description: parsed.title,
        input: PathBuf::from(&parsed.input),
        output: PathBuf::from(&parsed.output),
        ..Default::default()
    });
    (202, format!(r#"{{"id":{id},"status":"queued"}}"#))
}
