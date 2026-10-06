/// REST API for IMF Wizard.
///
/// Provides HTTP endpoints for IMP creation, validation, encoding,
/// transcoding, and job management through a queue that keeps its jobs in a
/// jobs file across restarts.
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use postkit::rest_api::{Request, RestServer, RouteResponse};

use postkit::job_queue::JobState;

use crate::executor::{JobType, RestJob, RestJobQueue};
use crate::tools;

/// API server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    pub host: String,
    pub port: u16,
    pub api_key: Option<String>,
    pub encode_threads: u32,
    pub jobs_file: PathBuf,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 8081,
            api_key: None,
            encode_threads: crate::preferences::AUTOMATIC_ENCODE_THREADS,
            jobs_file: postkit::job_queue::jobs_path(
                JOBS_FILE_VARIABLE,
                crate::store::data_dir().join(JOBS_FILE_NAME),
            ),
        }
    }
}

const JOBS_FILE_VARIABLE: &str = "IMFWIZARD_REST_JOBS_FILE";
const JOBS_FILE_NAME: &str = "rest-jobs.jsonl";

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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveRequest {
    before: Option<u64>,
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
/// - `GET  /api/v1/health`          , health check
/// - `POST /api/v1/create`          , submit IMP creation job
/// - `POST /api/v1/validate`        , submit validation job
/// - `POST /api/v1/encode`          , submit encoding job
/// - `POST /api/v1/transcode`       , submit transcode job
/// - `GET  /api/v1/jobs`            , list all jobs
/// - `GET  /api/v1/jobs/<id>`       , job status
/// - `DELETE /api/v1/jobs/<id>`     , cancel job
/// - `POST /api/v1/jobs/<id>/move`  , run a queued job next, or before `before`
/// - `GET  /api/v1/profiles`        , list delivery presets
/// - `GET  /api/v1/tools`           , dependency check
/// - `POST /api/v1/pause`           , refuse new submissions
/// - `POST /api/v1/resume`          , accept submissions again
/// - `GET  /metrics`                , Prometheus metrics
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

    let queue = Arc::new(RestJobQueue::new(config.jobs_file.clone()));
    let skipped = queue.load_jobs_file();
    if skipped > 0 {
        tracing::warn!(
            "skipped {skipped} unreadable lines in {}",
            config.jobs_file.display()
        );
    }
    let paused = Arc::new(AtomicBool::new(false));

    let _worker_stop = crate::executor::spawn_worker(&queue, config.encode_threads);

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
                serde_json::to_string(&jobs.snapshot()).unwrap_or_else(|_| "[]".into()),
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

    let jobs = queue.clone();
    server.route_with_parameter_and_suffix(
        "POST",
        "/api/v1/jobs/",
        "/move",
        Box::new(move |request, id| {
            let Ok(id) = id.parse::<u64>() else {
                return (400, r#"{"error":"invalid job id"}"#.into());
            };
            let before = if request.body.is_empty() {
                None
            } else {
                match parse_body::<MoveRequest>(&request.body) {
                    Ok(parsed) => parsed.before,
                    Err(refused) => return refused,
                }
            };
            if jobs.move_before(id, before) {
                return (200, r#"{"moved":true}"#.into());
            }
            (404, r#"{"error":"job not found or not queued"}"#.into())
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

fn metrics(queue: &RestJobQueue) -> String {
    let jobs = queue.snapshot();
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
    queue: &RestJobQueue,
    paused: &AtomicBool,
) -> (u16, String) {
    if paused.load(Ordering::Relaxed) {
        return (503, r#"{"error":"queue is paused"}"#.into());
    }

    let parsed: JobRequest = match parse_body(&request.body) {
        Ok(parsed) => parsed,
        Err(refused) => return refused,
    };

    let id = queue.reserve_job_id();
    queue.submit(RestJob {
        id,
        job_type,
        title: parsed.title,
        input: PathBuf::from(&parsed.input),
        output: PathBuf::from(&parsed.output),
    });
    (202, format!(r#"{{"id":{id},"status":"queued"}}"#))
}

// read as a map first, serde would otherwise fill a struct from a JSON array
fn parse_body<T: DeserializeOwned>(body: &str) -> Result<T, (u16, String)> {
    serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(body)
        .and_then(|object| serde_json::from_value(serde_json::Value::Object(object)))
        .map_err(|e| {
            (
                400,
                serde_json::json!({ "error": e.to_string() }).to_string(),
            )
        })
}
