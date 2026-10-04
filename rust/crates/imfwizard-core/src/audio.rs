use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Audio asset description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioAsset {
    pub path: PathBuf,
    pub channels: u32,
    pub sample_rate: u32,
    pub bit_depth: u32,
    pub language: String,
    pub layout: super::channel_map::ChannelLayout,
}

pub fn demux_source_audio(
    video: &std::path::Path,
    wav_out: &std::path::Path,
) -> std::io::Result<std::process::Output> {
    std::process::Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(video)
        .arg("-vn")
        .arg("-acodec")
        .arg("pcm_s24le")
        .arg("-ar")
        .arg("48000")
        .args(["-rf64", "auto"])
        .arg(wav_out)
        .output()
}

/// Probe audio file metadata via ffprobe.
pub fn probe_audio(path: &std::path::Path) -> Result<AudioAsset, String> {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "quiet", "-print_format", "json", "-show_streams"])
        .arg(path)
        .output()
        .map_err(|e| format!("Failed to run ffprobe: {e}"))?;

    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }

    let json: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("Failed to parse ffprobe JSON: {e}"))?;

    let stream = json["streams"]
        .as_array()
        .and_then(|s| s.iter().find(|s| s["codec_type"] == "audio"))
        .ok_or("No audio stream found")?;

    Ok(AudioAsset {
        path: path.to_path_buf(),
        channels: stream["channels"].as_u64().unwrap_or(0) as u32,
        sample_rate: stream["sample_rate"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        bit_depth: stream["bits_per_raw_sample"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        language: String::new(),
        layout: super::channel_map::ChannelLayout::Custom,
    })
}
