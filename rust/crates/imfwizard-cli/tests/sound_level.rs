use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const FRAMES_PER_SECOND: u32 = 24;
// one second of picture, long enough for an integrated loudness reading
const FRAMES: u32 = 24;
const TONE_AMPLITUDE: f64 = 0.5;

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

fn run_ffmpeg(arguments: &[&str]) {
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(arguments)
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
}

fn source_clip(work: &Path) -> PathBuf {
    let clip = work.join("source.mkv");
    run_ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("color=c=blue:s=1920x1080:r={FRAMES_PER_SECOND}"),
        "-frames:v",
        &FRAMES.to_string(),
        "-c:v",
        "ffv1",
        "-pix_fmt",
        "gbrp",
        &clip.to_string_lossy(),
    ]);
    clip
}

fn tone(work: &Path) -> PathBuf {
    let wav = work.join("tone.wav");
    let channel = format!("{TONE_AMPLITUDE}*sin(2*PI*1000*t)");
    run_ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("aevalsrc={channel}|{channel}:s=48000:d=1"),
        "-c:a",
        "pcm_s24le",
        &wav.to_string_lossy(),
    ]);
    wav
}

// the one sound track file of the package
fn packaged_sound(imp: &Path) -> PathBuf {
    let mut found: Vec<PathBuf> = std::fs::read_dir(imp)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("AUDIO_"))
        })
        .collect();
    assert_eq!(found.len(), 1, "expected one sound track file in {imp:?}");
    found.pop().unwrap()
}

// postkit reads only AS-DCP sound, so ffmpeg unwraps the AS-02 track file
fn packaged_sound_as_wav(imp: &Path, work: &Path) -> PathBuf {
    let wav = work.join("packaged.wav");
    run_ffmpeg(&[
        "-i",
        &packaged_sound(imp).to_string_lossy(),
        "-c:a",
        "pcm_s24le",
        &wav.to_string_lossy(),
    ]);
    wav
}

fn create_with(work: &Path, level_flags: &[&str]) -> PathBuf {
    let clip = source_clip(work);
    let sound = tone(work);
    let imp = work.join("imp");
    cmd()
        .args([
            "create",
            "-o",
            &imp.to_string_lossy(),
            "-t",
            "Sound Level",
            "--video",
            &clip.to_string_lossy(),
            "--audio",
            &sound.to_string_lossy(),
            "--fps-num",
            &FRAMES_PER_SECOND.to_string(),
            "--fps-den",
            "1",
        ])
        .args(level_flags)
        .assert()
        .success()
        .stdout(predicate::str::contains("IMP validation PASSED"));
    packaged_sound_as_wav(&imp, work)
}

#[test]
fn a_negative_gain_lowers_the_packaged_sound() {
    const GAIN_DB: f64 = -6.0;
    let work = TempDir::new().unwrap();
    let packaged = create_with(work.path(), &["--audio-gain", "-6"]);

    let peak = postkit::loudness::measure_true_peak_dbtp(&packaged).unwrap();
    let source_peak = 20.0 * TONE_AMPLITUDE.log10();
    assert!(
        (peak - source_peak - GAIN_DB).abs() < 0.2,
        "packaged peak {peak} dBTP, the tone peaks at {source_peak} dBFS"
    );
}

#[test]
fn a_loudness_target_brings_the_packaged_sound_to_it() {
    const TARGET_LUFS: f64 = -30.0;
    let work = TempDir::new().unwrap();
    let packaged = create_with(work.path(), &["--loudness-target", "lufs=-30"]);

    let measured = postkit::loudness::measure_loudness(&packaged);
    assert!(measured.success, "{}", measured.error);
    assert!(
        (measured.integrated_lufs - TARGET_LUFS).abs() < 0.2,
        "packaged sound at {} LUFS",
        measured.integrated_lufs
    );
}
