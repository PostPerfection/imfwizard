//! The commands that work on media beside a package: transcode, burn-in, lut,
//! frame-extract, info, completion and trailer.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const FRAMES: u32 = 8;
const FPS: u32 = 24;

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

fn ffmpeg(arguments: &[&str]) {
    let out = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(arguments)
        .output()
        .expect("ffmpeg");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn stream_entry(path: &Path, entry: &str) -> String {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .arg(format!("stream={entry}"))
        .args(["-of", "default=nw=1:nk=1"])
        .arg(path)
        .output()
        .expect("ffprobe");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn source_clip(dir: &Path) -> PathBuf {
    let clip = dir.join("source.mkv");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("testsrc2=s={WIDTH}x{HEIGHT}:r={FPS}"),
        "-frames:v",
        &FRAMES.to_string(),
        "-c:v",
        "ffv1",
        "-pix_fmt",
        "yuv420p",
        &clip.to_string_lossy(),
    ]);
    clip
}

/// The centre pixel of `path`'s first frame, as 8-bit RGB.
fn centre_pixel(path: &Path) -> [u8; 3] {
    let out = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-vf", "crop=1:1:160:90", "-frames:v", "1"])
        .args(["-pix_fmt", "rgb24", "-f", "rawvideo", "-"])
        .output()
        .expect("ffmpeg");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout.len(), 3, "one RGB pixel");
    [out.stdout[0], out.stdout[1], out.stdout[2]]
}

/// `transcode` runs ffmpeg with the codec asked for, and the written file
/// carries that codec at the source's raster and frame count.
#[test]
fn transcode_writes_the_codec_it_was_given() {
    let dir = TempDir::new().unwrap();
    let clip = source_clip(dir.path());

    for (codec, expected, extension) in [
        ("libx264", "h264", "mp4"),
        ("prores", "prores", "mov"),
        ("ffv1", "ffv1", "mkv"),
    ] {
        let output = dir.path().join(format!("out.{extension}"));
        cmd()
            .args([
                "transcode",
                "-i",
                &clip.to_string_lossy(),
                "-o",
                &output.to_string_lossy(),
                "-c",
                codec,
            ])
            .assert()
            .success();

        assert_eq!(
            stream_entry(&output, "codec_name"),
            expected,
            "--codec {codec} did not reach ffmpeg"
        );
        assert_eq!(stream_entry(&output, "width"), WIDTH.to_string());
        assert_eq!(stream_entry(&output, "height"), HEIGHT.to_string());
    }
}

/// A codec ffmpeg does not know fails the command in ffmpeg's own words rather
/// than leaving a half written file behind as a success.
#[test]
fn transcode_fails_on_a_codec_ffmpeg_does_not_know() {
    let dir = TempDir::new().unwrap();
    let clip = source_clip(dir.path());

    cmd()
        .args([
            "transcode",
            "-i",
            &clip.to_string_lossy(),
            "-o",
            &dir.path().join("out.mkv").to_string_lossy(),
            "-c",
            "libnosuchcodec",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("libnosuchcodec"));
}

/// `burn-in` draws the cues into the picture: the frame under a cue changes and
/// a frame outside every cue does not.
#[test]
fn burn_in_changes_only_the_frames_a_cue_covers() {
    let dir = TempDir::new().unwrap();
    // black, so drawn white text is the only thing that can lift a pixel
    let long = dir.path().join("long.mkv");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("color=c=black:s={WIDTH}x{HEIGHT}:r={FPS}"),
        "-frames:v",
        "48",
        "-c:v",
        "ffv1",
        "-pix_fmt",
        "gbrp",
        &long.to_string_lossy(),
    ]);

    let srt = dir.path().join("cues.srt");
    std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:01,000\nBURNT IN TEXT\n\n").unwrap();

    let output = dir.path().join("burnt.mkv");
    cmd()
        .args([
            "burn-in",
            "-i",
            &long.to_string_lossy(),
            "-s",
            &srt.to_string_lossy(),
            "-o",
            &output.to_string_lossy(),
        ])
        .assert()
        .success();

    let mean = |path: &Path, start: &str| -> f64 {
        let raw = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-ss", start, "-i"])
            .arg(path)
            .args(["-frames:v", "1", "-pix_fmt", "gray", "-f", "rawvideo", "-"])
            .output()
            .expect("ffmpeg");
        assert!(
            raw.status.success(),
            "{}",
            String::from_utf8_lossy(&raw.stderr)
        );
        let total: u64 = raw.stdout.iter().map(|sample| u64::from(*sample)).sum();
        total as f64 / raw.stdout.len() as f64
    };

    let under_cue = mean(&output, "0.2");
    let after_cue = mean(&output, "1.5");
    assert!(
        under_cue > after_cue,
        "the cue frame averages {under_cue} and the uncovered one {after_cue}, so nothing \
         was drawn"
    );
    assert!(
        after_cue < 1.0,
        "the frame outside every cue is not still black, it averages {after_cue}"
    );
}

/// A 3D LUT that swaps red for blue, written as a .cube the ffmpeg lut3d filter
/// reads. Two entries a side is enough for a permutation.
fn swap_red_and_blue_lut(dir: &Path) -> PathBuf {
    let lut = dir.join("swap.cube");
    let mut text = String::from("LUT_3D_SIZE 2\n");
    // .cube runs red fastest, then green, then blue
    for blue in 0..2 {
        for green in 0..2 {
            for red in 0..2 {
                text.push_str(&format!("{blue}.0 {green}.0 {red}.0\n"));
            }
        }
    }
    std::fs::write(&lut, text).unwrap();
    lut
}

/// `lut` applies the .cube through ffmpeg lut3d, and the pixels come out where
/// the LUT says: a red source becomes blue.
#[test]
fn a_lut_moves_the_pixels_where_it_says() {
    let dir = TempDir::new().unwrap();
    let red = dir.path().join("red.mkv");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("color=c=red:s={WIDTH}x{HEIGHT}:r={FPS}"),
        "-frames:v",
        "2",
        "-c:v",
        "ffv1",
        "-pix_fmt",
        "gbrp",
        &red.to_string_lossy(),
    ]);
    let [r, g, b] = centre_pixel(&red);
    assert!(
        r > 200 && g < 55 && b < 55,
        "the source is not red, it is {r},{g},{b}"
    );

    let lut = swap_red_and_blue_lut(dir.path());
    let output = dir.path().join("swapped.mkv");
    cmd()
        .args([
            "lut",
            "-i",
            &red.to_string_lossy(),
            "-o",
            &output.to_string_lossy(),
            "-l",
            &lut.to_string_lossy(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("LUT applied"));

    let [r, g, b] = centre_pixel(&output);
    assert!(
        b > 200 && r < 55 && g < 55,
        "the LUT swaps red for blue, and the pixel came out {r},{g},{b}"
    );
}

/// `frame-extract` writes the frame it was asked for, and two different frame
/// numbers give two different pictures.
#[test]
fn frame_extract_writes_the_frame_it_names() {
    let dir = TempDir::new().unwrap();
    let clip = source_clip(dir.path());

    let mut written = Vec::new();
    for frame in [0u32, 5] {
        let output = dir.path().join(format!("frame{frame}.png"));
        cmd()
            .args([
                "frame-extract",
                "-i",
                &clip.to_string_lossy(),
                "-f",
                &frame.to_string(),
                "-o",
                &output.to_string_lossy(),
            ])
            .assert()
            .success();
        assert!(output.is_file(), "frame {frame} was not written");
        assert_eq!(stream_entry(&output, "width"), WIDTH.to_string());
        assert_eq!(stream_entry(&output, "height"), HEIGHT.to_string());
        written.push(std::fs::read(&output).unwrap());
    }
    assert_ne!(
        written[0], written[1],
        "--frame picked the same picture twice, so the number was ignored"
    );
}

/// `info` reads a written package back: the JSON names the composition, how
/// many CPLs it holds and what the picture runs at.
#[test]
fn info_reports_what_the_package_holds() {
    let dir = TempDir::new().unwrap();
    let clip = source_clip(dir.path());
    let imp = dir.path().join("imp");
    cmd()
        .args(["create", "-o", &imp.to_string_lossy()])
        .args(["-t", "Inspected"])
        .args(["--video", &clip.to_string_lossy()])
        .args(["--raster", "1920x1080"])
        .args(["--fps-num", &FPS.to_string(), "--fps-den", "1"])
        .assert()
        .success();

    let output = cmd()
        .args(["info", &imp.to_string_lossy()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let info: serde_json::Value = serde_json::from_slice(&output).expect("info prints JSON");
    assert_eq!(info["title"], "Inspected");
    assert_eq!(info["cpl_count"], 1);
    assert_eq!(info["duration_frames"], FRAMES);
    assert_eq!(info["edit_rate"], format!("{FPS} 1"));

    // and a frame comes back out of the wrapped picture essence itself
    let picture = std::fs::read_dir(&imp)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(imfwizard_core::imp::PICTURE_PREFIX))
        })
        .expect("a picture track file");
    let still = dir.path().join("from_mxf.png");
    cmd()
        .args([
            "frame-extract",
            "-i",
            &picture.to_string_lossy(),
            "-f",
            "2",
            "-o",
            &still.to_string_lossy(),
        ])
        .assert()
        .success();
    assert_eq!(stream_entry(&still, "width"), "1920");
    assert_eq!(stream_entry(&still, "height"), "1080");
}

/// The completion script is a real script for the shell it names, and it names
/// the subcommands so a shell can offer them.
#[test]
fn completion_writes_a_script_naming_the_subcommands() {
    for (shell, marker) in [
        ("bash", "complete"),
        ("zsh", "#compdef"),
        ("fish", "complete -c"),
    ] {
        let output = cmd()
            .args(["completion", shell])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let script = String::from_utf8(output).expect("the script is text");
        assert!(
            script.contains(marker),
            "the {shell} script carries no {marker}: {}",
            &script[..script.len().min(200)]
        );
        for subcommand in ["create", "validate", "to-dcp", "trailer"] {
            assert!(
                script.contains(subcommand),
                "the {shell} script does not offer {subcommand}"
            );
        }
    }
}

/// The content a trailer is built around: a short clip and a matching sound.
fn trailer_sources(dir: &Path) -> (PathBuf, PathBuf) {
    let content = dir.join("content.mp4");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("testsrc2=s={WIDTH}x{HEIGHT}:r={FPS}"),
        "-frames:v",
        &FRAMES.to_string(),
        "-pix_fmt",
        "yuv420p",
        &content.to_string_lossy(),
    ]);
    let audio = dir.join("audio.wav");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "anullsrc=r=48000:cl=stereo",
        "-t",
        "1",
        &audio.to_string_lossy(),
    ]);
    (content, audio)
}

fn package_trailer(dir: &Path, name: &str, extra: &[&str]) -> PathBuf {
    let (content, audio) = trailer_sources(dir);
    let output = dir.join(name);
    cmd()
        .args(["trailer", "-c", &content.to_string_lossy()])
        .args(["-a", &audio.to_string_lossy()])
        .args(["-o", &output.to_string_lossy()])
        .args(["--title", "Feature Title"])
        .args(extra)
        .assert()
        .success()
        .stdout(predicate::str::contains("Trailer packaged"));
    output.join("ratings_card.mp4")
}

/// The first frame of a rendered card, decoded to 8-bit RGB.
fn card_pixels(card: &Path) -> Vec<u8> {
    let raw = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(card)
        .args(["-frames:v", "1", "-pix_fmt", "rgb24", "-f", "rawvideo", "-"])
        .output()
        .expect("ffmpeg");
    assert!(raw.status.success(), "the card does not decode");
    assert!(!raw.stdout.is_empty());
    raw.stdout
}

/// Each rating system draws its own lowest rating on the card, so the three
/// cards differ in the pixels the rating text covers. Two runs of one system
/// give the same picture, so a difference between two systems is the text and
/// not the encoder.
#[test]
fn every_rating_system_renders_a_card_of_its_own() {
    let control = TempDir::new().unwrap();
    assert_eq!(
        card_pixels(&package_trailer(
            control.path(),
            "again",
            &["--rating-system", "mpaa"]
        )),
        card_pixels(&package_trailer(
            control.path(),
            "once",
            &["--rating-system", "mpaa"]
        )),
        "the same rating system renders two different cards, so this test cannot tell \
         a rating apart from encoder noise"
    );

    let mut cards: Vec<(&str, Vec<u8>)> = Vec::new();
    for system in ["mpaa", "bbfc", "fsk"] {
        let dir = TempDir::new().unwrap();
        let card = package_trailer(dir.path(), system, &["--rating-system", system]);
        assert!(card.is_file(), "{system}: no ratings card was rendered");
        cards.push((system, card_pixels(&card)));
    }

    for (index, (system, pixels)) in cards.iter().enumerate() {
        for (other, other_pixels) in &cards[index + 1..] {
            assert_ne!(
                pixels, other_pixels,
                "the {system} and {other} cards decode to the same picture, so the \
                 rating system changed nothing"
            );
        }
    }
}

/// The band is the card's background, so the three bands render three colours.
#[test]
fn every_band_renders_its_own_colour() {
    // the drawn title and rating sit in the middle, so the corner is pure band
    fn corner_pixel(card: &Path) -> [u8; 3] {
        let raw = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(card)
            .args(["-vf", "crop=1:1:0:0", "-frames:v", "1"])
            .args(["-pix_fmt", "rgb24", "-f", "rawvideo", "-"])
            .output()
            .expect("ffmpeg");
        assert!(raw.status.success(), "the card does not decode");
        assert_eq!(raw.stdout.len(), 3);
        [raw.stdout[0], raw.stdout[1], raw.stdout[2]]
    }

    // the strongest component of each band's colour
    for (band, brightest) in [("green", 1usize), ("red", 0), ("yellow", 0)] {
        let dir = TempDir::new().unwrap();
        let card = package_trailer(dir.path(), band, &["--band", band, "--rating", "PG"]);
        let pixel = corner_pixel(&card);
        assert!(
            pixel[brightest] > 128,
            "the {band} band's corner is {pixel:?}"
        );
        match band {
            "green" => assert!(pixel[0] < 128 && pixel[2] < 128, "{band}: {pixel:?}"),
            "red" => assert!(pixel[1] < 128 && pixel[2] < 128, "{band}: {pixel:?}"),
            "yellow" => assert!(pixel[1] > 128 && pixel[2] < 128, "{band}: {pixel:?}"),
            _ => unreachable!(),
        }
    }
}

/// A rating system or band the command does not know is refused naming the ones
/// that work, before any frame is rendered.
#[test]
fn an_unknown_rating_system_or_band_is_refused_by_name() {
    let dir = TempDir::new().unwrap();
    let (content, audio) = trailer_sources(dir.path());

    for (flag, value, known) in [
        ("--rating-system", "nfvcb", "mpaa"),
        ("--band", "purple", "green"),
    ] {
        cmd()
            .args(["trailer", "-c", &content.to_string_lossy()])
            .args(["-a", &audio.to_string_lossy()])
            .args(["-o", &dir.path().join("refused").to_string_lossy()])
            .args(["--title", "Refused"])
            .args([flag, value])
            .assert()
            .failure()
            .stderr(predicate::str::contains(value))
            .stderr(predicate::str::contains(known));
    }
}
