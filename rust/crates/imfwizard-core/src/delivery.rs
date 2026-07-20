use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Delivery profile for a specific platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliverySpec {
    pub platform: String,
    pub video_codec: String,
    pub audio_codec: String,
    pub container: String,
    pub resolution: (u32, u32),
    pub fps: f64,
    pub bitrate: String,
    pub hdr: bool,
    pub dolby_vision: bool,
}

/// Resolve a target-convert preset to a delivery spec, or error on an unknown target.
///
/// Presets scale/crop to a standard cinema container and rewrap to a ProRes .mov master;
/// fps 0 keeps the source rate. Errors rather than silently falling back to 1080p.
pub fn spec_for_target(target: &str) -> Result<DeliverySpec, String> {
    let resolution = match target {
        "2k-scope" => (2048, 858),
        "2k-flat" => (1998, 1080),
        "2k-full" => (2048, 1080),
        "4k-scope" => (4096, 1716),
        "4k-flat" => (3996, 2160),
        "4k-full" => (4096, 2160),
        other => {
            return Err(format!(
                "unknown target '{other}' (supported: 2k-scope, 2k-flat, 2k-full, 4k-scope, 4k-flat, 4k-full)"
            ));
        }
    };
    Ok(DeliverySpec {
        platform: target.to_string(),
        video_codec: "prores".to_string(),
        audio_codec: "pcm_s24le".to_string(),
        container: "mov".to_string(),
        resolution,
        fps: 0.0,
        bitrate: String::new(),
        hdr: false,
        dolby_vision: false,
    })
}

/// Deliver an IMP to a target specification.
pub fn deliver(
    imp_dir: &std::path::Path,
    output_dir: &std::path::Path,
    spec: &DeliverySpec,
) -> Result<PathBuf, String> {
    tracing::info!(
        "Delivering IMP {} to {} format",
        imp_dir.display(),
        spec.platform
    );

    // Create output directory
    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create output directory: {e}"))?;

    // 1. Find essence files in the IMP
    let essence_files: Vec<std::path::PathBuf> = std::fs::read_dir(imp_dir)
        .map_err(|e| format!("Failed to read IMP directory: {e}"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e, "mxf" | "MXF"))
        })
        .collect();

    if essence_files.is_empty() {
        return Err("No MXF essence files found in IMP".into());
    }

    // 2. Build ffmpeg transcode command
    let output_file = output_dir.join(format!("delivery.{}", spec.container));

    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.arg("-y");

    for f in &essence_files {
        cmd.arg("-i").arg(f);
    }

    // Video codec
    match spec.video_codec.as_str() {
        "h264" | "libx264" => {
            cmd.arg("-c:v").arg("libx264");
            if !spec.bitrate.is_empty() {
                cmd.arg("-b:v").arg(&spec.bitrate);
            }
        }
        "h265" | "hevc" | "libx265" => {
            cmd.arg("-c:v").arg("libx265");
            if !spec.bitrate.is_empty() {
                cmd.arg("-b:v").arg(&spec.bitrate);
            }
        }
        "prores" => {
            cmd.arg("-c:v")
                .arg("prores_ks")
                .arg("-profile:v")
                .arg("4444");
        }
        codec => {
            cmd.arg("-c:v").arg(codec);
        }
    }

    // Audio codec
    match spec.audio_codec.as_str() {
        "aac" => {
            cmd.arg("-c:a").arg("aac").arg("-b:a").arg("256k");
        }
        "pcm" | "pcm_s24le" => {
            cmd.arg("-c:a").arg("pcm_s24le");
        }
        "eac3" => {
            cmd.arg("-c:a").arg("eac3");
        }
        codec => {
            cmd.arg("-c:a").arg(codec);
        }
    }

    // Resolution
    if spec.resolution.0 > 0 && spec.resolution.1 > 0 {
        cmd.arg("-s")
            .arg(format!("{}x{}", spec.resolution.0, spec.resolution.1));
    }

    // Frame rate
    if spec.fps > 0.0 {
        cmd.arg("-r").arg(spec.fps.to_string());
    }

    // HDR metadata
    if spec.hdr {
        cmd.arg("-colorspace")
            .arg("bt2020nc")
            .arg("-color_primaries")
            .arg("bt2020")
            .arg("-color_trc")
            .arg("smpte2084");
    }

    cmd.arg(&output_file);

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run ffmpeg: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "Transcode failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    // Dolby Vision: inject RPU if requested
    if spec.dolby_vision {
        tracing::info!("Injecting Dolby Vision metadata via dovi_tool");
        let _ = std::process::Command::new("dovi_tool")
            .arg("inject-rpu")
            .arg("-i")
            .arg(&output_file)
            .arg("-o")
            .arg(output_dir.join(format!("delivery_dv.{}", spec.container)))
            .output();
    }

    tracing::info!("Delivery complete: {}", output_file.display());
    Ok(output_file)
}
