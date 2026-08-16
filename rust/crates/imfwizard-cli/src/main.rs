use clap::{Parser, Subcommand, ValueEnum};
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
}

/// postkit's `KdmFormat` carries no clap or `FromStr` impl, so the command line
/// spelling of it lives here. Clap derives the two names from the variants.
#[derive(Copy, Clone, Default, PartialEq, Eq, ValueEnum)]
enum KdmFormatArgument {
    #[default]
    Smpte,
    Interop,
}

impl From<KdmFormatArgument> for postkit::certificate::KdmFormat {
    fn from(argument: KdmFormatArgument) -> Self {
        match argument {
            KdmFormatArgument::Smpte => Self::Smpte,
            KdmFormatArgument::Interop => Self::Interop,
        }
    }
}

/// Report a failure the way every command does, and stop.
fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("Error: {message}");
    std::process::exit(1);
}

/// Encode picture through the shared grok pipeline (grk_compress), printing
/// per-frame progress. Ctrl-C cancels the run.
fn encode_picture(
    input: &std::path::Path,
    output_dir: &std::path::Path,
    options: &postkit::pipeline::EncodeRunOptions,
) -> Result<postkit::pipeline::EncodeResult, String> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let cancel = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));
    let on_interrupt = cancel.clone();
    let _ = ctrlc::set_handler(move || on_interrupt.store(true, Ordering::Relaxed));

    let result = postkit::pipeline::run_encode_with_options(
        input,
        output_dir,
        options,
        &cancel,
        &pause,
        |p: &postkit::pipeline::PipelineProgress| {
            eprint!(
                "\r[encode] {}/{} frames ({:.0}%) {:.1} fps   ",
                p.frame, p.total_frames, p.percent, p.fps
            );
        },
        |msg: &str| tracing::info!("{msg}"),
    );
    eprintln!();
    result
}

#[derive(Subcommand)]
enum Commands {
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

        /// TTML/IMSC subtitle file to package (repeatable)
        #[arg(long = "subtitle")]
        subtitles: Vec<String>,

        /// Content kind (feature, trailer, etc.)
        #[arg(short, long, default_value = "feature")]
        kind: String,

        /// Delivery preset (netflix, disney, apple, hbo, amazon, dci-2k, dci-4k,
        /// broadcast, archival); sets the J2K target bitrate. See `profiles`.
        #[arg(long)]
        profile: Option<String>,

        /// Frame rate numerator
        #[arg(long, default_value = "24")]
        fps_num: u32,

        /// Frame rate denominator
        #[arg(long, default_value = "1")]
        fps_den: u32,

        /// HDR/WCG preset for the picture essence (ST 2067-21): pq-bt2020 or
        /// pq-p3d65. Writes the transfer/colour ULs into the MXF and CPL.
        #[arg(long)]
        hdr: Option<String>,

        /// ST 2086 mastering display, x265 master-display string, e.g.
        /// "G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(40000000,50)".
        /// Requires --hdr.
        #[arg(long = "mastering-display")]
        mastering_display: Option<String>,

        /// Maximum content light level in nits, written as a ST 2067-21 CPL
        /// ExtensionProperty. Requires --hdr.
        #[arg(long = "max-cll")]
        max_cll: Option<u16>,

        /// Maximum frame-average light level in nits, same placement as
        /// --max-cll. Requires --hdr.
        #[arg(long = "max-fall")]
        max_fall: Option<u16>,

        /// Shift the sound against the picture in milliseconds; positive is
        /// later. The running time never changes: the shift is padded at one end
        /// and truncated at the other.
        #[arg(long = "audio-delay")]
        audio_delay: Option<i64>,

        /// Colour space the picture source carries: rec709 (default), p3, xyz,
        /// rec2020, aces, acescg or logc. rec709 runs the encoder's X'Y'Z'
        /// transform, xyz leaves the frames alone.
        #[arg(long = "source-colourspace")]
        source_colourspace: Option<String>,

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
        #[arg(short, long, default_value = "250")]
        bitrate: f64,

        /// Number of threads
        #[arg(short, long, default_value = "0")]
        threads: u32,
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
        #[arg(long = "dir", short)]
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

    /// Watch directory for changes
    Watch {
        /// Directory to watch
        dir: PathBuf,
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
        /// Audio file to measure (WAV required for --adjust-to)
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

    /// Convert IMP to a delivery target format
    #[command(name = "target-convert")]
    TargetConvert {
        /// Input IMP directory
        #[arg(short, long)]
        input: String,

        /// Target platform (e.g. netflix, apple, amazon)
        #[arg(short, long)]
        target: String,

        /// Output directory (defaults to input + _delivery)
        #[arg(short, long)]
        output: Option<String>,
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

    /// Edit CPL metadata
    #[command(name = "metadata-edit")]
    MetadataEdit {
        /// IMP directory
        #[arg(short, long)]
        imp: String,

        /// Title to set
        #[arg(short, long)]
        title: Option<String>,

        /// Annotation text
        #[arg(short, long)]
        annotation: Option<String>,

        /// Issuer
        #[arg(long)]
        issuer: Option<String>,
    },

    /// Create DCDM (Digital Cinema Distribution Master) X'Y'Z' sequence
    Dcdm {
        /// Input image sequence directory
        #[arg(short, long)]
        input: String,

        /// Output DCDM TIFF directory
        #[arg(short, long)]
        output: String,

        /// Source colour space (rec709, p3, aces, logc)
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

        /// Source colour space (rec709, p3, xyz, rec2020, aces, acescg, logc)
        #[arg(short, long)]
        source: String,

        /// Target colour space
        #[arg(short, long)]
        target: String,

        /// Optional 3D LUT file for custom transform
        #[arg(long)]
        lut: Option<String>,
    },

    /// Import EDL/AAF/XML timeline for conforming
    Conform {
        /// Input timeline file (EDL, AAF, FCP XML, OTIO)
        #[arg(short, long)]
        input: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Ingest camera raw media
    Ingest {
        /// Camera card/media directory
        #[arg(short, long)]
        source: String,

        /// Output directory
        #[arg(short, long)]
        output: String,

        /// Output format (dpx, tiff, exr, prores)
        #[arg(short, long, default_value = "dpx")]
        format: String,

        /// Colour space (ACES, Rec.709, P3, LogC)
        #[arg(short, long, default_value = "ACES")]
        colour_space: String,
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

        /// Rating (e.g. PG-13, R)
        #[arg(long)]
        rating: String,
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
        /// Input video/image sequence
        #[arg(short, long)]
        input: String,

        /// Output ProRes file
        #[arg(short, long)]
        output: String,

        /// Profile (proxy, lt, standard, hq, 4444, 4444xq)
        #[arg(short, long, default_value = "hq")]
        profile: String,
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

    /// Set MCA (Multi-Channel Audio) labels
    Mca {
        /// Input MXF audio file
        #[arg(short, long)]
        input: String,

        /// Channel layout (e.g. "51", "71", "stereo")
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
    },

    /// Check external tool dependencies
    Doctor {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Preview via SDI output (Decklink/AJA)
    #[command(name = "sdi-preview")]
    SdiPreview {
        /// Input IMP directory or MXF file
        #[arg(short, long)]
        input: String,

        /// SDI device index (default 0)
        #[arg(short, long, default_value = "0")]
        device: u32,
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

        /// Standard (smpte, netflix, dolby, amazon)
        #[arg(short, long, default_value = "smpte")]
        standard: String,
    },

    /// Import EDL/AAF/XML timeline
    #[command(name = "edl-import")]
    EdlImport {
        /// Input timeline file (EDL, AAF, FCP XML, OTIO)
        #[arg(short, long)]
        input: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
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

    /// Generate a SMPTE 430-1 Key Delivery Message (KDM)
    ///
    /// There is no way to supply content keys yet, and imfwizard does not
    /// encrypt an IMP, so the KDM carries a freshly minted key and unlocks no
    /// existing essence. It is usable as a test or template KDM only.
    Kdm {
        /// CPL UUID to target
        #[arg(long)]
        cpl_id: String,

        /// Content title for the KDM
        #[arg(long)]
        content_title: String,

        /// Recipient certificate PEM file
        #[arg(long)]
        cert: PathBuf,

        /// Signer certificate PEM file
        #[arg(long)]
        signer_cert: PathBuf,

        /// Signer private key PEM file
        #[arg(long)]
        signer_key: PathBuf,

        /// Signer certificate chain PEM file (repeatable, leaf to root)
        #[arg(long = "signer-chain")]
        signer_chain: Vec<PathBuf>,

        /// Output KDM XML file
        #[arg(short, long)]
        output: PathBuf,

        /// Validity start (ISO 8601 or "now")
        #[arg(long, default_value = "now")]
        valid_from: String,

        /// Validity end (ISO 8601 or duration like "2 weeks")
        #[arg(long, default_value = "2 weeks")]
        valid_to: String,

        /// Device certificate PEM (repeatable): its thumbprint joins the KDM's
        /// authorized device list, and naming any device drops the assume-trust
        /// marker
        #[arg(long = "device-cert")]
        device_cert: Vec<PathBuf>,

        /// KDM formulation: the dci- ones add a ContentAuthenticator;
        /// multiple-modified-transitional-1 and dci-specific list the
        /// --device-cert devices, the other two trust any device
        #[arg(long, default_value_t = postkit::certificate::KdmFormulation::default())]
        formulation: postkit::certificate::KdmFormulation,

        /// KDM format: smpte (default) or interop (legacy, needs real-gear validation)
        #[arg(long, value_enum, default_value_t = KdmFormatArgument::default())]
        format: KdmFormatArgument,

        /// AnnotationText override (default: "<title> KDM for <recipient>")
        #[arg(long)]
        annotation: Option<String>,

        /// Disable forensic marking of the picture essence, as press and
        /// festival screenings are usually ordered
        #[arg(short = 'p', long)]
        disable_forensic_marking_picture: bool,

        /// Disable forensic marking of the audio essence, optionally only above
        /// a given channel (e.g. 12) so the HI/VI tracks below it keep theirs
        #[arg(short = 'a', long, num_args = 0..=1, value_name = "CHANNEL")]
        disable_forensic_marking_audio: Option<Option<u32>>,
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
        /// Input HEVC file with RPU
        #[arg(short, long)]
        input: String,

        /// Output file
        #[arg(short, long)]
        output: String,

        /// Target DV profile (8.1, 8.4)
        #[arg(long, default_value = "8.1")]
        target_profile: String,
    },
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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| level.into()),
        )
        .init();

    match cli.command {
        Commands::Create {
            output,
            title,
            video,
            audio,
            audio_lang,
            audio_role,
            subtitles,
            kind,
            profile,
            fps_num,
            fps_den,
            hdr,
            mastering_display,
            max_cll,
            max_fall,
            audio_delay,
            source_colourspace,
            trim_start,
            trim_end,
            still_length,
        } => {
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
            let hdr = match hdr.as_deref() {
                Some(preset) => match imfwizard_core::hdr_wcg::HdrWcg::from_flags(
                    preset,
                    mastering_display.as_deref(),
                ) {
                    Ok(h) => Some(h.with_content_light_levels(max_cll, max_fall)),
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                },
                None => None,
            };
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
            // the source colour space decides the encoder transform, so a bad
            // spelling has to fail before anything is encoded
            let colourspace = source_colourspace
                .as_deref()
                .map(|s| imfwizard_core::source_colourspace::parse(s).unwrap_or_else(|e| fail(e)));
            if let Some(space) = colourspace
                && hdr.is_some()
                && imfwizard_core::source_colourspace::applies_encoder_transform(space)
            {
                fail(format!(
                    "--hdr labels the picture as essence nothing transformed, but \
                     --source-colourspace {} makes the encoder run its own X'Y'Z' transform. \
                     Use xyz for a source that is already X'Y'Z'",
                    source_colourspace.as_deref().unwrap_or_default()
                ));
            }
            let source_colour = imfwizard_core::source_colourspace::to_source_colour(
                colourspace.unwrap_or(postkit::colour::ColourSpace::Rec709),
            )
            .unwrap_or_else(|e| fail(e));

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
                .filter(|p| imfwizard_core::still::is_still_image(p));
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

            let _ = std::fs::create_dir_all(&output);

            // If video is a file, run encode pipeline
            let (j2k_dir, audio_files) = if let (Some(image), Some(hold_for)) =
                (&still_input, still_frames)
            {
                tracing::info!("Holding {} for {hold_for} frames", image.display());
                let scratch = imfwizard_core::still::prepare_still_source(image, &output)
                    .unwrap_or_else(|e| fail(e));
                let encoded = encode_picture(
                    &scratch.source_dir,
                    &scratch.encode_dir,
                    &postkit::pipeline::EncodeRunOptions {
                        fps: fps_num / fps_den.max(1),
                        source_colour: source_colour.clone(),
                        ..Default::default()
                    },
                )
                .unwrap_or_else(|e| fail(format!("Encode failed: {e}")));
                let held = imfwizard_core::still::hold_frames(
                    &encoded.j2k_dir,
                    hold_for,
                    &output.join(imfwizard_core::still::HELD_PICTURE_DIR),
                )
                .unwrap_or_else(|e| fail(e));
                (
                    Some(held),
                    audio
                        .as_ref()
                        .map(|a| vec![PathBuf::from(a)])
                        .unwrap_or_default(),
                )
            } else if let Some(ref vid) = video {
                let video_path = PathBuf::from(vid);
                let is_video_file = video_path.is_file()
                    && video_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| {
                            matches!(
                                e.to_lowercase().as_str(),
                                "mp4" | "mov" | "mkv" | "avi" | "mxf" | "ts" | "m2ts" | "webm"
                            )
                        })
                        .unwrap_or(false);

                if is_video_file {
                    tracing::info!("Detected video file, encoding to J2K");

                    // Probe for actual frame rate
                    let probed = imfwizard_core::probe::probe_video(&video_path);
                    let actual_fps = probed
                        .as_ref()
                        .map(|v| v.fps_num / v.fps_den.max(1))
                        .unwrap_or(fps_num);
                    if let Some(ref info) = probed {
                        tracing::info!(
                            "Input: {}x{} @ {}/{} fps",
                            info.width,
                            info.height,
                            info.fps_num,
                            info.fps_den
                        );
                        if let Err(error) = imfwizard_core::mxf_wrap::validate_app2e_picture(
                            info.width,
                            info.height,
                            12,
                        ) {
                            eprintln!("Error: {error}");
                            std::process::exit(1);
                        }
                    }

                    tracing::info!("Compressor: Grok");
                    // A delivery preset sets the target bitrate; convert it to a J2K
                    // compression ratio (raw = w*h*36 bits/frame), else default 10x.
                    let compression_ratio = match (&preset, &probed) {
                        (Some(p), Some(info)) => {
                            let fps = (info.fps_num as f64 / info.fps_den.max(1) as f64).max(1.0);
                            let raw_bits = info.width as f64 * info.height as f64 * 36.0;
                            let target_bits = (p.bitrate_mbps * 1_000_000.0) / fps;
                            (raw_bits / target_bits).max(1.0)
                        }
                        _ => 10.0,
                    };
                    if let Some(p) = &preset {
                        tracing::info!(
                            "Preset {}: {} Mbps (ratio {compression_ratio:.1})",
                            p.name,
                            p.bitrate_mbps
                        );
                    }
                    // encode via the shared grok pipeline (grk_compress); no fallback
                    let encoded = encode_picture(
                        &video_path,
                        &output,
                        &postkit::pipeline::EncodeRunOptions {
                            compression_ratio,
                            fps: actual_fps,
                            source_colour: source_colour.clone(),
                            ..Default::default()
                        },
                    );
                    let j2k_out = match encoded {
                        Ok(r) => {
                            tracing::info!("Encoded {} frames", r.frames_encoded);
                            r.j2k_dir
                        }
                        Err(e) => {
                            eprintln!("Error: Encode failed: {e}");
                            std::process::exit(1);
                        }
                    };

                    // Auto-demux audio
                    let audio_files = if let Some(a) = &audio {
                        vec![PathBuf::from(a)]
                    } else {
                        let wav_out = output.join("audio_demux.wav");
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
                            Ok(o) if o.status.success() => {
                                tracing::info!("Demuxed audio: {}", wav_out.display());
                                vec![wav_out]
                            }
                            _ => vec![],
                        }
                    };

                    (Some(j2k_out), audio_files)
                } else {
                    // Assume it's a J2K directory
                    imfwizard_core::source_colourspace::reject_on_precompressed_picture(
                        &video_path,
                        &source_colour,
                    )
                    .unwrap_or_else(|e| fail(e));
                    (
                        Some(video_path),
                        audio
                            .as_ref()
                            .map(|a| vec![PathBuf::from(a)])
                            .unwrap_or_default(),
                    )
                }
            } else {
                (
                    None,
                    audio
                        .as_ref()
                        .map(|a| vec![PathBuf::from(a)])
                        .unwrap_or_default(),
                )
            };

            let source = imfwizard_core::source_edits::apply_source_edits(
                &edits,
                &imfwizard_core::source_edits::CompositionSource {
                    j2k_dir,
                    audio_files,
                    timed_text_files: subtitles.iter().map(PathBuf::from).collect(),
                },
                &output,
                fps_num,
                fps_den,
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
                    audio_files: audio_tracks,
                    timed_text_files: source.timed_text_files,
                    hdr,
                }],
                fps_num,
                fps_den,
                ..Default::default()
            };
            let result = imfwizard_core::imp::create_imp(&opts);
            if result.success {
                println!("IMP created at {}", result.output_dir.display());
                for cpl in &result.cpl_paths {
                    println!("  CPL: {}", cpl.display());
                }
                println!("  PKL: {}", result.pkl_path.display());
                println!("  ASSETMAP: {}", result.assetmap_path.display());
            } else {
                eprintln!("Error: {}", result.error);
                std::process::exit(1);
            }
        }

        Commands::Encode {
            input,
            output,
            bitrate,
            threads,
        } => {
            let opts = imfwizard_core::encode::EncodeOptions {
                input_dir: input,
                output_dir: output,
                bitrate_mbps: bitrate,
                num_threads: threads,
                ..Default::default()
            };
            let result = imfwizard_core::encode::encode(&opts);
            if result.success {
                println!("Encoding complete: {} frames", result.frames_encoded);
            } else {
                eprintln!("Error: {}", result.error);
                std::process::exit(1);
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

        Commands::SubtitleConvert { input, output } => {
            match imfwizard_core::subtitle_convert::convert_subtitles(
                &input,
                &output,
                imfwizard_core::subtitle_convert::SubtitleFormat::ImscTtml,
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
                Ok(meta) => println!("HDR10+ metadata written to {}", meta.json_path.display()),
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::DvExtract { input, output } => {
            match imfwizard_core::dolby_vision::extract_rpu(&input, &output) {
                Ok(()) => println!("RPU extracted to {}", output.display()),
                Err(e) => {
                    eprintln!("Error: {e}");
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

        Commands::Watch { dir } => {
            println!("Watching {} for changes...", dir.display());
            match imfwizard_core::watch::FileWatcher::new(&dir) {
                Ok(watcher) => {
                    while let Some(event) = watcher.next_event() {
                        println!("{event:?}");
                    }
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
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
            let mut failed = false;
            let result = imfwizard_core::validate::validate_imp(std::path::Path::new(&dir));
            if result.valid {
                println!("IMP validation PASSED");
                for w in &result.warnings {
                    println!("  warning: {w}");
                }
            } else {
                failed = true;
                eprintln!("IMP validation FAILED");
                for e in &result.errors {
                    eprintln!("  error: {e}");
                }
                for w in &result.warnings {
                    eprintln!("  warning: {w}");
                }
            }

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
                let jar = photon_jar.as_deref().map(std::path::Path::new);
                match imfwizard_core::photon::run_photon(std::path::Path::new(&dir), jar) {
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
        } => {
            let opts = imfwizard_core::to_dcp::ToDcpOptions {
                imp_dir: PathBuf::from(input),
                output_dir: PathBuf::from(output),
                title,
                content_kind: kind,
            };
            let result = imfwizard_core::to_dcp::imp_to_dcp(&opts);
            if result.success {
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

        Commands::TargetConvert {
            input,
            target,
            output,
        } => {
            let imp_dir = PathBuf::from(&input);
            let output_dir = output
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(format!("{input}_delivery")));

            let spec = match imfwizard_core::delivery::spec_for_target(&target) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            };

            match imfwizard_core::delivery::deliver(&imp_dir, &output_dir, &spec) {
                Ok(out) => {
                    println!("Delivered to: {}", out.display());
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Report {
            imp,
            output,
            format,
        } => {
            let imp_dir = std::path::Path::new(&imp);
            let validate_result = imfwizard_core::validate::validate_imp(imp_dir);
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
                report.error_count += 1;
                report.entries.push(postkit::report::ReportEntry {
                    severity: "error".to_string(),
                    category: "validation".to_string(),
                    message: err.clone(),
                    details: String::new(),
                });
            }
            for warn in &validate_result.warnings {
                report.warning_count += 1;
                report.entries.push(postkit::report::ReportEntry {
                    severity: "warning".to_string(),
                    category: "validation".to_string(),
                    message: warn.clone(),
                    details: String::new(),
                });
            }
            if report.error_count == 0 {
                report.pass_count = 1;
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
            let imp_dir = std::path::Path::new(&imp);
            let cpls = imfwizard_core::timeline::list_cpls(imp_dir);
            if cpls.is_empty() {
                eprintln!("Error: No CPL found in {imp}");
                std::process::exit(1);
            }
            let cpl_path = imp_dir.join(&cpls[0].file_path);
            let text = title
                .or(annotation)
                .unwrap_or_else(|| "Updated by imfwizard".to_string());
            let ann = imfwizard_core::cpl_annotation::CplAnnotation {
                author: issuer.unwrap_or_else(|| "imfwizard".to_string()),
                timestamp: String::new(),
                text,
                revision: String::new(),
            };
            match imfwizard_core::cpl_annotation::annotate_cpl(&cpl_path, &ann) {
                Ok(()) => println!("Metadata updated for {}", cpl_path.display()),
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

        Commands::Conform { input, json } => {
            let timeline = match postkit::conform::parse_timeline(std::path::Path::new(&input)) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&timeline).unwrap());
            } else {
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
        }

        Commands::Ingest {
            source,
            output,
            format,
            colour_space,
        } => {
            let opts = postkit::ingest::IngestOptions {
                source: PathBuf::from(&source),
                output_dir: PathBuf::from(output),
                output_format: format,
                colour_space,
                ..Default::default()
            };
            let exit = postkit::ingest::ingest(&opts);
            if exit != 0 {
                std::process::exit(exit);
            }
            println!("Ingest complete");
        }

        Commands::FrameExtract {
            input,
            frame,
            output,
        } => {
            let result = postkit::preview::extract_frame(
                std::path::Path::new(&input),
                frame,
                std::path::Path::new(&output),
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
                input: PathBuf::from(input),
                rpu_file: PathBuf::from(rpu),
                output: PathBuf::from(&output),
                ..Default::default()
            };
            let result = postkit::dolby_vision::inject_dolby_vision(&opts);
            if result == 0 {
                println!("Dolby Vision RPU injected: {output}");
            } else {
                eprintln!("Error: DV injection failed");
                std::process::exit(1);
            }
        }

        Commands::Hdr10Inject {
            input,
            output,
            max_cll,
            max_fall,
        } => {
            let opts = postkit::dolby_vision::HdrMetadataOptions {
                input: PathBuf::from(input),
                output: PathBuf::from(&output),
                hdr_type: postkit::dolby_vision::HdrType::Hdr10,
                hdr10: postkit::dolby_vision::Hdr10Metadata {
                    max_cll,
                    max_fall,
                    ..Default::default()
                },
                ..Default::default()
            };
            let result = postkit::dolby_vision::inject_hdr10_metadata(&opts);
            if result == 0 {
                println!("HDR10 metadata injected: {output}");
            } else {
                eprintln!("Error: HDR10 injection failed");
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
        } => {
            let opts = postkit::trailer::TrailerOptions {
                content_dir: PathBuf::from(&content),
                audio_file: PathBuf::from(&audio),
                output_dir: PathBuf::from(&output),
                title,
                rating,
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
        } => {
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
                .arg(&input)
                .arg("-c:v")
                .arg("prores_ks")
                .arg("-profile:v")
                .arg(prores_profile)
                .arg("-c:a")
                .arg("pcm_s24le")
                .arg(&output)
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

        Commands::PartialVersion { input, output, cpl } => {
            // Copy only files referencing the target CPL UUID
            std::fs::create_dir_all(&output).unwrap();
            let input_dir = std::path::Path::new(&input);
            let output_dir = std::path::Path::new(&output);
            let mut copied = 0u32;
            if let Ok(entries) = std::fs::read_dir(input_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().is_some_and(|e| e == "xml" || e == "mxf") {
                        // Check if file references the CPL
                        let name = path.file_name().unwrap().to_string_lossy();
                        if name.contains(&cpl) {
                            std::fs::copy(&path, output_dir.join(path.file_name().unwrap()))
                                .unwrap();
                            copied += 1;
                        } else if path.extension().is_some_and(|e| e == "xml")
                            && let Ok(content) = std::fs::read_to_string(&path)
                            && content.contains(&cpl)
                        {
                            std::fs::copy(&path, output_dir.join(path.file_name().unwrap()))
                                .unwrap();
                            copied += 1;
                        }
                    }
                }
            }
            println!("Partial version created: {copied} files copied to {output}");
        }

        Commands::Deliver { input, destination } => {
            let mut tracker = postkit::version_tracker::VersionTracker::new();
            let db_path = PathBuf::from(&input).join(".imfwizard_deliveries.db");
            tracker.open(&db_path);

            let delivery_method = if destination.starts_with("s3://") {
                "s3"
            } else if destination.starts_with("aspera://") || destination.starts_with("fasp://") {
                "aspera"
            } else {
                "rsync"
            };

            let record = postkit::version_tracker::DeliveryRecord {
                package_uuid: String::new(),
                title: String::new(),
                version: String::from("1"),
                destination: destination.clone(),
                delivery_method: delivery_method.to_string(),
                timestamp: String::new(),
                verified: false,
            };
            tracker.record(&record);

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
            let status = std::process::Command::new("ffmpeg")
                .arg("-y")
                .arg("-f")
                .arg("lavfi")
                .arg("-i")
                .arg(format!(
                    "color=black:s=1920x1080:d={},drawtext=text='{text}':fontsize=72:fontcolor=white:x=(w-text_w)/2:y=(h-text_h)/2",
                    frames as f64 / 24.0
                ))
                .arg("-i")
                .arg(&input)
                .arg("-filter_complex")
                .arg("[0:v][1:v]concat=n=2:v=1:a=0[out]")
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
            // Generate MCA labels and write into CPL
            let soundfield = match layout.as_str() {
                "51" => imfwizard_core::mca::soundfield_51(),
                "71" => imfwizard_core::mca::soundfield_71(),
                "stereo" | "20" => imfwizard_core::mca::soundfield_stereo(),
                "mono" | "10" => imfwizard_core::mca::McaSoundfield {
                    name: "10".to_string(),
                    channels: vec![imfwizard_core::mca::McaLabel {
                        symbol: imfwizard_core::mca::McaTagSymbol::M1,
                        tag_name: "Mono One".to_string(),
                        tag_symbol: "chM1".to_string(),
                        channel_index: 0,
                        spoken_language: String::new(),
                    }],
                },
                "51+hi+vi" | "51+HI+VI" => imfwizard_core::mca::soundfield_51_with_hi_vi(),
                _ => {
                    eprintln!("Unknown layout: {layout}. Use: mono, stereo, 51, 71, 51+HI+VI");
                    std::process::exit(1);
                }
            };

            // Set spoken language on all channels
            let mut soundfield = soundfield;
            for ch in &mut soundfield.channels {
                ch.spoken_language = language.clone();
            }

            let mca_xml = imfwizard_core::mca::generate_mca_xml(&soundfield);

            // Find CPL in IMP directory or use input as CPL path directly
            let input_path = std::path::Path::new(&input);
            let cpl_path = if input_path.is_dir() {
                // Search for CPL XML file in IMP
                let mut found = None;
                if let Ok(entries) = std::fs::read_dir(input_path) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_lowercase();
                        if name.starts_with("cpl") && name.ends_with(".xml") {
                            found = Some(entry.path());
                            break;
                        }
                    }
                }
                found.unwrap_or_else(|| {
                    eprintln!("No CPL XML found in {input}");
                    std::process::exit(1);
                })
            } else {
                input_path.to_path_buf()
            };

            // Read CPL and inject MCA labels
            let cpl_content = std::fs::read_to_string(&cpl_path).unwrap_or_else(|e| {
                eprintln!("Failed to read CPL: {e}");
                std::process::exit(1);
            });

            // Insert MCA labels before </MainSoundConfiguration> or before </CompositionPlaylist>
            let updated = if cpl_content.contains("</MainSoundConfiguration>") {
                cpl_content.replace(
                    "</MainSoundConfiguration>",
                    &format!("</MainSoundConfiguration>\n{mca_xml}"),
                )
            } else if cpl_content.contains("</CompositionPlaylist>") {
                cpl_content.replace(
                    "</CompositionPlaylist>",
                    &format!("{mca_xml}</CompositionPlaylist>"),
                )
            } else {
                eprintln!("Cannot find insertion point in CPL");
                std::process::exit(1);
            };

            std::fs::write(&cpl_path, &updated).unwrap_or_else(|e| {
                eprintln!("Failed to write CPL: {e}");
                std::process::exit(1);
            });

            println!("MCA labels written to {}", cpl_path.display());
            println!("  Layout: {} ({})", soundfield.name, layout);
            println!("  Language: {language}");
            println!("  Channels: {}", soundfield.channels.len());
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

        Commands::Atmos { input, output } => {
            // Import Dolby Atmos ADM BWF into IMF-compatible MXF
            let input_path = std::path::Path::new(&input);
            let output_path = std::path::Path::new(&output);

            let result = imfwizard_core::atmos::import_atmos(input_path, output_path);
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

        Commands::SdiPreview { input, device } => {
            let player = postkit::mpv::MpvPlayer::new("imfwizard");
            if let Err(e) = player.start_mpv() {
                eprintln!("Error starting mpv: {e}");
                std::process::exit(1);
            }
            // Configure Decklink SDI output
            let _ = player.send_command(&format!("set vo-decklink-device {device}"));
            if let Err(e) = player.load_package_dir(&input) {
                eprintln!("Error loading: {e}");
                std::process::exit(1);
            }
            println!("SDI preview on device {device}: {input}");
            while player.is_alive() {
                std::thread::sleep(std::time::Duration::from_millis(100));
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
            let platform = match standard.to_lowercase().as_str() {
                "netflix" => postkit::profiles::Platform::Netflix,
                "amazon" | "prime" => postkit::profiles::Platform::AmazonPrime,
                "disney" | "disney+" => postkit::profiles::Platform::Disney,
                "apple" | "appletv" => postkit::profiles::Platform::Apple,
                "hbo" => postkit::profiles::Platform::Hbo,
                "broadcast" => postkit::profiles::Platform::Broadcast,
                "archival" => postkit::profiles::Platform::ArchivalPreservation,
                "dci-2k" | "cinema-2k" => postkit::profiles::Platform::TheatricalDci2k,
                "dci-4k" | "cinema-4k" => postkit::profiles::Platform::TheatricalDci4k,
                _ => postkit::profiles::Platform::Netflix,
            };
            let profile = postkit::profiles::profile_for(platform);
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
            println!();

            // Structural validation
            let report = imfwizard_core::validate::validate_imp(std::path::Path::new(&input));
            let mut errors: Vec<String> = report.errors;
            let mut warnings: Vec<String> = report.warnings;

            // Probe MXF files for platform-specific parameter checks
            let imp_path = std::path::Path::new(&input);
            if imp_path.is_dir() {
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
                        }
                    }
                }
            }

            // Report
            if errors.is_empty() && warnings.is_empty() {
                println!("PASS: compliant with {}", profile.name);
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
                    println!("\nFAIL: not compliant with {}", profile.name);
                    std::process::exit(1);
                } else {
                    println!("\nPASS (with warnings): compliant with {}", profile.name);
                }
            }
        }

        Commands::EdlImport { input, json } => {
            // Alias for conform
            let timeline = match postkit::conform::parse_timeline(std::path::Path::new(&input)) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&timeline).unwrap());
            } else {
                println!("Timeline: {}", timeline.title);
                println!("Format: {:?}", timeline.format);
                println!("Frame rate: {}", timeline.frame_rate);
                println!("Events: {}", timeline.events.len());
                for (i, evt) in timeline.events.iter().enumerate() {
                    println!("  [{i}] {} -> {}", evt.source_in, evt.source_out);
                }
            }
        }

        Commands::Annotate { imp, text } => {
            // Alias for metadata-edit annotation
            let imp_dir = std::path::Path::new(&imp);
            let cpls = imfwizard_core::timeline::list_cpls(imp_dir);
            if cpls.is_empty() {
                eprintln!("Error: no CPLs found in {imp}");
                std::process::exit(1);
            }
            let cpl_path = imp_dir.join(&cpls[0].file_path);
            let annotation = imfwizard_core::cpl_annotation::CplAnnotation {
                author: String::from("imfwizard"),
                timestamp: String::new(),
                text,
                revision: String::from("1"),
            };
            match imfwizard_core::cpl_annotation::annotate_cpl(&cpl_path, &annotation) {
                Ok(()) => println!("Annotated: {}", cpl_path.display()),
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

        Commands::Kdm {
            cpl_id,
            content_title,
            cert,
            signer_cert,
            signer_key,
            signer_chain,
            output,
            valid_from,
            valid_to,
            device_cert,
            formulation,
            format,
            annotation,
            disable_forensic_marking_picture,
            disable_forensic_marking_audio,
        } => {
            use postkit::certificate::{AudioForensicMarking, PictureForensicMarking};

            let picture_forensic_marking = if disable_forensic_marking_picture {
                PictureForensicMarking::Disabled
            } else {
                PictureForensicMarking::Enabled
            };
            // dcpomatic's -a: absent leaves marking on, bare disables every
            // channel, and a number disables the channels above it
            let audio_forensic_marking = match disable_forensic_marking_audio {
                None => AudioForensicMarking::Enabled,
                Some(None) => AudioForensicMarking::Disabled,
                Some(Some(channel)) => AudioForensicMarking::DisabledAboveChannel(channel),
            };

            let config = postkit::certificate::KdmConfig {
                cpl_id,
                content_title,
                recipient_cert_file: cert,
                signer_cert_file: signer_cert,
                signer_key_file: signer_key,
                signer_chain_files: signer_chain,
                output_file: output.clone(),
                valid_from,
                valid_to,
                formulation,
                content_keys: Vec::new(),
                format: format.into(),
                annotation,
                // empty is the assume-trust thumbprint. postkit rejects a device
                // list that contradicts the formulation, so no check here
                device_cert_files: device_cert,
                picture_forensic_marking,
                audio_forensic_marking,
            };
            // postkit mints a fresh key when the caller supplies none, so this
            // goes quiet by itself once imfwizard can carry real content keys
            if config.content_keys.is_empty() {
                eprintln!(
                    "warning: no content keys were supplied, so this KDM carries a freshly \
                     minted key and will not unlock any existing essence"
                );
            }
            match postkit::certificate::generate_kdm(&config) {
                Ok(()) => println!("KDM written to {}", output.display()),
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
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
                    eprintln!("DV conversion failed: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
}

fn parse_colour_space(s: &str) -> postkit::colour::ColourSpace {
    match s.to_lowercase().as_str() {
        "p3" | "dci-p3" => postkit::colour::ColourSpace::P3,
        "xyz" => postkit::colour::ColourSpace::Xyz,
        "rec2020" | "2020" => postkit::colour::ColourSpace::Rec2020,
        "aces" | "ap0" => postkit::colour::ColourSpace::Aces,
        "acescg" | "ap1" => postkit::colour::ColourSpace::AcesCg,
        "logc" | "alexa" => postkit::colour::ColourSpace::LogC,
        _ => postkit::colour::ColourSpace::Rec709,
    }
}
