use serde::Serialize;
use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

// ─── Progress / Events ─────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct PipelineProgress {
    pub job_id: u64,
    pub stage: String,
    pub message: String,
    pub frame: u64,
    pub total_frames: u64,
    pub fps: f64,
    pub elapsed_secs: f64,
    pub percent: f64,
}

#[derive(Clone, Serialize)]
pub struct JobInfo {
    pub id: u64,
    pub title: String,
    pub status: String,
    pub percent: f64,
}

// ─── Job types ─────────────────────────────────────────────────────────────

#[derive(Clone)]
#[allow(dead_code)]
struct JobConfig {
    id: u64,
    video_path: PathBuf,
    title: String,
    output_dir: PathBuf,
    audio_path: Option<String>,
    subtitles: Vec<String>,
    fps_num: u32,
    fps_den: u32,
    content_kind: String,
    bandwidth: u32,
}

// ─── Queue state (managed by Tauri) ────────────────────────────────────────

pub struct JobQueue {
    queue: Mutex<VecDeque<JobConfig>>,
    next_id: AtomicU64,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    current_id: AtomicU64,
    current_title: Mutex<String>,
    current_status: Mutex<String>,
}

impl JobQueue {
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            next_id: AtomicU64::new(1),
            cancel: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
            current_id: AtomicU64::new(0),
            current_title: Mutex::new(String::new()),
            current_status: Mutex::new(String::new()),
        }
    }
}

// ─── Tauri commands ────────────────────────────────────────────────────────

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn submit_job(
    app: AppHandle,
    video_path: String,
    title: String,
    output_dir: String,
    audio_path: Option<String>,
    subtitles: Option<Vec<String>>,
    framerate: Option<String>,
    content_kind: Option<String>,
    bandwidth: Option<u32>,
) -> Result<u64, String> {
    let queue = app.state::<JobQueue>();
    let id = queue.next_id.fetch_add(1, Ordering::Relaxed);

    let (fps_num, fps_den) = match framerate.as_deref() {
        Some("25/1") => (25, 1),
        Some("30000/1001") => (30000, 1001),
        Some("30/1") => (30, 1),
        Some("48/1") => (48, 1),
        Some("60000/1001") => (60000, 1001),
        Some("60/1") => (60, 1),
        _ => (24, 1),
    };

    let job = JobConfig {
        id,
        video_path: PathBuf::from(&video_path),
        title: title.clone(),
        output_dir: PathBuf::from(&output_dir),
        audio_path,
        subtitles: subtitles.unwrap_or_default(),
        fps_num,
        fps_den,
        content_kind: content_kind.unwrap_or_else(|| "feature".to_string()),
        bandwidth: bandwidth.unwrap_or(250),
    };

    {
        let mut q = queue.queue.lock().unwrap();
        q.push_back(job);
    }

    if queue.current_id.load(Ordering::Relaxed) == 0 {
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            run_queue_worker(app2).await;
        });
    }

    Ok(id)
}

#[tauri::command]
pub async fn cancel_job(app: AppHandle, job_id: u64) -> Result<(), String> {
    let queue = app.state::<JobQueue>();
    if queue.current_id.load(Ordering::Relaxed) == job_id {
        queue.cancel.store(true, Ordering::Relaxed);
        return Ok(());
    }
    let mut q = queue.queue.lock().unwrap();
    q.retain(|j| j.id != job_id);
    Ok(())
}

#[tauri::command]
pub async fn pause_job(app: AppHandle) -> Result<(), String> {
    let queue = app.state::<JobQueue>();
    queue.pause.store(true, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub async fn resume_job(app: AppHandle) -> Result<(), String> {
    let queue = app.state::<JobQueue>();
    queue.pause.store(false, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub async fn list_jobs(app: AppHandle) -> Vec<JobInfo> {
    let queue = app.state::<JobQueue>();
    let mut jobs = Vec::new();

    let current_id = queue.current_id.load(Ordering::Relaxed);
    if current_id > 0 {
        let title = queue.current_title.lock().unwrap().clone();
        let status = queue.current_status.lock().unwrap().clone();
        jobs.push(JobInfo {
            id: current_id,
            title,
            status,
            percent: 0.0,
        });
    }

    let q = queue.queue.lock().unwrap();
    for job in q.iter() {
        jobs.push(JobInfo {
            id: job.id,
            title: job.title.clone(),
            status: "queued".to_string(),
            percent: 0.0,
        });
    }
    jobs
}

// ─── Queue worker ──────────────────────────────────────────────────────────

async fn run_queue_worker(app: AppHandle) {
    loop {
        let job = {
            let queue = app.state::<JobQueue>();
            let mut q = queue.queue.lock().unwrap();
            q.pop_front()
        };

        let Some(job) = job else {
            let queue = app.state::<JobQueue>();
            queue.current_id.store(0, Ordering::Relaxed);
            break;
        };

        {
            let queue = app.state::<JobQueue>();
            queue.current_id.store(job.id, Ordering::Relaxed);
            *queue.current_title.lock().unwrap() = job.title.clone();
            *queue.current_status.lock().unwrap() = "running".to_string();
            queue.cancel.store(false, Ordering::Relaxed);
            queue.pause.store(false, Ordering::Relaxed);
        }

        let result = tokio::task::spawn_blocking({
            let app = app.clone();
            let job = job.clone();
            move || run_job(&app, &job)
        })
        .await;

        let queue = app.state::<JobQueue>();
        match result {
            Ok(Ok(_)) => {
                *queue.current_status.lock().unwrap() = "done".to_string();
                emit_progress(&app, job.id, "done", "Complete", 0, 0, 0.0, 0.0, 100.0);
            }
            Ok(Err(e)) => {
                let status = if queue.cancel.load(Ordering::Relaxed) {
                    "cancelled".to_string()
                } else {
                    format!("failed: {e}")
                };
                *queue.current_status.lock().unwrap() = status;
                emit_progress(&app, job.id, "error", &e, 0, 0, 0.0, 0.0, 0.0);
            }
            Err(e) => {
                *queue.current_status.lock().unwrap() = format!("panic: {e}");
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

// ─── Job execution ─────────────────────────────────────────────────────────

fn log_to(log_file: &Arc<Mutex<Option<std::fs::File>>>, msg: &str) {
    eprintln!("[pipeline] {msg}");
    if let Some(f) = log_file.lock().unwrap().as_mut() {
        let _ = writeln!(f, "{msg}");
    }
}

fn run_job(app: &AppHandle, job: &JobConfig) -> Result<String, String> {
    let queue = app.state::<JobQueue>();
    let cancel = queue.cancel.clone();
    let pause = queue.pause.clone();

    let output = &job.output_dir;
    let log_path = output.join("imfwizard.log");
    let log_file: Arc<Mutex<Option<std::fs::File>>> =
        Arc::new(Mutex::new(std::fs::File::create(&log_path).ok()));

    log_to(&log_file, "=== IMF Wizard Pipeline ===");
    log_to(&log_file, &format!("Job ID: {}", job.id));
    log_to(&log_file, &format!("Title: {}", job.title));
    log_to(&log_file, &format!("Input: {}", job.video_path.display()));
    log_to(&log_file, &format!("Output: {}", output.display()));
    log_to(
        &log_file,
        &format!(
            "Started: {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        ),
    );

    // Map the target bandwidth (Mbps) to a J2K compression ratio, matching the
    // dcpwizard CLI convention (raw = w*h*36 bits/frame). Only honoured for video
    // input; image/J2K sequences fall back to the encoder default.
    let compression_ratio = imfwizard_core::probe::probe_video(&job.video_path)
        .map(|info| {
            let fps = (info.fps_num as f64 / info.fps_den.max(1) as f64).max(1.0);
            let raw_bits = info.width as f64 * info.height as f64 * 36.0;
            let target_bits = (job.bandwidth as f64 * 1_000_000.0) / fps;
            (raw_bits / target_bits).max(1.0)
        })
        .unwrap_or(10.0);

    // Encode using shared pipeline
    let job_id = job.id;
    let app_ref = app.clone();
    let log_ref = log_file.clone();
    let encode_result = postkit::pipeline::run_encode_with_ratio(
        &job.video_path,
        output,
        compression_ratio,
        job.fps_num,
        &cancel,
        &pause,
        |p| {
            emit_progress(
                &app_ref,
                job_id,
                &p.stage,
                &p.message,
                p.frame,
                p.total_frames,
                p.fps,
                p.elapsed_secs,
                p.percent,
            );
        },
        |msg| log_to(&log_ref, msg),
    )?;

    // Package IMP
    emit_progress(
        app,
        job.id,
        "package",
        "Creating IMP...",
        0,
        0,
        0.0,
        0.0,
        99.0,
    );
    log_to(&log_file, "[PACKAGE] Creating IMP...");

    let opts = imfwizard_core::imp::ImpOptions {
        title: job.title.clone(),
        output_dir: job.output_dir.clone(),
        fps_num: job.fps_num,
        fps_den: job.fps_den,
        content_kind: job.content_kind.clone(),
        j2k_dir: Some(encode_result.j2k_dir.clone()),
        audio_files: job.audio_path.iter().map(PathBuf::from).collect(),
        timed_text_files: job.subtitles.iter().map(PathBuf::from).collect(),
        ..Default::default()
    };

    let result = imfwizard_core::imp::create_imp(&opts);
    if !result.success {
        log_to(&log_file, &format!("[PACKAGE] FAILED: {}", result.error));
        return Err(format!("IMP packaging failed: {}", result.error));
    }
    log_to(&log_file, "[PACKAGE] Done");

    log_to(
        &log_file,
        &format!(
            "=== Pipeline finished in {:.1}s ===",
            encode_result.elapsed_secs
        ),
    );
    Ok(format!("IMP created in {:.1}s", encode_result.elapsed_secs))
}

// ─── Helpers ───────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn emit_progress(
    app: &AppHandle,
    job_id: u64,
    stage: &str,
    message: &str,
    frame: u64,
    total_frames: u64,
    fps: f64,
    elapsed_secs: f64,
    percent: f64,
) {
    let _ = app.emit(
        "pipeline-progress",
        PipelineProgress {
            job_id,
            stage: stage.to_string(),
            message: message.to_string(),
            frame,
            total_frames,
            fps,
            elapsed_secs,
            percent,
        },
    );
}
