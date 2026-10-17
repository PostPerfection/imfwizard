use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use postkit::conform::Timeline;

// EDL convention: a black or aux event names no source media
const REELS_WITH_NO_MEDIA: [&str; 2] = ["BL", "AX"];

// the plan and what each event became, written beside the IMP
pub const CONFORM_MANIFEST_NAME: &str = "conform_manifest.json";

// the file create --timeline reads, named as dcpwizard's conform names it
pub const CONFORM_TIMELINE_FILE: &str = "timeline.json";

/// One timeline event with its source media found.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConformEvent {
    pub event_number: u32,
    pub reel_name: String,
    pub media: PathBuf,
    pub source_in: u64,
    pub source_out: u64,
    /// The picture track file this event became, once the IMP is built.
    #[serde(default)]
    pub picture_track_file: Option<String>,
    /// The sound track file that plays this event's sound, once the IMP is built.
    #[serde(default)]
    pub sound_track_file: Option<String>,
}

impl ConformEvent {
    pub fn frames(&self) -> u64 {
        self.source_out.saturating_sub(self.source_in)
    }
}

/// A timeline resolved against a media directory: what to encode, in the order
/// the composition plays it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConformPlan {
    pub title: String,
    pub fps_num: u32,
    pub fps_den: u32,
    pub events: Vec<ConformEvent>,
}

// dcpwizard resolves a reel the same way, so a timeline that conforms there
// resolves to the same media here
fn comparable_file_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect()
}

fn resolve_media(reel_name: &str, media_dir: &Path) -> Option<PathBuf> {
    let wanted = comparable_file_name(reel_name);
    let mut found: Vec<PathBuf> = std::fs::read_dir(media_dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| comparable_file_name(name).contains(&wanted))
        })
        .collect();
    found.sort();
    found.into_iter().next()
}

/// The edit rate a timeline's frame rate names, as an exact fraction.
fn edit_rate(fps: f64) -> Result<(u32, u32), String> {
    let whole = fps.round();
    if (fps - whole).abs() < 1e-6 && whole >= 1.0 {
        return Ok((whole as u32, 1));
    }
    // 23.976, 29.97 and 59.94 are the 1000/1001 pull-downs of a whole rate
    let pulled = (fps * 1001.0 / 1000.0).round();
    if (fps - pulled * 1000.0 / 1001.0).abs() < 1e-3 && pulled >= 1.0 {
        return Ok((pulled as u32 * 1000, 1001));
    }
    Err(format!("no exact edit rate for {fps} frames per second"))
}

/// Resolve every picture event against `media_dir`, in record order.
pub fn build_plan(timeline: &Timeline, media_dir: &Path) -> Result<ConformPlan, String> {
    if !media_dir.is_dir() {
        return Err(format!(
            "media directory not found: {}",
            media_dir.display()
        ));
    }
    let (fps_num, fps_den) = edit_rate(timeline.frame_rate)?;

    let mut picture_events: Vec<&postkit::conform::EditEvent> = timeline
        .events
        .iter()
        .filter(|event| event.track_type.starts_with('V'))
        .filter(|event| !REELS_WITH_NO_MEDIA.contains(&event.reel_name.as_str()))
        .collect();
    picture_events.sort_by_key(|event| event.record_in);
    if picture_events.is_empty() {
        return Err("the timeline holds no picture events to conform".into());
    }

    let mut events = Vec::new();
    let mut missing = Vec::new();
    for event in picture_events {
        match resolve_media(&event.reel_name, media_dir) {
            Some(media) => events.push(ConformEvent {
                event_number: event.event_number,
                reel_name: event.reel_name.clone(),
                media,
                source_in: event.source_in as u64,
                source_out: event.source_out as u64,
                picture_track_file: None,
                sound_track_file: None,
            }),
            None => missing.push(event.reel_name.clone()),
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "no media under {} for reel {}",
            media_dir.display(),
            missing.join(", ")
        ));
    }
    if let Some(empty) = events.iter().find(|event| event.frames() == 0) {
        return Err(format!(
            "event {} on reel {} keeps no frames: source in {} is not before source out {}",
            empty.event_number, empty.reel_name, empty.source_in, empty.source_out
        ));
    }

    Ok(ConformPlan {
        title: timeline.title.clone(),
        fps_num,
        fps_den,
        events,
    })
}

pub fn write_conform_manifest(plan: &ConformPlan, output_dir: &Path) -> std::io::Result<PathBuf> {
    let path = output_dir.join(CONFORM_MANIFEST_NAME);
    let json = serde_json::to_string_pretty(plan).map_err(std::io::Error::other)?;
    std::fs::write(&path, json)?;
    Ok(path)
}

/// The timeline the plan plays: a clip an event, with the media's own sound when
/// it carries any, and one segment.
pub fn timeline_from_plan(plan: &ConformPlan) -> Result<postkit::timeline::Timeline, String> {
    let clips = plan
        .events
        .iter()
        .map(|event| {
            let info = postkit::probe::probe_video(&event.media).ok_or_else(|| {
                format!("ffprobe read no picture out of {}", event.media.display())
            })?;
            Ok(postkit::timeline::Clip {
                picture: event.media.clone(),
                in_frame: event.source_in,
                out_frame: event.source_out,
                sound: if info.has_audio {
                    postkit::timeline::ClipSound::Embedded
                } else {
                    postkit::timeline::ClipSound::None
                },
                subtitle: None,
                source_colour: Default::default(),
                picture_processing: Default::default(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(postkit::timeline::Timeline {
        version: postkit::timeline::TIMELINE_FORMAT_VERSION,
        fps: crate::encode::FrameRate::new(plan.fps_num, plan.fps_den),
        clips,
        sound: postkit::timeline::SoundTrack::PerClip,
        subtitle: None,
        reel_starts: Vec::new(),
    })
}

/// What a finished conform wrote.
#[derive(Debug, Clone)]
pub struct ConformResult {
    pub imp_dir: PathBuf,
    pub cpl_path: PathBuf,
    pub manifest_path: PathBuf,
    pub picture_frames: u64,
}

/// Build an IMP whose one composition follows the plan, through the timeline
/// the plan becomes, which is written beside the package for `create --timeline`.
pub fn conform_to_imp(
    plan: &ConformPlan,
    output_dir: &Path,
    encode_threads: u32,
) -> Result<ConformResult, String> {
    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("cannot create {}: {e}", output_dir.display()))?;
    let timeline = timeline_from_plan(plan)?;
    timeline.write(&output_dir.join(CONFORM_TIMELINE_FILE))?;
    let build = crate::timeline_imp::TimelineImp {
        timeline,
        output_dir: output_dir.to_path_buf(),
        title: plan.title.clone(),
        content_kind: "feature".to_string(),
        picture_options: Default::default(),
        source_colour: crate::source_colourspace::to_source_colour(
            crate::source_colourspace::APP2E_SOURCE_SPACE,
        )?,
        compression: crate::timeline_imp::PictureCompression {
            encode_threads,
            ..Default::default()
        },
        hdr: None,
        audio_language: None,
        audio_role: None,
        timed_text_language: None,
        soundfield: Default::default(),
        cancel: Default::default(),
    };
    let built = crate::timeline_imp::build_timeline_imp(&build, |_| {})?;

    let mut manifest = plan.clone();
    let sound_track_file = built
        .sound_track_file
        .as_ref()
        .and_then(|track| file_name(&track.path));
    for (event, picture) in manifest.events.iter_mut().zip(&built.picture_track_files) {
        event.picture_track_file = file_name(&picture.path);
        event.sound_track_file = sound_track_file.clone();
    }
    let manifest_path = write_conform_manifest(&manifest, output_dir)
        .map_err(|e| format!("Failed to write the conform manifest: {e}"))?;
    crate::intermediates::remove_intermediates(output_dir, &[]);

    Ok(ConformResult {
        imp_dir: output_dir.to_path_buf(),
        cpl_path: built.cpl_path,
        manifest_path,
        picture_frames: built.frames,
    })
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_whole_and_a_pulled_down_rate_both_have_an_edit_rate() {
        assert_eq!(edit_rate(24.0).unwrap(), (24, 1));
        assert_eq!(edit_rate(25.0).unwrap(), (25, 1));
        assert_eq!(edit_rate(23.976).unwrap(), (24000, 1001));
        assert_eq!(edit_rate(29.97).unwrap(), (30000, 1001));
        assert!(edit_rate(0.5).is_err());
    }

    #[test]
    fn a_reel_resolves_to_the_file_whose_name_holds_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("REEL001_v2.mov"), [0u8]).unwrap();
        std::fs::write(dir.path().join("REEL002.mov"), [0u8]).unwrap();
        assert_eq!(
            resolve_media("REEL001", dir.path()),
            Some(dir.path().join("REEL001_v2.mov"))
        );
        assert_eq!(resolve_media("REEL003", dir.path()), None);
    }

    #[test]
    fn an_unresolved_reel_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let timeline = Timeline {
            title: "Cut".into(),
            frame_rate: 24.0,
            events: vec![postkit::conform::EditEvent {
                event_number: 1,
                reel_name: "REEL001".into(),
                track_type: "V".into(),
                source_out: 10,
                ..Default::default()
            }],
            ..Default::default()
        };
        let error = build_plan(&timeline, dir.path()).unwrap_err();
        assert!(error.contains("REEL001"), "{error}");
    }
}
