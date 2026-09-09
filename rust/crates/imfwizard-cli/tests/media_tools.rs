use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const CLIP_SIZE: &str = "320x180";
const CLIP_FPS: u32 = 25;
const CLIP_FRAMES: u32 = 10;
const RETIMED_FPS: u32 = 30;
const SLATE_FRAMES: u32 = 10;
const CLIP_SECONDS: f64 = 2.0;
// a tenth of a second, the offset and the drift the sync fixtures carry
const SYNC_ERROR_MILLISECONDS: f64 = 100.0;
// the slate is black behind white text, so only the glyphs lift the average
const SLATE_MAX_MEAN_LUMA: f64 = 5.0;
const PICTURE_MIN_MEAN_LUMA: f64 = 50.0;
// zimg encodes to Rec.709 with the BT.1886 display gamma
const REC709_DISPLAY_GAMMA: f64 = 2.4;
const SIXTEEN_BIT_FULL_SCALE: f64 = 65535.0;

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

fn ffmpeg(arguments: &[&str]) {
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error"])
        .args(arguments)
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
}

fn ffprobe(arguments: &[&str], input: &Path) -> String {
    let probed = std::process::Command::new("ffprobe")
        .args(["-v", "error"])
        .args(arguments)
        .arg(input)
        .output()
        .expect("ffprobe");
    assert!(
        probed.status.success(),
        "{}",
        String::from_utf8_lossy(&probed.stderr)
    );
    String::from_utf8_lossy(&probed.stdout).trim().to_string()
}

fn video_stream_entry(input: &Path, entry: &str) -> String {
    ffprobe(
        &[
            "-select_streams",
            "v:0",
            "-count_frames",
            "-show_entries",
            &format!("stream={entry}"),
            "-of",
            "csv=p=0",
        ],
        input,
    )
}

fn reference_clip(directory: &Path) -> PathBuf {
    let clip = directory.join("reference.mkv");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("testsrc=size={CLIP_SIZE}:rate={CLIP_FPS}"),
        "-frames:v",
        &CLIP_FRAMES.to_string(),
        "-c:v",
        "ffv1",
        &clip.to_string_lossy(),
    ]);
    clip
}

// the same picture through a lossy encoder, worse the higher the quantiser
fn recompressed(directory: &Path, reference: &Path, quantiser: u32) -> PathBuf {
    let clip = directory.join(format!("recompressed_q{quantiser}.mkv"));
    ffmpeg(&[
        "-i",
        &reference.to_string_lossy(),
        "-c:v",
        "mpeg4",
        "-q:v",
        &quantiser.to_string(),
        &clip.to_string_lossy(),
    ]);
    clip
}

fn sync_clip(directory: &Path, name: &str, audio_offset: f64, audio_seconds: f64) -> PathBuf {
    let clip = directory.join(name);
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("testsrc=size={CLIP_SIZE}:rate={CLIP_FPS}:duration={CLIP_SECONDS}"),
        "-itsoffset",
        &audio_offset.to_string(),
        "-f",
        "lavfi",
        "-i",
        &format!("sine=frequency=440:sample_rate=48000:duration={audio_seconds}"),
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "pcm_s16le",
        &clip.to_string_lossy(),
    ]);
    clip
}

fn mean_luma(path: &Path) -> f64 {
    let rendered = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "rawvideo", "-pix_fmt", "gray", "-frames:v", "1", "-"])
        .output()
        .expect("ffmpeg");
    assert!(
        rendered.status.success(),
        "{}",
        String::from_utf8_lossy(&rendered.stderr)
    );
    assert!(
        !rendered.stdout.is_empty(),
        "{} has no frame",
        path.display()
    );
    rendered
        .stdout
        .iter()
        .map(|code| f64::from(*code))
        .sum::<f64>()
        / rendered.stdout.len() as f64
}

fn first_pixel(path: &Path) -> [f64; 3] {
    let rendered = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb48le",
            "-frames:v",
            "1",
            "-",
        ])
        .output()
        .expect("ffmpeg");
    assert!(
        rendered.status.success(),
        "{}",
        String::from_utf8_lossy(&rendered.stderr)
    );
    let bytes = &rendered.stdout;
    assert!(bytes.len() >= 6, "{} has no pixel", path.display());
    std::array::from_fn(|channel| {
        let code = u16::from_le_bytes([bytes[channel * 2], bytes[channel * 2 + 1]]);
        f64::from(code) / SIXTEEN_BIT_FULL_SCALE
    })
}

fn stdout_of(assert: assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).expect("the command wrote utf-8")
}

#[test]
fn compare_pixel_scores_every_frame_of_a_recompressed_clip() {
    let directory = TempDir::new().unwrap();
    let reference = reference_clip(directory.path());
    let distorted = recompressed(directory.path(), &reference, 25);

    let json = stdout_of(
        cmd()
            .args(["compare", "-a", &reference.to_string_lossy()])
            .args(["-b", &distorted.to_string_lossy()])
            .args(["--pixel", "--json"])
            .assert()
            .success(),
    );
    let result: Value = serde_json::from_str(&json).expect("the compare json");
    let scores = &result["psnr_ssim"];

    assert_eq!(
        scores["frames_compared"].as_u64(),
        Some(u64::from(CLIP_FRAMES))
    );
    assert_eq!(
        scores["per_frame"].as_array().map(Vec::len),
        Some(CLIP_FRAMES as usize)
    );

    let average_psnr = scores["avg_psnr"].as_f64().expect("avg_psnr");
    assert!(scores["min_psnr"].as_f64().unwrap() <= average_psnr);
    assert!(average_psnr <= scores["max_psnr"].as_f64().unwrap());
    assert!(
        average_psnr > 0.0 && average_psnr < 60.0,
        "a recompressed clip scored {average_psnr} dB"
    );

    let average_ssim = scores["avg_ssim"].as_f64().expect("avg_ssim");
    assert!(
        average_ssim > 0.5 && average_ssim < 1.0,
        "a recompressed clip scored {average_ssim} ssim"
    );
}

#[test]
fn compare_vmaf_ranks_the_better_encode_higher() {
    let directory = TempDir::new().unwrap();
    let reference = reference_clip(directory.path());

    let mut scores = Vec::new();
    for quantiser in [2, 25] {
        let distorted = recompressed(directory.path(), &reference, quantiser);
        let json = stdout_of(
            cmd()
                .args(["compare", "-a", &reference.to_string_lossy()])
                .args(["-b", &distorted.to_string_lossy()])
                .args(["--vmaf", "--json"])
                .assert()
                .success(),
        );
        let result: Value = serde_json::from_str(&json).expect("the vmaf json");
        let vmaf = &result["vmaf"];
        assert_eq!(vmaf["frames"].as_u64(), Some(u64::from(CLIP_FRAMES)));
        let mean = vmaf["mean"].as_f64().expect("mean");
        assert!(vmaf["min"].as_f64().unwrap() <= mean);
        assert!(mean <= vmaf["max"].as_f64().unwrap());
        scores.push(mean);
    }

    assert!(
        scores[0] > scores[1],
        "the finer quantiser scored {} against {}",
        scores[0],
        scores[1]
    );
}

#[test]
fn retime_rewrites_the_frame_rate_and_keeps_the_running_time() {
    let directory = TempDir::new().unwrap();
    let reference = reference_clip(directory.path());
    let retimed = directory.path().join("retimed.mkv");

    cmd()
        .args(["retime", "-i", &reference.to_string_lossy()])
        .args(["-o", &retimed.to_string_lossy()])
        .args(["-f", &RETIMED_FPS.to_string()])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Retimed to {RETIMED_FPS} fps"
        )));

    assert_eq!(
        video_stream_entry(&retimed, "r_frame_rate"),
        format!("{RETIMED_FPS}/1")
    );
    assert_eq!(
        video_stream_entry(&retimed, "nb_read_frames"),
        (CLIP_FRAMES * RETIMED_FPS / CLIP_FPS).to_string()
    );
}

#[test]
fn slate_prepends_black_frames_carrying_the_text() {
    let directory = TempDir::new().unwrap();
    let reference = reference_clip(directory.path());
    let frames = directory.path().join("slated_%04d.png");

    cmd()
        .args(["slate", "-i", &reference.to_string_lossy()])
        .args(["-o", &frames.to_string_lossy()])
        .args(["--text", "TEST SLATE"])
        .args(["--frames", &SLATE_FRAMES.to_string()])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Slate added ({SLATE_FRAMES} frames)"
        )));

    let written: Vec<PathBuf> = std::fs::read_dir(directory.path())
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "png"))
        .collect();
    assert_eq!(written.len() as u32, SLATE_FRAMES + CLIP_FRAMES);

    let first_slate = directory.path().join("slated_0001.png");
    let last_slate = directory
        .path()
        .join(format!("slated_{SLATE_FRAMES:04}.png"));
    let first_picture = directory
        .path()
        .join(format!("slated_{:04}.png", SLATE_FRAMES + 1));
    for slate in [&first_slate, &last_slate] {
        let luma = mean_luma(slate);
        assert!(
            luma > 0.0 && luma < SLATE_MAX_MEAN_LUMA,
            "{} averages {luma}, which is not black with text on it",
            slate.display()
        );
    }
    assert!(mean_luma(&first_picture) > PICTURE_MIN_MEAN_LUMA);
    assert_eq!(
        video_stream_entry(&first_slate, "width"),
        video_stream_entry(&reference, "width")
    );
}

// the milliseconds a labelled av-sync line reports
fn reported_milliseconds(report: &str, label: &str) -> f64 {
    let line = report
        .lines()
        .find(|line| line.trim_start().starts_with(label))
        .unwrap_or_else(|| panic!("no {label} line in\n{report}"));
    let (_, rest) = line.split_once(label).unwrap();
    let value = rest.split_whitespace().next().unwrap_or_default();
    value
        .trim_end_matches("ms")
        .parse()
        .unwrap_or_else(|e| panic!("{label} reads {value}: {e}"))
}

fn assert_milliseconds(report: &str, label: &str, expected: f64) {
    // the report prints one decimal, and a container clock rounds to its own tick
    const TOLERANCE_MILLISECONDS: f64 = 1.0;
    let reported = reported_milliseconds(report, label);
    assert!(
        (reported - expected).abs() < TOLERANCE_MILLISECONDS,
        "{label} reads {reported}ms, not {expected}ms, in\n{report}"
    );
}

#[test]
fn av_sync_passes_a_clip_whose_streams_start_and_end_together() {
    let directory = TempDir::new().unwrap();
    let clip = sync_clip(directory.path(), "sync.mov", 0.0, CLIP_SECONDS);

    let report = stdout_of(
        cmd()
            .args(["av-sync", "-i", &clip.to_string_lossy()])
            .assert()
            .success(),
    );
    assert_milliseconds(&report, "Initial offset:", 0.0);
    assert_milliseconds(&report, "Progressive drift over program:", 0.0);
    assert!(report.contains("Result: PASS"), "{report}");
}

#[test]
fn av_sync_separates_a_constant_offset_from_a_drift() {
    let directory = TempDir::new().unwrap();
    let offset_seconds = SYNC_ERROR_MILLISECONDS / 1000.0;

    // sound that starts late and runs the same length is offset, not drifting
    let offset = sync_clip(directory.path(), "offset.mov", offset_seconds, CLIP_SECONDS);
    let report = stdout_of(
        cmd()
            .args(["av-sync", "-i", &offset.to_string_lossy()])
            .assert()
            .failure(),
    );
    assert_milliseconds(&report, "Initial offset:", -SYNC_ERROR_MILLISECONDS);
    assert_milliseconds(&report, "Progressive drift over program:", 0.0);

    // sound that starts together and runs longer drifts apart by the end
    let drift = sync_clip(
        directory.path(),
        "drift.mov",
        0.0,
        CLIP_SECONDS + offset_seconds,
    );
    let report = stdout_of(
        cmd()
            .args(["av-sync", "-i", &drift.to_string_lossy()])
            .assert()
            .failure(),
    );
    assert_milliseconds(&report, "Initial offset:", 0.0);
    assert_milliseconds(&report, "End offset:", -SYNC_ERROR_MILLISECONDS);
    assert_milliseconds(
        &report,
        "Progressive drift over program:",
        -SYNC_ERROR_MILLISECONDS,
    );
}

#[test]
fn aces_falls_back_to_ffmpeg_when_ctlrender_is_missing() {
    let directory = TempDir::new().unwrap();
    let scene_linear = directory.path().join("aces.tiff");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=0x2E2E2E:s=64x64",
        "-frames:v",
        "1",
        "-pix_fmt",
        "gbrp16le",
        &scene_linear.to_string_lossy(),
    ]);
    let display = directory.path().join("rec709.tiff");

    cmd()
        .args(["aces", "-i", &scene_linear.to_string_lossy()])
        .args(["-o", &display.to_string_lossy()])
        .args(["--idt", "ACEScc", "--odt", "Rec709"])
        .assert()
        .success()
        .stderr(predicate::str::contains("ctlrender is not installed"));

    let source = first_pixel(&scene_linear);
    assert!(
        source[0] == source[1] && source[1] == source[2],
        "the fixture is not neutral: {source:?}"
    );
    let converted = first_pixel(&display);
    assert!(
        converted[0] == converted[1] && converted[1] == converted[2],
        "a neutral came out coloured: {converted:?}"
    );

    // a neutral keeps its value through the matrices, so only the transfer moves it
    let expected = source[0].powf(1.0 / REC709_DISPLAY_GAMMA);
    const TOLERANCE: f64 = 0.002;
    assert!(
        (converted[0] - expected).abs() < TOLERANCE,
        "linear {} came out at {}, not {expected}",
        source[0],
        converted[0]
    );
}

#[test]
fn doctor_reports_the_version_of_every_tool_it_finds() {
    let report = stdout_of(cmd().arg("doctor").assert().success());
    assert!(report.contains("[OK] ffmpeg"), "{report}");
    assert!(report.contains("ctlrender — NOT FOUND"), "{report}");

    let json = stdout_of(cmd().args(["doctor", "--json"]).assert().success());
    let result: Value = serde_json::from_str(&json).expect("the doctor json");
    let tools = result["tools"].as_array().expect("tools");

    let tool = |name: &str| {
        tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("doctor does not check {name}"))
            .clone()
    };

    let ffmpeg = tool("ffmpeg");
    assert_eq!(ffmpeg["status"], "available");
    assert_eq!(ffmpeg["required"], true);
    assert!(
        ffmpeg["version"].as_str().is_some_and(|v| !v.is_empty()),
        "no ffmpeg version was detected"
    );
    assert!(ffmpeg["path"].as_str().is_some());

    // nothing installs ctlrender, so the ACES pipeline reports it as optional
    let ctlrender = tool("ctlrender");
    assert_eq!(ctlrender["status"], "missing");
    assert_eq!(ctlrender["required"], false);

    assert_eq!(
        result["available"].as_u64().unwrap() + result["missing"].as_u64().unwrap(),
        tools.len() as u64
    );
    assert_eq!(result["required_missing"].as_u64(), Some(0));
}
