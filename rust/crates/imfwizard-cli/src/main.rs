use clap::{Parser, Subcommand};
use imfwizard_core::prores::ContainerRaster;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "imfwizard",
    version,
    about = "IMF Wizard - Interoperable Master Format creation tool"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Encode and decode on grok's accelerator plugin
    #[arg(long, global = true)]
    gpu: bool,

    #[arg(long, global = true, conflicts_with = "gpu")]
    no_gpu: bool,

    #[arg(long, global = true, help = "Grok accelerator plugin license")]
    license: Option<String>,

    #[arg(long, global = true, help = "Grok license registration URL")]
    registration_url: Option<String>,
}

#[derive(Subcommand)]
enum PreferencesCommand {
    #[command(about = "Print every preference as JSON")]
    Show,
    #[command(about = "Print the preferences file path")]
    Path,
    #[command(about = "Restore every preference to its default")]
    Reset,
    #[command(about = "Set one preference")]
    Set {
        #[arg(help = "JSON field name, with camelCase, kebab-case, or snake_case accepted")]
        name: String,
        #[arg(help = "New value")]
        value: String,
    },
}

/// The ceiling `create --bitrate` takes, matching the GUI's Bitrate field.
const MAXIMUM_BITRATE_MBPS: f64 = 1000.0;

/// Report a failure the way every command does, and stop.
fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("Error: {message}");
    std::process::exit(1);
}

/// Why the demux from `--video` produced no WAV, separating a source with no
/// sound in it from an ffmpeg that could not run or refused the file.
fn demux_failure_reason(
    video: &std::path::Path,
    demuxed: &std::io::Result<std::process::Output>,
) -> String {
    let run = match demuxed {
        Err(e) => return format!("ffmpeg could not run: {e}"),
        Ok(run) => run,
    };
    let probe = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a",
            "-show_entries",
            "stream=index",
            "-of",
            "csv=p=0",
        ])
        .arg(video)
        .output();
    match probe {
        Ok(probe) if probe.status.success() && probe.stdout.is_empty() => {
            "it carries no audio stream".to_string()
        }
        _ => format!(
            "ffmpeg failed to demux it: {}",
            String::from_utf8_lossy(&run.stderr).trim()
        ),
    }
}

/// Encode picture through the shared in-process grok pipeline, printing
/// per-frame progress. Ctrl-C cancels the run.
///
/// `wrap` writes the picture MXF as the frames finish instead of leaving the
/// codestreams for `create_imp` to read back, and returns the track file it wrote.
fn encode_picture(
    input: &std::path::Path,
    output_dir: &std::path::Path,
    options: &postkit::pipeline::EncodeRunOptions,
    wrap: Option<imfwizard_core::overlapped_picture::PictureWrapTarget>,
) -> Result<
    (
        postkit::pipeline::EncodeResult,
        Option<imfwizard_core::MxfTrackFile>,
    ),
    String,
> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    let cancel = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));
    let on_interrupt = cancel.clone();
    let _ = ctrlc::set_handler(move || on_interrupt.store(true, Ordering::Relaxed));

    let phase_breakdown: Mutex<Option<String>> = Mutex::new(None);
    let on_progress = |p: &postkit::pipeline::PipelineProgress| {
        eprint!(
            "\r[encode] {}/{} frames ({:.0}%) {:.1} fps   ",
            p.frame, p.total_frames, p.percent, p.fps
        );
        if measured_any_phase(p) {
            *phase_breakdown.lock().unwrap() = Some(p.phase_breakdown());
        }
    };
    let on_log = |msg: &str| tracing::info!("{msg}");
    let result = match wrap {
        Some(target) => imfwizard_core::overlapped_picture::encode_and_wrap_picture(
            input,
            output_dir,
            options,
            target,
            &cancel,
            &pause,
            on_progress,
            on_log,
        )
        .map(|(encode, track)| (encode, Some(track))),
        None => postkit::pipeline::run_encode_with_options(
            input,
            output_dir,
            options,
            &cancel,
            &pause,
            on_progress,
            on_log,
        )
        .map(|encode| (encode, None)),
    };
    eprintln!();
    if let Some(breakdown) = phase_breakdown.lock().unwrap().as_deref() {
        tracing::info!("Encode breakdown: {breakdown}");
    }
    result
}

/// Whether an encode timed anything. A still or a J2K sequence reports four
/// zeros.
fn measured_any_phase(progress: &postkit::pipeline::PipelineProgress) -> bool {
    progress.decode_wait_secs > 0.0
        || progress.prepare_secs > 0.0
        || progress.encode_secs > 0.0
        || progress.write_secs > 0.0
}

/// What the sound track file's MCA soundfield group says about the mix. Boxed
/// for the same reason `PictureArguments` is.
#[derive(clap::Args)]
struct SoundfieldArguments {
    /// MCATitleVersion on the sound track file's soundfield group.
    #[arg(long = "audio-title-version", default_value = "Original Version")]
    title_version: String,

    /// MCAAudioContentKind on the soundfield group: PRM is a primary mix.
    #[arg(long = "audio-content-kind", default_value = "PRM")]
    audio_content_kind: String,

    /// MCAAudioElementKind on the soundfield group: FCMP is a final complete mix.
    #[arg(long = "audio-element-kind", default_value = "FCMP")]
    audio_element_kind: String,
}

/// How many bits a frame of `create`'s picture gets. Boxed for the same reason
/// `PictureArguments` is.
#[derive(clap::Args)]
struct CompressionArguments {
    /// Delivery preset (netflix, disney, apple, hbo, amazon, dci-2k, dci-4k,
    /// broadcast, archival); sets the J2K target bitrate. See `profiles`.
    #[arg(long)]
    profile: Option<String>,

    /// Target bitrate in Mbps for the J2K encode, above 0 and at most 1000.
    /// Wins over the bitrate --profile carries. Without either the encode
    /// runs at a 10:1 compression ratio.
    #[arg(long)]
    bitrate: Option<f64>,

    /// PSNR target in dB for the J2K encode, at least 20 and at most 80.
    /// The encoder allocates to this quality instead of a compression
    /// ratio, and the bitrate becomes a per-frame byte cap no frame may
    /// exceed.
    #[arg(long)]
    quality_psnr: Option<f64>,
}

/// What `create` cuts and converts before the encode. Boxed for the same reason
/// `PictureArguments` is.
#[derive(clap::Args)]
struct SourceEditArguments {
    /// Shift the sound against the picture in milliseconds; positive is
    /// later. The running time never changes: the shift is padded at one end
    /// and truncated at the other.
    #[arg(long = "audio-delay")]
    audio_delay: Option<i64>,

    /// Colour space the picture source carries. An App 2E picture ships the
    /// Rec.709 RGB its essence descriptor declares: rec709 (the default) is
    /// compressed untransformed, and p3d65, rec2020 and logc are converted
    /// to Rec.709 RGB during the encode. xyz, p3, aces and acescg are
    /// refused by name.
    #[arg(long = "source-colourspace")]
    source_colourspace: Option<String>,

    /// 3D LUT (.cube) applied during decode, whose output must be Rec.709 RGB.
    #[arg(long = "source-lut", conflicts_with = "source_colourspace")]
    source_lut: Option<PathBuf>,

    /// Remove this much from the head of the source, as frames (48f) or
    /// seconds (2s). Picture, sound and timed text all move together.
    #[arg(long = "trim-start")]
    trim_start: Option<String>,

    /// Remove this much from the tail of the source, spelled as --trim-start.
    #[arg(long = "trim-end")]
    trim_end: Option<String>,

    /// Hold a single image for this long, as frames (48f) or seconds (2s).
    /// Requires --video to name one image file rather than a video or a
    /// directory of frames.
    #[arg(long = "still-length")]
    still_length: Option<String>,
}

/// What `create` does to the source picture before it is compressed. Boxed
/// where it is flattened, so `create` does not dwarf every other subcommand in
/// the parsed command enum.
#[derive(clap::Args)]
struct PictureArguments {
    /// Pixels cut off the left of the source, before any rotation.
    #[arg(long = "crop-left", default_value_t = 0)]
    crop_left: u32,

    /// Pixels cut off the right of the source, before any rotation.
    #[arg(long = "crop-right", default_value_t = 0)]
    crop_right: u32,

    /// Pixels cut off the top of the source, before any rotation.
    #[arg(long = "crop-top", default_value_t = 0)]
    crop_top: u32,

    /// Pixels cut off the bottom of the source, before any rotation.
    #[arg(long = "crop-bottom", default_value_t = 0)]
    crop_bottom: u32,

    /// Measure the black borders of the source and crop them away.
    #[arg(long = "auto-crop")]
    auto_crop: bool,

    /// How black a pixel has to be to count as border, 0 to 1. Requires
    /// --auto-crop.
    #[arg(long = "auto-crop-threshold")]
    auto_crop_threshold: Option<f32>,

    /// Crop the source to the target raster's aspect instead of padding it, so
    /// the picture fills the frame and the excess is cut away.
    #[arg(long = "fill-crop")]
    fill_crop: bool,

    /// Turn the source's fields into progressive frames (yadif).
    #[arg(long)]
    deinterlace: bool,

    /// Run the source through a denoiser (hqdn3d) at its defaults.
    #[arg(long)]
    denoise: bool,

    /// Turn the picture clockwise: 90, 180 or 270 degrees.
    #[arg(long)]
    rotate: Option<String>,

    /// Mirror the picture: horizontal, vertical or both.
    #[arg(long)]
    flip: Option<String>,

    /// Raster the picture is fitted into, one of 1920x1080, 2048x1080,
    /// 3840x2160 or 4096x2160. Defaults to the source's own raster.
    #[arg(long)]
    raster: Option<String>,
}

impl PictureArguments {
    /// Read the flags into the shared options, failing on a bad spelling.
    fn resolve(&self) -> imfwizard_core::source_picture::SourcePictureOptions {
        if self.auto_crop_threshold.is_some() && !self.auto_crop {
            fail("--auto-crop-threshold requires --auto-crop");
        }
        let rotation = self
            .rotate
            .as_deref()
            .map(|value| {
                postkit::picture_processing::parse_rotation(value).unwrap_or_else(|e| fail(e))
            })
            .unwrap_or_default();
        let (flip_horizontal, flip_vertical) = self
            .flip
            .as_deref()
            .map(|value| postkit::picture_processing::parse_flip(value).unwrap_or_else(|e| fail(e)))
            .unwrap_or((false, false));
        let options = imfwizard_core::source_picture::SourcePictureOptions {
            crop: postkit::picture_processing::Crop {
                left: self.crop_left,
                right: self.crop_right,
                top: self.crop_top,
                bottom: self.crop_bottom,
            },
            auto_crop: self.auto_crop,
            auto_crop_threshold: self
                .auto_crop_threshold
                .unwrap_or(postkit::picture_processing::DEFAULT_AUTO_CROP_THRESHOLD),
            fill_crop: self.fill_crop,
            deinterlace: self.deinterlace,
            denoise: self.denoise,
            rotation,
            flip_horizontal,
            flip_vertical,
            raster: self.raster.as_deref().map(|value| {
                imfwizard_core::source_picture::parse_raster(value).unwrap_or_else(|e| fail(e))
            }),
        };
        options.check().unwrap_or_else(|e| fail(e));
        options
    }
}

/// A whole, as the appearance flags spell their fractions.
const PERCENT_OF_A_WHOLE: f32 = 100.0;

/// Written out so the help text cannot drift from the ratio the rasteriser
/// actually starts from.
static BURN_FONT_SIZE_HELP: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "Burnt-in text height as a percent of the frame height (default: {:.1})",
        postkit::subtitle_raster::DEFAULT_FONT_SIZE_RATIO * PERCENT_OF_A_WHOLE
    )
});

static BURN_OUTLINE_WIDTH_HELP: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "Outline thickness as a percent of the burnt-in text height (default: {:.1})",
        postkit::subtitle_raster::DEFAULT_OUTLINE_WIDTH_RATIO * PERCENT_OF_A_WHOLE
    )
});

static BURN_LINE_HEIGHT_HELP: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "Line box height as a multiple of the burnt-in text height (default: {})",
        postkit::subtitle_raster::DEFAULT_LINE_HEIGHT_RATIO
    )
});

static BURN_MARGIN_HELP: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "Distance from the anchored edge as a percent of the frame height (default: {:.1})",
        postkit::subtitle_raster::DEFAULT_MARGIN_RATIO * PERCENT_OF_A_WHOLE
    )
});

/// Everything `create` takes about a burnt-in subtitle: the cue file, the face
/// to draw it with, and how the text looks. Boxed where it is flattened, for the
/// reason `PictureArguments` is. Every appearance flag is optional, so a value
/// the caller never names keeps the rasteriser's own default.
#[derive(clap::Args)]
struct BurnArguments {
    /// Subtitle file rendered into the picture during the encode (SRT, ASS,
    /// SCC, FCPXML or MKS). Burnt-in text is part of the image and registers
    /// no timed-text track.
    #[arg(long = "burn-subtitle")]
    burn_subtitle: Option<String>,

    /// TTF/OTF font to draw --burn-subtitle with (default: a system font)
    #[arg(long = "burn-subtitle-font")]
    burn_subtitle_font: Option<String>,

    #[arg(long = "burn-font-size", help = BURN_FONT_SIZE_HELP.as_str())]
    burn_font_size: Option<f32>,

    /// Burnt-in text colour as RRGGBB or RRGGBBAA (default: white).
    #[arg(long = "burn-colour")]
    burn_colour: Option<String>,

    /// What is drawn under the burnt-in text: none, outline or shadow
    /// (default: shadow).
    #[arg(long = "burn-effect")]
    burn_effect: Option<String>,

    /// Colour of that outline or shadow, as RRGGBB or RRGGBBAA (default: black).
    #[arg(long = "burn-effect-colour")]
    burn_effect_colour: Option<String>,

    #[arg(long = "burn-outline-width", help = BURN_OUTLINE_WIDTH_HELP.as_str())]
    burn_outline_width: Option<f32>,

    #[arg(long = "burn-line-height", help = BURN_LINE_HEIGHT_HELP.as_str())]
    burn_line_height: Option<f32>,

    #[arg(long = "burn-margin", help = BURN_MARGIN_HELP.as_str())]
    burn_margin: Option<f32>,

    /// Horizontal stretch of the burnt-in text, where 1.0 leaves it alone.
    #[arg(long = "burn-x-scale")]
    burn_x_scale: Option<f32>,

    /// Vertical stretch of the burnt-in text, where 1.0 leaves it alone.
    #[arg(long = "burn-y-scale")]
    burn_y_scale: Option<f32>,

    /// Milliseconds a burnt-in cue takes to ramp up from transparent.
    #[arg(long = "burn-fade-up")]
    burn_fade_up: Option<u64>,

    /// Milliseconds a burnt-in cue takes to ramp down to transparent.
    #[arg(long = "burn-fade-down")]
    burn_fade_down: Option<u64>,
}

// BT.2020 primaries and the D65 white point, in the 1/50000 chromaticity units
// SMPTE ST 2086 counts in
const BT2020_GREEN: (u16, u16) = (8500, 39850);
const BT2020_BLUE: (u16, u16) = (6550, 2300);
const BT2020_RED: (u16, u16) = (35400, 14600);
const D65_WHITE_POINT: (u16, u16) = (15635, 16450);
// 1000 cd/m² and 0.0001 cd/m², in ST 2086's 0.0001 cd/m² units
const MASTERING_DISPLAY_MAX_LUMINANCE: u32 = 10_000_000;
const MASTERING_DISPLAY_MIN_LUMINANCE: u32 = 1;

// slate text a fifteenth of the picture height, 72 points on a 1080 line raster
const SLATE_TEXT_HEIGHT_DIVISOR: u32 = 15;

const SMPTE_STRUCTURAL_STANDARD: &str = "SMPTE ST 2067 structure";

// deliver writes here and version reads here, so the two cannot drift
const DELIVERY_DB: &str = "deliveries.db";

// what `validate` prints, so `create`'s verify pass reads the same. False when
// the package has errors.
fn print_validation(result: &imfwizard_core::validate::ValidationResult) -> bool {
    if result.valid {
        println!("IMP validation PASSED");
        for warning in &result.warnings {
            println!("  warning: {warning}");
        }
        return true;
    }
    eprintln!("IMP validation FAILED");
    for error in &result.errors {
        eprintln!("  error: {error}");
    }
    for warning in &result.warnings {
        eprintln!("  warning: {warning}");
    }
    false
}

// the timestamp a delivery record carries
fn rfc3339_now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

// what `compliance --standard` takes: a delivery profile, or None for the
// structural checks alone, since ST 2067 fixes no raster or bit depth
const COMPLIANCE_STANDARDS: &[(&str, Option<postkit::profiles::Platform>)] = &[
    ("smpte", None),
    ("netflix", Some(postkit::profiles::Platform::Netflix)),
    ("amazon", Some(postkit::profiles::Platform::AmazonPrime)),
    ("prime", Some(postkit::profiles::Platform::AmazonPrime)),
    ("disney", Some(postkit::profiles::Platform::Disney)),
    ("disney+", Some(postkit::profiles::Platform::Disney)),
    ("apple", Some(postkit::profiles::Platform::Apple)),
    ("appletv", Some(postkit::profiles::Platform::Apple)),
    ("hbo", Some(postkit::profiles::Platform::Hbo)),
    ("broadcast", Some(postkit::profiles::Platform::Broadcast)),
    (
        "archival",
        Some(postkit::profiles::Platform::ArchivalPreservation),
    ),
    ("dci-2k", Some(postkit::profiles::Platform::TheatricalDci2k)),
    (
        "cinema-2k",
        Some(postkit::profiles::Platform::TheatricalDci2k),
    ),
    ("dci-4k", Some(postkit::profiles::Platform::TheatricalDci4k)),
    (
        "cinema-4k",
        Some(postkit::profiles::Platform::TheatricalDci4k),
    ),
];

/// The SMPTE ST 2086 mastering display `hdr10-inject` writes as HEVC SEI.
#[derive(clap::Args)]
struct MasteringDisplayArguments {
    /// Green primary x, in 1/50000
    #[arg(long, default_value_t = BT2020_GREEN.0)]
    display_primaries_gx: u16,

    /// Green primary y, in 1/50000
    #[arg(long, default_value_t = BT2020_GREEN.1)]
    display_primaries_gy: u16,

    /// Blue primary x, in 1/50000
    #[arg(long, default_value_t = BT2020_BLUE.0)]
    display_primaries_bx: u16,

    /// Blue primary y, in 1/50000
    #[arg(long, default_value_t = BT2020_BLUE.1)]
    display_primaries_by: u16,

    /// Red primary x, in 1/50000
    #[arg(long, default_value_t = BT2020_RED.0)]
    display_primaries_rx: u16,

    /// Red primary y, in 1/50000
    #[arg(long, default_value_t = BT2020_RED.1)]
    display_primaries_ry: u16,

    /// White point x, in 1/50000
    #[arg(long, default_value_t = D65_WHITE_POINT.0)]
    white_point_x: u16,

    /// White point y, in 1/50000
    #[arg(long, default_value_t = D65_WHITE_POINT.1)]
    white_point_y: u16,

    /// Mastering display peak luminance, in 0.0001 cd/m²
    #[arg(long, default_value_t = MASTERING_DISPLAY_MAX_LUMINANCE)]
    max_luminance: u32,

    /// Mastering display black luminance, in 0.0001 cd/m²
    #[arg(long, default_value_t = MASTERING_DISPLAY_MIN_LUMINANCE)]
    min_luminance: u32,
}

impl MasteringDisplayArguments {
    fn metadata(&self, max_cll: u16, max_fall: u16) -> postkit::dolby_vision::Hdr10Metadata {
        postkit::dolby_vision::Hdr10Metadata {
            display_primaries_gx: self.display_primaries_gx,
            display_primaries_gy: self.display_primaries_gy,
            display_primaries_bx: self.display_primaries_bx,
            display_primaries_by: self.display_primaries_by,
            display_primaries_rx: self.display_primaries_rx,
            display_primaries_ry: self.display_primaries_ry,
            white_point_x: self.white_point_x,
            white_point_y: self.white_point_y,
            max_luminance: self.max_luminance,
            min_luminance: self.min_luminance,
            max_cll,
            max_fall,
        }
    }
}

impl BurnArguments {
    /// Read the flags into the rasteriser's overrides, failing on a colour or an
    /// effect name that cannot be read.
    fn resolve(&self) -> postkit::subtitle_raster::BurnStyleOverrides {
        let colour = |flag: &str, value: &Option<String>| {
            value.as_deref().map(|text| {
                postkit::subtitle_formats::Rgba::parse_hex(text)
                    .unwrap_or_else(|e| fail(format!("{flag}: {e}")))
            })
        };
        postkit::subtitle_raster::BurnStyleOverrides {
            font_size_percent: self.burn_font_size,
            colour: colour("--burn-colour", &self.burn_colour),
            effect: self.burn_effect.as_deref().map(|text| {
                postkit::subtitle_raster::parse_burn_effect(text)
                    .unwrap_or_else(|e| fail(format!("--burn-effect: {e}")))
            }),
            effect_colour: colour("--burn-effect-colour", &self.burn_effect_colour),
            outline_width_percent: self.burn_outline_width,
            line_height_ratio: self.burn_line_height,
            margin_percent: self.burn_margin,
            x_scale: self.burn_x_scale,
            y_scale: self.burn_y_scale,
            fade_up_ms: self.burn_fade_up,
            fade_down_ms: self.burn_fade_down,
        }
    }

    /// The first appearance flag the caller named, so a burn-less run can be
    /// refused by the flag's own name rather than by the group's.
    fn first_named(&self) -> Option<&'static str> {
        [
            ("--burn-font-size", self.burn_font_size.is_some()),
            ("--burn-colour", self.burn_colour.is_some()),
            ("--burn-effect", self.burn_effect.is_some()),
            ("--burn-effect-colour", self.burn_effect_colour.is_some()),
            ("--burn-outline-width", self.burn_outline_width.is_some()),
            ("--burn-line-height", self.burn_line_height.is_some()),
            ("--burn-margin", self.burn_margin.is_some()),
            ("--burn-x-scale", self.burn_x_scale.is_some()),
            ("--burn-y-scale", self.burn_y_scale.is_some()),
            ("--burn-fade-up", self.burn_fade_up.is_some()),
            ("--burn-fade-down", self.burn_fade_down.is_some()),
        ]
        .into_iter()
        .find(|(_, given)| *given)
        .map(|(name, _)| name)
    }
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Show or change saved preferences")]
    Preferences {
        #[command(subcommand)]
        action: PreferencesCommand,
    },
    /// Create a new IMP (Interoperable Master Package)
    #[command(allow_negative_numbers = true)]
    Create {
        /// Output directory for the IMP
        #[arg(short, long)]
        output: PathBuf,

        /// Title of the content
        #[arg(short, long)]
        title: String,

        /// Video file (mp4/mov/mkv) or J2K directory
        #[arg(long)]
        video: Option<String>,

        /// Audio WAV file (auto-demuxed from video if not provided)
        #[arg(long)]
        audio: Option<String>,

        /// RFC 5646 language tag for the audio track (e.g. de-DE)
        #[arg(long = "audio-lang")]
        audio_lang: Option<String>,

        /// Accessibility role for the audio track: ad (audio description /
        /// visually impaired) or hi (hearing impaired). Emits an MCA descriptor.
        #[arg(long = "audio-role")]
        audio_role: Option<String>,

        #[command(flatten)]
        soundfield: Box<SoundfieldArguments>,

        /// TTML/IMSC subtitle file to package (repeatable)
        #[arg(long = "subtitle")]
        subtitles: Vec<String>,

        #[command(flatten)]
        burn: Box<BurnArguments>,

        /// Content kind (feature, trailer, etc.)
        #[arg(short, long, default_value = "feature")]
        kind: String,

        #[command(flatten)]
        compression: Box<CompressionArguments>,

        /// Frame rate numerator
        #[arg(long, default_value = "24")]
        fps_num: u32,

        /// Frame rate denominator
        #[arg(long, default_value = "1")]
        fps_den: u32,

        /// HDR/WCG preset for the picture essence (ST 2067-21): pq-bt2020,
        /// pq-p3d65 or hlg-bt2020. Writes the transfer/colour ULs into the MXF
        /// and CPL, and has to agree with what the source signals.
        #[arg(long)]
        hdr: Option<String>,

        /// ST 2086 mastering display, x265 master-display string, e.g.
        /// "G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(40000000,50)".
        /// Requires --hdr.
        #[arg(long = "mastering-display")]
        mastering_display: Option<String>,

        /// Maximum content light level in nits, written as a ST 2067-21 CPL
        /// ExtensionProperty. Requires a PQ --hdr preset.
        #[arg(long = "max-cll")]
        max_cll: Option<u16>,

        /// Maximum frame-average light level in nits, same placement as
        /// --max-cll. Requires a PQ --hdr preset.
        #[arg(long = "max-fall")]
        max_fall: Option<u16>,

        #[command(flatten)]
        source_edits: Box<SourceEditArguments>,

        #[command(flatten)]
        picture: Box<PictureArguments>,

        /// Route and mix the --audio channels, as comma-separated IN:OUT or
        /// IN:OUT@GAIN entries. IN is a 1-based input channel, OUT is a channel
        /// number or a name (L, R, C, LFE, Ls, Rs, Lrs, Rrs) and GAIN is
        /// decibels, e.g. "1:L,2:R,1:C@-6".
        #[arg(long = "audio-map")]
        audio_map: Option<String>,

        /// Run the pre-build check and stop: every refusal and every hint,
        /// without encoding or writing anything under --output.
        #[arg(long)]
        check: bool,

        /// Leave the codestreams and the edited sound in the output directory
        /// once the IMP is written. A finished package holds neither.
        #[arg(long = "keep-intermediates")]
        keep_intermediates: bool,

        /// Skip the validation that otherwise runs over the finished package.
        #[arg(long = "no-verify")]
        no_verify: bool,
    },

    /// Encode image sequence to J2K codestreams
    Encode {
        /// Input directory of image frames
        #[arg(short, long)]
        input: PathBuf,

        /// Output directory for J2K codestreams
        #[arg(short, long)]
        output: PathBuf,

        /// Target bitrate in Mbps
        #[arg(short, long, default_value_t = imfwizard_core::encode::DEFAULT_ENCODE_BITRATE_MBPS)]
        bitrate: f64,
    },

    /// Transcode media via ffmpeg
    Transcode {
        /// Input file
        #[arg(short, long)]
        input: PathBuf,

        /// Output file
        #[arg(short, long)]
        output: PathBuf,

        /// Video codec
        #[arg(short, long, default_value = "libx264")]
        codec: String,
    },

    /// Convert subtitles to TTML for IMF
    SubtitleConvert {
        /// Input subtitle file
        #[arg(short, long)]
        input: PathBuf,

        /// Output TTML file
        #[arg(short, long)]
        output: PathBuf,

        /// Default text height as a percent of the frame height, written as a
        /// cell-relative tts:fontSize on every cue.
        #[arg(long = "font-size")]
        font_size: Option<f32>,

        /// Default text colour as RRGGBB or RRGGBBAA, written as the document's
        /// tts:color. A run that carries its own colour keeps it.
        #[arg(long = "colour")]
        colour: Option<String>,
    },

    /// Extract HDR10+ metadata
    Hdr10plusExtract {
        /// Input HEVC file
        #[arg(short, long)]
        input: PathBuf,

        /// Output JSON file
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Extract Dolby Vision RPU
    DvExtract {
        /// Input HEVC file
        #[arg(short, long)]
        input: PathBuf,

        /// Output RPU file
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Analyze an IMP directory
    #[command(name = "analytics")]
    Analytics {
        /// IMP directory to analyze
        #[arg(long = "dir", short = 'd')]
        input: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Video MXF file for per-second bitrate analysis
        #[arg(long)]
        video: Option<PathBuf>,

        /// Number of histogram buckets for bitrate distribution
        #[arg(long, default_value = "20")]
        histogram_buckets: usize,
    },

    /// Compute hash of a file
    Hash {
        /// File to hash
        file: PathBuf,

        /// Hash algorithm (sha1, sha256)
        #[arg(short, long, default_value = "sha1")]
        algorithm: String,
    },

    /// Build an IMP from every master that lands in a watched folder
    Watch {
        /// Directory to watch for masters
        dir: PathBuf,

        /// Directory each package is written to, under the master's file stem
        #[arg(short, long)]
        output: PathBuf,

        /// POST a JSON notification to this URL when a package is built or fails
        #[arg(long)]
        webhook_url: Option<String>,

        /// Seconds between polls
        #[arg(
            long,
            default_value_t = imfwizard_core::watch::DEFAULT_POLL_INTERVAL_SECONDS,
            value_parser = clap::value_parser!(u64).range(
                imfwizard_core::watch::MINIMUM_POLL_INTERVAL_SECONDS..
            )
        )]
        interval: u64,

        /// Flags passed to `create` for every package, after a `--` separator
        #[arg(last = true)]
        create_arguments: Vec<String>,
    },

    /// List delivery profiles
    Profiles,

    /// Show timecode conversion
    Timecode {
        /// Timecode string (HH:MM:SS:FF)
        tc: String,

        /// Frame rate
        #[arg(short, long, default_value = "24")]
        fps: u8,
    },

    /// Validate an IMP directory
    Validate {
        /// IMP directory to validate
        dir: String,

        /// Also validate XML files against SMPTE ST 2067 XSD schemas
        #[arg(long)]
        xsd: bool,

        /// Directory containing SMPTE XSD schema files
        #[arg(long)]
        schema_dir: Option<String>,

        /// Also run Netflix Photon (needs a JRE + Photon jar)
        #[arg(long)]
        photon: bool,

        /// Path to the Photon jar (else PHOTON_JAR env var)
        #[arg(long)]
        photon_jar: Option<String>,
    },

    /// Measure audio loudness (EBU R128), or adjust to a target with --adjust-to
    #[command(allow_negative_numbers = true)]
    Loudness {
        /// Audio file to measure: a WAV or a PCM MXF (--adjust-to needs a WAV)
        audio_file: String,
        /// Adjust integrated loudness to this target in LUFS, writing --output
        #[arg(long)]
        adjust_to: Option<f64>,
        /// Output WAV for the adjusted audio (required with --adjust-to)
        #[arg(short, long)]
        output: Option<String>,
        /// True-peak ceiling in dBTP for the clip-safe check
        #[arg(long, default_value_t = postkit::loudness::DEFAULT_TRUE_PEAK_CEILING_DBTP)]
        true_peak: f64,
    },

    /// Burn subtitles into video
    #[command(name = "burn-in")]
    BurnIn {
        /// Input video file
        #[arg(short, long)]
        input: String,

        /// Subtitle file
        #[arg(short, long)]
        subtitles: String,

        /// Output video file
        #[arg(short, long)]
        output: String,
    },

    /// Show IMP metadata
    Info {
        /// IMP directory
        dir: String,
    },

    /// Create a supplemental IMP
    Supplement {
        /// Original Version (OV) IMP directory
        #[arg(long)]
        ov: String,

        /// Title for the supplemental package
        #[arg(short, long)]
        title: String,

        /// Output directory
        #[arg(short, long)]
        output: String,

        /// Replace an OV track: <path>@<track> where track is video, audio[:N], subtitle[:N] (repeatable)
        #[arg(long = "replace")]
        replace: Vec<String>,

        /// Add a new track: <path>@<track> where track is audio or subtitle (repeatable)
        #[arg(long = "add")]
        add: Vec<String>,
    },

    /// Convert IMP to DCP
    #[command(name = "to-dcp")]
    ToDcp {
        /// Input IMP directory
        #[arg(short, long)]
        input: String,

        /// Output DCP directory
        #[arg(short, long)]
        output: String,

        /// Content kind (feature, trailer, etc.)
        #[arg(short, long, default_value = "feature")]
        kind: String,

        /// Title override
        #[arg(short, long)]
        title: Option<String>,

        /// Mb/s the transcode path compresses at, up to the DCI maximum of 250
        #[arg(long)]
        bitrate: Option<f64>,
    },

    /// Export a composition's picture track to a numbered image sequence
    #[command(name = "export-frames")]
    ExportFrames {
        /// Input IMP directory
        #[arg(short, long)]
        input: String,

        /// Output directory for the image sequence
        #[arg(short, long)]
        output: String,

        /// Output image format: tiff or png (native bit depth, no colour transform)
        #[arg(short, long, default_value = "tiff")]
        format: String,

        /// CPL to export by UUID or 0-based index; defaults to the sole CPL
        #[arg(long)]
        cpl: Option<String>,

        /// First composition frame to export (0-based)
        #[arg(long, default_value = "0")]
        start: u32,

        /// Number of frames to export (defaults to all remaining)
        #[arg(long)]
        count: Option<u32>,
    },

    /// Sign an IMF XML document (CPL/PKL/OPL) with an enveloped XML signature
    Sign {
        /// Input XML file
        #[arg(short, long)]
        input: PathBuf,

        /// Output signed XML file (defaults to overwriting the input)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Signer certificate PEM file
        #[arg(long)]
        cert: PathBuf,

        /// Signer private key PEM file
        #[arg(long)]
        key: PathBuf,

        /// CA chain PEM file (repeatable, leaf to root)
        #[arg(long = "chain")]
        chain: Vec<PathBuf>,
    },

    /// Verify an IMF XML document's enveloped signature
    #[command(name = "verify-sig")]
    VerifySig {
        /// Input signed XML file
        #[arg(short, long)]
        input: PathBuf,

        /// Trusted certificate PEM (the embedded signing cert must match it)
        #[arg(long)]
        trusted_cert: Option<PathBuf>,
    },

    /// Generate QC report for an IMP
    #[command(alias = "qc-report")]
    Report {
        /// IMP directory
        #[arg(long)]
        imp: String,

        /// Output report file
        #[arg(short, long)]
        output: String,

        /// Format: text, json, html
        #[arg(short, long, default_value = "html")]
        format: String,

        /// Decode every picture frame to find black and frozen runs
        #[arg(long)]
        scan_picture: bool,
    },

    /// Start REST API server
    #[command(alias = "rest-api")]
    Serve {
        /// Listen address (host:port)
        #[arg(short, long, default_value = "127.0.0.1:8081")]
        bind: String,
        /// Require this key on requests (X-Api-Key or Authorization: Bearer)
        #[arg(long)]
        api_key: Option<String>,
    },

    /// Generate shell completions
    #[command(alias = "completions")]
    Completion {
        /// Shell (bash|zsh|fish)
        #[arg(default_value = "bash")]
        shell: String,
    },

    /// Add a revision annotation to a CPL. To change the ContentTitle, use
    /// `retitle`
    #[command(name = "metadata-edit")]
    MetadataEdit {
        /// IMP directory
        #[arg(short, long)]
        imp: String,

        /// Annotation text (an alias of --annotation; this does not touch the
        /// CPL ContentTitle)
        #[arg(short, long)]
        title: Option<String>,

        /// Annotation text
        #[arg(short, long)]
        annotation: Option<String>,

        /// Annotation author
        #[arg(long)]
        issuer: Option<String>,
    },

    /// Give a written IMP a new ContentTitle, without re-wrapping essence. The
    /// CPL gets a new composition id, and the PKL and ASSETMAP are repointed at
    /// it, so any KDM or supplemental IMP made from the old id stops matching.
    /// A signed document is rewritten unsigned
    Retitle {
        /// IMP directory
        #[arg(short, long)]
        imp: String,

        /// New CPL ContentTitle
        #[arg(short, long)]
        title: String,
    },

    /// Create DCDM (Digital Cinema Distribution Master) X'Y'Z' sequence
    Dcdm {
        /// Input image sequence directory
        #[arg(short, long)]
        input: String,

        /// Output DCDM TIFF directory
        #[arg(short, long)]
        output: String,

        /// Source colour space (rec709, p3, rec2020, xyz or logc). aces and
        /// acescg are scene-referred and need --lut instead.
        #[arg(short, long, default_value = "rec709")]
        colour_space: String,

        /// Optional 3D LUT for colour transform
        #[arg(long)]
        lut: Option<String>,

        /// Resolution width
        #[arg(long, default_value = "4096")]
        width: u32,

        /// Resolution height
        #[arg(long, default_value = "2160")]
        height: u32,
    },

    /// Convert colour space of images/video
    Colour {
        /// Input file or directory
        #[arg(short, long)]
        input: String,

        /// Output file or directory
        #[arg(short, long)]
        output: String,

        /// Source colour space (rec709, p3 or rec2020). xyz, aces, acescg and
        /// logc have no ffmpeg colorspace model and need --lut.
        #[arg(short, long)]
        source: String,

        /// Target colour space
        #[arg(short, long)]
        target: String,

        /// Optional 3D LUT file for custom transform
        #[arg(long)]
        lut: Option<String>,
    },

    /// Import an EDL or XML timeline for conforming
    ///
    /// With --media-dir and --output it builds an IMP whose CPL follows the
    /// timeline, one picture and sound resource per event. With neither it
    /// prints the parsed timeline and writes nothing.
    Conform {
        /// Input timeline file (EDL, FCP XML, OTIO)
        #[arg(short, long)]
        input: String,

        /// Directory holding the source media the timeline names
        #[arg(long)]
        media_dir: Option<String>,

        /// Output IMP directory
        #[arg(short, long)]
        output: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Extract a frame from video/MXF as image
    #[command(name = "frame-extract")]
    FrameExtract {
        /// Input video/MXF file
        #[arg(short, long)]
        input: String,

        /// Frame number to extract
        #[arg(short, long, default_value = "0")]
        frame: u32,

        /// Output image file (png, jpg, tiff)
        #[arg(short, long)]
        output: String,

        /// Content key for encrypted picture essence, 32 hex chars. Other users
        /// of the machine can read it from the process list: prefer --keys-json
        #[arg(long, conflicts_with = "keys_json")]
        key: Option<String>,

        /// KEYS.json holding the content key for encrypted essence
        #[arg(long)]
        keys_json: Option<String>,
    },

    /// Inject Dolby Vision RPU into HEVC stream
    #[command(name = "dv-inject")]
    DvInject {
        /// Input HEVC file
        #[arg(short, long)]
        input: String,

        /// RPU file (.bin)
        #[arg(short, long)]
        rpu: String,

        /// Output file
        #[arg(short, long)]
        output: String,
    },

    /// Inject HDR10 static metadata
    #[command(name = "hdr10-inject")]
    Hdr10Inject {
        /// Input video file
        #[arg(short, long)]
        input: String,

        /// Output video file
        #[arg(short, long)]
        output: String,

        /// Max content light level (MaxCLL)
        #[arg(long, default_value = "1000")]
        max_cll: u16,

        /// Max frame average light level (MaxFALL)
        #[arg(long, default_value = "400")]
        max_fall: u16,

        #[command(flatten)]
        mastering_display: Box<MasteringDisplayArguments>,
    },

    /// Burn a visible operator/session watermark into an image sequence
    Watermark {
        /// Input image sequence directory
        #[arg(short, long)]
        input: String,

        /// Output directory
        #[arg(short, long)]
        output: String,

        /// Operator ID for watermark payload
        #[arg(long)]
        operator_id: String,

        /// Session ID for watermark payload
        #[arg(long)]
        session_id: String,

        /// Burn-in opacity (0.0 to 1.0)
        #[arg(long, default_value = "0.5")]
        strength: f32,
    },

    /// Package a trailer (ratings card + countdown + content)
    Trailer {
        /// Content image sequence directory
        #[arg(short, long)]
        content: String,

        /// Audio file for trailer
        #[arg(short, long)]
        audio: String,

        /// Output directory
        #[arg(short, long)]
        output: String,

        /// Trailer title
        #[arg(long)]
        title: String,

        /// Rating (e.g. PG-13, R). Left out, the rating system's lowest rating
        /// is drawn
        #[arg(long, default_value = "")]
        rating: String,

        /// Rating system the card follows: mpaa, bbfc or fsk
        #[arg(long = "rating-system", default_value = "mpaa")]
        rating_system: String,

        /// Band colour behind the card: green, red or yellow
        #[arg(long, default_value = "green")]
        band: String,
    },

    /// Preview IMP via mpv
    Preview {
        /// IMP directory
        #[arg(short, long)]
        input: String,
    },

    /// Check accessibility compliance or mix audio description with ducking
    #[command(name = "audio-desc")]
    AudioDesc {
        /// Package directory (for compliance check) or main audio file (for mix)
        #[arg(short, long)]
        input: String,

        /// Standard to check against (cvaa, eaa, aoda, ofcom)
        #[arg(short, long, default_value = "cvaa")]
        standard: String,

        /// Audio description narration file to mix with main audio
        #[arg(long)]
        narration: Option<String>,

        /// Output mixed audio file (required with --narration)
        #[arg(short, long)]
        output: Option<String>,

        /// Duck level in dB (how much to reduce main audio during narration)
        #[arg(long, default_value = "-12.0")]
        duck_level: f64,
    },

    /// Compare two IMPs
    Compare {
        /// First IMP directory or video file
        #[arg(short, long)]
        a: String,

        /// Second IMP directory or video file
        #[arg(short, long)]
        b: String,

        /// Enable pixel-level PSNR/SSIM comparison (requires video MXF inputs)
        #[arg(long)]
        pixel: bool,

        /// Compute VMAF via ffmpeg's libvmaf filter (requires video inputs)
        #[arg(long)]
        vmaf: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Apply 3D LUT to image sequence
    Lut {
        /// Input image sequence
        #[arg(short, long)]
        input: String,

        /// Output image sequence
        #[arg(short, long)]
        output: String,

        /// 3D LUT file (.cube, .3dl)
        #[arg(short, long)]
        lut: String,
    },

    /// Encode to ProRes
    Prores {
        /// Input video/image sequence, or an IMP directory to export as ProRes 4444
        #[arg(short, long)]
        input: String,

        /// Output ProRes file
        #[arg(short, long)]
        output: String,

        /// Profile (proxy, lt, standard, hq, 4444, 4444xq). Ignored for an IMP, which is always 4444
        #[arg(short, long, default_value = "hq")]
        profile: String,

        #[arg(long, value_name = "NAME", help = prores_container_help())]
        container: Option<String>,

        /// UUID of the IMP's CPL to export, when it holds more than one
        #[arg(long)]
        cpl: Option<String>,

        /// OV directory holding the track files a supplemental IMP does not ship
        #[arg(long)]
        ov: Option<String>,
    },

    /// Create partial IMP version
    #[command(name = "partial-version")]
    PartialVersion {
        /// Source IMP directory
        #[arg(short, long)]
        input: String,

        /// Output partial IMP
        #[arg(short, long)]
        output: String,

        /// CPL UUID to include
        #[arg(long)]
        cpl: String,
    },

    /// Deliver IMP to destination
    Deliver {
        /// Source IMP directory
        #[arg(short, long)]
        input: String,

        /// Destination path or URI
        #[arg(short, long)]
        destination: String,

        /// Tracker database file, the one `version list` reads
        #[arg(long, default_value = DELIVERY_DB)]
        db: String,
    },

    /// Retime video to target frame rate
    Retime {
        /// Input video file
        #[arg(short, long)]
        input: String,

        /// Output video file
        #[arg(short, long)]
        output: String,

        /// Target FPS
        #[arg(short, long)]
        fps: f64,
    },

    /// Add slate frame(s) to content
    Slate {
        /// Input image sequence or video
        #[arg(short, long)]
        input: String,

        /// Output path
        #[arg(short, long)]
        output: String,

        /// Slate text
        #[arg(long)]
        text: String,

        /// Number of frames
        #[arg(long, default_value = "24")]
        frames: u32,
    },

    /// Write MCA (Multi-Channel Audio) labels into a sound MXF's descriptor
    Mca {
        /// Input MXF audio file, rewrapped in place under the same asset id
        #[arg(short, long)]
        input: String,

        /// Channel layout: mono, stereo, 51 or 71
        #[arg(short, long)]
        layout: String,

        /// Language (e.g. "en", "fr")
        #[arg(short = 'L', long, default_value = "en")]
        language: String,
    },

    /// Check audio/video sync
    #[command(name = "av-sync")]
    AvSync {
        /// Input video file
        #[arg(short, long)]
        input: String,
    },

    /// Wrap Dolby Atmos ADM BWF master into MXF
    Atmos {
        /// Input Dolby Atmos BWF file (with ADM axml chunk)
        #[arg(short, long)]
        input: String,

        /// Output directory for MXF and ADM sidecar
        #[arg(short, long)]
        output: String,

        /// Frame rate numerator
        #[arg(long, default_value = "24")]
        fps_num: u32,

        /// Frame rate denominator
        #[arg(long, default_value = "1")]
        fps_den: u32,
    },

    /// Check external tool dependencies
    Doctor {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// ACES colour pipeline conversion (IDT → RRT → ODT via ctlrender)
    Aces {
        /// Input image/video
        #[arg(short, long)]
        input: String,

        /// Output image/video
        #[arg(short, long)]
        output: String,

        /// Target space (acescg, aces, rec709, p3, xyz) — used for simple conversion
        #[arg(short, long, default_value = "acescg")]
        target: String,

        /// Input Device Transform CTL name (enables full IDT→RRT→ODT pipeline)
        #[arg(long)]
        idt: Option<String>,

        /// Output Device Transform CTL name
        #[arg(long)]
        odt: Option<String>,

        /// CTL transforms directory (defaults to system ACES install)
        #[arg(long)]
        ctl_dir: Option<String>,
    },

    /// Check regulatory compliance
    Compliance {
        /// IMP directory
        #[arg(short, long)]
        input: String,

        #[arg(short, long, default_value = "smpte", help = compliance_standard_help())]
        standard: String,
    },

    /// Annotate CPL metadata
    Annotate {
        /// IMP directory
        #[arg(short, long)]
        imp: String,

        /// Annotation text
        #[arg(short, long)]
        text: String,
    },

    /// Extract/restore tracks from an IMP back to raw essence files
    Restore {
        /// Input IMP directory
        #[arg(short, long)]
        input: String,

        /// Output directory for extracted raw files
        #[arg(short, long)]
        output: String,

        /// Extract only video tracks
        #[arg(long)]
        video_only: bool,

        /// Extract only audio tracks
        #[arg(long)]
        audio_only: bool,
    },

    /// Convert Dolby Vision profile (e.g., profile 5 → profile 8.1)
    #[command(name = "dv-convert")]
    DvConvert {
        /// Input RPU file (.bin), as written by dv-extract
        #[arg(short, long)]
        input: String,

        /// Output file
        #[arg(short, long)]
        output: String,

        /// Target DV profile (8.1, 8.4)
        #[arg(long, default_value = "8.1")]
        target_profile: String,
    },

    /// Content version / delivery history tracker (SQLite)
    Version {
        #[command(subcommand)]
        action: VersionAction,
    },
}

#[derive(Subcommand)]
enum VersionAction {
    /// Record a delivery
    Record {
        /// Tracker database file
        #[arg(long, default_value = DELIVERY_DB)]
        db: String,

        /// Package UUID
        #[arg(long)]
        package_uuid: String,

        /// Title
        #[arg(long, default_value = "")]
        title: String,

        /// Version label (e.g. OV, VF)
        #[arg(long, default_value = "")]
        version: String,

        /// Destination
        #[arg(long, default_value = "")]
        destination: String,

        /// Delivery method (e.g. hard_drive, satellite)
        #[arg(long, default_value = "")]
        method: String,

        /// Mark as verified
        #[arg(long)]
        verified: bool,
    },

    /// List recorded deliveries
    List {
        /// Tracker database file
        #[arg(long, default_value = DELIVERY_DB)]
        db: String,

        /// Filter by package UUID
        #[arg(long)]
        package_uuid: Option<String>,

        /// Filter by destination
        #[arg(long)]
        destination: Option<String>,
    },

    /// Export delivery history (format by extension: .json or .csv)
    Export {
        /// Tracker database file
        #[arg(long, default_value = DELIVERY_DB)]
        db: String,

        /// Output file (.json or .csv)
        #[arg(short, long)]
        output: String,
    },
}

fn nonempty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

fn run_preferences_command(action: &PreferencesCommand) -> i32 {
    let result = match action {
        PreferencesCommand::Show => imfwizard_core::preferences::load_preferences()
            .map_err(|error| error.to_string())
            .and_then(|preferences| {
                serde_json::to_string_pretty(&preferences).map_err(|error| error.to_string())
            }),
        PreferencesCommand::Path => Ok(imfwizard_core::preferences::preferences_path()
            .display()
            .to_string()),
        PreferencesCommand::Reset => imfwizard_core::preferences::reset_preferences()
            .map_err(|error| error.to_string())
            .and_then(|preferences| {
                serde_json::to_string_pretty(&preferences).map_err(|error| error.to_string())
            }),
        PreferencesCommand::Set { name, value } => {
            imfwizard_core::preferences::set_preference(name, value).and_then(|preferences| {
                serde_json::to_string_pretty(&preferences).map_err(|error| error.to_string())
            })
        }
    };

    match result {
        Ok(output) => {
            println!("{output}");
            0
        }
        Err(error) => {
            tracing::error!("{error}");
            1
        }
    }
}

fn main() {
    // Windows debug builds overflow the default 1MB stack due to large clap
    // derive enum (many subcommands with args). Spawn with 8MB stack.
    let thread = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(run)
        .expect("failed to spawn main thread");
    thread.join().unwrap();
}

fn run() {
    std::panic::set_hook(Box::new(|info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unexpected error".to_string()
        };
        let location = info
            .location()
            .map(|l| format!(" ({}:{})", l.file(), l.line()))
            .unwrap_or_default();
        eprintln!("\nerror: imfwizard crashed: {payload}{location}");
        eprintln!(
            "This is a bug. Please report it at https://github.com/PostPerfection/imfwizard/issues"
        );
        if std::env::var("RUST_BACKTRACE").is_ok() {
            eprintln!(
                "\nBacktrace:\n{:?}",
                std::backtrace::Backtrace::force_capture()
            );
        } else {
            eprintln!("Set RUST_BACKTRACE=1 for a detailed backtrace.");
        }
    }));

    let cli = Cli::parse();

    let level = if cli.verbose { "debug" } else { "info" };
    // stdout carries the JSON a `--json` run is piped for, so the log goes beside it
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| level.into()),
        )
        .init();

    if let Commands::Preferences { action } = &cli.command {
        std::process::exit(run_preferences_command(action));
    }

    let preferences = match imfwizard_core::preferences::load_preferences() {
        Ok(preferences) => preferences,
        Err(error) => {
            tracing::error!("could not load preferences: {error}");
            std::process::exit(1);
        }
    };
    let gpu_enabled = if cli.no_gpu {
        false
    } else {
        cli.gpu || preferences.gpu
    };
    let license = cli
        .license
        .as_deref()
        .or_else(|| nonempty(&preferences.gpu_license));
    let registration_url = cli
        .registration_url
        .as_deref()
        .or_else(|| nonempty(&preferences.gpu_registration_url));

    if (cli.license.is_some() || cli.registration_url.is_some()) && !gpu_enabled {
        eprintln!("--license and --registration-url require GPU encoding");
        std::process::exit(2);
    }
    if registration_url.is_some() && license.is_none() {
        eprintln!("--registration-url requires --license");
        std::process::exit(2);
    }

    postkit::grok_encoder::initialize(0);

    if gpu_enabled
        && let Err(e) =
            postkit::grok_encoder::use_gpu_with_authentication(license, registration_url)
    {
        // the preference file is the GUI's too
        if cli.gpu {
            tracing::error!("{e}");
            std::process::exit(1);
        }
        tracing::warn!("{e} The GPU preference is on, so this run stays on the CPU.");
    }

    match cli.command {
        Commands::Preferences { .. } => unreachable!(),
        Commands::Create {
            output,
            title,
            video,
            audio,
            audio_lang,
            audio_role,
            soundfield,
            subtitles,
            burn: burn_arguments,
            kind,
            compression,
            fps_num,
            fps_den,
            hdr,
            mastering_display,
            max_cll,
            max_fall,
            source_edits,
            picture: picture_arguments,
            audio_map,
            check,
            keep_intermediates,
            no_verify,
        } => {
            let CompressionArguments {
                profile,
                bitrate,
                quality_psnr,
            } = *compression;
            let SourceEditArguments {
                audio_delay,
                source_colourspace,
                source_lut,
                trim_start,
                trim_end,
                still_length,
            } = *source_edits;
            // the HDR detail flags only make sense with an HDR preset
            for (name, given) in [
                ("--mastering-display", mastering_display.is_some()),
                ("--max-cll", max_cll.is_some()),
                ("--max-fall", max_fall.is_some()),
            ] {
                if given && hdr.is_none() {
                    eprintln!("Error: {name} requires --hdr");
                    std::process::exit(1);
                }
            }
            // build the HDR/WCG metadata up front so a bad preset/string fails fast
            let hdr = hdr.as_deref().map(|preset| {
                imfwizard_core::hdr_wcg::HdrWcg::from_flags(preset, mastering_display.as_deref())
                    .and_then(|hdr| hdr.with_content_light_levels(max_cll, max_fall))
                    .unwrap_or_else(|e| fail(e))
            });
            // parse the accessibility role up front so a bad value fails fast
            let audio_role = match audio_role.as_deref() {
                Some(s) => match imfwizard_core::imp::AudioRole::from_flag(s) {
                    Some(r) => Some(r),
                    None => {
                        eprintln!("Error: unknown audio role '{s}' (expected ad or hi)");
                        std::process::exit(1);
                    }
                },
                None => None,
            };
            // resolve a delivery preset once; applied to the encode bitrate below
            let preset = match profile.as_deref() {
                Some(name) => match imfwizard_core::profiles::platform_from_name(name) {
                    Some(p) => Some(imfwizard_core::profiles::profile_for(p)),
                    None => {
                        eprintln!("Error: unknown delivery preset '{name}' (see `profiles`)");
                        std::process::exit(1);
                    }
                },
                None => None,
            };
            if let Some(mbps) = bitrate
                && (mbps <= 0.0 || mbps > MAXIMUM_BITRATE_MBPS)
            {
                fail(format!(
                    "--bitrate {mbps} is outside the range: above 0 and at most {MAXIMUM_BITRATE_MBPS} Mbps"
                ));
            }
            let psnr_range = imfwizard_core::encode::MINIMUM_QUALITY_PSNR_DB
                ..=imfwizard_core::encode::MAXIMUM_QUALITY_PSNR_DB;
            if let Some(db) = quality_psnr
                && !psnr_range.contains(&db)
            {
                fail(format!(
                    "--quality-psnr {db} is outside the range: at least {} and at most {} dB",
                    psnr_range.start(),
                    psnr_range.end()
                ));
            }
            // a spelling the encode cannot take has to fail before anything is
            // encoded
            let colourspace = source_colourspace
                .as_deref()
                .map(|s| imfwizard_core::source_colourspace::parse(s).unwrap_or_else(|e| fail(e)));
            let source_colour = match source_lut {
                Some(lut) => postkit::encode::SourceColour::KeepRgbAfterLut(lut),
                None => imfwizard_core::source_colourspace::to_source_colour(
                    colourspace.unwrap_or(imfwizard_core::source_colourspace::APP2E_SOURCE_SPACE),
                )
                .unwrap_or_else(|e| fail(e)),
            };

            let picture_options = picture_arguments.resolve();

            if fps_num == 0 || fps_den == 0 {
                fail(format!(
                    "--fps-num {fps_num} --fps-den {fps_den}: an edit rate needs both above 0"
                ));
            }

            // durations are in edit-rate frames, so they parse against the
            // declared frame rate and fail before the encode
            let frames_from_spec = |spec: &Option<String>| -> Option<u64> {
                spec.as_deref().map(|spec| {
                    imfwizard_core::duration_spec::parse_duration_frames(spec, fps_num, fps_den)
                        .unwrap_or_else(|e| fail(e))
                })
            };
            let edits = imfwizard_core::source_edits::SourceEdits {
                audio_delay_ms: audio_delay.unwrap_or(0),
                trim_start_frames: frames_from_spec(&trim_start).unwrap_or(0),
                trim_end_frames: frames_from_spec(&trim_end).unwrap_or(0),
            };
            let still_frames = frames_from_spec(&still_length);

            let still_input = video
                .as_deref()
                .map(PathBuf::from)
                .filter(|p| postkit::still::is_still_image(p));
            match (&still_input, still_frames) {
                (None, Some(_)) => fail(
                    "--still-length needs --video to name a single image file (dpx, tif, exr, png or bmp)",
                ),
                (Some(image), None) => fail(format!(
                    "--video {} is a single image; --still-length says how long to hold it",
                    image.display()
                )),
                _ => {}
            }

            let timed_text_files: Vec<PathBuf> = subtitles.iter().map(PathBuf::from).collect();

            // a burn draws display-RGB text onto decoded frames, so refuse every
            // route that hands the encoder X'Y'Z' or nothing to draw on, before
            // anything is encoded
            if burn_arguments.burn_subtitle.is_none()
                && let Some(name) = burn_arguments.first_named()
            {
                fail(format!("{name} needs --burn-subtitle"));
            }
            let burn_style = burn_arguments.resolve();
            imfwizard_core::subtitle_burn::resolve_burn_style(&burn_style)
                .unwrap_or_else(|e| fail(e));

            if burn_arguments.burn_subtitle.is_some() && video.is_none() {
                fail("--burn-subtitle needs --video: there is no picture to draw on");
            }

            let plan = imfwizard_core::preflight::CreatePlan {
                picture: video.as_deref().map(PathBuf::from),
                audio_files: audio.iter().map(PathBuf::from).collect(),
                audio_language: audio_lang.clone(),
                timed_text_files: timed_text_files.clone(),
                fps_num,
                fps_den,
                edits,
                audio_map: audio_map.clone(),
                burn_subtitle: burn_arguments.burn_subtitle.as_deref().map(PathBuf::from),
                burn_subtitle_font: burn_arguments
                    .burn_subtitle_font
                    .as_deref()
                    .map(PathBuf::from),
                burn_style: burn_style.clone(),
                picture_options: picture_options.clone(),
                source_colour: source_colour.clone(),
                hdr: hdr.clone(),
                still_frames,
            };
            imfwizard_core::preflight::check_before_encode(&plan).unwrap_or_else(|e| fail(e));

            // a Dolby Vision source carries its own light levels, so the flags can be left off
            let hdr = imfwizard_core::hdr_source::resolve(
                video.as_deref().map(std::path::Path::new),
                hdr,
            )
            .unwrap_or_else(|e| fail(e));

            // the audio level hint measures the whole WAV, minutes on a feature
            let hints_pass = std::thread::spawn(move || imfwizard_core::hints::gather_hints(&plan));
            let print_hints =
                |hints_pass: std::thread::JoinHandle<Vec<postkit::hints::Hint>>| -> usize {
                    let hints = hints_pass.join().expect("the hint pass does not panic");
                    for hint in &hints {
                        tracing::warn!("hint: {}", hint.text);
                    }
                    hints.len()
                };
            if check {
                println!(
                    "Pre-build check passed with {} hint(s); nothing was encoded or written",
                    print_hints(hints_pass)
                );
                return;
            }

            // the edit rate is settled inside each branch, and the cue timings are
            // read against it, so the burn is built there
            let build_subtitle_burn = |fps: imfwizard_core::encode::FrameRate| -> Option<
                std::sync::Arc<postkit::subtitle_raster::SubtitleBurn>,
            > {
                let path = burn_arguments.burn_subtitle.as_deref()?;
                Some(
                    imfwizard_core::subtitle_burn::prepare_subtitle_burn(
                        std::path::Path::new(path),
                        burn_arguments
                            .burn_subtitle_font
                            .as_deref()
                            .map(std::path::Path::new),
                        &burn_style,
                        fps,
                    )
                    .unwrap_or_else(|e| fail(e)),
                )
            };

            let _ = std::fs::create_dir_all(&output);

            // If video is a file, run encode pipeline
            // set by the video branch when the encode narrows to the trim window
            let mut encode_window = None;
            let (j2k_dir, picture_mxf, audio_files) = if let (Some(image), Some(hold_for)) =
                (&still_input, still_frames)
            {
                tracing::info!("Holding {} for {hold_for} frames", image.display());
                let info = imfwizard_core::probe::probe_video(image).unwrap_or_else(|| {
                    fail(format!("cannot read the size of {}", image.display()))
                });
                let picture = imfwizard_core::source_picture::resolve_picture(
                    &picture_options,
                    image,
                    info.width,
                    info.height,
                    false,
                )
                .unwrap_or_else(|e| fail(e));
                tracing::info!("Picture: {}", picture.plan.describe());
                let fps = imfwizard_core::encode::FrameRate::new(fps_num, fps_den);
                // a hold encodes at the default ratio whatever --bitrate says,
                // so the sub level is the one that ratio reaches
                let still_bitrate_mbps = imfwizard_core::encode::bitrate_mbps_for_job(
                    None,
                    None,
                    picture.encode_width,
                    picture.encode_height,
                    fps.as_f64(),
                );
                let rsiz = imfwizard_core::encode::imf_rsiz_for_encode(
                    picture.encode_width,
                    picture.encode_height,
                    fps.as_f64(),
                    still_bitrate_mbps,
                )
                .unwrap_or_else(|e| fail(e));
                let held = output.join(postkit::still::HELD_PICTURE_DIR);
                postkit::still::build_still_frames(&postkit::still::StillHold {
                    image,
                    frames: hold_for,
                    fps,
                    width: picture.encode_width,
                    height: picture.encode_height,
                    filters: &picture.plan.filters,
                    apply_xyz_transform: source_colour.applies_xyz_transform(),
                    rsiz,
                    colour_transform: source_colour.frame_transform().unwrap_or_else(|e| fail(e)),
                    burn: build_subtitle_burn(fps),
                    watermark: None,
                    out_dir: &held,
                })
                .unwrap_or_else(|e| fail(e));
                (
                    Some(held),
                    None,
                    audio
                        .as_ref()
                        .map(|a| vec![PathBuf::from(a)])
                        .unwrap_or_default(),
                )
            } else if let Some(ref vid) = video {
                let video_path = PathBuf::from(vid);
                let named_audio = || {
                    audio
                        .as_ref()
                        .map(|a| vec![PathBuf::from(a)])
                        .unwrap_or_default()
                };
                // the classification the GUI and the preflight already use, so
                // one answer decides what decodes and what reaches the wrapper
                let input_type = postkit::encode::detect_input_type(&video_path);
                match input_type {
                    postkit::encode::InputType::Unknown => fail(
                        imfwizard_core::preflight::unclassified_picture_refusal(&video_path),
                    ),
                    postkit::encode::InputType::J2kSequence => {
                        (Some(video_path), None, named_audio())
                    }
                    postkit::encode::InputType::Video
                    | postkit::encode::InputType::ImageSequence => {
                        let is_image_sequence =
                            input_type == postkit::encode::InputType::ImageSequence;
                        // an image sequence's frames carry no rate of their own,
                        // so the requested one is the rate it is encoded at
                        let probed = match is_image_sequence {
                            true => None,
                            false => imfwizard_core::probe::probe_video(&video_path),
                        };
                        if let Some(ref info) = probed {
                            tracing::info!(
                                "Input: {}x{} @ {}/{} fps",
                                info.width,
                                info.height,
                                info.fps_num,
                                info.fps_den
                            );
                        }
                        // ffprobe answers 0/0 for a stream whose rate it cannot
                        // read, and the declared rate is the better guess
                        let (rate_num, rate_den) = match &probed {
                            Some(info) if info.fps_num > 0 && info.fps_den > 0 => {
                                (info.fps_num, info.fps_den)
                            }
                            _ => (fps_num, fps_den),
                        };
                        let encode_fps = imfwizard_core::encode::FrameRate::new(rate_num, rate_den);
                        tracing::info!(
                            "Detected {}, encoding to J2K at {:.2} fps ({rate_num}/{rate_den})",
                            match is_image_sequence {
                                true => "image sequence",
                                false => "video file",
                            },
                            encode_fps.as_f64()
                        );

                        let (source_width, source_height) =
                            postkit::encode::source_raster(&video_path).unwrap_or_else(|e| fail(e));
                        let picture = imfwizard_core::source_picture::resolve_picture(
                            &picture_options,
                            &video_path,
                            source_width,
                            source_height,
                            is_image_sequence,
                        )
                        .unwrap_or_else(|e| fail(e));
                        tracing::info!("Picture: {}", picture.plan.describe());
                        tracing::info!("Compressor: Grok");

                        let preset_bitrate_mbps = preset.as_ref().map(|p| p.bitrate_mbps);
                        if let (Some(mbps), Some(p)) = (bitrate, &preset)
                            && mbps != p.bitrate_mbps
                        {
                            tracing::info!(
                                "--bitrate {mbps} Mbps overrides preset {} ({} Mbps)",
                                p.name,
                                p.bitrate_mbps
                            );
                        }
                        let target_codestream_bytes =
                            imfwizard_core::encode::target_codestream_bytes_for_job(
                                bitrate,
                                preset_bitrate_mbps,
                                encode_fps.as_f64(),
                            );
                        // the codestreams declare an IMF profile, not the cinema
                        // one a DCP carries: the levels come from this raster,
                        // this rate and the bits a second the job allows
                        let rsiz = imfwizard_core::encode::imf_rsiz_for_encode(
                            picture.encode_width,
                            picture.encode_height,
                            encode_fps.as_f64(),
                            imfwizard_core::encode::bitrate_mbps_for_job(
                                bitrate,
                                preset_bitrate_mbps,
                                picture.encode_width,
                                picture.encode_height,
                                encode_fps.as_f64(),
                            ),
                        )
                        .unwrap_or_else(|e| fail(e));
                        tracing::info!("JPEG 2000 profile: IMF, RSIZ {rsiz:#06x}");
                        // under a PSNR target the bitrate is a ceiling per frame
                        // rather than what the allocation aims at
                        let codestream_byte_cap = quality_psnr
                            .and(bitrate.or(preset_bitrate_mbps))
                            .map(|mbps| {
                                imfwizard_core::encode::codestream_byte_cap_for_bitrate(
                                    encode_fps.as_f64(),
                                    mbps,
                                )
                            });
                        match (quality_psnr, bitrate.or(preset_bitrate_mbps)) {
                            (Some(db), Some(mbps)) => tracing::info!(
                                "PSNR {db} dB (bitrate {mbps} Mbps, at most {} bytes a frame)",
                                codestream_byte_cap.unwrap_or_default()
                            ),
                            (Some(db), None) => tracing::info!("PSNR {db} dB (no byte cap)"),
                            (None, Some(mbps)) => tracing::info!(
                                "Bitrate {mbps} Mbps ({} bytes a frame)",
                                target_codestream_bytes.unwrap_or_default()
                            ),
                            (None, None) => tracing::info!(
                                "Ratio {:.1}",
                                imfwizard_core::encode::DEFAULT_COMPRESSION_RATIO
                            ),
                        }

                        // the picture MXF is written as the frames finish where the
                        // job allows it, so the wrap costs nothing after the encode
                        let overlap_refusal = imfwizard_core::overlapped_picture::overlap_refusal(
                            &imfwizard_core::overlapped_picture::PictureJob {
                                input_type,
                                still_hold: false,
                            },
                        );
                        let wrap_target = match overlap_refusal {
                            Some(reason) => {
                                tracing::info!(
                                    "Wrapping the picture MXF after the encode: {reason}"
                                );
                                None
                            }
                            None => Some(imfwizard_core::overlapped_picture::PictureWrapTarget {
                                imp_dir: output.clone(),
                                fps_num,
                                fps_den,
                                colour: imfwizard_core::mxf_wrap::picture_colour(hdr.as_ref()),
                            }),
                        };

                        // a trimmed video encodes the kept window only, so the
                        // frames the trim drops are never compressed
                        encode_window = imfwizard_core::source_edits::trimmed_encode_window(
                            &edits,
                            &video_path,
                            input_type,
                            rate_num,
                            rate_den,
                        )
                        .unwrap_or_else(|e| fail(e));
                        if let Some(window) = encode_window {
                            tracing::info!(
                                "Encoding frames {}..{} only",
                                window.first_frame,
                                window.end_frame()
                            );
                        }

                        let encoded = encode_picture(
                            &video_path,
                            &output,
                            &postkit::pipeline::EncodeRunOptions {
                                compression_ratio:
                                    imfwizard_core::encode::DEFAULT_COMPRESSION_RATIO,
                                target_codestream_bytes,
                                quality_psnr,
                                codestream_byte_cap,
                                fps: encode_fps,
                                frame_range: encode_window,
                                source_colour: source_colour.clone(),
                                rsiz,
                                subtitle_burn: build_subtitle_burn(encode_fps),
                                picture: picture.processing.clone(),
                                ..Default::default()
                            },
                            wrap_target,
                        );
                        let (j2k_out, picture_mxf) = match encoded {
                            Ok((r, track)) => {
                                tracing::info!("Encoded {} frames", r.frames_encoded);
                                for finding in r.picture_findings.describe(encode_fps.as_f64()) {
                                    tracing::warn!("{finding}");
                                }
                                if let Some(track) = &track {
                                    tracing::info!(
                                        "Picture MXF written during the encode: {} ({} frames)",
                                        track.path.display(),
                                        track.duration
                                    );
                                }
                                (r.j2k_dir, track)
                            }
                            Err(e) => {
                                eprintln!("Error: Encode failed: {e}");
                                std::process::exit(1);
                            }
                        };

                        // an image sequence carries no sound, so only a container
                        // is worth demuxing
                        let audio_files = match (&audio, is_image_sequence) {
                            (Some(a), _) => vec![PathBuf::from(a)],
                            (None, true) => vec![],
                            (None, false) => {
                                let wav_out =
                                    output.join(imfwizard_core::intermediates::DEMUXED_AUDIO_NAME);
                                let demux = std::process::Command::new("ffmpeg")
                                    .arg("-y")
                                    .arg("-i")
                                    .arg(&video_path)
                                    .arg("-vn")
                                    .arg("-acodec")
                                    .arg("pcm_s24le")
                                    .arg("-ar")
                                    .arg("48000")
                                    .arg(&wav_out)
                                    .output();
                                match demux {
                                    Ok(run) if run.status.success() => {
                                        tracing::info!("Demuxed audio: {}", wav_out.display());
                                        vec![wav_out]
                                    }
                                    demuxed => {
                                        if audio_map.is_some() {
                                            fail(format!(
                                                "--audio-map has nothing to apply to: no sound came out of {}, {}",
                                                video_path.display(),
                                                demux_failure_reason(&video_path, &demuxed)
                                            ));
                                        }
                                        vec![]
                                    }
                                }
                            }
                        };

                        (Some(j2k_out), picture_mxf, audio_files)
                    }
                }
            } else {
                (
                    None,
                    None,
                    audio
                        .as_ref()
                        .map(|a| vec![PathBuf::from(a)])
                        .unwrap_or_default(),
                )
            };

            // the map runs before the delay, the trim and the MCA labels, so the
            // labelled layout describes the file that is actually packaged
            let audio_files = match &audio_map {
                Some(spec) => audio_files
                    .iter()
                    .map(|wav| {
                        imfwizard_core::audio_map::map_audio_file(spec, wav, &output, |line| {
                            tracing::info!("{line}")
                        })
                        .unwrap_or_else(|e| fail(e))
                    })
                    .collect(),
                None => audio_files,
            };

            let source = imfwizard_core::source_edits::apply_source_edits(
                &edits,
                &imfwizard_core::source_edits::CompositionSource {
                    j2k_dir,
                    audio_files,
                    timed_text_files,
                },
                &output,
                fps_num,
                fps_den,
                encode_window,
            )
            .unwrap_or_else(|e| fail(e));

            let audio_tracks = source
                .audio_files
                .into_iter()
                .map(|path| imfwizard_core::imp::AudioTrack {
                    path,
                    language: audio_lang.clone(),
                    role: audio_role,
                })
                .collect();
            let opts = imfwizard_core::imp::ImpOptions {
                output_dir: output,
                compositions: vec![imfwizard_core::imp::Composition {
                    title,
                    content_kind: kind,
                    j2k_dir: source.j2k_dir,
                    picture_mxf,
                    audio_files: audio_tracks,
                    timed_text_files: source.timed_text_files,
                    hdr,
                }],
                fps_num,
                fps_den,
                soundfield: imfwizard_core::imp::SoundfieldLabels {
                    title_version: soundfield.title_version,
                    audio_content_kind: soundfield.audio_content_kind,
                    audio_element_kind: soundfield.audio_element_kind,
                },
                ..Default::default()
            };
            // the picture wrap's hash ran while the hints finished, so this waits less
            print_hints(hints_pass);
            let result = imfwizard_core::imp::create_imp(&opts);
            if result.success {
                if !keep_intermediates {
                    // a codestream directory handed to --video sits where the
                    // encode would have written its own
                    let handed_in: Vec<&std::path::Path> =
                        video.iter().map(std::path::Path::new).collect();
                    imfwizard_core::intermediates::remove_intermediates(
                        &result.output_dir,
                        &handed_in,
                    );
                }
                println!("IMP created at {}", result.output_dir.display());
                for cpl in &result.cpl_paths {
                    println!("  CPL: {}", cpl.display());
                }
                println!("  PKL: {}", result.pkl_path.display());
                println!("  ASSETMAP: {}", result.assetmap_path.display());

                if no_verify {
                    eprintln!(
                        "--no-verify: the package was not validated, run `imfwizard validate {}` before delivering it",
                        result.output_dir.display()
                    );
                } else {
                    let validation = imfwizard_core::validate::validate_imp_with_photon(
                        &result.output_dir,
                        None,
                    );
                    if !print_validation(&validation) {
                        std::process::exit(1);
                    }
                }
            } else {
                eprintln!("Error: {}", result.error);
                std::process::exit(1);
            }
        }

        Commands::Encode {
            input,
            output,
            bitrate,
        } => {
            let result = imfwizard_core::encode::encode_image_sequence(
                &input,
                &output,
                bitrate,
                imfwizard_core::encode::FrameRate::default(),
            );
            match result {
                Ok(result) => println!(
                    "Encoding complete: {} frames in {}",
                    result.frames_encoded,
                    result.j2k_dir.display()
                ),
                Err(error) => {
                    eprintln!("Error: {error}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Transcode {
            input,
            output,
            codec,
        } => {
            let opts = imfwizard_core::transcode::TranscodeOptions {
                input,
                output,
                codec,
                ..Default::default()
            };
            let result = imfwizard_core::transcode::transcode(&opts);
            if result.success {
                println!("Transcode complete: {}", result.output.display());
            } else {
                eprintln!("Error: {}", result.error);
                std::process::exit(1);
            }
        }

        Commands::SubtitleConvert {
            input,
            output,
            font_size,
            colour,
        } => {
            let appearance = imfwizard_core::subtitle_convert::TextAppearance {
                font_size_percent: font_size,
                colour: colour.as_deref().map(|text| {
                    postkit::subtitle_formats::Rgba::parse_hex(text)
                        .unwrap_or_else(|e| fail(format!("--colour: {e}")))
                }),
            };
            match imfwizard_core::subtitle_convert::convert_subtitles(
                &input,
                &output,
                imfwizard_core::subtitle_convert::SubtitleFormat::ImscTtml,
                &appearance,
            ) {
                Ok(()) => println!("Converted to {}", output.display()),
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Hdr10plusExtract { input, output } => {
            match imfwizard_core::hdr::extract_hdr10plus(&input, &output) {
                Ok(meta) => println!(
                    "HDR10+ metadata written to {}: {} scenes",
                    meta.json_path.display(),
                    meta.scene_count
                ),
                Err(e) => {
                    eprintln!(
                        "Error: HDR10+ extraction from {} failed: {e}",
                        input.display()
                    );
                    std::process::exit(1);
                }
            }
        }

        Commands::DvExtract { input, output } => {
            match imfwizard_core::dolby_vision::extract_rpu(&input, &output) {
                Ok(()) => println!("RPU extracted to {}", output.display()),
                Err(e) => {
                    eprintln!("Error: RPU extraction from {} failed: {e}", input.display());
                    std::process::exit(1);
                }
            }
        }

        Commands::Analytics {
            input,
            json,
            video,
            histogram_buckets,
        } => {
            match imfwizard_core::analytics::analyze_imp(&input) {
                Ok(a) => {
                    if json && video.is_none() {
                        println!("{}", serde_json::to_string_pretty(&a).unwrap());
                    } else if video.is_none() {
                        println!("Total assets: {}", a.total_assets);
                        println!("Video tracks: {}", a.video_tracks);
                        println!("Audio tracks: {}", a.audio_tracks);
                        println!("Subtitle tracks: {}", a.subtitle_tracks);
                        println!("Total size: {} bytes", a.total_size_bytes);
                    }
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }

            if let Some(ref video_path) = video {
                match imfwizard_core::analytics::analyze_bitrate(video_path, histogram_buckets) {
                    Ok(bitrate) => {
                        if json {
                            println!("{}", serde_json::to_string_pretty(&bitrate).unwrap());
                        } else {
                            println!("\nBitrate Analysis:");
                            println!("  Duration:    {:.2} s", bitrate.duration_seconds);
                            println!("  Frames:      {}", bitrate.total_frames);
                            println!("  Min:         {:.1} kbps", bitrate.min_kbps);
                            println!("  Max:         {:.1} kbps", bitrate.max_kbps);
                            println!("  Average:     {:.1} kbps", bitrate.avg_kbps);
                            println!("  Std Dev:     {:.1} kbps", bitrate.stddev_kbps);
                            println!("\n  Histogram:");
                            for bucket in &bitrate.histogram {
                                let bar = "#".repeat(bucket.count.min(50));
                                println!(
                                    "    {:>8.0}-{:<8.0} kbps [{:>4}] {}",
                                    bucket.range_min_kbps, bucket.range_max_kbps, bucket.count, bar
                                );
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Bitrate analysis error: {e}");
                        std::process::exit(1);
                    }
                }
            }
        }

        Commands::Hash { file, algorithm } => {
            let algo = match algorithm.as_str() {
                "sha256" => imfwizard_core::hash::HashAlgorithm::Sha256,
                _ => imfwizard_core::hash::HashAlgorithm::Sha1,
            };
            match imfwizard_core::hash::hash_file(&file, algo) {
                Ok(h) => {
                    println!("{} {}", h.hex, file.display());
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Watch {
            dir,
            output,
            webhook_url,
            interval,
            create_arguments,
        } => {
            use imfwizard_core::watch::{
                AUDIO_SIDECAR_EXTENSION, DONE_DIRECTORY_NAME, FAILED_DIRECTORY_NAME,
                SUBTITLE_SIDECAR_EXTENSION,
            };
            use std::path::Path;

            fn free_destination(directory: &Path, file_name: &str) -> PathBuf {
                let taken = directory.join(file_name);
                if !taken.exists() {
                    return taken;
                }
                let stem = Path::new(file_name)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(file_name);
                let extension = Path::new(file_name).extension().and_then(|e| e.to_str());
                let mut suffix = 1u32;
                loop {
                    let candidate = match extension {
                        Some(extension) => directory.join(format!("{stem}-{suffix}.{extension}")),
                        None => directory.join(format!("{stem}-{suffix}")),
                    };
                    if !candidate.exists() {
                        return candidate;
                    }
                    suffix += 1;
                }
            }

            fn move_aside(source: &Path, directory: &Path) {
                if let Err(e) = std::fs::create_dir_all(directory) {
                    tracing::error!("cannot create {}: {e}", directory.display());
                    return;
                }
                let Some(file_name) = source.file_name().and_then(|n| n.to_str()) else {
                    tracing::error!("cannot read a file name from {}", source.display());
                    return;
                };
                let destination = free_destination(directory, file_name);
                if let Err(e) = std::fs::rename(source, &destination) {
                    tracing::error!(
                        "cannot move {} into {}: {e}",
                        source.display(),
                        directory.display()
                    );
                }
            }

            fn post_event(
                webhook: Option<&postkit::webhook::WebhookConfig>,
                event_type: &str,
                job_id: &str,
                payload_json: String,
            ) {
                let Some(config) = webhook else {
                    return;
                };
                let event = postkit::webhook::WebhookEvent {
                    event_type: event_type.to_string(),
                    job_id: job_id.to_string(),
                    payload_json,
                    timestamp: String::new(),
                };
                let result = postkit::webhook::send_webhook(config, &event);
                if !result.success {
                    tracing::warn!("webhook delivery failed: {}", result.error);
                }
            }

            if let Err(e) = std::fs::create_dir_all(&output) {
                tracing::error!("cannot create {}: {e}", output.display());
                std::process::exit(1);
            }
            let executable = match std::env::current_exe() {
                Ok(path) => path,
                Err(e) => {
                    tracing::error!("cannot find the running imfwizard binary: {e}");
                    std::process::exit(1);
                }
            };
            let webhook = webhook_url.map(|url| postkit::webhook::WebhookConfig {
                url,
                ..Default::default()
            });

            imfwizard_core::watch::watch_directory(
                &dir,
                std::time::Duration::from_secs(interval),
                &|| false,
                |master| {
                    let Some(stem) = master.file_stem().and_then(|s| s.to_str()) else {
                        tracing::error!("cannot read a file stem from {}", master.display());
                        return;
                    };
                    let package_dir = output.join(stem);
                    let log_path = output.join(format!("{stem}.log"));

                    let audio = dir.join(format!("{stem}.{AUDIO_SIDECAR_EXTENSION}"));
                    let subtitle = dir.join(format!("{stem}.{SUBTITLE_SIDECAR_EXTENSION}"));
                    let audio = audio.is_file().then_some(audio);
                    let subtitle = subtitle.is_file().then_some(subtitle);

                    let mut arguments: Vec<std::ffi::OsString> = vec![
                        "create".into(),
                        "--title".into(),
                        stem.into(),
                        "--video".into(),
                        master.into(),
                        "--output".into(),
                        package_dir.as_path().into(),
                    ];
                    if let Some(audio) = audio.as_deref() {
                        arguments.push("--audio".into());
                        arguments.push(audio.into());
                    }
                    if let Some(subtitle) = subtitle.as_deref() {
                        arguments.push("--subtitle".into());
                        arguments.push(subtitle.into());
                    }
                    arguments.extend(create_arguments.iter().map(std::ffi::OsString::from));

                    let log = match std::fs::File::create(&log_path) {
                        Ok(file) => file,
                        Err(e) => {
                            tracing::error!("cannot write {}: {e}", log_path.display());
                            return;
                        }
                    };
                    let log_for_stderr = match log.try_clone() {
                        Ok(file) => file,
                        Err(e) => {
                            tracing::error!("cannot write {}: {e}", log_path.display());
                            return;
                        }
                    };

                    tracing::info!(
                        "building {} from {}",
                        package_dir.display(),
                        master.display()
                    );
                    let started = std::time::Instant::now();
                    let outcome = std::process::Command::new(&executable)
                        .args(&arguments)
                        .stdout(std::process::Stdio::from(log))
                        .stderr(std::process::Stdio::from(log_for_stderr))
                        .status();
                    let elapsed_seconds = started.elapsed().as_secs_f64();

                    let failure = match outcome {
                        Ok(status) if status.success() => None,
                        Ok(status) => Some(match status.code() {
                            Some(code) => {
                                format!("create exited {code}, log at {}", log_path.display())
                            }
                            None => format!(
                                "create was killed by a signal, log at {}",
                                log_path.display()
                            ),
                        }),
                        Err(e) => Some(format!("could not run create: {e}")),
                    };
                    let sidecars = [audio.as_deref(), subtitle.as_deref()]
                        .into_iter()
                        .flatten();

                    let Some(message) = failure else {
                        let done = dir.join(DONE_DIRECTORY_NAME);
                        move_aside(master, &done);
                        for sidecar in sidecars {
                            move_aside(sidecar, &done);
                        }
                        tracing::info!("built {} in {elapsed_seconds:.1} s", package_dir.display());
                        post_event(
                            webhook.as_ref(),
                            "imp.created",
                            stem,
                            postkit::webhook::build_job_completed_payload(
                                stem,
                                &package_dir,
                                elapsed_seconds,
                            ),
                        );
                        return;
                    };

                    let failed = dir.join(FAILED_DIRECTORY_NAME);
                    move_aside(master, &failed);
                    for sidecar in sidecars {
                        move_aside(sidecar, &failed);
                    }
                    tracing::error!("create failed for {stem}: see {}", log_path.display());
                    post_event(
                        webhook.as_ref(),
                        "imp.failed",
                        stem,
                        postkit::webhook::build_job_failed_payload(stem, &message),
                    );
                },
            );
        }

        Commands::Profiles => {
            for p in imfwizard_core::profiles::all_profiles() {
                println!(
                    "{:?}: {}x{} @ {} Mbps, {} fps",
                    p.platform, p.width, p.height, p.bitrate_mbps, p.frame_rate,
                );
            }
        }

        Commands::Timecode { tc, fps } => {
            match imfwizard_core::timecode::Timecode::parse(&tc, fps) {
                Ok(parsed) => {
                    println!("Timecode: {parsed}");
                    println!("Total frames: {}", parsed.to_frames());
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Validate {
            dir,
            xsd,
            schema_dir,
            photon,
            photon_jar,
        } => {
            let photon_path = photon_jar.as_deref().map(std::path::Path::new);
            let result = imfwizard_core::validate::validate_imp_with_photon(
                std::path::Path::new(&dir),
                photon_path,
            );
            let mut failed = !print_validation(&result);

            if xsd {
                let sd = schema_dir.as_deref().map(std::path::Path::new);
                match imfwizard_core::xsd_validate::validate_imp_schemas(
                    std::path::Path::new(&dir),
                    sd,
                ) {
                    Ok(results) => {
                        for r in &results {
                            if r.valid {
                                println!("  XSD {}: PASS", r.file);
                            } else {
                                failed = true;
                                eprintln!("  XSD {}: FAIL", r.file);
                                for err in &r.errors {
                                    eprintln!("    {err}");
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("XSD validation error: {e}");
                        std::process::exit(1);
                    }
                }
            }

            if photon {
                match imfwizard_core::photon::run_photon(std::path::Path::new(&dir), photon_path) {
                    Ok(p) => {
                        if p.errors.is_empty() && p.warnings.is_empty() {
                            println!("  Photon: PASS");
                        }
                        for e in &p.errors {
                            failed = true;
                            eprintln!("  error: {e}");
                        }
                        for w in &p.warnings {
                            println!("  warning: {w}");
                        }
                    }
                    Err(e) => {
                        eprintln!("Photon error: {e}");
                        std::process::exit(1);
                    }
                }
            }

            if failed {
                std::process::exit(1);
            }
        }

        Commands::Loudness {
            audio_file,
            adjust_to,
            output,
            true_peak,
        } => {
            let input = std::path::Path::new(&audio_file);
            match adjust_to {
                Some(target_lufs) => {
                    let Some(output) = output else {
                        eprintln!("Error: --adjust-to requires --output");
                        std::process::exit(1);
                    };
                    let out = std::path::Path::new(&output);
                    let target = postkit::loudness::LoudnessTarget::IntegratedLufs(target_lufs);
                    match postkit::loudness::adjust_loudness(input, out, target, true_peak) {
                        Ok(plan) => {
                            println!("Measured: {:.1} LUFS", plan.measured_db);
                            println!("Target: {:.1} LUFS", plan.target_db);
                            println!("Gain applied: {:+.2} dB", plan.gain_db);
                            println!(
                                "True peak: {:.2} -> {:.2} dBTP (ceiling {:.2}, headroom {:.2})",
                                plan.input_true_peak_dbtp,
                                plan.resulting_true_peak_dbtp,
                                plan.true_peak_ceiling_dbtp,
                                plan.true_peak_ceiling_dbtp - plan.resulting_true_peak_dbtp
                            );
                            println!("Adjusted audio written to {}", out.display());
                        }
                        Err(e) => {
                            // clip-safe: on a ceiling breach nothing is written
                            eprintln!("Error: {e}");
                            std::process::exit(1);
                        }
                    }
                }
                None => {
                    let result = postkit::loudness::measure_loudness(input);
                    if result.success {
                        println!("Integrated: {:.1} LUFS", result.integrated_lufs);
                        println!("True Peak: {:.1} dBTP", result.true_peak_dbtp);
                        println!("Range: {:.1} LU", result.range_lu);
                    } else {
                        eprintln!("Error: {}", result.error);
                        std::process::exit(1);
                    }
                }
            }
        }

        Commands::BurnIn {
            input,
            subtitles,
            output,
        } => {
            let status = std::process::Command::new("ffmpeg")
                .arg("-y")
                .arg("-i")
                .arg(&input)
                .arg("-vf")
                .arg(format!("subtitles={}", subtitles))
                .arg("-c:a")
                .arg("copy")
                .arg(&output)
                .status();
            match status {
                Ok(s) if s.success() => {
                    println!("Burned subtitles into: {output}");
                }
                Ok(s) => {
                    eprintln!("Error: ffmpeg exited with code {}", s.code().unwrap_or(-1));
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("Error: Failed to run ffmpeg: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Info { dir } => {
            match imfwizard_core::info::inspect_imp(std::path::Path::new(&dir)) {
                Ok(info) => {
                    println!("{}", serde_json::to_string_pretty(&info).unwrap());
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Supplement {
            ov,
            title,
            output,
            replace,
            add,
        } => {
            let opts = imfwizard_core::supplement::SupplementOptions {
                ov_dir: PathBuf::from(ov),
                title,
                output_dir: PathBuf::from(output),
                replace,
                add,
            };
            let result = imfwizard_core::supplement::create_supplement(&opts);
            if result.success {
                println!(
                    "Supplemental IMP created at {}",
                    result.output_dir.display()
                );
            } else {
                eprintln!("Error: {}", result.error);
                std::process::exit(1);
            }
        }

        Commands::ToDcp {
            input,
            output,
            kind,
            title,
            bitrate,
        } => {
            let opts = imfwizard_core::to_dcp::ToDcpOptions {
                imp_dir: PathBuf::from(input),
                output_dir: PathBuf::from(output),
                title,
                content_kind: kind,
                bitrate_mbps: bitrate,
            };
            let result = imfwizard_core::to_dcp::imp_to_dcp(&opts);
            if result.success {
                println!("{}", result.picture_report);
                println!("DCP created at {}", result.output_dir.display());
            } else {
                eprintln!("Error: {}", result.error);
                std::process::exit(1);
            }
        }

        Commands::ExportFrames {
            input,
            output,
            format,
            cpl,
            start,
            count,
        } => {
            let format = match imfwizard_core::export_frames::ExportFormat::from_flag(&format) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            };
            let opts = imfwizard_core::export_frames::ExportFramesOptions {
                imp_dir: PathBuf::from(input),
                output_dir: PathBuf::from(output),
                format,
                cpl,
                start,
                count,
            };
            match imfwizard_core::export_frames::export_frames(&opts) {
                Ok(r) => println!(
                    "Exported {} frame(s) ({}x{}) to {}",
                    r.frames_written,
                    r.width,
                    r.height,
                    r.output_dir.display()
                ),
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Sign {
            input,
            output,
            cert,
            key,
            chain,
        } => {
            let out = output.unwrap_or_else(|| input.clone());
            match imfwizard_core::signature::sign_document(&input, &out, &cert, &key, &chain) {
                Ok(()) => println!("Signed {} -> {}", input.display(), out.display()),
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::VerifySig {
            input,
            trusted_cert,
        } => match imfwizard_core::signature::verify_signature(&input, trusted_cert.as_deref()) {
            Ok(()) => println!("Signature valid: {}", input.display()),
            Err(e) => {
                eprintln!("Signature invalid: {e}");
                std::process::exit(1);
            }
        },

        Commands::Report {
            imp,
            output,
            format,
            scan_picture,
        } => {
            let imp_dir = std::path::Path::new(&imp);
            let validate_result = imfwizard_core::validate::validate_imp_for_report(imp_dir);
            let report_format = match format.as_str() {
                "json" => postkit::report::ReportFormat::Json,
                "text" => postkit::report::ReportFormat::Text,
                _ => postkit::report::ReportFormat::Html,
            };
            let mut report = postkit::report::Report {
                title: format!("IMF Wizard QC Report — {}", imp),
                timestamp: time::OffsetDateTime::now_utc().to_string(),
                ..Default::default()
            };
            for err in &validate_result.errors {
                report.add_entry(postkit::report::ReportEntry {
                    severity: "error".to_string(),
                    category: "validation".to_string(),
                    message: err.clone(),
                    details: String::new(),
                });
            }
            for warn in &validate_result.warnings {
                report.add_entry(postkit::report::ReportEntry {
                    severity: "warning".to_string(),
                    category: "validation".to_string(),
                    message: warn.clone(),
                    details: String::new(),
                });
            }
            for info in &validate_result.infos {
                report.add_entry(postkit::report::ReportEntry {
                    severity: "info".to_string(),
                    category: "validation".to_string(),
                    message: info.clone(),
                    details: String::new(),
                });
            }
            if scan_picture {
                for entry in imfwizard_core::report::scan_picture_entries(imp_dir) {
                    report.add_entry(entry);
                }
            } else {
                report.add_entry(imfwizard_core::report::picture_not_scanned_entry());
            }
            if report.error_count == 0 {
                report.pass_count += 1;
                report.summary = "IMP validation PASSED".to_string();
            } else {
                report.summary = format!("{} errors found", report.error_count);
            }
            let output_path = PathBuf::from(&output);
            match report.write_to_file(&output_path, report_format) {
                Ok(()) => println!("Report written to {output}"),
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Serve { bind, api_key } => {
            let parts: Vec<&str> = bind.split(':').collect();
            let config = imfwizard_core::rest_api::ApiConfig {
                host: parts.first().unwrap_or(&"127.0.0.1").to_string(),
                port: parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(8081),
                api_key,
            };
            if let Err(e) = imfwizard_core::rest_api::start_server(&config) {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }

        Commands::Completion { shell } => {
            use clap::CommandFactory;
            use clap_complete::{Shell, generate};
            let mut cmd = Cli::command();
            let shell = match shell.as_str() {
                "zsh" => Shell::Zsh,
                "fish" => Shell::Fish,
                _ => Shell::Bash,
            };
            generate(shell, &mut cmd, "imfwizard", &mut std::io::stdout());
        }

        Commands::MetadataEdit {
            imp,
            title,
            annotation,
            issuer,
        } => {
            let edit = postkit::package_edit::PackageEdit {
                input: PathBuf::from(&imp),
                annotation: title.or(annotation),
                issuer,
                ..Default::default()
            };
            match postkit::package_edit::edit_package(&edit) {
                Ok(edited) => {
                    println!(
                        "Metadata updated for {}: it now carries composition id {}",
                        edited.cpl_path.display(),
                        edited.composition_id
                    );
                    if !edited.unsigned_documents.is_empty() {
                        println!(
                            "Wrote unsigned, the rewrite changed the bytes their signature covered: {}. Re-sign the package if it has to stay signed",
                            edited.unsigned_documents.join(", ")
                        );
                    }
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Retitle { imp, title } => {
            let edit = postkit::package_edit::PackageEdit {
                input: PathBuf::from(&imp),
                title: Some(title),
                ..Default::default()
            };
            match postkit::package_edit::edit_package(&edit) {
                Ok(edited) => {
                    println!(
                        "Retitled {imp}: {} now carries composition id {}",
                        edited.cpl_path.display(),
                        edited.composition_id
                    );
                    if !edited.unsigned_documents.is_empty() {
                        println!(
                            "Wrote unsigned, the rewrite changed the bytes their signature covered: {}. Re-sign the package if it has to stay signed",
                            edited.unsigned_documents.join(", ")
                        );
                    }
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Dcdm {
            input,
            output,
            colour_space,
            lut,
            width,
            height,
        } => {
            let opts = postkit::dcdm::DcdmOptions {
                input_dir: PathBuf::from(input),
                output_dir: PathBuf::from(output),
                encoding: postkit::dcdm::DcdmColourEncoding::Xyz12Bit,
                width,
                height,
                colour_space,
                lut_path: lut.map(PathBuf::from).unwrap_or_default(),
                ..Default::default()
            };
            let result = postkit::dcdm::create_dcdm(&opts);
            if result.success {
                println!("DCDM created: {} frames", result.frames_written);
            } else {
                eprintln!("Error: {}", result.error);
                std::process::exit(1);
            }
        }

        Commands::Colour {
            input,
            output,
            source,
            target,
            lut,
        } => {
            let source_space = parse_colour_space(&source);
            let target_space = parse_colour_space(&target);
            let opts = postkit::colour::ColourConvertOptions {
                input: PathBuf::from(input),
                output: PathBuf::from(output),
                source_space,
                target_space,
                lut_path: lut.map(PathBuf::from),
            };
            match postkit::colour::convert_colour(&opts) {
                Ok(()) => println!("Colour conversion complete"),
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Conform {
            input,
            media_dir,
            output,
            json,
        } => {
            let timeline = match postkit::conform::parse_timeline(std::path::Path::new(&input)) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            };
            match (media_dir, output) {
                (Some(media_dir), Some(output)) => {
                    let plan = imfwizard_core::conform::build_plan(
                        &timeline,
                        std::path::Path::new(&media_dir),
                    )
                    .unwrap_or_else(|e| fail(e));
                    let conformed = imfwizard_core::conform::conform_to_imp(
                        &plan,
                        std::path::Path::new(&output),
                    )
                    .unwrap_or_else(|e| fail(e));
                    println!(
                        "Conformed {} events into {} ({} frames)",
                        plan.events.len(),
                        conformed.cpl_path.display(),
                        conformed.picture_frames
                    );
                    println!("Conform manifest: {}", conformed.manifest_path.display());
                }
                (None, None) if json => {
                    println!("{}", serde_json::to_string_pretty(&timeline).unwrap());
                }
                (None, None) => {
                    println!("Timeline: {} ({:?})", timeline.title, timeline.format);
                    println!("Frame rate: {}", timeline.frame_rate);
                    println!("Events: {}", timeline.events.len());
                    for event in &timeline.events {
                        println!(
                            "  #{}: {} [{}-{}] → [{}-{}]",
                            event.event_number,
                            event.reel_name,
                            event.source_in,
                            event.source_out,
                            event.record_in,
                            event.record_out,
                        );
                    }
                }
                _ => fail("conforming to an IMP needs both --media-dir and --output"),
            }
        }

        Commands::FrameExtract {
            input,
            frame,
            output,
            key,
            keys_json,
        } => {
            let key = match postkit::preview::resolve_picture_key(
                std::path::Path::new(&input),
                key.as_deref(),
                keys_json.as_deref().map(std::path::Path::new),
            ) {
                Ok(key) => key,
                Err(error) => {
                    eprintln!("Error: {error}");
                    std::process::exit(1);
                }
            };
            let result = postkit::preview::extract_frame(
                std::path::Path::new(&input),
                frame,
                std::path::Path::new(&output),
                key,
            );
            if result == 0 {
                println!("Frame {frame} extracted to {output}");
            } else {
                eprintln!("Error: frame extraction failed");
                std::process::exit(1);
            }
        }

        Commands::DvInject { input, rpu, output } => {
            let opts = postkit::dolby_vision::DolbyVisionOptions {
                input: PathBuf::from(&input),
                rpu_file: PathBuf::from(&rpu),
                output: PathBuf::from(&output),
                ..Default::default()
            };
            let result = postkit::dolby_vision::inject_dolby_vision(&opts);
            if result == 0 {
                println!("Dolby Vision RPU injected: {output}");
            } else {
                eprintln!("Error: DV injection of {rpu} into {input} failed");
                std::process::exit(1);
            }
        }

        Commands::Hdr10Inject {
            input,
            output,
            max_cll,
            max_fall,
            mastering_display,
        } => {
            let hdr10 = mastering_display.metadata(max_cll, max_fall);
            let opts = postkit::dolby_vision::HdrMetadataOptions {
                input: PathBuf::from(&input),
                output: PathBuf::from(&output),
                hdr_type: postkit::dolby_vision::HdrType::Hdr10,
                hdr10,
                ..Default::default()
            };
            let result = postkit::dolby_vision::inject_hdr10_metadata(&opts);
            if result == 0 {
                println!(
                    "HDR10 metadata injected: {output}, {}",
                    postkit::dolby_vision::x265_hdr10_params(&hdr10)
                );
            } else {
                eprintln!("Error: HDR10 injection into {input} failed");
                std::process::exit(1);
            }
        }

        Commands::Watermark {
            input,
            output,
            operator_id,
            session_id,
            strength,
        } => {
            let opts = postkit::watermark::WatermarkOptions {
                operator_id,
                session_id,
                strength,
                input_dir: PathBuf::from(&input),
                output_dir: PathBuf::from(&output),
            };
            let result = postkit::watermark::embed_watermark(&opts);
            if result.success {
                println!(
                    "Watermark embedded: {} frames, hash={}",
                    result.frames_processed, result.payload_hash
                );
            } else {
                eprintln!("Error: {}", result.error);
                std::process::exit(1);
            }
        }

        Commands::Trailer {
            content,
            audio,
            output,
            title,
            rating,
            rating_system,
            band,
        } => {
            let opts = postkit::trailer::TrailerOptions {
                content_dir: PathBuf::from(&content),
                audio_file: PathBuf::from(&audio),
                output_dir: PathBuf::from(&output),
                title,
                rating,
                rating_system: imfwizard_core::trailer::rating_system_from_name(&rating_system)
                    .unwrap_or_else(|e| fail(e)),
                band: imfwizard_core::trailer::band_from_name(&band).unwrap_or_else(|e| fail(e)),
                ..Default::default()
            };
            let result = postkit::trailer::package_trailer(&opts);
            if result.success {
                println!("Trailer packaged: {}", result.output_dir.display());
            } else {
                eprintln!("Error: {}", result.error);
                std::process::exit(1);
            }
        }

        Commands::Preview { input } => {
            let player = postkit::mpv::MpvPlayer::new("imfwizard");
            if let Err(e) = player.start_mpv() {
                eprintln!("Error starting mpv: {e}");
                std::process::exit(1);
            }
            if let Err(e) = player.load_package_dir(&input) {
                eprintln!("Error loading package: {e}");
                std::process::exit(1);
            }
            println!("Playing IMP: {input} (press q to quit)");
            while player.is_alive() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }

        Commands::AudioDesc {
            input,
            standard,
            narration,
            output,
            duck_level,
        } => {
            if let Some(narration_path) = narration {
                // Audio description mixing with ducking
                let out = output.unwrap_or_else(|| {
                    eprintln!("Error: --output is required when mixing audio description");
                    std::process::exit(1);
                });
                match imfwizard_core::audio_desc::mix_audio_description(
                    std::path::Path::new(&input),
                    std::path::Path::new(&narration_path),
                    std::path::Path::new(&out),
                    duck_level,
                    -30.0, // threshold
                    20.0,  // attack ms
                    200.0, // release ms
                ) {
                    Ok(()) => println!("Audio description mixed: {out}"),
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                }
            } else {
                // Accessibility compliance check
                let std_enum = match standard.to_lowercase().as_str() {
                    "eaa" => postkit::accessibility::AccessibilityStandard::Eaa,
                    "aoda" => postkit::accessibility::AccessibilityStandard::Aoda,
                    "ofcom" => postkit::accessibility::AccessibilityStandard::Ofcom,
                    _ => postkit::accessibility::AccessibilityStandard::Cvaa,
                };
                let result = postkit::accessibility::check_accessibility(
                    std::path::Path::new(&input),
                    std_enum,
                );
                println!(
                    "Accessibility ({standard}): {}",
                    if result.compliant { "PASS" } else { "FAIL" }
                );
                if !result.tracks_present.is_empty() {
                    println!("  Present: {:?}", result.tracks_present);
                }
                if !result.tracks_missing.is_empty() {
                    println!("  Missing: {:?}", result.tracks_missing);
                }
                for f in &result.findings {
                    println!("  [{:?}] {}", f.severity, f.description);
                }
                if !result.compliant {
                    std::process::exit(1)
                };
            }
        }

        Commands::Compare {
            a,
            b,
            pixel,
            vmaf,
            json,
        } => {
            let vmaf_score = if vmaf {
                match imfwizard_core::frame_compare::compute_vmaf(
                    std::path::Path::new(&a),
                    std::path::Path::new(&b),
                ) {
                    Ok(s) => Some(s),
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                }
            } else {
                None
            };

            if pixel {
                // Pixel-level PSNR/SSIM comparison
                match imfwizard_core::frame_compare::compare_frames(
                    std::path::Path::new(&a),
                    std::path::Path::new(&b),
                ) {
                    Ok(result) => {
                        if json {
                            let out = serde_json::json!({
                                "psnr_ssim": result,
                                "vmaf": vmaf_score,
                            });
                            println!("{}", serde_json::to_string_pretty(&out).unwrap());
                        } else {
                            println!("Frame Comparison: {} vs {}", a, b);
                            println!("  Frames compared: {}", result.frames_compared);
                            println!(
                                "  PSNR (avg/min/max): {:.2} / {:.2} / {:.2} dB",
                                result.avg_psnr, result.min_psnr, result.max_psnr
                            );
                            println!(
                                "  SSIM (avg/min/max): {:.6} / {:.6} / {:.6}",
                                result.avg_ssim, result.min_ssim, result.max_ssim
                            );
                            if let Some(v) = &vmaf_score {
                                println!(
                                    "  VMAF (mean/min/max): {:.2} / {:.2} / {:.2}",
                                    v.mean, v.min, v.max
                                );
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                }
            } else if let Some(v) = vmaf_score {
                // VMAF-only comparison
                if json {
                    let out = serde_json::json!({ "vmaf": v });
                    println!("{}", serde_json::to_string_pretty(&out).unwrap());
                } else {
                    println!("VMAF: {} vs {}", a, b);
                    println!("  Frames: {}", v.frames);
                    println!(
                        "  VMAF (mean/min/max): {:.2} / {:.2} / {:.2}",
                        v.mean, v.min, v.max
                    );
                    println!("  Harmonic mean: {:.2}", v.harmonic_mean);
                }
            } else {
                // Metadata-level comparison
                let info_a = imfwizard_core::info::inspect_imp(std::path::Path::new(&a));
                let info_b = imfwizard_core::info::inspect_imp(std::path::Path::new(&b));
                match (info_a, info_b) {
                    (Ok(ia), Ok(ib)) => {
                        println!("IMP A: {} ({})", a, ia.title);
                        println!(
                            "  CPLs: {}, Duration: {} frames",
                            ia.cpl_count, ia.duration_frames
                        );
                        println!("IMP B: {} ({})", b, ib.title);
                        println!(
                            "  CPLs: {}, Duration: {} frames",
                            ib.cpl_count, ib.duration_frames
                        );
                        if ia.edit_rate != ib.edit_rate {
                            println!("  DIFF: edit rate {} vs {}", ia.edit_rate, ib.edit_rate);
                        }
                        if ia.duration_frames != ib.duration_frames {
                            println!(
                                "  DIFF: duration {} vs {} frames",
                                ia.duration_frames, ib.duration_frames
                            );
                        }
                    }
                    (Err(e), _) | (_, Err(e)) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                }
            }
        }

        Commands::Lut { input, output, lut } => {
            let opts = postkit::colour::ColourConvertOptions {
                input: PathBuf::from(&input),
                output: PathBuf::from(&output),
                source_space: postkit::colour::ColourSpace::Rec709,
                target_space: postkit::colour::ColourSpace::Rec709,
                lut_path: Some(PathBuf::from(&lut)),
            };
            match postkit::colour::convert_colour(&opts) {
                Ok(()) => println!("LUT applied: {output}"),
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Prores {
            input,
            output,
            profile,
            container,
            cpl,
            ov,
        } => {
            let input_path = PathBuf::from(&input);
            if input_path.is_dir() {
                let raster = container.as_deref().map(|name| {
                    ContainerRaster::parse(name).unwrap_or_else(|| {
                        eprintln!(
                            "Error: unknown container '{name}', use one of {}",
                            ContainerRaster::names()
                        );
                        std::process::exit(1);
                    })
                });
                match imfwizard_core::prores::export_imp_to_prores(
                    &input_path,
                    &PathBuf::from(&output),
                    cpl.as_deref(),
                    raster,
                    ov.as_deref().map(std::path::Path::new),
                ) {
                    Ok(()) => println!("ProRes 4444 exported: {output}"),
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                }
            } else {
                encode_prores_file(&input, &output, &profile, container.as_deref());
            }
        }

        Commands::PartialVersion { input, output, cpl } => {
            match imfwizard_core::partial_version::create_partial_version(
                std::path::Path::new(&input),
                std::path::Path::new(&output),
                &cpl,
            ) {
                Ok(partial) => {
                    println!(
                        "Partial version created: {} and {} track file(s) copied to {output}",
                        partial.cpl_path.display(),
                        partial.track_files.len()
                    );
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Deliver {
            input,
            destination,
            db,
        } => {
            let Some(composition) =
                imfwizard_core::timeline::list_cpls(std::path::Path::new(&input))
                    .into_iter()
                    .next()
            else {
                eprintln!("No CPL found in {input}, so there is no package to deliver");
                std::process::exit(1);
            };

            let delivery_method = if destination.starts_with("s3://") {
                "s3"
            } else if destination.starts_with("aspera://") || destination.starts_with("fasp://") {
                "aspera"
            } else {
                "rsync"
            };

            let status = match delivery_method {
                "s3" => {
                    println!("Uploading to S3: {destination}");
                    std::process::Command::new("aws")
                        .arg("s3")
                        .arg("sync")
                        .arg(&input)
                        .arg(&destination)
                        .arg("--no-progress")
                        .status()
                }
                "aspera" => {
                    let remote = destination
                        .strip_prefix("aspera://")
                        .or_else(|| destination.strip_prefix("fasp://"))
                        .unwrap_or(&destination);
                    println!("Delivering via Aspera FASP: {remote}");
                    std::process::Command::new("ascp")
                        .arg("-QT")
                        .arg("-l")
                        .arg("1000m")
                        .arg("-r")
                        .arg(&input)
                        .arg(remote)
                        .status()
                }
                _ => std::process::Command::new("rsync")
                    .arg("-av")
                    .arg("--progress")
                    .arg(format!("{}/", input))
                    .arg(&destination)
                    .status(),
            };

            match status {
                Ok(s) if s.success() => println!("Delivered to {destination}"),
                Ok(s) => {
                    eprintln!(
                        "{delivery_method} exited with code {}",
                        s.code().unwrap_or(-1)
                    );
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("Failed to run {delivery_method}: {e}");
                    std::process::exit(1);
                }
            }

            // a record of a transfer that never landed is worse than none
            let mut tracker = postkit::version_tracker::VersionTracker::new();
            if !tracker.open(std::path::Path::new(&db)) {
                eprintln!("Delivered, but the tracker database {db} could not be opened");
                std::process::exit(1);
            }
            let record = postkit::version_tracker::DeliveryRecord {
                package_uuid: composition.id.clone(),
                title: composition.title.clone(),
                version: String::from("1"),
                destination: destination.clone(),
                delivery_method: delivery_method.to_string(),
                timestamp: rfc3339_now(),
                verified: false,
            };
            if !tracker.record(&record) {
                eprintln!("Delivered, but the delivery could not be recorded in {db}");
                std::process::exit(1);
            }
            println!("Recorded delivery of {} in {db}", record.package_uuid);
        }

        Commands::Retime { input, output, fps } => {
            let status = std::process::Command::new("ffmpeg")
                .arg("-y")
                .arg("-i")
                .arg(&input)
                .arg("-filter:v")
                .arg(format!("fps={fps}"))
                .arg("-c:a")
                .arg("copy")
                .arg(&output)
                .status();
            match status {
                Ok(s) if s.success() => println!("Retimed to {fps} fps: {output}"),
                Ok(s) => {
                    eprintln!("ffmpeg exited with code {}", s.code().unwrap_or(-1));
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("Failed to run ffmpeg: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Slate {
            input,
            output,
            text,
            frames,
        } => {
            let Some(picture) = postkit::probe::probe_video(std::path::Path::new(&input)) else {
                eprintln!("Cannot read the picture size and frame rate of {input}");
                std::process::exit(1);
            };
            // concat refuses a join unless both sides carry the same raster,
            // pixel format and aspect, so the slate is cut to the picture
            let slate = format!(
                "color=black:s={width}x{height}:r={fps_num}/{fps_den},\
                 drawtext=text='{text}':fontsize={font_size}:fontcolor=white:x=(w-text_w)/2:y=(h-text_h)/2,\
                 trim=end_frame={frames},setpts=PTS-STARTPTS,format=yuv420p,setsar=1[slate];\
                 [0:v]format=yuv420p,setsar=1[picture];\
                 [slate][picture]concat=n=2:v=1:a=0[out]",
                width = picture.width,
                height = picture.height,
                fps_num = picture.fps_num,
                fps_den = picture.fps_den,
                font_size = (picture.height / SLATE_TEXT_HEIGHT_DIVISOR).max(1),
            );
            let status = std::process::Command::new("ffmpeg")
                .arg("-y")
                .arg("-i")
                .arg(&input)
                .arg("-filter_complex")
                .arg(&slate)
                .arg("-map")
                .arg("[out]")
                .arg(&output)
                .status();
            match status {
                Ok(s) if s.success() => println!("Slate added ({frames} frames): {output}"),
                Ok(s) => {
                    eprintln!("ffmpeg exited with code {}", s.code().unwrap_or(-1));
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("Failed to run ffmpeg: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Mca {
            input,
            layout,
            language,
        } => {
            let Some(chosen) = imfwizard_core::mca::layout(&layout) else {
                eprintln!(
                    "Unknown layout: {layout}. Use one of: {}",
                    imfwizard_core::mca::layout_names()
                );
                std::process::exit(1);
            };
            let written = imfwizard_core::mca::write_mca_labels(
                std::path::Path::new(&input),
                chosen,
                &language,
            )
            .unwrap_or_else(|error| fail(error));

            println!("MCA labels written into {input}");
            println!("  Layout: {} ({})", chosen.name, chosen.labels);
            println!("  Language: {}", written.language);
            println!("  Channels: {}", chosen.channels);
            for document in &written.package_documents {
                println!("  Rewritten: {}", document.display());
            }
        }

        Commands::AvSync { input } => {
            // Compare each stream's start and end on the container clock. The initial
            // offset is the lip-sync delay; comparing the ends too catches drift that
            // accumulates over the program (e.g. an audio rate mismatch), which a
            // first-PTS-only check misses.
            let span = |stream: &str| -> Option<(f64, f64)> {
                let out = std::process::Command::new("ffprobe")
                    .args([
                        "-v",
                        "quiet",
                        "-select_streams",
                        stream,
                        "-show_entries",
                        "stream=start_time,duration",
                        "-of",
                        "csv=p=0",
                        &input,
                    ])
                    .output()
                    .ok()?;
                let text = String::from_utf8_lossy(&out.stdout);
                let mut it = text.lines().next()?.split(',');
                let start = it.next()?.trim().parse::<f64>().ok()?;
                let dur = it.next()?.trim().parse::<f64>().ok()?;
                Some((start, dur))
            };

            println!("A/V sync analysis: {input}");
            let (Some((v_start, v_dur)), Some((a_start, a_dur))) = (span("v:0"), span("a:0"))
            else {
                eprintln!(
                    "  Could not read stream start_time/duration (missing stream or ffprobe?)"
                );
                std::process::exit(1);
            };

            let initial_ms = (v_start - a_start) * 1000.0;
            let end_ms = ((v_start + v_dur) - (a_start + a_dur)) * 1000.0;
            let drift_ms = end_ms - initial_ms;
            println!("  Video: start {v_start:.3}s  duration {v_dur:.3}s");
            println!("  Audio: start {a_start:.3}s  duration {a_dur:.3}s");
            println!("  Initial offset: {initial_ms:+.1}ms (video - audio)");
            println!("  End offset:     {end_ms:+.1}ms");
            println!("  Progressive drift over program: {drift_ms:+.1}ms");
            if drift_ms.abs() > 20.0 {
                println!("  Note: offset grows over time (likely an audio/video rate mismatch)");
            }

            let worst = initial_ms.abs().max(end_ms.abs());
            // EBU R128 recommends < ±40ms for broadcast
            if worst < 5.0 {
                println!("  Result: PASS (offset < 5ms, frame-accurate)");
            } else if worst < 40.0 {
                println!(
                    "  Result: WARNING (offset {worst:.1}ms, within EBU tolerance but audible)"
                );
            } else {
                println!("  Result: FAIL (offset {worst:.1}ms, exceeds ±40ms tolerance)");
                std::process::exit(1);
            }
        }

        Commands::Atmos {
            input,
            output,
            fps_num,
            fps_den,
        } => {
            // Import Dolby Atmos ADM BWF into IMF-compatible MXF
            let input_path = std::path::Path::new(&input);
            let output_path = std::path::Path::new(&output);

            if fps_num == 0 || fps_den == 0 {
                fail(format!(
                    "--fps-num {fps_num} --fps-den {fps_den}: an edit rate needs both above 0"
                ));
            }

            let result =
                imfwizard_core::atmos::import_atmos(input_path, output_path, fps_num, fps_den);
            if result.success {
                println!(
                    "Atmos import complete: {} beds, {} objects, {} channels",
                    result.bed_count, result.object_count, result.total_channels
                );
                println!("  MXF: {}", result.mxf_output.display());
                println!("  ADM sidecar: {}", result.adm_sidecar.display());
            } else {
                eprintln!("Atmos import failed: {}", result.error);
                std::process::exit(1);
            }
        }

        Commands::Aces {
            input,
            output,
            target,
            idt,
            odt,
            ctl_dir,
        } => {
            if idt.is_some() || odt.is_some() {
                // Full CTL pipeline: IDT → RRT → ODT
                let opts = imfwizard_core::aces::AcesPipelineOptions {
                    input: std::path::Path::new(&input),
                    output: std::path::Path::new(&output),
                    idt: idt.as_deref(),
                    odt: odt.as_deref(),
                    ctl_dir: ctl_dir.as_deref().map(std::path::Path::new),
                };
                match imfwizard_core::aces::run_aces_pipeline(&opts) {
                    Ok(()) => println!("ACES pipeline complete: {output}"),
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                }
            } else {
                // Simple colour space conversion via postkit
                let target_space = parse_colour_space(&target);
                let opts = postkit::colour::ColourConvertOptions {
                    input: PathBuf::from(&input),
                    output: PathBuf::from(&output),
                    source_space: postkit::colour::ColourSpace::Aces,
                    target_space,
                    lut_path: None,
                };
                match postkit::colour::convert_colour(&opts) {
                    Ok(()) => println!("ACES converted to {target}: {output}"),
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                }
            }
        }

        Commands::Compliance { input, standard } => {
            // Validate IMP against platform-specific delivery requirements
            let Some(&(_, platform)) = COMPLIANCE_STANDARDS
                .iter()
                .find(|(name, _)| *name == standard.to_lowercase())
            else {
                eprintln!(
                    "Unknown standard: {standard} (known: {})",
                    compliance_standard_names()
                );
                std::process::exit(1);
            };
            let profile = platform.map(postkit::profiles::profile_for);
            let checked = match &profile {
                Some(profile) => {
                    println!("Checking compliance against: {}", profile.name);
                    println!(
                        "  Required: {}x{} @ {}fps, {} colour, {}-bit, {} audio",
                        profile.width,
                        profile.height,
                        profile.frame_rate,
                        profile.colour_space,
                        profile.bit_depth,
                        profile.audio_channels
                    );
                    profile.name.clone()
                }
                None => {
                    println!("Checking compliance against: {SMPTE_STRUCTURAL_STANDARD}");
                    SMPTE_STRUCTURAL_STANDARD.to_string()
                }
            };
            println!();

            // Structural validation
            let report = imfwizard_core::validate::validate_imp(std::path::Path::new(&input));
            let mut errors: Vec<String> = report.errors;
            let mut warnings: Vec<String> = report.warnings;

            // Probe MXF files for platform-specific parameter checks
            let imp_path = std::path::Path::new(&input);
            if let Some(profile) = &profile
                && imp_path.is_dir()
            {
                let mxf_files: Vec<_> = std::fs::read_dir(imp_path)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| {
                        e.path()
                            .extension()
                            .and_then(|x| x.to_str())
                            .is_some_and(|x| x.eq_ignore_ascii_case("mxf"))
                    })
                    .collect();
                for entry in &mxf_files {
                    let path = entry.path();
                    let fname = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let Ok(out) = std::process::Command::new("ffprobe")
                        .args(["-v", "quiet", "-show_streams", "-of", "json"])
                        .arg(&path)
                        .output()
                    else {
                        continue;
                    };
                    let json_str = String::from_utf8_lossy(&out.stdout);
                    let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) else {
                        continue;
                    };
                    let Some(streams) = val.get("streams").and_then(|s| s.as_array()) else {
                        continue;
                    };
                    for stream in streams {
                        let codec_type = stream
                            .get("codec_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if codec_type == "video" {
                            let w =
                                stream.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                            let h =
                                stream.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                            let bits = stream
                                .get("bits_per_raw_sample")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<u32>().ok())
                                .unwrap_or(0);
                            if w > 0 && w != profile.width {
                                errors.push(format!(
                                    "{fname}: width {w} != required {}",
                                    profile.width
                                ));
                            }
                            if h > 0
                                && h != profile.height
                                && !(profile.height == 2160
                                    && (h == 1600 || h == 1800 || h == 2160))
                                && !(profile.height == 1080 && (h == 858 || h == 1080))
                            {
                                warnings.push(format!(
                                    "{fname}: height {h} != nominal {}",
                                    profile.height
                                ));
                            }
                            if bits > 0 && bits < profile.bit_depth {
                                errors.push(format!(
                                    "{fname}: bit depth {bits} < required {}",
                                    profile.bit_depth
                                ));
                            }
                        } else if codec_type == "audio" {
                            let sr = stream
                                .get("sample_rate")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<u32>().ok())
                                .unwrap_or(0);
                            let bits = stream
                                .get("bits_per_raw_sample")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<u32>().ok())
                                .unwrap_or(0);
                            if sr > 0 && sr != profile.audio_sample_rate {
                                errors.push(format!(
                                    "{fname}: sample rate {sr}Hz != required {}Hz",
                                    profile.audio_sample_rate
                                ));
                            }
                            if bits > 0 && bits < profile.audio_bit_depth {
                                errors.push(format!(
                                    "{fname}: audio bit depth {bits} < required {}",
                                    profile.audio_bit_depth
                                ));
                            }
                            let channels = stream
                                .get("channels")
                                .and_then(|v| v.as_u64())
                                .unwrap_or_default()
                                as u32;
                            if let Some(required) =
                                imfwizard_core::profiles::required_audio_channels(profile)
                                && channels > 0
                                && channels != required
                            {
                                errors.push(format!(
                                    "{fname}: {channels} audio channels != the {required} of {}",
                                    profile.audio_channels
                                ));
                            }
                        }
                    }
                }
            }

            // Report
            if errors.is_empty() && warnings.is_empty() {
                println!("PASS: compliant with {checked}");
            } else {
                if !errors.is_empty() {
                    println!("ERRORS ({}):", errors.len());
                    for e in &errors {
                        println!("  ✗ {e}");
                    }
                }
                if !warnings.is_empty() {
                    println!("WARNINGS ({}):", warnings.len());
                    for w in &warnings {
                        println!("  ⚠ {w}");
                    }
                }
                if !errors.is_empty() {
                    println!("\nFAIL: not compliant with {checked}");
                    std::process::exit(1);
                } else {
                    println!("\nPASS (with warnings): compliant with {checked}");
                }
            }
        }

        Commands::Annotate { imp, text } => {
            let edit = postkit::package_edit::PackageEdit {
                input: PathBuf::from(&imp),
                annotation: Some(text),
                ..Default::default()
            };
            match postkit::package_edit::edit_package(&edit) {
                Ok(edited) => {
                    println!(
                        "Annotated {imp}: {} now carries composition id {}",
                        edited.cpl_path.display(),
                        edited.composition_id
                    );
                    if !edited.unsigned_documents.is_empty() {
                        println!(
                            "Wrote unsigned, the rewrite changed the bytes their signature covered: {}. Re-sign the package if it has to stay signed",
                            edited.unsigned_documents.join(", ")
                        );
                    }
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Doctor { json } => {
            let result = imfwizard_core::tools::check_all_tools();
            if json {
                let out = serde_json::to_string_pretty(&result).unwrap_or_default();
                println!("{out}");
            } else {
                print!("{}", imfwizard_core::tools::format_doctor_report(&result));
            }
            if result.required_missing > 0 {
                std::process::exit(1);
            }
        }

        Commands::Restore {
            input,
            output,
            video_only,
            audio_only,
        } => {
            let input_path = PathBuf::from(&input);
            let output_path = PathBuf::from(&output);
            std::fs::create_dir_all(&output_path).unwrap_or_default();

            let mxf_files: Vec<PathBuf> = std::fs::read_dir(&input_path)
                .unwrap_or_else(|e| {
                    eprintln!("Cannot read IMP directory: {e}");
                    std::process::exit(1);
                })
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case("mxf"))
                })
                .collect();

            if mxf_files.is_empty() {
                eprintln!("No MXF files found in {input}");
                std::process::exit(1);
            }

            let mut extracted = 0u32;
            for mxf in &mxf_files {
                let stem = mxf.file_stem().unwrap_or_default().to_string_lossy();
                let track_dir = output_path.join(stem.as_ref());
                std::fs::create_dir_all(&track_dir).unwrap_or_default();

                // Use asdcp-unwrap to extract essence from MXF
                let status = std::process::Command::new("asdcp-unwrap")
                    .arg(mxf)
                    .arg("-d")
                    .arg(&track_dir)
                    .status();

                match status {
                    Ok(s) if s.success() => {
                        // Check if we should filter by type
                        let has_j2c = std::fs::read_dir(&track_dir)
                            .map(|rd| {
                                rd.filter_map(|e| e.ok()).any(|e| {
                                    e.path()
                                        .extension()
                                        .and_then(|x| x.to_str())
                                        .is_some_and(|x| x == "j2c")
                                })
                            })
                            .unwrap_or(false);
                        let has_wav = std::fs::read_dir(&track_dir)
                            .map(|rd| {
                                rd.filter_map(|e| e.ok()).any(|e| {
                                    e.path()
                                        .extension()
                                        .and_then(|x| x.to_str())
                                        .is_some_and(|x| x == "wav" || x == "pcm")
                                })
                            })
                            .unwrap_or(false);

                        if (video_only && !has_j2c) || (audio_only && !has_wav) {
                            // Remove the track dir if it doesn't match the filter
                            let _ = std::fs::remove_dir_all(&track_dir);
                            continue;
                        }

                        println!("Extracted {}", mxf.display());
                        extracted += 1;
                    }
                    Ok(s) => {
                        eprintln!(
                            "asdcp-unwrap failed for {} (exit {})",
                            mxf.display(),
                            s.code().unwrap_or(-1)
                        );
                    }
                    Err(e) => {
                        eprintln!("Failed to run asdcp-unwrap: {e}");
                        eprintln!("Install asdcplib tools or ensure asdcp-unwrap is in PATH");
                        std::process::exit(1);
                    }
                }
            }
            println!("Restored {extracted} track(s) to {output}");
        }

        Commands::DvConvert {
            input,
            output,
            target_profile,
        } => {
            let mode = match target_profile.as_str() {
                "8.1" => postkit::dolby_vision::DvMode::Mode2,
                "8.4" => postkit::dolby_vision::DvMode::Mode5,
                other => {
                    eprintln!("Unsupported target profile: {other} (supported: 8.1, 8.4)");
                    std::process::exit(1);
                }
            };
            let input_path = std::path::Path::new(&input);
            let output_path = std::path::Path::new(&output);
            match postkit::dolby_vision::convert_dv_mode(input_path, output_path, mode) {
                Ok(()) => {
                    println!("Converted to Dolby Vision profile {target_profile}: {output}");
                }
                Err(e) => {
                    eprintln!("DV conversion of {input} failed: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Version { action } => {
            use postkit::version_tracker::{DeliveryRecord, VersionQuery, VersionTracker};

            fn open_tracker(db: &str) -> VersionTracker {
                let mut tracker = VersionTracker::new();
                if !tracker.open(std::path::Path::new(db)) {
                    tracing::error!("Failed to open tracker database: {db}");
                    std::process::exit(1);
                }
                tracker
            }

            match action {
                VersionAction::Record {
                    db,
                    package_uuid,
                    title,
                    version,
                    destination,
                    method,
                    verified,
                } => {
                    let tracker = open_tracker(&db);
                    let timestamp = rfc3339_now();
                    let record = DeliveryRecord {
                        package_uuid,
                        title,
                        version,
                        destination,
                        delivery_method: method,
                        timestamp,
                        verified,
                    };
                    if !tracker.record(&record) {
                        tracing::error!("Failed to record delivery");
                        std::process::exit(1);
                    }
                    println!("Recorded delivery of {}", record.package_uuid);
                }

                VersionAction::List {
                    db,
                    package_uuid,
                    destination,
                } => {
                    let tracker = open_tracker(&db);
                    let query = VersionQuery {
                        package_uuid,
                        destination,
                        ..Default::default()
                    };
                    let records = tracker.query(&query);
                    if records.is_empty() {
                        println!("No deliveries recorded");
                    }
                    for record in &records {
                        println!(
                            "{}  {}  {}  -> {}  ({}, verified={})",
                            record.timestamp,
                            record.package_uuid,
                            record.title,
                            record.destination,
                            record.delivery_method,
                            record.verified
                        );
                    }
                }

                VersionAction::Export { db, output } => {
                    let tracker = open_tracker(&db);
                    let out = PathBuf::from(&output);
                    let exported = if output.to_lowercase().ends_with(".csv") {
                        tracker.export_csv(&out)
                    } else {
                        tracker.export_json(&out)
                    };
                    if !exported {
                        tracing::error!("Failed to export delivery history");
                        std::process::exit(1);
                    }
                    println!("Exported delivery history to {output}");
                }
            }
        }
    }

    if gpu_enabled {
        tracing::info!(
            "grok's accelerator plugin ran {} frames on the device",
            postkit::grok_encoder::accelerated_frames()
        );
    }
}

fn compliance_standard_names() -> String {
    COMPLIANCE_STANDARDS
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

fn compliance_standard_help() -> String {
    format!("Standard to check against: {}", compliance_standard_names())
}

fn prores_container_help() -> String {
    format!(
        "Fit an IMP's picture into a named container: {}",
        ContainerRaster::names()
    )
}

fn encode_prores_file(input: &str, output: &str, profile: &str, container: Option<&str>) {
    if container.is_some() {
        eprintln!("Error: --container fits an IMP's picture, and {input} is not an IMP directory");
        std::process::exit(1);
    }
    let prores_profile = match profile.to_lowercase().as_str() {
        "proxy" => "0",
        "lt" => "1",
        "standard" => "2",
        "hq" => "3",
        "4444" => "4",
        "4444xq" => "5",
        _ => "3",
    };
    let status = std::process::Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-c:v")
        .arg("prores_ks")
        .arg("-profile:v")
        .arg(prores_profile)
        .arg("-c:a")
        .arg("pcm_s24le")
        .arg(output)
        .status();
    match status {
        Ok(s) if s.success() => println!("ProRes encoded: {output}"),
        Ok(s) => {
            eprintln!("ffmpeg exited with code {}", s.code().unwrap_or(-1));
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Failed to run ffmpeg: {e}");
            std::process::exit(1);
        }
    }
}

fn parse_colour_space(s: &str) -> postkit::colour::ColourSpace {
    postkit::colour::parse_colour_space(s).unwrap_or(postkit::colour::ColourSpace::Rec709)
}
