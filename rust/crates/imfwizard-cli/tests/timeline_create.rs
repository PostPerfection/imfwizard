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
    write_media_of(dir, name, &format!("testsrc2={}", media_picture()), tone_hz)
}

fn media_picture() -> String {
    format!("size={WIDTH}x{HEIGHT}:rate={FPS}:duration=1")
}

fn write_media_of(dir: &Path, name: &str, picture_source: &str, tone_hz: u32) -> PathBuf {
    let media = dir.join(name);
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error"])
        .args(["-f", "lavfi", "-i", picture_source])
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

fn clip(picture: &Path, start: u64, (in_frame, out_frame): (u64, u64)) -> Clip {
    Clip {
        picture: picture.to_path_buf(),
        start,
        layer: 0,
        opacity: 1.0,
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
        vec![
            clip(&first, 0, FIRST_CLIP),
            clip(&second, FIRST_CLIP.1 - FIRST_CLIP.0, SECOND_CLIP),
        ],
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
        vec![clip(&media, 0, (in_frame, out_frame))],
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
        vec![clip(Path::new("/media/a.mov"), 0, (0, 1))],
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
        vec![
            clip(&media, 0, FIRST_CLIP),
            clip(&media, FIRST_CLIP.1 - FIRST_CLIP.0, SECOND_CLIP),
        ],
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

// layer 0 is black for GAP_FRAMES, then blue, with a still sequence on layer 1
// that is red on its left half and transparent on its right
const GAP_FRAMES: u64 = 3;
const OVERLAY_START: u64 = 5;
const OVERLAY_FRAMES: u64 = 4;
// the stretches: the gap, blue, red over blue, blue
const STRETCH_WINDOWS: [(u64, u64); 4] = [(0, 3), (0, 2), (0, 4), (0, 2)];
// a full scale 12-bit sample has to come back above this, and an empty one below the ceiling
const FULL_FLOOR: i32 = 3500;
const EMPTY_CEILING: i32 = 400;

fn write_half_red_stills(dir: &Path) -> PathBuf {
    let stills = dir.join("half_red");
    std::fs::create_dir_all(&stills).unwrap();
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg(format!(
            "color=c=red:size={}x{HEIGHT}:rate={FPS}",
            WIDTH / 2
        ))
        .arg("-vf")
        .arg(format!(
            "format=rgba,pad={WIDTH}:{HEIGHT}:0:0:color=black@0"
        ))
        .args(["-frames:v", &OVERLAY_FRAMES.to_string(), "-pix_fmt", "rgba"])
        .arg(stills.join("frame_%04d.png"))
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    stills
}

// the first frame's samples left and right of the centre
fn first_frame_samples(mxf: &Path, unwrapped: &Path) -> [[i32; 3]; 2] {
    let info = postkit::mxf_unwrap::unwrap_picture_mxf(
        mxf,
        unwrapped,
        &std::sync::atomic::AtomicBool::new(false),
        &mut |_, _| {},
    )
    .expect("the track file reads back");
    assert_eq!((info.width, info.height), (WIDTH, HEIGHT));
    let mut codestreams: Vec<PathBuf> = std::fs::read_dir(unwrapped)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    codestreams.sort();
    assert_eq!(codestreams.len() as u64, info.frames);
    let frame = postkit::grok_decoder::decode(std::fs::read(&codestreams[0]).unwrap(), 0)
        .expect("the codestream decodes");
    assert_eq!(frame.precision, 12);
    [WIDTH / 4, WIDTH * 3 / 4].map(|x| {
        let at = (HEIGHT / 2 * WIDTH + x) as usize;
        [0, 1, 2].map(|component| frame.components[component][at])
    })
}

fn only(sample: [i32; 3], full_component: Option<usize>) -> bool {
    (0..3).all(|component| {
        if Some(component) == full_component {
            sample[component] > FULL_FLOOR
        } else {
            sample[component] < EMPTY_CEILING
        }
    })
}

#[test]
fn a_gap_and_an_overlay_with_alpha_build_a_track_file_a_stretch() {
    const RED: Option<usize> = Some(0);
    const BLUE: Option<usize> = Some(2);
    const BLACK: Option<usize> = None;
    let dir = TempDir::new().unwrap();
    let blue = write_media_of(
        dir.path(),
        "blue.mov",
        &format!("color=c=blue:{}", media_picture()),
        440,
    );
    let half_red = write_half_red_stills(dir.path());
    let timeline = write_timeline(
        dir.path(),
        vec![
            clip(&blue, GAP_FRAMES, (0, u64::from(MEDIA_FRAMES))),
            Clip {
                layer: 1,
                sound: ClipSound::None,
                ..clip(&half_red, OVERLAY_START, (0, OVERLAY_FRAMES))
            },
        ],
        vec![],
    );
    let imp = dir.path().join("imp");
    cmd()
        .args(["create", "-o", &imp.to_string_lossy(), "-t", "Layered"])
        .args(["--timeline", &timeline.to_string_lossy()])
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
    assert_eq!(
        resource_windows(&cpl, "MainImageSequence"),
        STRETCH_WINDOWS.to_vec(),
        "{cpl}"
    );
    assert_eq!(
        resource_windows(&cpl, "MainAudioSequence"),
        vec![(0, GAP_FRAMES + u64::from(MEDIA_FRAMES))]
    );
    let track_files: Vec<PathBuf> = sequence_values(&cpl, "MainImageSequence", "TrackFileId")
        .iter()
        .map(|id| imp.join(format!("VIDEO_{}.mxf", id.trim_start_matches("urn:uuid:"))))
        .collect();
    let want = [[BLACK, BLACK], [BLUE, BLUE], [RED, BLUE], [BLUE, BLUE]];
    assert_eq!(track_files.len(), want.len(), "{cpl}");
    for (index, (track_file, want)) in track_files.iter().zip(want).enumerate() {
        let samples =
            first_frame_samples(track_file, &dir.path().join(format!("unwrapped_{index}")));
        assert!(
            samples
                .iter()
                .zip(want)
                .all(|(&sample, want)| only(sample, want)),
            "stretch {index} samples {samples:?} left and right, not {want:?}"
        );
    }
}
