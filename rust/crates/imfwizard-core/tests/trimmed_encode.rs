//! A trimmed video encodes the kept frames and nothing else: the codestreams
//! that come out are the ones a whole-source encode would have written inside
//! the window, and the trim leaves them where they are instead of linking them
//! into a second directory.

use imfwizard_core::source_edits::{
    CompositionSource, SourceEdits, apply_source_edits, trimmed_encode_window,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const SOURCE_FRAMES: u64 = 10;
const TRIM_START: u64 = 2;
const TRIM_END: u64 = 3;
const KEPT_FRAMES: u64 = SOURCE_FRAMES - TRIM_START - TRIM_END;
const FPS: u32 = 24;

fn have_ffmpeg() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn make_clip(path: &Path) {
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc=s={WIDTH}x{HEIGHT}:r={FPS}"),
            "-frames:v",
            &SOURCE_FRAMES.to_string(),
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(path)
        .output()
        .expect("ffmpeg");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn encode(video: &Path, out_dir: &Path, window: Option<postkit::encode::FrameRange>) -> PathBuf {
    let result = postkit::pipeline::run_encode_with_options(
        video,
        out_dir,
        &postkit::pipeline::EncodeRunOptions {
            fps: postkit::encode::FrameRate::whole(FPS),
            frame_range: window,
            ..Default::default()
        },
        &Arc::new(AtomicBool::new(false)),
        &Arc::new(AtomicBool::new(false)),
        |_| {},
        |_| {},
    )
    .expect("encode");
    assert_eq!(
        result.frames_encoded,
        window.map_or(SOURCE_FRAMES, |w| w.frame_count)
    );
    result.j2k_dir
}

fn codestreams(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("j2c" | "j2k")
            )
        })
        .collect();
    paths.sort();
    paths
}

#[test]
fn a_trimmed_video_encodes_only_the_kept_frames() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("clip.mp4");
    make_clip(&video);

    let edits = SourceEdits {
        audio_delay_ms: 0,
        trim_start_frames: TRIM_START,
        trim_end_frames: TRIM_END,
    };
    let window = trimmed_encode_window(
        &edits,
        &video,
        postkit::encode::detect_input_type(&video),
        FPS,
        1,
    )
    .expect("the window is measurable")
    .expect("a trimmed video encodes a window");
    assert_eq!(window.first_frame, TRIM_START);
    assert_eq!(window.frame_count, KEPT_FRAMES);

    let trimmed_dir = dir.path().join("trimmed");
    let j2k_dir = encode(&video, &trimmed_dir, Some(window));
    let kept = codestreams(&j2k_dir);
    assert_eq!(kept.len() as u64, KEPT_FRAMES);

    let source = CompositionSource {
        j2k_dir: Some(j2k_dir.clone()),
        ..Default::default()
    };
    let edited = apply_source_edits(&edits, &source, &trimmed_dir, FPS, 1, Some(window)).unwrap();
    assert_eq!(
        edited.j2k_dir,
        Some(j2k_dir),
        "the windowed codestreams are already the kept frames"
    );
    assert!(
        !trimmed_dir.join("j2k_trimmed").exists(),
        "nothing may be linked into a second picture directory"
    );

    // the window has to land on the same frames the old encode-then-relink path
    // kept, so the whole source is encoded here and compared against it
    let whole_dir = dir.path().join("whole");
    let whole = codestreams(&encode(&video, &whole_dir, None));
    assert_eq!(whole.len() as u64, SOURCE_FRAMES);
    for (index, frame) in kept.iter().enumerate() {
        let source_index = TRIM_START as usize + index;
        assert_eq!(
            std::fs::read(frame).unwrap(),
            std::fs::read(&whole[source_index]).unwrap(),
            "window frame {index} is not source frame {source_index}"
        );
    }
}
