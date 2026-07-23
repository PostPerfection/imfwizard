use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

#[test]
fn version_flag() {
    cmd().arg("--version").assert().success().stdout(
        predicate::str::contains("imfwizard")
            .and(predicate::str::contains(env!("CARGO_PKG_VERSION"))),
    );
}

#[test]
fn help_flag() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("create"))
        .stdout(predicate::str::contains("encode"))
        .stdout(predicate::str::contains("analytics"))
        .stdout(predicate::str::contains("profiles"));
}

#[test]
fn create_subcommand_help() {
    cmd()
        .args(["create", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Create"));
}

#[test]
fn encode_subcommand_help() {
    cmd()
        .args(["encode", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Encode"));
}

#[test]
fn profiles_lists_output() {
    cmd().args(["profiles"]).assert().success();
}

#[test]
fn analyze_missing_directory() {
    let dir = TempDir::new().unwrap();
    let nonexistent = dir.path().join("does_not_exist");

    cmd()
        .args(["analyze", nonexistent.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn analyze_empty_directory() {
    let dir = TempDir::new().unwrap();

    cmd()
        .args(["analyze", dir.path().to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn hash_missing_file() {
    cmd()
        .args(["hash", "/nonexistent/file.mxf"])
        .assert()
        .failure();
}

#[test]
fn hash_existing_file() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("test.bin");
    std::fs::write(&file, b"hello world").unwrap();

    cmd()
        .args(["hash", file.to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn timecode_conversion() {
    cmd()
        .args(["timecode", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("timecode"));
}

#[test]
fn verbose_flag_accepted() {
    cmd().args(["-v", "profiles"]).assert().success();
}

#[test]
fn create_requires_input() {
    // create without arguments should fail
    cmd().args(["create"]).assert().failure();
}

#[test]
fn transcode_subcommand_help() {
    cmd()
        .args(["transcode", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Transcode"));
}

#[test]
fn subtitle_convert_help() {
    cmd()
        .args(["subtitle-convert", "--help"])
        .assert()
        .success();
}

// write a mono 16-bit 48k WAV of a 1 kHz sine at the given peak amplitude (0..1)
fn write_sine_wav(path: &std::path::Path, amplitude: f64) {
    let sample_rate = 48_000u32;
    let samples: Vec<i16> = (0..sample_rate)
        .map(|n| {
            let t = n as f64 / sample_rate as f64;
            let v = amplitude * (2.0 * std::f64::consts::PI * 1000.0 * t).sin();
            (v * i16::MAX as f64).round() as i16
        })
        .collect();
    let data_bytes = samples.len() * 2;
    let mut buf = Vec::with_capacity(44 + data_bytes);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&((36 + data_bytes) as u32).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // mono
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    buf.extend_from_slice(&2u16.to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    for s in samples {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, buf).unwrap();
}

#[test]
fn loudness_adjust_to_target_writes_output() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("in.wav");
    let output = dir.path().join("out.wav");
    write_sine_wav(&input, 0.25);
    cmd()
        .args([
            "loudness",
            input.to_str().unwrap(),
            "--adjust-to",
            "-24",
            "-o",
            output.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Gain applied"))
        .stdout(predicate::str::contains("Adjusted audio written"));
    assert!(output.exists(), "adjusted wav should exist");
}

#[test]
fn loudness_adjust_refuses_when_true_peak_would_clip() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("quiet.wav");
    let output = dir.path().join("loud.wav");
    // quiet source + loud target forces a large positive gain that breaches the ceiling
    write_sine_wav(&input, 0.05);
    cmd()
        .args([
            "loudness",
            input.to_str().unwrap(),
            "--adjust-to",
            "-3",
            "-o",
            output.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("true-peak ceiling exceeded"));
    assert!(!output.exists(), "clip-safe: nothing written on breach");
}
