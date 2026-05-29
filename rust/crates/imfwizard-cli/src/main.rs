use clap::{Parser, Subcommand};
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

#[derive(Subcommand)]
enum Commands {
    /// Create a new IMP (Interoperable Master Package)
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

        /// Content kind (feature, trailer, etc.)
        #[arg(short, long, default_value = "feature")]
        kind: String,

        /// Frame rate numerator
        #[arg(long, default_value = "24")]
        fps_num: u32,

        /// Frame rate denominator
        #[arg(long, default_value = "1")]
        fps_den: u32,
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
    },

    /// Measure audio loudness (EBU R128)
    Loudness {
        /// Audio file to measure
        audio_file: String,
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

        /// Video directory (J2K codestreams)
        #[arg(short, long)]
        video: Option<String>,

        /// Entry point (frame offset)
        #[arg(long, default_value = "0")]
        entry_point: u64,

        /// Duration
        #[arg(long)]
        duration: Option<String>,
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
    Serve {
        /// Listen address (host:port)
        #[arg(short, long, default_value = "127.0.0.1:8081")]
        bind: String,
    },

    /// Start job queue daemon
    Daemon,

    /// Manage job queue
    Batch {
        #[command(subcommand)]
        action: BatchAction,
    },

    /// Generate shell completions
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
}

#[derive(Subcommand)]
enum BatchAction {
    /// List all jobs
    List,
    /// Submit a new job
    Add {
        /// Job type (create|encode|transcode|validate|loudness)
        #[arg(short = 'T', long)]
        r#type: String,
        /// Job parameters (JSON string)
        #[arg(short, long)]
        params: String,
    },
    /// Cancel a job
    Cancel {
        /// Job ID to cancel
        id: String,
    },
}

fn main() {
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
            kind,
            fps_num,
            fps_den,
        } => {
            let _ = std::fs::create_dir_all(&output);

            // If video is a file, run encode pipeline
            let (j2k_dir, audio_files) = if let Some(ref vid) = video {
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
                    use postkit::encode::{StreamEncodeOptions, find_compressor, stream_encode};
                    use std::sync::Arc;
                    use std::sync::atomic::AtomicBool;

                    let (compressor_path, lib_dir) = match find_compressor() {
                        Some(c) => c,
                        None => {
                            eprintln!(
                                "Error: grk_compress not found (required for video encoding)"
                            );
                            std::process::exit(1);
                        }
                    };

                    let j2k_out = output.join("j2k");
                    let _ = std::fs::create_dir_all(&j2k_out);

                    tracing::info!("Detected video file — encoding to J2K");
                    tracing::info!("Compressor: {}", compressor_path.display());

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
                    }

                    let opts = StreamEncodeOptions {
                        input: video_path.clone(),
                        output_dir: j2k_out.clone(),
                        compression_ratio: 10.0,
                        num_resolutions: 6,
                        codeblock_size: 32,
                        progression: "CPRL".to_string(),
                        fps: actual_fps,
                        compressor_path,
                        lib_dir,
                    };

                    let cancel = Arc::new(AtomicBool::new(false));
                    let pause = Arc::new(AtomicBool::new(false));
                    let cancel_clone = cancel.clone();
                    let _ = ctrlc::set_handler(move || {
                        cancel_clone.store(true, std::sync::atomic::Ordering::Relaxed);
                    });

                    let result = stream_encode(&opts, &cancel, &pause, |p| {
                        let percent = if p.total_frames > 0 {
                            (p.frame as f64 / p.total_frames as f64) * 100.0
                        } else {
                            0.0
                        };
                        eprint!(
                            "\r[encode] {}/{} frames ({:.0}%) {:.1} fps   ",
                            p.frame, p.total_frames, percent, p.fps
                        );
                    });
                    eprintln!();

                    if !result.success {
                        eprintln!("Error: Encode failed: {}", result.error);
                        std::process::exit(1);
                    }
                    tracing::info!("Encoded {} frames", result.frames_encoded);

                    // Auto-demux audio
                    let audio_files = if let Some(a) = audio {
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
                    (
                        Some(video_path),
                        audio.map(|a| vec![PathBuf::from(a)]).unwrap_or_default(),
                    )
                }
            } else {
                (
                    None,
                    audio.map(|a| vec![PathBuf::from(a)]).unwrap_or_default(),
                )
            };

            let opts = imfwizard_core::imp::ImpOptions {
                output_dir: output,
                title,
                content_kind: kind,
                fps_num,
                fps_den,
                j2k_dir,
                audio_files,
                ..Default::default()
            };
            let result = imfwizard_core::imp::create_imp(&opts);
            if result.success {
                println!("IMP created at {}", result.output_dir.display());
                println!("  CPL: {}", result.cpl_path.display());
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

        Commands::Analytics { input, json } => match imfwizard_core::analytics::analyze_imp(&input)
        {
            Ok(a) => {
                if json {
                    println!("{}", serde_json::to_string_pretty(&a).unwrap());
                } else {
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
        },

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

        Commands::Validate { dir } => {
            let result = imfwizard_core::validate::validate_imp(std::path::Path::new(&dir));
            if result.valid {
                println!("IMP validation PASSED");
                if !result.warnings.is_empty() {
                    for w in &result.warnings {
                        println!("  warning: {w}");
                    }
                }
            } else {
                eprintln!("IMP validation FAILED");
                for e in &result.errors {
                    eprintln!("  error: {e}");
                }
                for w in &result.warnings {
                    eprintln!("  warning: {w}");
                }
                std::process::exit(1);
            }
        }

        Commands::Loudness { audio_file } => {
            let result = postkit::loudness::measure_loudness(std::path::Path::new(&audio_file));
            if result.success {
                println!("Integrated: {:.1} LUFS", result.integrated_lufs);
                println!("True Peak: {:.1} dBTP", result.true_peak_dbtp);
                println!("Range: {:.1} LU", result.range_lu);
            } else {
                eprintln!("Error: {}", result.error);
                std::process::exit(1);
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
            video,
            entry_point,
            duration,
        } => {
            let opts = imfwizard_core::supplement::SupplementOptions {
                ov_dir: PathBuf::from(ov),
                title,
                output_dir: PathBuf::from(output),
                video: video.map(PathBuf::from),
                entry_point,
                duration,
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

        Commands::TargetConvert {
            input,
            target,
            output,
        } => {
            let imp_dir = PathBuf::from(&input);
            let output_dir = output
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(format!("{input}_delivery")));

            let spec = imfwizard_core::delivery::DeliverySpec {
                platform: target.clone(),
                video_codec: "h264".to_string(),
                audio_codec: "aac".to_string(),
                container: "mp4".to_string(),
                resolution: (1920, 1080),
                fps: 24.0,
                bitrate: "20M".to_string(),
                hdr: false,
                dolby_vision: false,
                atmos: false,
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

        Commands::Serve { bind } => {
            let parts: Vec<&str> = bind.split(':').collect();
            let config = imfwizard_core::rest_api::ApiConfig {
                host: parts.first().unwrap_or(&"127.0.0.1").to_string(),
                port: parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(8081),
            };
            if let Err(e) = imfwizard_core::rest_api::start_server(&config) {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }

        Commands::Daemon => {
            println!("Starting imfwizard job queue daemon...");
            let queue = imfwizard_core::job_queue::JobQueue::new();
            loop {
                if let Some(job) = queue.next_runnable() {
                    tracing::info!("Processing job {}: {:?}", job.id, job.job_type);
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }

        Commands::Batch { action } => match action {
            BatchAction::List => {
                let queue = imfwizard_core::job_queue::JobQueue::new();
                for job in queue.list() {
                    println!(
                        "[{}] {:?} — {:?} ({}%)",
                        job.id,
                        job.job_type,
                        job.state,
                        (job.progress * 100.0) as u32
                    );
                }
            }
            BatchAction::Add { r#type, params } => {
                let queue = imfwizard_core::job_queue::JobQueue::new();
                let job_type = match r#type.as_str() {
                    "encode" => imfwizard_core::job_queue::JobType::Encode,
                    "transcode" => imfwizard_core::job_queue::JobType::Transcode,
                    "validate" => imfwizard_core::job_queue::JobType::Validate,
                    "loudness" => imfwizard_core::job_queue::JobType::Loudness,
                    _ => imfwizard_core::job_queue::JobType::Create,
                };
                let id = queue.submit(imfwizard_core::job_queue::Job {
                    job_type,
                    description: params,
                    ..Default::default()
                });
                println!("Job submitted: {id}");
            }
            BatchAction::Cancel { id } => {
                let queue = imfwizard_core::job_queue::JobQueue::new();
                let job_id: u64 = id.parse().unwrap_or(0);
                queue.cancel(job_id);
                println!("Job {id} cancelled");
            }
        },

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
            issuer: _,
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
                author: "imfwizard".to_string(),
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
            let timeline = postkit::conform::parse_timeline(std::path::Path::new(&input));
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
