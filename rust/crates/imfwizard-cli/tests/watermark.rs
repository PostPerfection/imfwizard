use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const FRAMES: u32 = 3;
const FPS: u32 = 24;
const OPERATOR_ID: &str = "operator-7";
const SESSION_ID: &str = "session-42";

// drawtext draws the mark 10 pixels in from the left, 20 up from the bottom
const MARK_LEFT: u32 = 10;
const MARK_TOP: u32 = HEIGHT - 20;
const MARK_WIDTH: u32 = 140;
// the font is 10 pixels, with a row either side of it
const MARK_HEIGHT: u32 = 12;
const MINIMUM_MARKED_PIXELS: usize = 20;
const LEVEL_TOLERANCE: i32 = 4;

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

fn grey_frames(dir: &Path) -> PathBuf {
    let frames = dir.join("frames");
    std::fs::create_dir_all(&frames).unwrap();
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "lavfi"])
        .args(["-i", &format!("color=c=gray:s={WIDTH}x{HEIGHT}:r={FPS}")])
        .args(["-frames:v", &FRAMES.to_string()])
        .args(["-pix_fmt", "rgb24"])
        .arg(frames.join("frame_%06d.png"))
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    frames
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

fn region(frame: &Path, x: u32, y: u32, width: u32, height: u32) -> Vec<u8> {
    let out = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(frame)
        .args(["-vf", &format!("crop={width}:{height}:{x}:{y}")])
        .args(["-frames:v", "1", "-pix_fmt", "rgb24", "-f", "rawvideo", "-"])
        .output()
        .expect("ffmpeg");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout.len(), (width * height * 3) as usize);
    out.stdout
}

fn differing_pixels(marked: &[u8], source: &[u8]) -> usize {
    marked
        .as_chunks::<3>()
        .0
        .iter()
        .zip(source.as_chunks::<3>().0)
        .filter(|(after, before)| after != before)
        .count()
}

#[test]
fn watermark_marks_the_bottom_left_of_every_frame_and_leaves_the_picture_alone() {
    let dir = TempDir::new().unwrap();
    let frames = grey_frames(dir.path());
    let output = dir.path().join("marked");

    cmd()
        .args([
            "watermark",
            "-i",
            &frames.to_string_lossy(),
            "-o",
            &output.to_string_lossy(),
            "--operator-id",
            OPERATOR_ID,
            "--session-id",
            SESSION_ID,
            "--strength",
            "0.5",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Watermark embedded: {FRAMES} frames"
        )));

    assert_eq!(
        file_names(&output),
        vec!["000001.png", "000002.png", "000003.png"],
        "the marked frames are not numbered stills in the source's format"
    );

    for (index, name) in file_names(&output).iter().enumerate() {
        let marked = output.join(name);
        let source = frames.join(format!("frame_{:06}.png", index + 1));

        let centre_after = region(&marked, WIDTH / 2, HEIGHT / 2, 1, 1);
        let centre_before = region(&source, WIDTH / 2, HEIGHT / 2, 1, 1);
        for (component, (after, before)) in centre_after.iter().zip(&centre_before).enumerate() {
            assert!(
                (i32::from(*after) - i32::from(*before)).abs() <= LEVEL_TOLERANCE,
                "{name} component {component} came back as {after}, not the {before} the source carried"
            );
        }

        let mark_after = region(&marked, MARK_LEFT, MARK_TOP, MARK_WIDTH, MARK_HEIGHT);
        let mark_before = region(&source, MARK_LEFT, MARK_TOP, MARK_WIDTH, MARK_HEIGHT);
        let drawn = differing_pixels(&mark_after, &mark_before);
        assert!(
            drawn > MINIMUM_MARKED_PIXELS,
            "{name} carries only {drawn} marked pixels where the text sits"
        );
    }
}
