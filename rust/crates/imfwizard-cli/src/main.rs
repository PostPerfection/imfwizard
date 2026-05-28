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
    Analyze {
        /// IMP directory to analyze
        #[arg(short, long)]
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

        Commands::Analyze { input, json } => match imfwizard_core::analytics::analyze_imp(&input) {
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
    }
}
