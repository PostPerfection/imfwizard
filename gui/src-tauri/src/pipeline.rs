use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
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

// ─── Job types ─────────────────────────────────────────────────────────────

/// One composition submitted by the GUI, packaged as its own CPL.
#[derive(Clone, Serialize, Deserialize)]
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
    // settled at submit from the panel's preset and this source's own signalling
    #[serde(default)]
    pub hdr: Option<imfwizard_core::hdr_wcg::HdrWcg>,
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
    /// The `--hdr` preset the picture is packaged as. None or empty is SDR.
    #[serde(default)]
    pub hdr: Option<String>,
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
    fn hdr(&self) -> Result<Option<imfwizard_core::hdr_wcg::HdrWcg>, String> {
        match self.hdr.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(preset) => imfwizard_core::hdr_wcg::HdrWcg::from_flags(preset, None).map(Some),
            None => Ok(None),
        }
    }

    /// Read the picture fields into the shared options.
    fn picture(&self) -> Result<imfwizard_core::source_picture::SourcePictureOptions, String> {
        let rotation = match self.rotate.as_deref().filter(|value| !value.is_empty()) {
            Some(value) => postkit::picture_processing::parse_rotation(value)?,
            None => postkit::picture_processing::Rotation::None,
        };
        let (flip_horizontal, flip_vertical) =
            match self.flip.as_deref().filter(|value| !value.is_empty()) {
                Some(value) => postkit::picture_processing::parse_flip(value)?,
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
            auto_crop_threshold: postkit::picture_processing::DEFAULT_AUTO_CROP_THRESHOLD,
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

#[derive(Clone, Serialize, Deserialize)]
pub struct JobConfig {
    id: u64,
    title: String,
    output_dir: PathBuf,
    compositions: Vec<CompositionInput>,
    fps_num: u32,
    fps_den: u32,
    bandwidth: u32,
    /// PSNR target in dB for the J2K encode, None when the encode allocates by
    /// compression ratio.
    quality_psnr: Option<f64>,
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

impl postkit::gui_job_queue::GuiJob for JobConfig {
    fn id(&self) -> u64 {
        self.id
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn output_dir(&self) -> &std::path::Path {
        &self.output_dir
    }
}

// ─── Queue state (managed by Tauri) ────────────────────────────────────────

pub type JobQueue = postkit::gui_job_queue::GuiJobQueue<JobConfig>;

const JOBS_FILE_VARIABLE: &str = "IMFWIZARD_GUI_JOBS_FILE";

/// Where the Jobs panel keeps its queue.
pub fn jobs_path() -> PathBuf {
    postkit::gui_job_queue::jobs_path(JOBS_FILE_VARIABLE, imfwizard_core::store::data_dir())
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

// each composition brings its own source, so its hdr is settled against that source
fn plan_compositions(
    compositions: &mut [CompositionInput],
    shared_plan: &imfwizard_core::preflight::CreatePlan,
) -> Result<Vec<String>, String> {
    let mut hints: Vec<String> = Vec::new();
    for composition in compositions {
        let picture = PathBuf::from(&composition.video_path);
        if shared_plan.audio_map.is_some() && composition.audio_path.is_none() {
            return Err(format!(
                "{} has no sound to map: drop a WAV on its Sound track or clear the audio map",
                composition.title
            ));
        }
        match (
            postkit::still::is_still_image(&picture),
            shared_plan.still_frames,
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
            picture: Some(picture.clone()),
            audio_files: composition.audio_path.iter().map(PathBuf::from).collect(),
            audio_language: composition.audio_lang.clone(),
            timed_text_files: composition.subtitles.iter().map(PathBuf::from).collect(),
            ..shared_plan.clone()
        };
        imfwizard_core::preflight::check_before_encode(&plan)?;
        // a Dolby Vision source carries its own light levels, so the panel can leave them unset
        composition.hdr =
            imfwizard_core::hdr_source::resolve(Some(&picture), shared_plan.hdr.clone())?;
        hints.extend(
            imfwizard_core::hints::gather_hints(&plan)
                .into_iter()
                .map(|hint| hint.text),
        );
    }
    Ok(hints)
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
    quality_psnr: Option<f64>,
    compositions: Option<Vec<CompositionInput>>,
    source_settings: Option<SourceSettings>,
    hints_accepted: Option<bool>,
) -> Result<SubmitResult, String> {
    let queue = app.state::<JobQueue>();
    let id = queue.reserve_job_id();

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
    let mut compositions = match compositions {
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
                hdr: None,
            }]
        }
    };

    // the colour space and the durations decide the encode, so a bad spelling
    // has to fail here rather than partway through it
    let settings = source_settings.unwrap_or_default();
    let source_colour = imfwizard_core::source_colourspace::to_source_colour(
        match settings.source_colourspace.as_deref() {
            Some(spelling) => imfwizard_core::source_colourspace::parse(spelling)?,
            None => imfwizard_core::source_colourspace::APP2E_SOURCE_SPACE,
        },
    )?;
    let hdr = settings.hdr()?;
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
    // what every composition shares, with its own picture and sound filled in per composition
    let shared_plan = imfwizard_core::preflight::CreatePlan {
        fps_num,
        fps_den,
        edits,
        audio_map: audio_map.clone(),
        burn_subtitle: burn_subtitle.clone(),
        burn_subtitle_font: burn_subtitle_font.clone(),
        burn_style: burn_style.clone(),
        picture_options: picture_options.clone(),
        source_colour: source_colour.clone(),
        hdr,
        still_frames,
        ..Default::default()
    };
    let hints = plan_compositions(&mut compositions, &shared_plan)?;

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
        quality_psnr,
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

    queue.submit(job);

    if !queue.has_running_job() {
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
    let (source_width, source_height) = postkit::encode::source_raster(&picture)?;
    let options = imfwizard_core::source_picture::SourcePictureOptions {
        auto_crop: true,
        auto_crop_threshold: threshold
            .unwrap_or(postkit::picture_processing::DEFAULT_AUTO_CROP_THRESHOLD),
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
///
/// `subtitle_path` is either that timed text or a built IMP directory, whose
/// packaged timed text is unwrapped out of its AS-02 track file first. A package
/// carrying no timed text gives back nothing to show.
#[tauri::command]
pub async fn subtitle_file_for_preview(
    app: AppHandle,
    subtitle_path: String,
    framerate: String,
) -> Result<Option<String>, String> {
    let work_dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| format!("no cache folder to write the preview subtitles into: {e}"))?
        .join(PREVIEW_SUBTITLE_DIRECTORY);
    let input = PathBuf::from(&subtitle_path);
    let playable = if input.is_dir() {
        imfwizard_core::subtitle_preview::packaged_subtitle_file(&input, &work_dir)?
    } else {
        Some(imfwizard_core::subtitle_preview::playable_subtitle_file(
            &input,
            parse_framerate(&framerate)?,
            &work_dir,
        )?)
    };
    Ok(playable.map(|path| path.to_string_lossy().into_owned()))
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
        input_channels: postkit::wav_io::channel_count(&PathBuf::from(&audio_path))?,
        destination_names: imfwizard_core::audio_map::destination_names(),
    })
}

#[tauri::command]
pub async fn cancel_job(app: AppHandle, job_id: u64) -> Result<(), String> {
    app.state::<JobQueue>().cancel(job_id);
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
    let (free, total) = postkit::free_space::volume_bytes(&dir)
        .map_err(|e| format!("Could not read free space: {e}"))?;
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

/// Give a built IMP a new content title without re-wrapping essence: the CPL is
/// rewritten with a new composition id and the track files are left alone. The
/// folder is renamed too when it is still named after the old title. Returns the
/// package path, which changes when the folder is renamed.
#[tauri::command]
pub async fn retitle_imp(path: String, title: String) -> Result<String, String> {
    let dir = PathBuf::from(&path);
    if !holds_imp(&dir) {
        return Err(format!("{path} does not hold an IMP"));
    }
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("Enter a new title".into());
    }
    let old_title = imfwizard_core::timeline::list_cpls(&dir)
        .first()
        .map(|cpl| cpl.title.clone())
        .ok_or_else(|| format!("No CPL found in {path}"))?;

    postkit::package_edit::edit_package(&postkit::package_edit::PackageEdit {
        input: dir.clone(),
        title: Some(title.clone()),
        ..Default::default()
    })
    .map_err(|e| format!("Could not retitle {path}: {e}"))?;

    let folder_is_named_after_the_title =
        dir.file_name().and_then(|name| name.to_str()) == Some(&old_title);
    let title_works_as_a_folder_name =
        !title.contains(std::path::MAIN_SEPARATOR) && !title.contains('/');
    if !folder_is_named_after_the_title || !title_works_as_a_folder_name {
        return Ok(path);
    }
    let renamed = dir.with_file_name(&title);
    if renamed.exists() {
        return Ok(path);
    }
    std::fs::rename(&dir, &renamed)
        .map_err(|e| format!("Retitled, but could not rename the folder: {e}"))?;
    Ok(renamed.to_string_lossy().into_owned())
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
    app.state::<JobQueue>().pause();
    Ok(())
}

#[tauri::command]
pub async fn resume_job(app: AppHandle) -> Result<(), String> {
    app.state::<JobQueue>().resume();
    Ok(())
}

#[tauri::command]
pub async fn list_jobs(app: AppHandle) -> Vec<postkit::gui_job_queue::JobInfo> {
    app.state::<JobQueue>().snapshot()
}

// ─── Queue worker ──────────────────────────────────────────────────────────

async fn run_queue_worker(app: AppHandle) {
    loop {
        let Some(job) = app.state::<JobQueue>().take_next() else {
            app.state::<JobQueue>().clear_current();
            break;
        };

        app.state::<JobQueue>().start(&job);

        let result = tokio::task::spawn_blocking({
            let app = app.clone();
            let job = job.clone();
            move || run_job(&app, &job)
        })
        .await;

        let queue = app.state::<JobQueue>();
        match result {
            Ok(Ok(_)) => {
                queue.finish(&job, postkit::gui_job_queue::StoredJobState::Done, "");
                emit_progress(&app, job.id, "done", "Complete", 0, 0, 0.0, 0.0, 100.0);
            }
            Ok(Err(e)) => {
                let cancelled = queue.is_cancelled();
                let state = if cancelled {
                    postkit::gui_job_queue::StoredJobState::Cancelled
                } else {
                    postkit::gui_job_queue::StoredJobState::Failed
                };
                queue.finish(&job, state, &e);
                let stage = if cancelled { "cancelled" } else { "error" };
                emit_progress(&app, job.id, stage, &e, 0, 0, 0.0, 0.0, 0.0);
            }
            // a panic leaves no error event, so the panel would wait forever
            Err(e) => {
                queue.finish(
                    &job,
                    postkit::gui_job_queue::StoredJobState::Failed,
                    &format!("Build panicked: {e}"),
                );
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

fn log_to(log_file: &Arc<Mutex<std::fs::File>>, msg: &str) {
    eprintln!("[pipeline] {msg}");
    let _ = writeln!(log_file.lock().unwrap(), "{msg}");
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

/// The `[TIMING]` line naming where the time inside an encode went, or None
/// when nothing was measured, which is a still or a J2K sequence.
fn format_encode_breakdown(
    stage: &str,
    progress: &postkit::pipeline::PipelineProgress,
) -> Option<String> {
    let measured = progress.decode_wait_secs > 0.0
        || progress.prepare_secs > 0.0
        || progress.encode_secs > 0.0
        || progress.write_secs > 0.0;
    measured.then(|| format!("[TIMING] {stage} breakdown: {}", progress.phase_breakdown()))
}

// the same validation `imfwizard validate` runs, over the package the build just
// wrote. None when the verify preference is off.
fn verify_package(
    imp_dir: &std::path::Path,
    preferences: &imfwizard_core::preferences::Preferences,
) -> Option<imfwizard_core::validate::ValidationResult> {
    preferences
        .verify_after_build
        .then(|| imfwizard_core::validate::validate_imp_with_photon(imp_dir, None))
}

// the `[VERIFY]` lines the job log carries, findings and all
fn verification_log(
    validation: Option<&imfwizard_core::validate::ValidationResult>,
) -> Vec<String> {
    let Some(validation) = validation else {
        return vec!["[VERIFY] Skipped, the verify preference is off".to_string()];
    };
    let mut lines = vec![format!(
        "[VERIFY] {}",
        if validation.valid { "PASSED" } else { "FAILED" }
    )];
    lines.extend(
        validation
            .errors
            .iter()
            .map(|error| format!("[VERIFY]   error: {error}")),
    );
    lines.extend(
        validation
            .warnings
            .iter()
            .map(|warning| format!("[VERIFY]   warning: {warning}")),
    );
    lines
}

fn run_job(app: &AppHandle, job: &JobConfig) -> Result<String, String> {
    let job_started = Instant::now();
    let queue = app.state::<JobQueue>();
    let cancel = queue.cancel_flag();
    let pause = queue.pause_flag();

    let output = &job.output_dir;
    std::fs::create_dir_all(output)
        .map_err(|e| format!("Cannot create the output folder {}: {e}", output.display()))?;
    let log_path = output.join("imfwizard.log");
    let log_file = Arc::new(Mutex::new(std::fs::File::create(&log_path).map_err(
        |e| format!("Cannot create the job log {}: {e}", log_path.display()),
    )?));

    log_to(&log_file, "=== IMF Wizard Pipeline ===");
    log_to(&log_file, &format!("Job ID: {}", job.id));
    log_to(&log_file, &format!("Title: {}", job.title));
    log_to(&log_file, &format!("Output: {}", output.display()));
    log_to(
        &log_file,
        &format!("Accelerator: {}", guikit::gpu::accelerator_status()),
    );
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

        // only video input carries a bandwidth, an image or J2K sequence has none
        let probe_started = Instant::now();
        let probed = imfwizard_core::probe::probe_video(&video_path);
        let input_type = postkit::encode::detect_input_type(&video_path);
        // a J2K directory reaches the wrapper with no decode at all, so it has
        // no picture to plan; submit_job already refused any processing on one
        let picture = match input_type {
            postkit::encode::InputType::J2kSequence => None,
            _ => {
                let (source_width, source_height) = postkit::encode::source_raster(&video_path)?;
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
        let target_codestream_bytes = match (&probed, &picture) {
            (Some(_), Some(_)) => Some(imfwizard_core::encode::codestream_byte_cap_for_bitrate(
                encode_fps.as_f64(),
                job.bandwidth as f64,
            )),
            _ => None,
        };
        // the codestreams declare an IMF profile, not the cinema one a DCP
        // carries. A J2K sequence skips the encode, so no profile is chosen for
        // it and the wrap checks the one its codestreams already carry.
        let rsiz = match &picture {
            Some(resolved) => Some(imfwizard_core::encode::imf_rsiz_for_encode(
                resolved.encode_width,
                resolved.encode_height,
                encode_fps.as_f64(),
                imfwizard_core::encode::bitrate_mbps_for_job(
                    Some(job.bandwidth as f64),
                    None,
                    resolved.encode_width,
                    resolved.encode_height,
                    encode_fps.as_f64(),
                ),
            )?),
            None => None,
        };
        if let Some(rsiz) = rsiz {
            log_to(
                &log_file,
                &format!("[ENCODE] JPEG 2000 profile: IMF, RSIZ {rsiz:#06x}"),
            );
        }
        // under a PSNR target the bandwidth is a ceiling per frame rather than
        // what the allocation aims at
        let codestream_byte_cap = job.quality_psnr.map(|_| {
            imfwizard_core::encode::codestream_byte_cap_for_bitrate(
                encode_fps.as_f64(),
                job.bandwidth as f64,
            )
        });
        if let (None, Some(target)) = (job.quality_psnr, target_codestream_bytes) {
            log_to(
                &log_file,
                &format!(
                    "[ENCODE] Target: {target} bytes a frame ({} Mbit/s)",
                    job.bandwidth
                ),
            );
        }
        log_to(
            &log_file,
            &format_stage_timing(
                &format!("probe composition {}", idx + 1),
                probe_started.elapsed(),
            ),
        );

        // per-composition scratch dir so multiple encodes don't clobber each other
        let enc_dir = output.join(format!(
            "{}{idx}",
            imfwizard_core::intermediates::ENCODE_SCRATCH_PREFIX
        ));
        let app_ref = app.clone();
        let log_ref = log_file.clone();
        let encode_stage_name = format!("encode composition {}", idx + 1);
        let encode_breakdown: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let encode_breakdown_ref = encode_breakdown.clone();
        // a still never reaches the pipeline: it is one encode per run of frames
        // sharing a cue set, linked for the rest of the hold
        let encode_started = Instant::now();
        // the picture MXF is written as the frames finish where the job allows
        // it, so the wrap costs nothing after the encode
        let overlap_refusal = imfwizard_core::overlapped_picture::overlap_refusal(
            &imfwizard_core::overlapped_picture::PictureJob {
                input_type,
                still_hold: job.still_frames.is_some(),
            },
        );
        let wrap_target = match overlap_refusal {
            Some(reason) => {
                log_to(
                    &log_file,
                    &format!("[ENCODE] Wrapping the picture MXF after the encode: {reason}"),
                );
                None
            }
            None => Some(imfwizard_core::overlapped_picture::PictureWrapTarget {
                imp_dir: job.output_dir.clone(),
                fps_num: job.fps_num,
                fps_den: job.fps_den,
                // the GUI has no HDR control, so every build it makes is SDR
                colour: imfwizard_core::mxf_wrap::picture_colour(None),
            }),
        };
        // a trimmed video encodes the kept window only, so the frames the trim
        // drops are never compressed
        let encode_window = imfwizard_core::source_edits::trimmed_encode_window(
            &job.edits,
            &video_path,
            input_type,
            job.fps_num,
            job.fps_den,
        )?;
        if let Some(window) = encode_window {
            log_to(
                &log_file,
                &format!(
                    "[ENCODE] Encoding frames {}..{} only",
                    window.first_frame,
                    window.end_frame()
                ),
            );
        }
        let mut picture_mxf = None;
        let picture_dir = match job.still_frames {
            Some(hold_for) => {
                let plan = picture
                    .as_ref()
                    .ok_or_else(|| format!("cannot read the size of {}", video_path.display()))?;
                let held = enc_dir.join(postkit::still::HELD_PICTURE_DIR);
                // a hold encodes at the default ratio whatever the Bitrate field
                // says, so the sub level is the one that ratio reaches
                let still_rsiz = imfwizard_core::encode::imf_rsiz_for_encode(
                    plan.encode_width,
                    plan.encode_height,
                    encode_fps.as_f64(),
                    imfwizard_core::encode::bitrate_mbps_for_job(
                        None,
                        None,
                        plan.encode_width,
                        plan.encode_height,
                        encode_fps.as_f64(),
                    ),
                )?;
                postkit::still::build_still_frames(&postkit::still::StillHold {
                    image: &video_path,
                    frames: hold_for,
                    fps: encode_fps,
                    width: plan.encode_width,
                    height: plan.encode_height,
                    filters: &plan.plan.filters,
                    apply_xyz_transform: job.source_colour.applies_xyz_transform(),
                    rsiz: still_rsiz,
                    colour_transform: job.source_colour.frame_transform()?,
                    burn: subtitle_burn.clone(),
                    watermark: None,
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
                let encode_options = postkit::pipeline::EncodeRunOptions {
                    compression_ratio: imfwizard_core::encode::DEFAULT_COMPRESSION_RATIO,
                    target_codestream_bytes,
                    quality_psnr: job.quality_psnr,
                    codestream_byte_cap,
                    fps: encode_fps,
                    frame_range: encode_window,
                    source_colour: job.source_colour.clone(),
                    // unset only for a J2K sequence, which postkit never encodes
                    rsiz: rsiz.unwrap_or_default(),
                    subtitle_burn: subtitle_burn.clone(),
                    picture: picture
                        .as_ref()
                        .map(|resolved| resolved.processing.clone())
                        .unwrap_or_default(),
                    ..Default::default()
                };
                let on_progress = |p: &postkit::pipeline::PipelineProgress| {
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
                    if let Some(line) = format_encode_breakdown(&encode_stage_name, p) {
                        *encode_breakdown_ref.lock().unwrap() = Some(line);
                    }
                };
                let on_log = |msg: &str| log_to(&log_ref, msg);
                let device_frames_before = postkit::grok_encoder::accelerated_frames();
                let encode_result = match wrap_target {
                    Some(target) => {
                        let (encode, track) =
                            imfwizard_core::overlapped_picture::encode_and_wrap_picture(
                                &video_path,
                                &enc_dir,
                                &encode_options,
                                target,
                                &cancel,
                                &pause,
                                on_progress,
                                on_log,
                            )?;
                        log_to(
                            &log_file,
                            &format!(
                                "[ENCODE] Picture MXF written during the encode: {} ({} frames)",
                                track.path.display(),
                                track.duration
                            ),
                        );
                        picture_mxf = Some(track);
                        encode
                    }
                    None => postkit::pipeline::run_encode_with_options(
                        &video_path,
                        &enc_dir,
                        &encode_options,
                        &cancel,
                        &pause,
                        on_progress,
                        on_log,
                    )?,
                };
                total_elapsed += encode_result.elapsed_secs;
                let device_frames = postkit::grok_encoder::accelerated_frames()
                    .saturating_sub(device_frames_before);
                log_to(
                    &log_file,
                    &format!(
                        "[ENCODE] Frames on the device: {device_frames} of {}",
                        encode_result.frames_encoded
                    ),
                );
                if device_frames == 0 && guikit::gpu::accelerator_status().requested {
                    log_to(
                        &log_file,
                        "[ENCODE] WARNING: the GPU was requested and no frame ran on the device",
                    );
                }
                for finding in encode_result.picture_findings.describe(encode_fps.as_f64()) {
                    log_to(&log_file, &format!("[ENCODE] {finding}"));
                }
                encode_result.j2k_dir
            }
        };
        log_to(
            &log_file,
            &format_stage_timing(&encode_stage_name, encode_started.elapsed()),
        );
        if let Some(breakdown) = encode_breakdown.lock().unwrap().as_deref() {
            log_to(&log_file, breakdown);
        }

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
            encode_window,
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
            picture_mxf,
            audio_files,
            timed_text_files: source.timed_text_files,
            hdr: ci.hdr.clone(),
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
    // a codestream directory a composition names sits where the encode would
    // have written its own
    let handed_in: Vec<&std::path::Path> = job
        .compositions
        .iter()
        .map(|composition| std::path::Path::new(&composition.video_path))
        .collect();
    imfwizard_core::intermediates::remove_intermediates(output, &handed_in);
    log_to(
        &log_file,
        &format_stage_timing("package", package_started.elapsed()),
    );

    emit_progress(
        app,
        job.id,
        "verify",
        "Validating IMP...",
        0,
        0,
        0.0,
        0.0,
        99.5,
    );
    let verify_started = Instant::now();
    let preferences = imfwizard_core::preferences::load_preferences().unwrap_or_default();
    let validation = verify_package(output, &preferences);
    for line in verification_log(validation.as_ref()) {
        log_to(&log_file, &line);
    }
    if validation.is_some() {
        log_to(
            &log_file,
            &format_stage_timing("verify", verify_started.elapsed()),
        );
    }
    log_to(
        &log_file,
        &format_stage_timing("total", job_started.elapsed()),
    );
    if let Some(failed) = validation.filter(|found| !found.valid) {
        return Err(format!(
            "The IMP was written to {} but failed validation: {}",
            output.display(),
            failed.errors.join("; ")
        ));
    }

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
    use super::{
        format_encode_breakdown, format_stage_timing, verification_log, verify_package,
        CompositionInput, SourceSettings,
    };
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    const CLIP_WIDTH: u32 = 256;
    const CLIP_HEIGHT: u32 = 144;
    const CLIP_FRAMES: u32 = 2;
    const FPS: u32 = 24;
    // the clips are small, so the plan pads them into an App 2E raster
    const APP2E_RASTER: (u32, u32) = (1920, 1080);
    const PQ_TRANSFER_TAG: &str = "smpte2084";
    const HLG_TRANSFER_TAG: &str = "arib-std-b67";

    fn scratch_directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("imfwizard-gui-{name}"));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn run_ffmpeg(arguments: &[&str]) {
        let made = std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error"])
            .args(arguments)
            .output()
            .expect("ffmpeg");
        assert!(
            made.status.success(),
            "{}",
            String::from_utf8_lossy(&made.stderr)
        );
    }

    fn clip(directory: &Path, name: &str, transfer: Option<&str>) -> PathBuf {
        let path = directory.join(format!("{name}.mkv"));
        let source = format!("color=c=gray:s={CLIP_WIDTH}x{CLIP_HEIGHT}:r={FPS}");
        let frames = CLIP_FRAMES.to_string();
        let mut arguments = vec![
            "-f",
            "lavfi",
            "-i",
            &source,
            "-frames:v",
            &frames,
            "-c:v",
            "ffv1",
            "-pix_fmt",
            "yuv420p10le",
        ];
        // the lavfi source carries no colour of its own, so setparams tags the frames
        // and the output flags carry the tag into the container
        let set_parameters = format!(
            "setparams=color_primaries=bt2020:color_trc={}:colorspace=bt2020nc:range=tv",
            transfer.unwrap_or_default()
        );
        if let Some(transfer) = transfer {
            arguments.extend_from_slice(&[
                "-vf",
                &set_parameters,
                "-color_primaries",
                "bt2020",
                "-color_trc",
                transfer,
                "-colorspace",
                "bt2020nc",
                "-color_range",
                "tv",
            ]);
        }
        let path_argument = path.to_string_lossy().into_owned();
        arguments.push(&path_argument);
        run_ffmpeg(&arguments);
        path
    }

    fn composition(picture: &Path) -> CompositionInput {
        CompositionInput {
            title: "Feature".to_string(),
            content_kind: "feature".to_string(),
            video_path: picture.to_string_lossy().into_owned(),
            audio_path: None,
            audio_lang: None,
            subtitles: Vec::new(),
            hdr: None,
        }
    }

    fn shared_plan(preset: Option<&str>) -> imfwizard_core::preflight::CreatePlan {
        imfwizard_core::preflight::CreatePlan {
            fps_num: FPS,
            fps_den: 1,
            picture_options: imfwizard_core::source_picture::SourcePictureOptions {
                raster: Some(APP2E_RASTER),
                ..Default::default()
            },
            source_colour: imfwizard_core::source_colourspace::to_source_colour(
                imfwizard_core::source_colourspace::APP2E_SOURCE_SPACE,
            )
            .unwrap(),
            hdr: settings(preset).hdr().unwrap(),
            ..Default::default()
        }
    }

    // the panel carries one preset for the job, so what each source adds to it is
    // the part that has to be settled per composition
    #[test]
    fn each_composition_settles_its_own_hdr_against_its_own_source() {
        let directory = scratch_directory("hdr-per-composition");
        let mut compositions = vec![
            composition(&clip(&directory, "tagged", Some(PQ_TRANSFER_TAG))),
            composition(&clip(&directory, "untagged", None)),
        ];

        super::plan_compositions(&mut compositions, &shared_plan(Some("pq-bt2020"))).unwrap();

        let preset = imfwizard_core::hdr_wcg::HdrWcg::from_flags("pq-bt2020", None).unwrap();
        for composition in &compositions {
            assert_eq!(composition.hdr.as_ref(), Some(&preset));
        }
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn a_source_whose_transfer_contradicts_the_preset_is_refused_naming_both() {
        let directory = scratch_directory("hdr-contradicts-preset");
        let mut compositions = vec![
            composition(&clip(&directory, "untagged", None)),
            composition(&clip(&directory, "tagged", Some(PQ_TRANSFER_TAG))),
        ];

        let error = super::plan_compositions(&mut compositions, &shared_plan(Some("hlg-bt2020")))
            .unwrap_err();

        assert!(error.contains(PQ_TRANSFER_TAG), "{error}");
        assert!(error.contains("hlg-bt2020"), "{error}");
        assert!(error.contains("pq-bt2020 or pq-p3d65"), "{error}");
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn an_hdr_source_with_no_preset_is_refused_naming_the_control() {
        let directory = scratch_directory("hdr-without-preset");
        let mut compositions = vec![composition(&clip(
            &directory,
            "tagged",
            Some(HLG_TRANSFER_TAG),
        ))];

        let error = super::plan_compositions(&mut compositions, &shared_plan(None)).unwrap_err();

        assert!(error.contains("hlg-bt2020"), "{error}");
        assert!(error.contains("Rec.709 SDR"), "{error}");
        assert!(
            error.contains("the HDR panel control in the GUI"),
            "{error}"
        );
        std::fs::remove_dir_all(&directory).ok();
    }

    fn settings(hdr: Option<&str>) -> SourceSettings {
        SourceSettings {
            hdr: hdr.map(str::to_string),
            ..Default::default()
        }
    }

    // the Properties panel's HDR control carries the preset the CLI takes as --hdr
    #[test]
    fn the_hdr_control_builds_the_preset_the_cli_builds() {
        let hlg = settings(Some("hlg-bt2020")).hdr().unwrap().unwrap();
        assert_eq!(
            hlg.transfer,
            imfwizard_core::hdr_wcg::TRANSFER_CHARACTERISTIC_HLG
        );
        assert!(hlg.is_hlg());
        assert_eq!(
            settings(Some("pq-p3d65")).hdr().unwrap(),
            Some(imfwizard_core::hdr_wcg::HdrWcg::from_flags("pq-p3d65", None).unwrap())
        );

        // the SDR option sends an empty string, and an older job sends nothing
        assert_eq!(settings(Some("")).hdr(), Ok(None));
        assert_eq!(settings(None).hdr(), Ok(None));

        let error = settings(Some("hlg-p3d65")).hdr().unwrap_err();
        assert!(error.contains("hlg-bt2020"), "{error}");
    }

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

    #[test]
    fn the_encode_breakdown_is_one_timing_line_and_absent_when_nothing_was_measured() {
        let measured = postkit::pipeline::PipelineProgress {
            stage: "encode".into(),
            message: "Frame 100/200".into(),
            frame: 100,
            total_frames: 200,
            fps: 12.0,
            elapsed_secs: 300.0,
            percent: 50.0,
            decode_wait_secs: 12.0,
            prepare_secs: 30.4,
            encode_secs: 250.0,
            write_secs: 7.6,
        };
        assert_eq!(
            format_encode_breakdown("encode composition 1", &measured).as_deref(),
            Some(
                "[TIMING] encode composition 1 breakdown: decoder wait 12s, frame prep 30s, j2k 4m10s, write 8s"
            )
        );

        let unmeasured = postkit::pipeline::PipelineProgress {
            decode_wait_secs: 0.0,
            prepare_secs: 0.0,
            encode_secs: 0.0,
            write_secs: 0.0,
            ..measured
        };
        assert_eq!(
            format_encode_breakdown("encode composition 1", &unmeasured),
            None
        );
    }

    const J2K_BIT_DEPTH: u8 = 12;

    fn created_package(name: &str) -> PathBuf {
        let work = scratch_directory(name);
        let codestreams = work.join("codestreams");
        std::fs::create_dir_all(&codestreams).unwrap();
        std::fs::write(
            codestreams.join("0001.j2c"),
            imfwizard_core::mxf_wrap::synthetic_j2k_codestream(
                APP2E_RASTER.0,
                APP2E_RASTER.1,
                J2K_BIT_DEPTH,
            ),
        )
        .unwrap();

        let imp = work.join("imp");
        let result = imfwizard_core::imp::create_imp(&imfwizard_core::imp::ImpOptions {
            output_dir: imp.clone(),
            compositions: vec![imfwizard_core::imp::Composition {
                title: "Verified".to_string(),
                content_kind: "feature".to_string(),
                j2k_dir: Some(codestreams),
                ..Default::default()
            }],
            fps_num: FPS,
            fps_den: 1,
            ..Default::default()
        });
        assert!(result.success, "create_imp failed: {}", result.error);
        imp
    }

    fn preferences(verify_after_build: bool) -> imfwizard_core::preferences::Preferences {
        imfwizard_core::preferences::Preferences {
            verify_after_build,
            ..Default::default()
        }
    }

    #[test]
    fn the_build_validates_the_package_unless_the_preference_is_off() {
        let imp = created_package("verify-stage");

        let ran = verify_package(&imp, &preferences(true)).expect("the verify stage ran");
        assert!(ran.valid, "the built package did not validate: {ran:?}");
        assert_eq!(verification_log(Some(&ran))[0], "[VERIFY] PASSED");

        assert!(
            verify_package(&imp, &preferences(false)).is_none(),
            "the verify stage ran with the preference off"
        );
        assert_eq!(
            verification_log(None),
            vec!["[VERIFY] Skipped, the verify preference is off"]
        );
    }

    #[test]
    fn a_broken_package_puts_its_errors_in_the_job_log() {
        let imp = created_package("verify-stage-broken");
        let cpl = std::fs::read_dir(&imp)
            .unwrap()
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("CPL_"))
            })
            .expect("the package holds a CPL");
        std::fs::remove_file(&cpl).unwrap();

        let broken = verify_package(&imp, &preferences(true)).expect("the verify stage ran");
        assert!(!broken.valid);
        let log = verification_log(Some(&broken));
        assert_eq!(log[0], "[VERIFY] FAILED");
        assert!(
            log.iter()
                .any(|line| line.starts_with("[VERIFY]   error: ")),
            "the findings are missing from the log: {log:?}"
        );
    }
}
