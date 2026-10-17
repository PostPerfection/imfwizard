use assert_cmd::Command;
use postkit::timeline::{Clip, ClipSound, SoundTrack, TIMELINE_FORMAT_VERSION, Timeline};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FPS: u32 = 24;
const MEDIA_FRAMES: u32 = 8;
// both clips keep six frames, and the reel start falls inside the first
const FIRST_CLIP: (u64, u64) = (1, 7);
const SECOND_CLIP: (u64, u64) = (2, 8);
const REEL_START: u64 = 4;
const KEPT_FRAMES: u64 = 12;

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

fn write_media(dir: &Path, name: &str, tone_hz: u32) -> PathBuf {
    let media = dir.join(name);
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error"])
        .args([
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size={WIDTH}x{HEIGHT}:rate={FPS}:duration=1"),
        ])
        .args([
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency={tone_hz}:sample_rate=48000:duration=1"),
        ])
        .args(["-frames:v", &MEDIA_FRAMES.to_string()])
        .args(["-c:v", "ffv1", "-c:a", "pcm_s24le", "-shortest"])
        .arg(&media)
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    media
}

fn clip(picture: &Path, (in_frame, out_frame): (u64, u64)) -> Clip {
    Clip {
        picture: picture.to_path_buf(),
        in_frame,
        out_frame,
        sound: ClipSound::Embedded,
        subtitle: None,
        source_colour: Default::default(),
        picture_processing: Default::default(),
    }
}

fn write_timeline(dir: &Path, clips: Vec<Clip>, reel_starts: Vec<u64>) -> PathBuf {
    let path = dir.join("timeline.json");
    Timeline {
        version: TIMELINE_FORMAT_VERSION,
        fps: postkit::encode::FrameRate::whole(FPS),
        clips,
        sound: SoundTrack::PerClip,
        subtitle: None,
        reel_starts,
    }
    .write(&path)
    .unwrap();
    path
}

fn read_cpl(imp: &Path) -> String {
    let cpl = std::fs::read_dir(imp)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("CPL_"))
        })
        .expect("the IMP has a CPL");
    std::fs::read_to_string(cpl).unwrap()
}

/// The text of every `<tag>` inside the `element` sequence of `xml`.
fn sequence_values(xml: &str, element: &str, tag: &str) -> Vec<String> {
    let Some(start) = xml.find(&format!("<cc:{element}")) else {
        return Vec::new();
    };
    let end = xml[start..]
        .find(&format!("</cc:{element}>"))
        .expect("the sequence closes")
        + start;
    xml[start..end]
        .split(&format!("<{tag}>"))
        .skip(1)
        .map(|rest| rest.split(&format!("</{tag}>")).next().unwrap().to_string())
        .collect()
}

/// Each resource of the sequence as (EntryPoint, SourceDuration), EntryPoint 0 when left out.
fn resource_windows(segment: &str, element: &str) -> Vec<(u64, u64)> {
    let Some(start) = segment.find(&format!("<cc:{element}")) else {
        return Vec::new();
    };
    let end = segment[start..]
        .find(&format!("</cc:{element}>"))
        .expect("the sequence closes")
        + start;
    segment[start..end]
        .split("<Resource ")
        .skip(1)
        .map(|resource| {
            let value = |tag: &str| {
                resource
                    .split(&format!("<{tag}>"))
                    .nth(1)
                    .map(|rest| rest.split('<').next().unwrap().parse::<u64>().unwrap())
            };
            (
                value("EntryPoint").unwrap_or(0),
                value("SourceDuration").unwrap(),
            )
        })
        .collect()
}

#[test]
fn two_clips_with_a_reel_start_build_two_segments() {
    let dir = TempDir::new().unwrap();
    let first = write_media(dir.path(), "first.mov", 440);
    let second = write_media(dir.path(), "second.mov", 880);
    let timeline = write_timeline(
        dir.path(),
        vec![clip(&first, FIRST_CLIP), clip(&second, SECOND_CLIP)],
        vec![REEL_START],
    );
    let imp = dir.path().join("imp");
    cmd()
        .args(["create", "-o", &imp.to_string_lossy(), "-t", "Two Clips"])
        .args(["--timeline", &timeline.to_string_lossy()])
        // TODO: dcpdoctor reads each segment's sequence as a virtual track of its own and fails the verify
        .arg("--no-verify")
        .assert()
        .success();
    let photon = imfwizard_core::photon::run_photon(&imp, None).expect("Photon analyses the IMP");
    assert!(
        photon.errors.is_empty() && photon.warnings.is_empty(),
        "Photon errors {:?}, warnings {:?}",
        photon.errors,
        photon.warnings
    );

    let cpl = read_cpl(&imp);
    let segments: Vec<&str> = cpl.split("<Segment>").skip(1).collect();
    assert_eq!(segments.len(), 2, "{cpl}");
    let picture: Vec<Vec<(u64, u64)>> = segments
        .iter()
        .map(|segment| resource_windows(segment, "MainImageSequence"))
        .collect();
    let kept: u64 = picture.iter().flatten().map(|(_, length)| length).sum();
    assert_eq!(kept, KEPT_FRAMES, "{cpl}");
    assert_eq!(
        picture[1][0],
        (REEL_START, FIRST_CLIP.1 - FIRST_CLIP.0 - REEL_START),
        "the second segment plays the rest of the first clip: {cpl}"
    );
    let sound: Vec<Vec<(u64, u64)>> = segments
        .iter()
        .map(|segment| resource_windows(segment, "MainAudioSequence"))
        .collect();
    assert_eq!(
        sound,
        vec![
            vec![(0, REEL_START)],
            vec![(REEL_START, KEPT_FRAMES - REEL_START)]
        ]
    );
    let picture_tracks: Vec<String> = segments
        .iter()
        .map(|segment| sequence_values(segment, "MainImageSequence", "TrackId").remove(0))
        .collect();
    assert_eq!(picture_tracks[0], picture_tracks[1]);
}

#[test]
fn one_clip_builds_what_the_trimmed_video_build_does() {
    let dir = TempDir::new().unwrap();
    let media = write_media(dir.path(), "feature.mov", 440);
    let (in_frame, out_frame) = (2, 7);
    let timeline = write_timeline(
        dir.path(),
        vec![clip(&media, (in_frame, out_frame))],
        vec![],
    );

    let from_timeline = dir.path().join("from_timeline");
    cmd()
        .args([
            "create",
            "-o",
            &from_timeline.to_string_lossy(),
            "-t",
            "One",
        ])
        .args(["--timeline", &timeline.to_string_lossy(), "--no-verify"])
        .assert()
        .success();
    let from_video = dir.path().join("from_video");
    cmd()
        .args(["create", "-o", &from_video.to_string_lossy(), "-t", "One"])
        .args(["--video", &media.to_string_lossy(), "--no-verify"])
        .args(["--trim-start", &format!("{in_frame}f")])
        .args([
            "--trim-end",
            &format!("{}f", u64::from(MEDIA_FRAMES) - out_frame),
        ])
        .assert()
        .success();

    let (timeline_cpl, video_cpl) = (read_cpl(&from_timeline), read_cpl(&from_video));
    for element in ["MainImageSequence", "MainAudioSequence"] {
        let durations = sequence_values(&timeline_cpl, element, "SourceDuration");
        assert_eq!(
            durations,
            vec![(out_frame - in_frame).to_string()],
            "{element}"
        );
        assert_eq!(
            durations,
            sequence_values(&video_cpl, element, "SourceDuration"),
            "{element}"
        );
    }
}

#[test]
fn a_timeline_beside_a_video_is_refused_naming_the_flag() {
    let dir = TempDir::new().unwrap();
    let timeline = write_timeline(
        dir.path(),
        vec![clip(Path::new("/media/a.mov"), (0, 1))],
        vec![],
    );
    cmd()
        .args(["create", "-o", &dir.path().join("imp").to_string_lossy()])
        .args(["-t", "Refused", "--timeline", &timeline.to_string_lossy()])
        .args(["--video", "/media/a.mov"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("--video"));
}

#[test]
fn a_burn_on_a_timeline_of_two_clips_is_refused_by_name() {
    let dir = TempDir::new().unwrap();
    let media = write_media(dir.path(), "feature.mov", 440);
    let timeline = write_timeline(
        dir.path(),
        vec![clip(&media, FIRST_CLIP), clip(&media, SECOND_CLIP)],
        vec![],
    );
    let imp = dir.path().join("imp");
    cmd()
        .args(["create", "-o", &imp.to_string_lossy(), "-t", "Refused"])
        .args(["--timeline", &timeline.to_string_lossy()])
        .args([
            "--burn-subtitle",
            &dir.path().join("cues.srt").to_string_lossy(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "--burn-subtitle is not supported on a timeline of more than one clip",
        ));
    assert!(!imp.exists(), "a refusal writes nothing");
}
