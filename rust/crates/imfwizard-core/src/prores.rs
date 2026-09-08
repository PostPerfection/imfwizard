pub use postkit::prores::*;

use std::path::{Path, PathBuf};

use crate::timeline::{get_timeline, list_cpls};

const STDERR_TAIL_LINES: usize = 10;
const PRORES_4444_PROFILE: &str = "4";
const PRORES_4444_PIXEL_FORMAT: &str = "yuv444p10le";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerRaster {
    TwoK,
    FourK,
}

impl ContainerRaster {
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "2k" => Some(Self::TwoK),
            "4k" => Some(Self::FourK),
            _ => None,
        }
    }

    pub fn size(self) -> (u32, u32) {
        match self {
            Self::TwoK => (2048, 1080),
            Self::FourK => (4096, 2160),
        }
    }
}

pub fn export_imp_to_prores(
    imp_dir: &Path,
    output: &Path,
    cpl_id: Option<&str>,
    container: Option<ContainerRaster>,
) -> Result<(), String> {
    let cpls = list_cpls(imp_dir);
    if cpls.is_empty() {
        return Err(format!("{} holds no CPL", imp_dir.display()));
    }
    let cpl = match cpl_id {
        Some(wanted) => cpls
            .iter()
            .find(|cpl| cpl.id.eq_ignore_ascii_case(wanted))
            .ok_or_else(|| format!("{} holds no CPL {wanted}", imp_dir.display()))?,
        None => &cpls[0],
    };

    let segments = get_timeline(&imp_dir.join(&cpl.file_path));
    let segment = segments
        .first()
        .ok_or_else(|| format!("CPL {} holds no segment", cpl.id))?;
    let picture = PathBuf::from(&segment.video_file);
    if segment.video_file.is_empty() || !picture.exists() {
        return Err(format!("CPL {} names no picture track file", cpl.id));
    }
    let sound = PathBuf::from(&segment.audio_file);
    let has_sound = !segment.audio_file.is_empty() && sound.exists();

    let mut command = std::process::Command::new("ffmpeg");
    command
        .args(["-hide_banner", "-nostats", "-nostdin", "-y", "-i"])
        .arg(&picture);
    if has_sound {
        command.arg("-i").arg(&sound);
        command.args(["-map", "0:v:0", "-map", "1:a:0"]);
    }
    if let Some(container) = container {
        let (width, height) = container.size();
        command.args([
            "-vf",
            &format!(
                "scale={width}:{height}:force_original_aspect_ratio=decrease,\
                 pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:black"
            ),
        ]);
    }
    command.args(["-c:v", "prores_ks", "-profile:v", PRORES_4444_PROFILE]);
    command.args(["-pix_fmt", PRORES_4444_PIXEL_FORMAT]);
    if let Some(frame_rate) = ffmpeg_frame_rate(&segment.edit_rate) {
        command.args(["-r", &frame_rate]);
    }
    if has_sound {
        command.args(["-c:a", "pcm_s24le"]);
    }
    command.arg(output);

    let run = command
        .output()
        .map_err(|e| format!("Failed to run ffmpeg: {e}"))?;
    if !run.status.success() {
        let stderr = String::from_utf8_lossy(&run.stderr);
        let lines: Vec<&str> = stderr.lines().collect();
        let tail = lines[lines.len().saturating_sub(STDERR_TAIL_LINES)..].join("\n");
        return Err(format!("ffmpeg ProRes export failed:\n{tail}"));
    }
    Ok(())
}

// a cpl edit rate is a space separated pair, "24 1"
fn ffmpeg_frame_rate(edit_rate: &str) -> Option<String> {
    let mut parts = edit_rate.split_whitespace();
    let numerator: u32 = parts.next()?.parse().ok()?;
    let denominator: u32 = parts.next()?.parse().ok()?;
    if numerator == 0 || denominator == 0 {
        return None;
    }
    Some(format!("{numerator}/{denominator}"))
}

#[cfg(test)]
mod imp_export_tests {
    use super::*;

    #[test]
    fn container_sizes() {
        assert_eq!(ContainerRaster::parse("2k").unwrap().size(), (2048, 1080));
        assert_eq!(ContainerRaster::parse("4K").unwrap().size(), (4096, 2160));
        assert!(ContainerRaster::parse("8k").is_none());
    }

    #[test]
    fn edit_rate_becomes_a_fraction() {
        assert_eq!(ffmpeg_frame_rate("24 1").as_deref(), Some("24/1"));
        assert_eq!(
            ffmpeg_frame_rate("24000 1001").as_deref(),
            Some("24000/1001")
        );
        assert_eq!(ffmpeg_frame_rate("24"), None);
        assert_eq!(ffmpeg_frame_rate("24 0"), None);
    }

    #[test]
    fn a_directory_without_a_cpl_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let error =
            export_imp_to_prores(dir.path(), &dir.path().join("out.mov"), None, None).unwrap_err();
        assert!(error.contains("holds no CPL"), "{error}");
    }
}
