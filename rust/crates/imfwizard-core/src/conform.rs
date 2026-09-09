use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};

use postkit::conform::Timeline;

// EDL convention: a black or aux event names no source media
const REELS_WITH_NO_MEDIA: [&str; 2] = ["BL", "AX"];

// the plan and what each event became, written beside the IMP
pub const CONFORM_MANIFEST_NAME: &str = "conform_manifest.json";

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
    /// The sound track file this event became, once the IMP is built.
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

/// What the picture of every event has to agree on, since one virtual track
/// carries them all.
struct Raster {
    width: u32,
    height: u32,
    has_sound: bool,
}

fn checked_raster(plan: &ConformPlan) -> Result<Raster, String> {
    let mut agreed: Option<Raster> = None;
    for event in &plan.events {
        let info = postkit::probe::probe_video(&event.media)
            .ok_or_else(|| format!("ffprobe read no picture out of {}", event.media.display()))?;
        // a source in and out are frames from the start of the media, so a
        // timeline whose source timecode starts at 01:00:00:00 runs off the end
        if info.total_frames > 0 && event.source_out > info.total_frames as u64 {
            return Err(format!(
                "event {} asks for frames {}..{} of {}, which is {} frames long",
                event.event_number,
                event.source_in,
                event.source_out,
                event.media.display(),
                info.total_frames
            ));
        }
        match &agreed {
            None => {
                agreed = Some(Raster {
                    width: info.width,
                    height: info.height,
                    has_sound: info.has_audio,
                });
            }
            Some(first) => {
                if (first.width, first.height) != (info.width, info.height) {
                    return Err(format!(
                        "{} is {}x{} where the first event is {}x{}, and one image track carries one raster",
                        event.media.display(),
                        info.width,
                        info.height,
                        first.width,
                        first.height
                    ));
                }
                if first.has_sound != info.has_audio {
                    return Err(format!(
                        "{} carries {} sound where an earlier event carries {}",
                        event.media.display(),
                        if info.has_audio { "" } else { "no" },
                        if first.has_sound { "some" } else { "none" }
                    ));
                }
            }
        }
    }
    agreed.ok_or_else(|| "the plan holds no events".to_string())
}

/// What a finished conform wrote.
#[derive(Debug, Clone)]
pub struct ConformResult {
    pub imp_dir: PathBuf,
    pub cpl_path: PathBuf,
    pub manifest_path: PathBuf,
    pub picture_frames: u64,
}

/// Build an IMP whose one composition follows the plan: one picture track file
/// per event, played in order through the main image track, and the sound cut
/// from the same media through the main audio track.
pub fn conform_to_imp(plan: &ConformPlan, output_dir: &Path) -> Result<ConformResult, String> {
    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("cannot create {}: {e}", output_dir.display()))?;
    let raster = checked_raster(plan)?;
    let fps = crate::encode::FrameRate::new(plan.fps_num, plan.fps_den);
    let rsiz = crate::encode::imf_rsiz_for_encode(
        raster.width,
        raster.height,
        fps.as_f64(),
        crate::encode::bitrate_mbps_for_job(None, None, raster.width, raster.height, fps.as_f64()),
    )?;

    let composition = crate::imp::Composition {
        title: plan.title.clone(),
        content_kind: "feature".to_string(),
        ..Default::default()
    };
    let mut options = crate::imp::ImpOptions {
        output_dir: output_dir.to_path_buf(),
        compositions: vec![composition.clone()],
        fps_num: plan.fps_num,
        fps_den: plan.fps_den,
        edit_rate: format!("{} {}", plan.fps_num, plan.fps_den),
        duration: 0,
        ..Default::default()
    };

    let mut manifest = plan.clone();
    let mut picture_resources = Vec::new();
    let mut sound_resources = Vec::new();
    let mut track_files = Vec::new();
    for (index, event) in plan.events.iter().enumerate() {
        tracing::info!(
            "Conforming event {} from {} frames {}..{}",
            event.event_number,
            event.media.display(),
            event.source_in,
            event.source_out
        );
        let picture = encode_event_picture(event, index, output_dir, plan, rsiz)?;
        let frames = picture.duration;
        options.duration = frames;
        manifest.events[index].picture_track_file = file_name(&picture.path);
        track_files.push(picture.clone());
        picture_resources.push(crate::cpl::SequenceResource {
            track_file: picture,
            entry_point: 0,
            source_duration: frames,
        });

        if raster.has_sound {
            let sound = wrap_event_sound(event, index, output_dir, &options, &composition, frames)?;
            manifest.events[index].sound_track_file = file_name(&sound.path);
            track_files.push(sound.clone());
            sound_resources.push(crate::cpl::SequenceResource {
                track_file: sound,
                entry_point: 0,
                source_duration: frames,
            });
        }
    }

    let cpl_uuid = uuid::Uuid::new_v4().to_string();
    let cpl_path = output_dir.join(format!("CPL_{cpl_uuid}.xml"));
    crate::cpl::write_sequenced_cpl(
        &cpl_path,
        &cpl_uuid,
        &options,
        &composition,
        &picture_resources,
        &sound_resources,
    )
    .map_err(|e| format!("Failed to write CPL: {e}"))?;

    let cpls = vec![crate::imp::CplEntry {
        uuid: cpl_uuid,
        path: cpl_path.clone(),
    }];
    let pkl_uuid = uuid::Uuid::new_v4().to_string();
    let pkl_path = output_dir.join(format!("PKL_{pkl_uuid}.xml"));
    crate::pkl::write_pkl(&pkl_path, &pkl_uuid, &cpls, &track_files)
        .map_err(|e| format!("Failed to write PKL: {e}"))?;
    let assetmap_path = output_dir.join("ASSETMAP.xml");
    crate::assetmap::write_assetmap(&assetmap_path, &pkl_uuid, &cpls, &track_files)
        .map_err(|e| format!("Failed to write ASSETMAP: {e}"))?;

    let manifest_path = write_conform_manifest(&manifest, output_dir)
        .map_err(|e| format!("Failed to write the conform manifest: {e}"))?;
    crate::intermediates::remove_intermediates(output_dir, &[]);

    Ok(ConformResult {
        imp_dir: output_dir.to_path_buf(),
        cpl_path,
        manifest_path,
        picture_frames: picture_resources
            .iter()
            .map(|resource| resource.source_duration)
            .sum(),
    })
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
}

fn encode_event_picture(
    event: &ConformEvent,
    index: usize,
    output_dir: &Path,
    plan: &ConformPlan,
    rsiz: u16,
) -> Result<crate::MxfTrackFile, String> {
    let encode_dir = output_dir.join(format!(
        "{}{index}",
        crate::intermediates::ENCODE_SCRATCH_PREFIX
    ));
    let options = postkit::pipeline::EncodeRunOptions {
        compression_ratio: crate::encode::DEFAULT_COMPRESSION_RATIO,
        fps: crate::encode::FrameRate::new(plan.fps_num, plan.fps_den),
        source_colour: crate::source_colourspace::to_source_colour(
            crate::source_colourspace::APP2E_SOURCE_SPACE,
        )?,
        frame_range: Some(postkit::encode::FrameRange {
            first_frame: event.source_in,
            frame_count: event.frames(),
        }),
        rsiz,
        ..Default::default()
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));
    let (_, track) = crate::overlapped_picture::encode_and_wrap_picture(
        &event.media,
        &encode_dir,
        &options,
        crate::overlapped_picture::PictureWrapTarget {
            imp_dir: output_dir.to_path_buf(),
            fps_num: plan.fps_num,
            fps_den: plan.fps_den,
            colour: crate::mxf_wrap::picture_colour(None),
        },
        &cancel,
        &pause,
        |_| {},
        |message| tracing::info!("{message}"),
    )?;
    Ok(track)
}

fn wrap_event_sound(
    event: &ConformEvent,
    index: usize,
    output_dir: &Path,
    options: &crate::imp::ImpOptions,
    composition: &crate::imp::Composition,
    picture_frames: u64,
) -> Result<crate::MxfTrackFile, String> {
    let demuxed = output_dir.join(format!(
        "{}{index}_{}",
        crate::intermediates::ENCODE_SCRATCH_PREFIX,
        crate::intermediates::DEMUXED_AUDIO_NAME
    ));
    demux_sound(&event.media, &demuxed)?;
    let cut = output_dir.join(format!(
        "{}{index}.wav",
        crate::source_edits::TRIMMED_AUDIO_PREFIX
    ));
    cut_wav_frames(
        &demuxed,
        &cut,
        event.source_in,
        picture_frames,
        options.fps_num,
        options.fps_den,
    )?;
    let fitted = output_dir.join(format!(
        "{}{index}.wav",
        crate::source_edits::FITTED_AUDIO_PREFIX
    ));
    let sound = match crate::source_edits::fit_audio_to_picture(
        &cut,
        &fitted,
        picture_frames,
        options.fps_num,
        options.fps_den,
    )? {
        None => cut,
        Some(_) => fitted,
    };
    let track = crate::imp::AudioTrack {
        path: sound.clone(),
        language: None,
        role: None,
    };
    let mca = crate::imp::soundfield_config(&track, composition, &options.soundfield)?;
    crate::imp::wrap_one(
        options,
        output_dir,
        &sound,
        crate::EssenceType::Wav,
        None,
        Some(mca),
        picture_frames,
    )
}

fn demux_sound(media: &Path, output: &Path) -> Result<(), String> {
    let run = std::process::Command::new("ffmpeg")
        .arg("-y")
        .args(["-i".as_ref(), media.as_os_str()])
        .args(["-vn", "-acodec", "pcm_s24le", "-ar", "48000"])
        .arg(output)
        .output()
        .map_err(|e| format!("cannot run ffmpeg: {e}"))?;
    if !run.status.success() {
        return Err(format!(
            "ffmpeg demuxed no sound out of {}: {}",
            media.display(),
            String::from_utf8_lossy(&run.stderr).trim()
        ));
    }
    Ok(())
}

/// Copy `frames` picture frames of sound starting at `first_frame`, so the cut
/// carries the samples the event's source range covers.
fn cut_wav_frames(
    input: &Path,
    output: &Path,
    first_frame: u64,
    frames: u64,
    fps_num: u32,
    fps_den: u32,
) -> Result<(), String> {
    use postkit::wav_io::Samples;
    let (spec, samples) = postkit::wav_io::read_interleaved_exact(input)
        .map_err(|e| format!("cannot read {}: {e}", input.display()))?;
    let channels = spec.channels.max(1) as usize;
    let fps = fps_num.max(1) as f64 / fps_den.max(1) as f64;
    let per_frame = (spec.sample_rate.max(1) as f64 / fps).ceil() as u64;
    let start = (first_frame * per_frame) as usize * channels;
    let wanted = (frames * per_frame) as usize * channels;

    fn window<T: Copy + Default>(samples: &[T], start: usize, wanted: usize) -> Vec<T> {
        let mut out = vec![T::default(); wanted];
        let available = samples.len().saturating_sub(start);
        let kept = wanted.min(available);
        out[..kept].copy_from_slice(&samples[start..start + kept]);
        out
    }
    let cut = match &samples {
        Samples::Int(v) => Samples::Int(window(v, start, wanted)),
        Samples::Float(v) => Samples::Float(window(v, start, wanted)),
    };
    postkit::wav_io::write_interleaved_exact(output, spec, &cut)
        .map_err(|e| format!("cannot write {}: {e}", output.display()))
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
