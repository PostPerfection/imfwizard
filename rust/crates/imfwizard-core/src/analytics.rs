use serde::{Deserialize, Serialize};
use std::path::Path;

/// IMP analytics summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImpAnalytics {
    pub total_assets: usize,
    pub video_tracks: usize,
    pub audio_tracks: usize,
    pub subtitle_tracks: usize,
    pub total_duration_frames: u64,
    pub total_size_bytes: u64,
    pub video_bitrate_mbps: f64,
}

/// Per-second bitrate sample.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BitrateSample {
    pub second: f64,
    pub bitrate_kbps: f64,
}

/// Histogram bucket for bitrate distribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramBucket {
    pub range_min_kbps: f64,
    pub range_max_kbps: f64,
    pub count: usize,
}

/// Full bitrate analytics result.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BitrateAnalytics {
    pub samples: Vec<BitrateSample>,
    pub histogram: Vec<HistogramBucket>,
    pub min_kbps: f64,
    pub max_kbps: f64,
    pub avg_kbps: f64,
    pub stddev_kbps: f64,
    pub duration_seconds: f64,
    pub total_frames: u64,
}

/// Analyze an IMP directory and return summary statistics.
pub fn analyze_imp(imp_dir: &Path) -> Result<ImpAnalytics, String> {
    let mut analytics = ImpAnalytics::default();

    // Count files by type
    let entries =
        std::fs::read_dir(imp_dir).map_err(|e| format!("Failed to read IMP directory: {e}"))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read entry: {e}"))?;
        let path = entry.path();
        if path.is_file() {
            analytics.total_assets += 1;
            analytics.total_size_bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);

            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                match ext.to_lowercase().as_str() {
                    "mxf" => {
                        // Heuristic: first MXF is video, rest are audio
                        if analytics.video_tracks == 0 {
                            analytics.video_tracks += 1;
                        } else {
                            analytics.audio_tracks += 1;
                        }
                    }
                    "ttml" | "xml" => analytics.subtitle_tracks += 1,
                    _ => {}
                }
            }
        }
    }

    Ok(analytics)
}

/// Analyze per-second bitrate of a video MXF file using ffprobe.
///
/// Returns per-second samples, histogram, and statistics.
pub fn analyze_bitrate(mxf_path: &Path, num_buckets: usize) -> Result<BitrateAnalytics, String> {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-show_packets",
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=pts_time,size",
            "-of",
            "csv=p=0",
        ])
        .arg(mxf_path)
        .output()
        .map_err(|e| format!("Failed to run ffprobe: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "ffprobe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse packet sizes grouped by second
    let mut second_bytes: std::collections::BTreeMap<u64, u64> = std::collections::BTreeMap::new();
    let mut total_frames: u64 = 0;
    let mut max_pts: f64 = 0.0;

    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() >= 2
            && let (Ok(pts), Ok(size)) = (parts[0].parse::<f64>(), parts[1].parse::<u64>())
        {
            let sec = pts.floor() as u64;
            *second_bytes.entry(sec).or_insert(0) += size;
            total_frames += 1;
            if pts > max_pts {
                max_pts = pts;
            }
        }
    }

    if second_bytes.is_empty() {
        return Err("No video packets found in file".to_string());
    }

    // Build per-second samples (bytes → kbps: bytes * 8 / 1000)
    let samples: Vec<BitrateSample> = second_bytes
        .iter()
        .map(|(&sec, &bytes)| BitrateSample {
            second: sec as f64,
            bitrate_kbps: (bytes as f64 * 8.0) / 1000.0,
        })
        .collect();

    // Statistics
    let bitrates: Vec<f64> = samples.iter().map(|s| s.bitrate_kbps).collect();
    let min_kbps = bitrates.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_kbps = bitrates.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let avg_kbps = bitrates.iter().sum::<f64>() / bitrates.len() as f64;
    let variance =
        bitrates.iter().map(|b| (b - avg_kbps).powi(2)).sum::<f64>() / bitrates.len() as f64;
    let stddev_kbps = variance.sqrt();

    // Build histogram
    let bucket_count = num_buckets.max(1);
    let range = max_kbps - min_kbps;
    let bucket_width = if range > 0.0 {
        range / bucket_count as f64
    } else {
        1.0
    };

    let mut histogram: Vec<HistogramBucket> = (0..bucket_count)
        .map(|i| HistogramBucket {
            range_min_kbps: min_kbps + (i as f64 * bucket_width),
            range_max_kbps: min_kbps + ((i + 1) as f64 * bucket_width),
            count: 0,
        })
        .collect();

    for &br in &bitrates {
        let idx = ((br - min_kbps) / bucket_width).floor() as usize;
        let idx = idx.min(bucket_count - 1);
        histogram[idx].count += 1;
    }

    Ok(BitrateAnalytics {
        samples,
        histogram,
        min_kbps,
        max_kbps,
        avg_kbps,
        stddev_kbps,
        duration_seconds: max_pts,
        total_frames,
    })
}
