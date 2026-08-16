use serde::{Deserialize, Serialize};
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

/// One composition submitted by the GUI, packaged as its own CPL.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompositionInput {
    pub title: String,
    #[serde(default)]
    pub content_kind: String,
    pub video_path: String,
    #[serde(default)]
    pub audio_path: Option<String>,
    #[serde(default)]
    pub audio_lang: Option<String>,
    #[serde(default)]
    pub subtitles: Vec<String>,
}

/// How the Properties panel says to treat the source: where the sound sits
/// against the picture, what colour the picture is in, and what to cut or hold.
/// Durations are spelled as the CLI spells them, "48f" or "2s".
#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSettings {
    #[serde(default)]
    pub audio_delay_ms: i64,
    #[serde(default)]
    pub source_colourspace: Option<String>,
    #[serde(default)]
    pub trim_start: Option<String>,
    #[serde(default)]
    pub trim_end: Option<String>,
    #[serde(default)]
    pub still_length: Option<String>,
    /// Subtitle file drawn into the picture during the encode. Registers no
    /// timed-text track: burnt-in text is part of the image.
    #[serde(default)]
    pub burn_subtitle: Option<String>,
    #[serde(default)]
    pub burn_subtitle_font: Option<String>,
    #[serde(default)]
    pub crop_left: u32,
    #[serde(default)]
    pub crop_right: u32,
    #[serde(default)]
    pub crop_top: u32,
    #[serde(default)]
    pub crop_bottom: u32,
    /// Crop to the target raster's aspect rather than padding to it.
    #[serde(default)]
    pub fill_crop: bool,
    #[serde(default)]
    pub deinterlace: bool,
    #[serde(default)]
    pub denoise: bool,
    /// Clockwise quarter turns, as "90", "180" or "270".
    #[serde(default)]
    pub rotate: Option<String>,
    /// "horizontal", "vertical" or "both".
    #[serde(default)]
    pub flip: Option<String>,
    /// Raster the picture is fitted into, as "2048x1080". None keeps the
    /// source's own.
    #[serde(default)]
    pub raster: Option<String>,
    /// Channel map for the composition's sound, in the CLI's `--audio-map`
    /// grammar.
    #[serde(default)]
    pub audio_map: Option<String>,
}

impl SourceSettings {
    /// Read the picture fields into the shared options.
    fn picture(&self) -> Result<imfwizard_core::source_picture::SourcePictureOptions, String> {
        let rotation = match self.rotate.as_deref().filter(|value| !value.is_empty()) {
            Some(value) => imfwizard_core::source_picture::parse_rotation(value)?,
            None => postkit::picture_processing::Rotation::None,
        };
        let (flip_horizontal, flip_vertical) =
            match self.flip.as_deref().filter(|value| !value.is_empty()) {
                Some(value) => imfwizard_core::source_picture::parse_flip(value)?,
                None => (false, false),
            };
        let raster = match self.raster.as_deref().filter(|value| !value.is_empty()) {
            Some(value) => Some(imfwizard_core::source_picture::parse_raster(value)?),
            None => None,
        };
        Ok(imfwizard_core::source_picture::SourcePictureOptions {
            crop: postkit::picture_processing::Crop {
                left: self.crop_left,
                right: self.crop_right,
                top: self.crop_top,
                bottom: self.crop_bottom,
            },
            auto_crop: false,
            auto_crop_threshold: imfwizard_core::source_picture::DEFAULT_AUTO_CROP_THRESHOLD,
            fill_crop: self.fill_crop,
            deinterlace: self.deinterlace,
            denoise: self.denoise,
            rotation,
            flip_horizontal,
            flip_vertical,
            raster,
        })
    }
}

#[derive(Clone)]
struct JobConfig {
    id: u64,
    title: String,
    output_dir: PathBuf,
    compositions: Vec<CompositionInput>,
    fps_num: u32,
    fps_den: u32,
    bandwidth: u32,
    edits: imfwizard_core::source_edits::SourceEdits,
    source_colour: postkit::encode::SourceColour,
    /// Frames to hold a still input for; None when the input is not a still.
    still_frames: Option<u64>,
    burn_subtitle: Option<PathBuf>,
    burn_subtitle_font: Option<PathBuf>,
    picture: imfwizard_core::source_picture::SourcePictureOptions,
    audio_map: Option<String>,
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
    /// Output folder of the running job, so a second build cannot write into it
    current_output: Mutex<Option<PathBuf>>,
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
            current_output: Mutex::new(None),
        }
    }

    /// Is a job already running or queued that writes into `output`?
    pub fn is_building_into(&self, output: &std::path::Path) -> bool {
        if self.current_output.lock().unwrap().as_deref() == Some(output) {
            return true;
        }
        self.queue
            .lock()
            .unwrap()
            .iter()
            .any(|job| job.output_dir == output)
    }
}

/// Files a finished IMP always has at its root.
const IMP_ROOT_FILES: [&str; 2] = ["ASSETMAP.xml", "VOLINDEX.xml"];

fn holds_imp(dir: &std::path::Path) -> bool {
    IMP_ROOT_FILES.iter().any(|name| dir.join(name).exists())
}

// ─── Tauri commands ────────────────────────────────────────────────────────

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn submit_job(
    app: AppHandle,
    video_path: Option<String>,
    title: String,
    output_dir: String,
    audio_path: Option<String>,
    subtitles: Option<Vec<String>>,
    framerate: Option<String>,
    content_kind: Option<String>,
    bandwidth: Option<u32>,
    compositions: Option<Vec<CompositionInput>>,
    source_settings: Option<SourceSettings>,
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

    // Build/Delivery may pass a compositions array; legacy single-video callers
    // pass video_path and are treated as a one-composition job.
    let compositions = match compositions {
        Some(c) if !c.is_empty() => c,
        _ => {
            let Some(video_path) = video_path else {
                return Err("no video or compositions provided".into());
            };
            vec![CompositionInput {
                title: title.clone(),
                content_kind: content_kind.unwrap_or_else(|| "feature".to_string()),
                video_path,
                audio_path,
                audio_lang: None,
                subtitles: subtitles.unwrap_or_default(),
            }]
        }
    };

    // the colour space and the durations decide the encode, so a bad spelling
    // has to fail here rather than partway through it
    let settings = source_settings.unwrap_or_default();
    let source_colour = imfwizard_core::source_colourspace::to_source_colour(
        match settings.source_colourspace.as_deref() {
            Some(spelling) => imfwizard_core::source_colourspace::parse(spelling)?,
            None => postkit::colour::ColourSpace::Rec709,
        },
    )?;
    let frames_from_spec = |spec: &Option<String>| match spec.as_deref() {
        Some(spec) => imfwizard_core::duration_spec::parse_duration_frames(spec, fps_num, fps_den),
        None => Ok(0),
    };
    let edits = imfwizard_core::source_edits::SourceEdits {
        audio_delay_ms: settings.audio_delay_ms,
        trim_start_frames: frames_from_spec(&settings.trim_start)?,
        trim_end_frames: frames_from_spec(&settings.trim_end)?,
    };
    let still_frames = match settings.still_length.as_deref() {
        Some(spec) => Some(imfwizard_core::duration_spec::parse_duration_frames(
            spec, fps_num, fps_den,
        )?),
        None => None,
    };
    let picture_options = settings.picture()?;
    let burn_subtitle = settings
        .burn_subtitle
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let burn_subtitle_font = settings
        .burn_subtitle_font
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let audio_map = settings
        .audio_map
        .clone()
        .filter(|spec| !spec.trim().is_empty());
    for composition in &compositions {
        let picture = PathBuf::from(&composition.video_path);
        imfwizard_core::source_colourspace::reject_on_precompressed_picture(
            &picture,
            &source_colour,
        )?;
        imfwizard_core::source_picture::reject_on_precompressed_picture(
            &picture,
            &picture_options,
        )?;
        if audio_map.is_some() && composition.audio_path.is_none() {
            return Err(format!(
                "{} has no sound to map: drop a WAV on its Sound track or clear the audio map",
                composition.title
            ));
        }
        // a burn draws display-RGB text onto decoded frames, so refuse every
        // route that hands the encoder X'Y'Z' or nothing to draw on
        if let Some(burn) = &burn_subtitle {
            let timed_text: Vec<PathBuf> =
                composition.subtitles.iter().map(PathBuf::from).collect();
            imfwizard_core::subtitle_burn::check_burn_supported(
                burn,
                &imfwizard_core::subtitle_burn::BurnTarget {
                    timed_text: &timed_text,
                    frames_already_xyz: !source_colour.applies_xyz_transform(),
                    input_is_codestreams: postkit::encode::detect_input_type(&picture)
                        == postkit::encode::InputType::J2kSequence,
                },
            )?;
        }
        match (
            imfwizard_core::still::is_still_image(&picture),
            still_frames,
        ) {
            (false, Some(_)) => {
                return Err(format!(
                    "{} is a video or a frame directory, so a still length has nothing to hold",
                    composition.video_path
                ));
            }
            (true, None) => {
                return Err(format!(
                    "{} is a single image; set a still length to say how long to hold it",
                    composition.video_path
                ));
            }
            _ => {}
        }
    }

    // parse the cue file and build the burn now, so a bad file or a missing font
    // fails here instead of part way through the encode
    if let Some(burn) = &burn_subtitle {
        imfwizard_core::subtitle_burn::prepare_subtitle_burn(
            burn,
            burn_subtitle_font.as_deref(),
            fps_num,
        )?;
    }

    // packages are folders named by title, so a reused title lands in the old
    // package. refuse now, not after the encode.
    let output_path = PathBuf::from(&output_dir);
    if holds_imp(&output_path) {
        return Err(format!(
            "Output folder already holds an IMP: {output_dir}. Use a new title or output folder, or delete the old package first."
        ));
    }
    if queue.is_building_into(&output_path) {
        return Err(format!(
            "A build is already running into {output_dir}. Wait for it to finish or cancel it."
        ));
    }

    let job = JobConfig {
        id,
        title: title.clone(),
        output_dir: output_path,
        compositions,
        fps_num,
        fps_den,
        bandwidth: bandwidth.unwrap_or(250),
        edits,
        source_colour,
        still_frames,
        burn_subtitle,
        burn_subtitle_font,
        picture: picture_options,
        audio_map,
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

/// The black borders of a source, and the plan cropping them away resolves to.
#[derive(Serialize)]
pub struct DetectedCrop {
    pub left: u32,
    pub right: u32,
    pub top: u32,
    pub bottom: u32,
    pub description: String,
}

/// Measure the black borders of a picture source, for the Auto-crop button.
#[tauri::command]
pub async fn detect_source_crop(
    video_path: String,
    threshold: Option<f32>,
) -> Result<DetectedCrop, String> {
    let picture = PathBuf::from(&video_path);
    let (source_width, source_height) = imfwizard_core::source_picture::source_raster(&picture)?;
    let options = imfwizard_core::source_picture::SourcePictureOptions {
        auto_crop: true,
        auto_crop_threshold: threshold
            .unwrap_or(imfwizard_core::source_picture::DEFAULT_AUTO_CROP_THRESHOLD),
        ..Default::default()
    };
    let resolved = imfwizard_core::source_picture::resolve_picture(
        &options,
        &picture,
        source_width,
        source_height,
        postkit::encode::detect_input_type(&picture) == postkit::encode::InputType::ImageSequence,
    )?;
    Ok(DetectedCrop {
        left: resolved.plan.crop.left,
        right: resolved.plan.crop.right,
        top: resolved.plan.crop.top,
        bottom: resolved.plan.crop.bottom,
        description: resolved.plan.describe(),
    })
}

/// The channel count and lane names an audio mapping matrix is laid out from.
#[derive(Serialize)]
pub struct AudioMapShape {
    pub input_channels: usize,
    pub destination_names: Vec<String>,
}

/// How many channels a WAV carries, for sizing the mapping matrix.
#[tauri::command]
pub async fn audio_map_shape(audio_path: String) -> Result<AudioMapShape, String> {
    Ok(AudioMapShape {
        input_channels: imfwizard_core::audio_map::input_channels(&PathBuf::from(&audio_path))?,
        destination_names: imfwizard_core::audio_map::destination_names(),
    })
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

#[derive(Serialize)]
pub struct DiskSpace {
    pub free_bytes: u64,
    pub total_bytes: u64,
    pub percent_free: f64,
}

/// Free space on the volume holding `path`.
#[tauri::command]
pub async fn disk_space(path: String) -> Result<DiskSpace, String> {
    // the output folder is only created once the build starts, so report the
    // volume of the nearest folder that does exist
    let mut dir = PathBuf::from(&path);
    while !dir.exists() {
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => return Err(format!("no existing folder above {path}")),
        }
    }
    let stats = fs4::statvfs(&dir).map_err(|e| format!("Could not read free space: {e}"))?;
    let (free, total) = (stats.available_space(), stats.total_space());
    Ok(DiskSpace {
        free_bytes: free,
        total_bytes: total,
        percent_free: if total == 0 {
            0.0
        } else {
            free as f64 * 100.0 / total as f64
        },
    })
}

/// Delete a built IMP folder and everything in it. Refuses any folder that is
/// not an IMP, so a stale recent entry cannot take out a folder of source media.
#[tauri::command]
pub async fn delete_imp(app: AppHandle, path: String) -> Result<(), String> {
    let dir = PathBuf::from(&path);
    if !dir.exists() {
        return Err(format!("{path} no longer exists"));
    }
    if !holds_imp(&dir) {
        return Err(format!(
            "{path} does not hold an IMP, refusing to delete it"
        ));
    }
    let queue = app.state::<JobQueue>();
    if queue.is_building_into(&dir) {
        return Err(format!(
            "A build is writing into {path}. Cancel it before deleting."
        ));
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("Could not delete {path}: {e}"))
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
            *queue.current_output.lock().unwrap() = None;
            break;
        };

        {
            let queue = app.state::<JobQueue>();
            queue.current_id.store(job.id, Ordering::Relaxed);
            *queue.current_title.lock().unwrap() = job.title.clone();
            *queue.current_output.lock().unwrap() = Some(job.output_dir.clone());
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
                let cancelled = queue.cancel.load(Ordering::Relaxed);
                *queue.current_status.lock().unwrap() = if cancelled {
                    "cancelled".to_string()
                } else {
                    format!("failed: {e}")
                };
                let stage = if cancelled { "cancelled" } else { "error" };
                emit_progress(&app, job.id, stage, &e, 0, 0, 0.0, 0.0, 0.0);
            }
            // a panic leaves no error event, so the panel would wait forever
            Err(e) => {
                *queue.current_status.lock().unwrap() = format!("panic: {e}");
                emit_progress(
                    &app,
                    job.id,
                    "error",
                    &format!("Build panicked: {e}"),
                    0,
                    0,
                    0.0,
                    0.0,
                    0.0,
                );
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
    log_to(&log_file, &format!("Output: {}", output.display()));
    log_to(
        &log_file,
        &format!("Compositions: {}", job.compositions.len()),
    );
    log_to(
        &log_file,
        &format!(
            "Started: {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        ),
    );

    // submit_job already proved the file parses, so a failure here is a file that
    // changed underneath
    let subtitle_burn = match &job.burn_subtitle {
        Some(path) => Some(imfwizard_core::subtitle_burn::prepare_subtitle_burn(
            path,
            job.burn_subtitle_font.as_deref(),
            job.fps_num,
        )?),
        None => None,
    };

    let job_id = job.id;
    let n = job.compositions.len();
    let mut total_elapsed = 0.0;
    let mut comps: Vec<imfwizard_core::imp::Composition> = Vec::new();

    // Encode each composition's picture, then package all into one multi-CPL IMP.
    for (idx, ci) in job.compositions.iter().enumerate() {
        let video_path = PathBuf::from(&ci.video_path);
        log_to(
            &log_file,
            &format!(
                "[ENCODE] composition {} of {n}: {}",
                idx + 1,
                video_path.display()
            ),
        );

        // Map the target bandwidth (Mbps) to a J2K compression ratio, matching the
        // dcpwizard CLI convention (raw = w*h*36 bits/frame). Only honoured for video
        // input; image/J2K sequences fall back to the encoder default.
        let probed = imfwizard_core::probe::probe_video(&video_path);
        let input_type = postkit::encode::detect_input_type(&video_path);
        // a J2K directory reaches the wrapper with no decode at all, so it has
        // no picture to plan; submit_job already refused any processing on one
        let picture = match input_type {
            postkit::encode::InputType::J2kSequence => None,
            _ => {
                let (source_width, source_height) =
                    imfwizard_core::source_picture::source_raster(&video_path)?;
                let resolved = imfwizard_core::source_picture::resolve_picture(
                    &job.picture,
                    &video_path,
                    source_width,
                    source_height,
                    input_type == postkit::encode::InputType::ImageSequence,
                )?;
                log_to(&log_file, &format!("[ENCODE] {}", resolved.plan.describe()));
                // the wrapper refuses an illegal raster too, but only after the
                // encode has already run
                imfwizard_core::mxf_wrap::validate_app2e_raster(
                    resolved.encode_width,
                    resolved.encode_height,
                )?;
                Some(resolved)
            }
        };
        let compression_ratio = match (&probed, &picture) {
            (Some(info), Some(picture)) => {
                let fps = (info.fps_num as f64 / info.fps_den.max(1) as f64).max(1.0);
                // the bitrate target is against the raster that is encoded, which
                // the picture plan may have changed
                let raw_bits = picture.encode_width as f64 * picture.encode_height as f64 * 36.0;
                let target_bits = (job.bandwidth as f64 * 1_000_000.0) / fps;
                (raw_bits / target_bits).max(1.0)
            }
            _ => 10.0,
        };

        // per-composition scratch dir so multiple encodes don't clobber each other
        let enc_dir = output.join(format!("enc_{idx}"));
        let app_ref = app.clone();
        let log_ref = log_file.clone();
        // a still never reaches the pipeline: it is one encode per run of frames
        // sharing a cue set, linked for the rest of the hold
        let picture_dir = match job.still_frames {
            Some(hold_for) => {
                let plan = picture
                    .as_ref()
                    .ok_or_else(|| format!("cannot read the size of {}", video_path.display()))?;
                let held = enc_dir.join(imfwizard_core::still::HELD_PICTURE_DIR);
                imfwizard_core::still::build_still_frames(&imfwizard_core::still::StillHold {
                    image: &video_path,
                    frames: hold_for,
                    fps: job.fps_num,
                    width: plan.encode_width,
                    height: plan.encode_height,
                    filters: &plan.plan.filters,
                    source_colour: &job.source_colour,
                    burn: subtitle_burn.clone(),
                    out_dir: &held,
                })?;
                log_to(
                    &log_file,
                    &format!(
                        "[ENCODE] Still held for {hold_for} frame(s) at {}x{}",
                        plan.encode_width, plan.encode_height
                    ),
                );
                held
            }
            None => {
                let encode_result = postkit::pipeline::run_encode_with_options(
                    &video_path,
                    &enc_dir,
                    &postkit::pipeline::EncodeRunOptions {
                        compression_ratio,
                        fps: job.fps_num,
                        source_colour: job.source_colour.clone(),
                        subtitle_burn: subtitle_burn.clone(),
                        picture: picture
                            .as_ref()
                            .map(|resolved| resolved.processing.clone())
                            .unwrap_or_default(),
                        ..Default::default()
                    },
                    &cancel,
                    &pause,
                    |p| {
                        // scale each composition's 0..100 into its slice of the whole job
                        let scaled = (idx as f64 + p.percent / 100.0) / n as f64 * 100.0;
                        emit_progress(
                            &app_ref,
                            job_id,
                            &p.stage,
                            &p.message,
                            p.frame,
                            p.total_frames,
                            p.fps,
                            p.elapsed_secs,
                            scaled,
                        );
                    },
                    |msg| log_to(&log_ref, msg),
                )?;
                total_elapsed += encode_result.elapsed_secs;
                encode_result.j2k_dir
            }
        };

        // the map runs before the delay, the trim and the MCA labels, so the
        // labelled layout describes the file that is actually packaged
        let audio_files: Vec<PathBuf> = match &job.audio_map {
            Some(spec) => ci
                .audio_path
                .iter()
                .map(|wav| {
                    imfwizard_core::audio_map::map_audio_file(
                        spec,
                        &PathBuf::from(wav),
                        &enc_dir,
                        |line| log_to(&log_file, line),
                    )
                })
                .collect::<Result<Vec<_>, String>>()?,
            None => ci.audio_path.iter().map(PathBuf::from).collect(),
        };

        let source = imfwizard_core::source_edits::apply_source_edits(
            &job.edits,
            &imfwizard_core::source_edits::CompositionSource {
                j2k_dir: Some(picture_dir),
                audio_files,
                timed_text_files: ci.subtitles.iter().map(PathBuf::from).collect(),
            },
            &enc_dir,
            job.fps_num,
            job.fps_den,
        )?;

        let audio_files = source
            .audio_files
            .into_iter()
            .map(|path| imfwizard_core::imp::AudioTrack {
                path,
                language: ci.audio_lang.clone(),
                role: None,
            })
            .collect();
        comps.push(imfwizard_core::imp::Composition {
            title: ci.title.clone(),
            content_kind: if ci.content_kind.is_empty() {
                "feature".to_string()
            } else {
                ci.content_kind.clone()
            },
            j2k_dir: source.j2k_dir,
            audio_files,
            timed_text_files: source.timed_text_files,
            hdr: None,
        });
    }

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
        output_dir: job.output_dir.clone(),
        compositions: comps,
        fps_num: job.fps_num,
        fps_den: job.fps_den,
        ..Default::default()
    };

    let result = imfwizard_core::imp::create_imp(&opts);
    if !result.success {
        log_to(&log_file, &format!("[PACKAGE] FAILED: {}", result.error));
        return Err(format!("IMP packaging failed: {}", result.error));
    }
    log_to(
        &log_file,
        &format!("[PACKAGE] Done, {} CPL(s)", result.cpl_paths.len()),
    );

    log_to(
        &log_file,
        &format!("=== Pipeline finished in {total_elapsed:.1}s ==="),
    );
    Ok(format!("IMP created in {total_elapsed:.1}s"))
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
