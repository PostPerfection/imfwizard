pub use postkit::prores::*;

use std::path::{Path, PathBuf};

use crate::timeline::{SegmentEntry, get_timeline_with_ov, list_cpls};

const STDERR_TAIL_LINES: usize = 10;
const PRORES_4444_PROFILE: &str = "4";
const PRORES_4444_PIXEL_FORMAT: &str = "yuv444p10le";
const MISSING_OV_HINT: &str = "; a supplemental IMP needs --ov naming the OV directory";

const CONTAINER_RASTERS: [(&str, ContainerRaster); 6] = [
    ("2k-scope", ContainerRaster::new(2048, 858)),
    ("2k-flat", ContainerRaster::new(1998, 1080)),
    ("2k-full", ContainerRaster::new(2048, 1080)),
    ("4k-scope", ContainerRaster::new(4096, 1716)),
    ("4k-flat", ContainerRaster::new(3996, 2160)),
    ("4k-full", ContainerRaster::new(4096, 2160)),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerRaster {
    pub width: u32,
    pub height: u32,
}

impl ContainerRaster {
    const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn parse(name: &str) -> Option<Self> {
        let wanted = name.to_lowercase();
        CONTAINER_RASTERS
            .iter()
            .find(|(spelling, _)| *spelling == wanted)
            .map(|(_, raster)| *raster)
    }

    pub fn names() -> String {
        CONTAINER_RASTERS
            .iter()
            .map(|(spelling, _)| *spelling)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub fn export_imp_to_prores(
    imp_dir: &Path,
    output: &Path,
    cpl_id: Option<&str>,
    container: Option<ContainerRaster>,
    ov_dir: Option<&Path>,
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

    let segments = get_timeline_with_ov(&imp_dir.join(&cpl.file_path), ov_dir);
    if segments.is_empty() {
        return Err(format!("CPL {} holds no segment", cpl.id));
    }

    let track_file = |name: &String| {
        let path = PathBuf::from(name);
        (!name.is_empty() && path.exists()).then_some(path)
    };
    let hint = if ov_dir.is_some() {
        ""
    } else {
        MISSING_OV_HINT
    };
    let mut pictures = Vec::new();
    for segment in &segments {
        let picture = track_file(&segment.video_file)
            .ok_or_else(|| format!("CPL {} names no picture track file{hint}", cpl.id))?;
        pictures.push(picture);
    }
    let sounds: Vec<Option<PathBuf>> = segments
        .iter()
        .map(|segment| track_file(&segment.audio_file))
        .collect();
    // a segment with no sound would leave the concat filter short an input
    let has_sound = sounds.iter().all(Option::is_some);
    let edit_rate = edit_rate_pair(&segments[0].edit_rate);
    if has_sound && edit_rate.is_none() {
        return Err(format!(
            "CPL {} names no edit rate, so its sound cannot be cut to its segments",
            cpl.id
        ));
    }

    let mut command = std::process::Command::new("ffmpeg");
    command.args(["-hide_banner", "-nostats", "-nostdin", "-y"]);
    for (index, picture) in pictures.iter().enumerate() {
        command.arg("-i").arg(picture);
        if has_sound {
            command
                .arg("-i")
                .arg(sounds[index].as_ref().expect("every segment has sound"));
        }
    }

    let inputs_a_segment = if has_sound { 2 } else { 1 };
    let mut graph = String::new();
    let mut concat_inputs = String::new();
    for (index, segment) in segments.iter().enumerate() {
        let video_input = index * inputs_a_segment;
        graph.push_str(&format!(
            "[{video_input}:v:0]{},setpts=PTS-STARTPTS[v{index}];",
            frame_trim(segment)
        ));
        concat_inputs.push_str(&format!("[v{index}]"));
        if has_sound {
            let audio_input = video_input + 1;
            graph.push_str(&format!(
                "[{audio_input}:a:0]{},asetpts=PTS-STARTPTS[a{index}];",
                second_trim(
                    segment,
                    edit_rate.expect("sound needs an edit rate, checked above")
                )
            ));
            concat_inputs.push_str(&format!("[a{index}]"));
        }
    }
    graph.push_str(&concat_inputs);
    graph.push_str(&format!(
        "concat=n={}:v=1:a={}[picture]",
        segments.len(),
        u8::from(has_sound)
    ));
    if has_sound {
        graph.push_str("[sound]");
    }
    let picture_out = match container {
        Some(ContainerRaster { width, height }) => {
            graph.push_str(&format!(
                ";[picture]scale={width}:{height}:force_original_aspect_ratio=decrease,\
                 pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:black[fitted]"
            ));
            "[fitted]"
        }
        None => "[picture]",
    };

    command.args(["-filter_complex", &graph]);
    command.args(["-map", picture_out]);
    if has_sound {
        command.args(["-map", "[sound]"]);
    }
    command.args(["-c:v", "prores_ks", "-profile:v", PRORES_4444_PROFILE]);
    command.args(["-pix_fmt", PRORES_4444_PIXEL_FORMAT]);
    if let Some(frame_rate) = ffmpeg_frame_rate(&segments[0].edit_rate) {
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
fn edit_rate_pair(edit_rate: &str) -> Option<(u32, u32)> {
    let mut parts = edit_rate.split_whitespace();
    let numerator: u32 = parts.next()?.parse().ok()?;
    let denominator: u32 = parts.next()?.parse().ok()?;
    (numerator != 0 && denominator != 0).then_some((numerator, denominator))
}

fn ffmpeg_frame_rate(edit_rate: &str) -> Option<String> {
    let (numerator, denominator) = edit_rate_pair(edit_rate)?;
    Some(format!("{numerator}/{denominator}"))
}

// a resource plays SourceDuration frames from EntryPoint, and one that names no duration runs to the end
fn frame_trim(segment: &SegmentEntry) -> String {
    let start = segment.entry_point;
    match segment.duration_frames {
        0 => format!("trim=start_frame={start}"),
        frames => format!("trim=start_frame={start}:end_frame={}", start + frames),
    }
}

const TRIM_SECOND_DECIMALS: usize = 6;

fn second_trim(segment: &SegmentEntry, edit_rate: (u32, u32)) -> String {
    let (numerator, denominator) = edit_rate;
    let seconds = |frames: u64| frames as f64 * denominator as f64 / numerator as f64;
    let start = seconds(segment.entry_point);
    let decimals = TRIM_SECOND_DECIMALS;
    match segment.duration_frames {
        0 => format!("atrim=start={start:.decimals$}"),
        frames => {
            let end = seconds(segment.entry_point + frames);
            format!("atrim=start={start:.decimals$}:end={end:.decimals$}")
        }
    }
}

#[cfg(test)]
mod imp_export_tests {
    use super::*;

    #[test]
    fn container_sizes() {
        let scope = ContainerRaster::parse("2k-scope").unwrap();
        assert_eq!((scope.width, scope.height), (2048, 858));
        let flat = ContainerRaster::parse("4K-FLAT").unwrap();
        assert_eq!((flat.width, flat.height), (3996, 2160));
        assert!(ContainerRaster::parse("2k").is_none());
        assert!(ContainerRaster::parse("8k-full").is_none());
    }

    #[test]
    fn every_container_name_is_offered() {
        assert_eq!(
            ContainerRaster::names(),
            "2k-scope, 2k-flat, 2k-full, 4k-scope, 4k-flat, 4k-full"
        );
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
        let error = export_imp_to_prores(dir.path(), &dir.path().join("out.mov"), None, None, None)
            .unwrap_err();
        assert!(error.contains("holds no CPL"), "{error}");
    }
}
