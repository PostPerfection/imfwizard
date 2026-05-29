use serde::Serialize;
use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

use postkit::encode::{
    detect_input_type, encode_parallel, find_compressor, stream_encode, InputType,
    ParallelProgress, StreamEncodeOptions, StreamProgress,
};

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
pub async fn submit_job(
    app: AppHandle,
    video_path: String,
    title: String,
    output_dir: String,
    audio_path: Option<String>,
) -> Result<u64, String> {
    let queue = app.state::<JobQueue>();
    let id = queue.next_id.fetch_add(1, Ordering::Relaxed);

    let job = JobConfig {
        id,
        video_path: PathBuf::from(&video_path),
        title: title.clone(),
        output_dir: PathBuf::from(&output_dir),
        audio_path,
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

macro_rules! log {
    ($file:expr, $($arg:tt)*) => {{
        let msg = format!($($arg)*);
        eprintln!("[pipeline] {}", msg);
        if let Some(f) = $file.as_mut() {
            let _ = writeln!(f, "{}", msg);
        }
    }};
}

fn run_job(app: &AppHandle, job: &JobConfig) -> Result<String, String> {
    let queue = app.state::<JobQueue>();
    let cancel = queue.cancel.clone();
    let pause = queue.pause.clone();

    let video = &job.video_path;
    let output = &job.output_dir;

    if !video.exists() {
        return Err(format!("Input not found: {}", video.display()));
    }

    std::fs::create_dir_all(output)
        .map_err(|e| format!("Failed to create output directory: {e}"))?;

    let log_path = output.join("imfwizard.log");
    let mut log_file: Option<std::fs::File> = std::fs::File::create(&log_path).ok();

    log!(log_file, "=== IMF Wizard Pipeline ===");
    log!(log_file, "Job ID: {}", job.id);
    log!(log_file, "Title: {}", job.title);
    log!(log_file, "Input: {}", video.display());
    log!(log_file, "Output: {}", output.display());
    log!(
        log_file,
        "Started: {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );

    let start_time = std::time::Instant::now();
    let input_type = detect_input_type(video);
    log!(log_file, "Input type: {:?}", input_type);

    let j2k_dir = output.join("j2k");

    match input_type {
        InputType::Video => {
            let (compressor_path, lib_dir) = find_compressor().ok_or("grk_compress not found")?;

            let opts = StreamEncodeOptions {
                input: video.clone(),
                output_dir: j2k_dir.clone(),
                compression_ratio: 10.0,
                num_resolutions: 6,
                codeblock_size: 32,
                progression: "CPRL".to_string(),
                fps: 24,
                compressor_path,
                lib_dir,
            };

            emit_progress(app, job.id, "encode", "Starting...", 0, 0, 0.0, 0.0, 0.0);

            let app_ref = app.clone();
            let job_id = job.id;
            let result = stream_encode(&opts, &cancel, &pause, |p: StreamProgress| {
                let percent = if p.total_frames > 0 {
                    (p.frame as f64 / p.total_frames as f64) * 100.0
                } else {
                    0.0
                };
                emit_progress(
                    &app_ref,
                    job_id,
                    "encode",
                    &format!("Frame {}/{}", p.frame, p.total_frames),
                    p.frame,
                    p.total_frames,
                    p.fps,
                    p.elapsed_secs,
                    percent.min(99.0),
                );
            });

            if !result.success {
                return Err(result.error);
            }
            log!(log_file, "[ENCODE] Done: {} frames", result.frames_encoded);
        }
        InputType::ImageSequence => {
            let input_dir = if video.is_dir() {
                video.clone()
            } else {
                video.parent().unwrap_or(video).to_path_buf()
            };

            emit_progress(
                app,
                job.id,
                "encode",
                "Encoding images...",
                0,
                0,
                0.0,
                0.0,
                0.0,
            );

            let app_ref = app.clone();
            let job_id = job.id;
            let result = encode_parallel(
                &input_dir,
                &j2k_dir,
                &cancel,
                &pause,
                |p: ParallelProgress| {
                    let percent = if p.total > 0 {
                        (p.done as f64 / p.total as f64) * 100.0
                    } else {
                        0.0
                    };
                    emit_progress(
                        &app_ref,
                        job_id,
                        "encode",
                        &format!("Frame {}/{}", p.done, p.total),
                        p.done,
                        p.total,
                        p.fps,
                        p.elapsed_secs,
                        percent.min(99.0),
                    );
                },
            );

            if !result.success {
                return Err(result.error);
            }
            log!(log_file, "[ENCODE] Done: {} frames", result.frames_encoded);
        }
        InputType::J2kSequence => {
            log!(log_file, "Input is already J2K, skipping encode");
        }
        InputType::Unknown => {
            return Err(format!("Cannot determine input type: {}", video.display()));
        }
    }

    if cancel.load(Ordering::Relaxed) {
        log!(log_file, "=== CANCELLED ===");
        return Err("Cancelled".to_string());
    }

    // Package as IMF
    let package_j2k = match input_type {
        InputType::J2kSequence => video.to_path_buf(),
        _ => j2k_dir,
    };
    package_imp(app, job, &package_j2k, &mut log_file)?;

    let total_elapsed = start_time.elapsed().as_secs_f64();
    log!(log_file, "=== Pipeline finished in {total_elapsed:.1}s ===");
    Ok(format!("IMP created in {total_elapsed:.1}s"))
}

// ─── IMF packaging ─────────────────────────────────────────────────────────

fn package_imp(
    app: &AppHandle,
    job: &JobConfig,
    j2k_dir: &Path,
    log_file: &mut Option<std::fs::File>,
) -> Result<(), String> {
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
    log!(log_file, "[PACKAGE] Creating IMP...");

    let opts = imfwizard_core::imp::ImpOptions {
        title: job.title.clone(),
        output_dir: job.output_dir.clone(),
        fps_num: 24,
        fps_den: 1,
        j2k_dir: Some(j2k_dir.to_path_buf()),
        ..Default::default()
    };

    let result = imfwizard_core::imp::create_imp(&opts);
    if !result.success {
        log!(log_file, "[PACKAGE] FAILED: {}", result.error);
        return Err(format!("IMP packaging failed: {}", result.error));
    }
    log!(log_file, "[PACKAGE] Done");
    Ok(())
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
