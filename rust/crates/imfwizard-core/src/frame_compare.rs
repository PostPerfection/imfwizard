use serde::{Deserialize, Serialize};
use std::path::Path;

/// Per-frame quality metric.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameMetric {
    pub frame: u64,
    pub psnr_y: f64,
    pub psnr_u: f64,
    pub psnr_v: f64,
    pub psnr_avg: f64,
    pub ssim_y: f64,
    pub ssim_avg: f64,
}

/// Aggregate comparison result.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompareResult {
    pub frames_compared: u64,
    pub avg_psnr: f64,
    pub min_psnr: f64,
    pub max_psnr: f64,
    pub avg_ssim: f64,
    pub min_ssim: f64,
    pub max_ssim: f64,
    pub per_frame: Vec<FrameMetric>,
}

/// Compare two video files frame-by-frame using ffmpeg PSNR and SSIM filters.
pub fn compare_frames(reference: &Path, distorted: &Path) -> Result<CompareResult, String> {
    let psnr_log = std::env::temp_dir().join("imfwizard_psnr.log");
    let ssim_log = std::env::temp_dir().join("imfwizard_ssim.log");

    // Run ffmpeg with both PSNR and SSIM filters simultaneously
    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-i"])
        .arg(reference)
        .args(["-i"])
        .arg(distorted)
        .args([
            "-lavfi",
            &format!(
                "[0:v][1:v]psnr=stats_file={}[psnr];[0:v][1:v]ssim=stats_file={}",
                psnr_log.display(),
                ssim_log.display()
            ),
            "-f",
            "null",
            "-",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .status()
        .map_err(|e| format!("Failed to run ffmpeg: {e}"))?;

    if !status.success() {
        return Err("ffmpeg comparison failed".to_string());
    }

    // Parse PSNR log
    let psnr_data =
        std::fs::read_to_string(&psnr_log).map_err(|e| format!("Failed to read PSNR log: {e}"))?;
    let _ = std::fs::remove_file(&psnr_log);

    // Parse SSIM log
    let ssim_data =
        std::fs::read_to_string(&ssim_log).map_err(|e| format!("Failed to read SSIM log: {e}"))?;
    let _ = std::fs::remove_file(&ssim_log);

    let psnr_frames = parse_psnr_log(&psnr_data);
    let ssim_frames = parse_ssim_log(&ssim_data);

    let frame_count = psnr_frames.len().min(ssim_frames.len());
    if frame_count == 0 {
        return Err("No frames compared".to_string());
    }

    let mut result = CompareResult {
        frames_compared: frame_count as u64,
        min_psnr: f64::INFINITY,
        max_psnr: f64::NEG_INFINITY,
        min_ssim: f64::INFINITY,
        max_ssim: f64::NEG_INFINITY,
        ..Default::default()
    };

    let mut psnr_sum = 0.0;
    let mut ssim_sum = 0.0;

    for i in 0..frame_count {
        let (psnr_y, psnr_u, psnr_v, psnr_avg) = psnr_frames[i];
        let (ssim_y, ssim_avg) = ssim_frames[i];

        let metric = FrameMetric {
            frame: i as u64,
            psnr_y,
            psnr_u,
            psnr_v,
            psnr_avg,
            ssim_y,
            ssim_avg,
        };

        if psnr_avg < result.min_psnr {
            result.min_psnr = psnr_avg;
        }
        if psnr_avg > result.max_psnr {
            result.max_psnr = psnr_avg;
        }
        if ssim_avg < result.min_ssim {
            result.min_ssim = ssim_avg;
        }
        if ssim_avg > result.max_ssim {
            result.max_ssim = ssim_avg;
        }

        psnr_sum += psnr_avg;
        ssim_sum += ssim_avg;
        result.per_frame.push(metric);
    }

    result.avg_psnr = psnr_sum / frame_count as f64;
    result.avg_ssim = ssim_sum / frame_count as f64;

    Ok(result)
}

/// Parse ffmpeg PSNR stats file.
/// Format: n:1 mse_avg:0.00 mse_y:0.00 mse_u:0.00 mse_v:0.00 psnr_avg:inf psnr_y:inf psnr_u:inf psnr_v:inf
fn parse_psnr_log(data: &str) -> Vec<(f64, f64, f64, f64)> {
    data.lines()
        .filter_map(|line| {
            let get_val = |key: &str| -> Option<f64> {
                line.split_whitespace()
                    .find(|s| s.starts_with(key))
                    .and_then(|s| s.split(':').nth(1))
                    .and_then(|v| {
                        if v == "inf" {
                            Some(100.0)
                        } else {
                            v.parse().ok()
                        }
                    })
            };
            let psnr_y = get_val("psnr_y")?;
            let psnr_u = get_val("psnr_u")?;
            let psnr_v = get_val("psnr_v")?;
            let psnr_avg = get_val("psnr_avg")?;
            Some((psnr_y, psnr_u, psnr_v, psnr_avg))
        })
        .collect()
}

/// Parse ffmpeg SSIM stats file.
/// Format: n:1 Y:1.000000 (inf) U:1.000000 (inf) V:1.000000 (inf) All:1.000000 (inf)
fn parse_ssim_log(data: &str) -> Vec<(f64, f64)> {
    data.lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            let ssim_y = parts
                .iter()
                .find(|s| s.starts_with("Y:"))
                .and_then(|s| s.strip_prefix("Y:"))
                .and_then(|v| v.parse::<f64>().ok())?;
            let ssim_all = parts
                .iter()
                .find(|s| s.starts_with("All:"))
                .and_then(|s| s.strip_prefix("All:"))
                .and_then(|v| v.parse::<f64>().ok())?;
            Some((ssim_y, ssim_all))
        })
        .collect()
}
