use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
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
    /// How the burnt-in text looks. Each field is None until the panel names it,
    /// so the rest keep the rasteriser's own defaults. The horizontal and
    /// vertical stretches are CLI-only.
    #[serde(default)]
    pub burn_font_size: Option<f32>,
    #[serde(default)]
    pub burn_colour: Option<String>,
    #[serde(default)]
    pub burn_effect: Option<String>,
    #[serde(default)]
    pub burn_effect_colour: Option<String>,
    #[serde(default)]
    pub burn_outline_width: Option<f32>,
    #[serde(default)]
    pub burn_line_height: Option<f32>,
    #[serde(default)]
    pub burn_margin: Option<f32>,
    #[serde(default)]
    pub burn_fade_up: Option<u64>,
    #[serde(default)]
    pub burn_fade_down: Option<u64>,
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
        let options = imfwizard_core::source_picture::SourcePictureOptions {
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
        };
        options.check()?;
        Ok(options)
    }

    /// Read the burn appearance fields into the rasteriser's overrides.
    fn burn_style(&self) -> Result<postkit::subtitle_raster::BurnStyleOverrides, String> {
        let colour = |label: &str, value: &Option<String>| match value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            Some(text) => postkit::subtitle_formats::Rgba::parse_hex(text)
                .map(Some)
                .map_err(|e| format!("{label}: {e}")),
            None => Ok(None),
        };
        let effect = match self
            .burn_effect
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            Some(text) => Some(
                postkit::subtitle_raster::parse_burn_effect(text)
                    .map_err(|e| format!("Burn-in effect: {e}"))?,
            ),
            None => None,
        };
        Ok(postkit::subtitle_raster::BurnStyleOverrides {
            font_size_percent: self.burn_font_size,
            colour: colour("Burn-in colour", &self.burn_colour)?,
            effect,
            effect_colour: colour("Burn-in effect colour", &self.burn_effect_colour)?,
            outline_width_percent: self.burn_outline_width,
            line_height_ratio: self.burn_line_height,
            margin_percent: self.burn_margin,
            x_scale: None,
            y_scale: None,
            fade_up_ms: self.burn_fade_up,
            fade_down_ms: self.burn_fade_down,
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
    burn_style: postkit::subtitle_raster::BurnStyleOverrides,
    picture: imfwizard_core::source_picture::SourcePictureOptions,
    audio_map: Option<String>,
    /// What the pre-build check found, carried through so the job log lists it
    /// without measuring the source a second time.
    hints: Vec<String>,
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

/// What a submitted build came back with: the queued job, or the hints that
/// stopped it short of queueing so the panel can show them.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitResult {
    pub job_id: Option<u64>,
    pub hints: Vec<String>,
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
    hints_accepted: Option<bool>,
) -> Result<SubmitResult, String> {
    let queue = app.state::<JobQueue>();
    let id = queue.next_id.fetch_add(1, Ordering::Relaxed);

    let (fps_num, fps_den) = match framerate.as_deref() {
        Some("24000/1001") => (24000, 1001),
        Some("25/1") => (25, 1),
        Some("30000/1001") => (30000, 1001),
        Some("30/1") => (30, 1),
        Some("48/1") => (48, 1),
        Some("50/1") => (50, 1),
        Some("60000/1001") => (60000, 1001),
        Some("60/1") => (60, 1),
        Some("100/1") => (100, 1),
        Some("120000/1001") => (120000, 1001),
        Some("120/1") => (120, 1),
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
    // a bad size or colour has to stop the build here, not part way through the encode
    let burn_style = settings.burn_style()?;
    imfwizard_core::subtitle_burn::resolve_burn_style(&burn_style)?;
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
    let mut hints: Vec<String> = Vec::new();
    for composition in &compositions {
        let picture = PathBuf::from(&composition.video_path);
        if audio_map.is_some() && composition.audio_path.is_none() {
            return Err(format!(
                "{} has no sound to map: drop a WAV on its Sound track or clear the audio map",
                composition.title
            ));
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

        let plan = imfwizard_core::preflight::CreatePlan {
            picture: Some(picture),
            audio_files: composition.audio_path.iter().map(PathBuf::from).collect(),
            audio_language: composition.audio_lang.clone(),
            timed_text_files: composition.subtitles.iter().map(PathBuf::from).collect(),
            fps_num,
            fps_den,
            edits,
            audio_map: audio_map.clone(),
            burn_subtitle: burn_subtitle.clone(),
            burn_subtitle_font: burn_subtitle_font.clone(),
            burn_style: burn_style.clone(),
            picture_options: picture_options.clone(),
            source_colour: source_colour.clone(),
            still_frames,
        };
        imfwizard_core::preflight::check_before_encode(&plan)?;
        hints.extend(
            imfwizard_core::hints::gather_hints(&plan)
                .into_iter()
                .map(|hint| hint.text),
        );
    }

    // the pref lives in the panel, which says it has taken the hints by sending
    // hintsAccepted rather than by naming the pref here
    if !hints.is_empty() && hints_accepted != Some(true) {
        return Ok(SubmitResult {
            job_id: None,
            hints,
        });
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
        burn_style,
        picture: picture_options,
        audio_map,
        hints: hints.clone(),
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

    Ok(SubmitResult {
        job_id: Some(id),
        hints,
    })
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

/// Where the preview's SRT copies of the packaged timed text are written, inside
/// the app's cache folder.
const PREVIEW_SUBTITLE_DIRECTORY: &str = "preview-subtitles";

/// A subtitle file the preview player can render, converting the composition's
/// timed text to SRT when mpv cannot read it as it stands.
#[tauri::command]
pub async fn subtitle_file_for_preview(
    app: AppHandle,
    subtitle_path: String,
    framerate: String,
) -> Result<String, String> {
    let work_dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| format!("no cache folder to write the preview subtitles into: {e}"))?
        .join(PREVIEW_SUBTITLE_DIRECTORY);
    let playable = imfwizard_core::subtitle_preview::playable_subtitle_file(
        &PathBuf::from(&subtitle_path),
        parse_framerate(&framerate)?,
        &work_dir,
    )?;
    Ok(playable.to_string_lossy().into_owned())
}

/// The Frame Rate menu's `num/den` spelling as frames per second.
fn parse_framerate(framerate: &str) -> Result<f64, String> {
    let parsed = || {
        let (numerator, denominator) = framerate.split_once('/')?;
        let numerator: f64 = numerator.parse().ok()?;
        let denominator: f64 = denominator.parse().ok()?;
        (denominator > 0.0).then(|| numerator / denominator)
    };
    parsed().ok_or_else(|| format!("{framerate} is not a frame rate"))
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

const SECONDS_PER_MINUTE: u64 = 60;

/// One `[TIMING]` line for the job log, sitting alongside the `[ENCODE]` and
/// `[PACKAGE]` lines the same stage writes.
fn format_stage_timing(stage: &str, duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    format!(
        "[TIMING] {stage} took {}m{}s",
        seconds / SECONDS_PER_MINUTE,
        seconds % SECONDS_PER_MINUTE
    )
}

fn run_job(app: &AppHandle, job: &JobConfig) -> Result<String, String> {
    let job_started = Instant::now();
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

    for hint in &job.hints {
        log_to(&log_file, &format!("[HINT] {hint}"));
    }

    let encode_fps = imfwizard_core::encode::FrameRate::new(job.fps_num, job.fps_den);
    log_to(
        &log_file,
        &format!(
            "Frame rate: {:.2} fps ({}/{})",
            encode_fps.as_f64(),
            job.fps_num,
            job.fps_den
        ),
    );

    // submit_job already proved the file parses, so a failure here is a file that
    // changed underneath
    let subtitle_burn = match &job.burn_subtitle {
        Some(path) => Some(imfwizard_core::subtitle_burn::prepare_subtitle_burn(
            path,
            job.burn_subtitle_font.as_deref(),
            &job.burn_style,
            encode_fps,
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
        let probe_started = Instant::now();
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
            (Some(info), Some(picture)) => imfwizard_core::encode::compression_ratio_for_bitrate(
                picture.encode_width,
                picture.encode_height,
                info.fps_num as f64 / info.fps_den.max(1) as f64,
                job.bandwidth as f64,
            ),
            _ => imfwizard_core::encode::DEFAULT_COMPRESSION_RATIO,
        };
        log_to(
            &log_file,
            &format_stage_timing(
                &format!("probe composition {}", idx + 1),
                probe_started.elapsed(),
            ),
        );

        // per-composition scratch dir so multiple encodes don't clobber each other
        let enc_dir = output.join(format!("enc_{idx}"));
        let app_ref = app.clone();
        let log_ref = log_file.clone();
        // a still never reaches the pipeline: it is one encode per run of frames
        // sharing a cue set, linked for the rest of the hold
        let encode_started = Instant::now();
        let picture_dir = match job.still_frames {
            Some(hold_for) => {
                let plan = picture
                    .as_ref()
                    .ok_or_else(|| format!("cannot read the size of {}", video_path.display()))?;
                let held = enc_dir.join(imfwizard_core::still::HELD_PICTURE_DIR);
                imfwizard_core::still::build_still_frames(&imfwizard_core::still::StillHold {
                    image: &video_path,
                    frames: hold_for,
                    fps: encode_fps,
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
                        fps: encode_fps,
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
        log_to(
            &log_file,
            &format_stage_timing(
                &format!("encode composition {}", idx + 1),
                encode_started.elapsed(),
            ),
        );

        // the map runs before the delay, the trim and the MCA labels, so the
        // labelled layout describes the file that is actually packaged
        let audio_map_started = Instant::now();
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
        if job.audio_map.is_some() {
            log_to(
                &log_file,
                &format_stage_timing(
                    &format!("audio map composition {}", idx + 1),
                    audio_map_started.elapsed(),
                ),
            );
        }

        let edits_started = Instant::now();
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
        log_to(
            &log_file,
            &format_stage_timing(
                &format!("source edits composition {}", idx + 1),
                edits_started.elapsed(),
            ),
        );

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
    let package_started = Instant::now();

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
        &format_stage_timing("package", package_started.elapsed()),
    );
    log_to(
        &log_file,
        &format_stage_timing("total", job_started.elapsed()),
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

#[cfg(test)]
mod tests {
    use super::format_stage_timing;
    use std::time::Duration;

    #[test]
    fn stage_timing_reads_as_minutes_and_seconds() {
        assert_eq!(
            format_stage_timing("encode composition 1", Duration::from_secs(192)),
            "[TIMING] encode composition 1 took 3m12s"
        );
        assert_eq!(
            format_stage_timing("package", Duration::from_millis(1900)),
            "[TIMING] package took 0m1s"
        );
        assert_eq!(
            format_stage_timing("total", Duration::from_secs(3600)),
            "[TIMING] total took 60m0s"
        );
    }
}
