use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const RASTER: &str = "1920x1080";
const FRAMES_PER_SECOND: u32 = 24;
const SHORT_FRAMES: u32 = 6;
// two whole seconds of picture, so the per-second bitrate has more than one sample
const ANALYTICS_SECONDS: u32 = 2;
const ANALYTICS_FRAMES: u32 = FRAMES_PER_SECOND * ANALYTICS_SECONDS;
const HISTOGRAM_BUCKETS: usize = 4;
const AUDIO_SAMPLE_RATE: u32 = 48000;
// Netflix delivers 4K, so a 1920x1080 package has to be refused
const NETFLIX_WIDTH: u32 = 3840;

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

fn testsrc_clip(directory: &Path, name: &str, frames: u32, fps: u32) -> PathBuf {
    let clip = directory.join(name);
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("testsrc=size={RASTER}:rate={fps}"),
        "-frames:v",
        &frames.to_string(),
        "-c:v",
        "ffv1",
        "-pix_fmt",
        "yuv420p10le",
        &clip.to_string_lossy(),
    ]);
    clip
}

fn sine_wav(directory: &Path, name: &str, frames: u32, fps: u32) -> PathBuf {
    let wav = directory.join(name);
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("sine=frequency=1000:duration=10:sample_rate={AUDIO_SAMPLE_RATE}"),
        "-af",
        &format!("atrim=end_sample={}", frames * AUDIO_SAMPLE_RATE / fps),
        "-ac",
        "2",
        "-c:a",
        "pcm_s24le",
        &wav.to_string_lossy(),
    ]);
    wav
}

fn build_imp(directory: &Path, name: &str, title: &str, frames: u32, fps: u32) -> PathBuf {
    let clip = testsrc_clip(directory, &format!("{name}.mkv"), frames, fps);
    let wav = sine_wav(directory, &format!("{name}.wav"), frames, fps);
    let imp = directory.join(name);
    cmd()
        .args(["create", "-o", &imp.to_string_lossy()])
        .args(["-t", title])
        .args(["--video", &clip.to_string_lossy()])
        .args(["--raster", RASTER])
        .args(["--audio", &wav.to_string_lossy()])
        .args(["--audio-lang", "en-US"])
        .args(["--fps-num", &fps.to_string()])
        .args(["--fps-den", "1"])
        .assert()
        .success();
    imp
}

fn track_file(imp: &Path, prefix: &str) -> PathBuf {
    std::fs::read_dir(imp)
        .expect("the IMP directory")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix) && name.ends_with(".mxf"))
        })
        .unwrap_or_else(|| panic!("no {prefix} track file in {}", imp.display()))
}

fn cpl_uuid(imp: &Path) -> String {
    let cpl = std::fs::read_dir(imp)
        .expect("the IMP directory")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("CPL_") && name.ends_with(".xml"))
        })
        .expect("no CPL in the IMP");
    cpl.file_stem()
        .unwrap()
        .to_string_lossy()
        .trim_start_matches("CPL_")
        .to_string()
}

fn stdout_of(assert: assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).expect("the command wrote utf-8")
}

#[test]
fn analytics_reports_per_second_throughput_a_histogram_and_the_spread() {
    let directory = TempDir::new().unwrap();
    let imp = build_imp(
        directory.path(),
        "analytics",
        "Analytics",
        ANALYTICS_FRAMES,
        FRAMES_PER_SECOND,
    );

    let json = stdout_of(
        cmd()
            .args(["analytics", "-d", &imp.to_string_lossy()])
            .args(["--video", &track_file(&imp, "VIDEO_").to_string_lossy()])
            .args(["--histogram-buckets", &HISTOGRAM_BUCKETS.to_string()])
            .arg("--json")
            .assert()
            .success(),
    );
    let bitrate: Value = serde_json::from_str(&json).expect("the bitrate analytics json");

    let samples = bitrate["samples"].as_array().expect("samples");
    assert_eq!(samples.len(), ANALYTICS_SECONDS as usize);
    let throughput: Vec<f64> = samples
        .iter()
        .map(|sample| sample["bitrate_kbps"].as_f64().expect("bitrate_kbps"))
        .collect();
    assert!(
        throughput.iter().all(|kbps| *kbps > 0.0),
        "a second of picture carried no bits: {throughput:?}"
    );
    assert_eq!(
        bitrate["total_frames"].as_u64(),
        Some(u64::from(ANALYTICS_FRAMES))
    );

    let mean = throughput.iter().sum::<f64>() / throughput.len() as f64;
    let variance = throughput
        .iter()
        .map(|kbps| (kbps - mean).powi(2))
        .sum::<f64>()
        / throughput.len() as f64;
    assert_close(bitrate["avg_kbps"].as_f64().unwrap(), mean);
    assert_close(bitrate["stddev_kbps"].as_f64().unwrap(), variance.sqrt());
    assert_close(
        bitrate["min_kbps"].as_f64().unwrap(),
        throughput.iter().cloned().fold(f64::INFINITY, f64::min),
    );
    assert_close(
        bitrate["max_kbps"].as_f64().unwrap(),
        throughput.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
    );

    let histogram = bitrate["histogram"].as_array().expect("histogram");
    assert_eq!(histogram.len(), HISTOGRAM_BUCKETS);
    let counted: u64 = histogram
        .iter()
        .map(|bucket| bucket["count"].as_u64().expect("count"))
        .sum();
    assert_eq!(counted, samples.len() as u64);
}

// the analytics report the README shows, without a video to profile
#[test]
fn analytics_counts_the_tracks_of_an_imp() {
    let directory = TempDir::new().unwrap();
    let imp = build_imp(
        directory.path(),
        "summary",
        "Summary",
        SHORT_FRAMES,
        FRAMES_PER_SECOND,
    );

    cmd()
        .args(["analytics", "-d", &imp.to_string_lossy()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Video tracks: 1"))
        .stdout(predicate::str::contains("Audio tracks: 1"));

    let json = stdout_of(
        cmd()
            .args(["analytics", "-d", &imp.to_string_lossy(), "--json"])
            .assert()
            .success(),
    );
    let summary: Value = serde_json::from_str(&json).expect("the analytics json");
    assert_eq!(summary["video_tracks"].as_u64(), Some(1));
    assert_eq!(summary["audio_tracks"].as_u64(), Some(1));
    assert_eq!(summary["subtitle_tracks"].as_u64(), Some(0));
    assert_eq!(
        summary["total_size_bytes"].as_u64(),
        Some(directory_size(&imp))
    );
}

#[test]
fn compare_names_the_metadata_two_imps_disagree_on() {
    let directory = TempDir::new().unwrap();
    let first = build_imp(directory.path(), "first", "First cut", SHORT_FRAMES, 24);
    let second = build_imp(
        directory.path(),
        "second",
        "Second cut",
        SHORT_FRAMES * 2,
        25,
    );

    let output = stdout_of(
        cmd()
            .args(["compare", "-a", &first.to_string_lossy()])
            .args(["-b", &second.to_string_lossy()])
            .assert()
            .success(),
    );

    assert!(output.contains("First cut"), "{output}");
    assert!(output.contains("Second cut"), "{output}");
    // the sound track runs beside the picture rather than after it
    assert!(
        output.contains(&format!("CPLs: 1, Duration: {SHORT_FRAMES} frames")),
        "{output}"
    );
    assert!(
        output.contains(&format!("CPLs: 1, Duration: {} frames", SHORT_FRAMES * 2)),
        "{output}"
    );
    assert!(output.contains("DIFF: edit rate 24 1 vs 25 1"), "{output}");
    assert!(
        output.contains(&format!(
            "DIFF: duration {SHORT_FRAMES} vs {} frames",
            SHORT_FRAMES * 2
        )),
        "{output}"
    );
}

#[test]
fn annotate_writes_the_note_into_a_cpl_the_package_still_matches() {
    let directory = TempDir::new().unwrap();
    let imp = build_imp(
        directory.path(),
        "annotated",
        "Annotated",
        SHORT_FRAMES,
        FRAMES_PER_SECOND,
    );
    let note = "Color correction pass 2";

    cmd()
        .args(["annotate", "-i", &imp.to_string_lossy(), "-t", note])
        .assert()
        .success()
        .stdout(predicate::str::contains("now carries composition id"));

    let cpl = std::fs::read_to_string(imp.join(format!("CPL_{}.xml", cpl_uuid(&imp)))).unwrap();
    assert!(
        cpl.contains(&format!("<Annotation>{note}</Annotation>")),
        "the note is not in the CPL"
    );

    // the CPL changed, so the PKL hash and the ASSETMAP have to have moved with it
    cmd()
        .args(["validate", &imp.to_string_lossy()])
        .assert()
        .success()
        .stdout(predicate::str::contains("IMP validation PASSED"));
}

#[test]
fn metadata_edit_writes_the_note_and_author_the_package_still_matches() {
    let directory = TempDir::new().unwrap();
    let imp = build_imp(
        directory.path(),
        "edited",
        "Edited",
        SHORT_FRAMES,
        FRAMES_PER_SECOND,
    );
    let note = "Conform check passed";
    let author = "QC desk";

    cmd()
        .args([
            "metadata-edit",
            "-i",
            &imp.to_string_lossy(),
            "-a",
            note,
            "--issuer",
            author,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("now carries composition id"));

    let cpl = std::fs::read_to_string(imp.join(format!("CPL_{}.xml", cpl_uuid(&imp)))).unwrap();
    assert!(
        cpl.contains(&format!("<Annotation>{note}</Annotation>")),
        "the note is not in the CPL"
    );
    assert!(
        cpl.contains(&format!("<Issuer>{author}</Issuer>")),
        "the author is not in the CPL"
    );

    // the CPL changed, so the PKL hash and the ASSETMAP have to have moved with it
    cmd()
        .args(["validate", &imp.to_string_lossy()])
        .assert()
        .success()
        .stdout(predicate::str::contains("IMP validation PASSED"));
}

#[test]
fn partial_version_copies_the_track_files_the_cpl_names() {
    let directory = TempDir::new().unwrap();
    let imp = build_imp(
        directory.path(),
        "whole",
        "Whole",
        SHORT_FRAMES,
        FRAMES_PER_SECOND,
    );
    let partial = directory.path().join("partial");
    let uuid = cpl_uuid(&imp);

    cmd()
        .args(["partial-version", "-i", &imp.to_string_lossy()])
        .args(["-o", &partial.to_string_lossy()])
        .args(["--cpl", &uuid])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 track file(s) copied"));

    for prefix in ["VIDEO_", "AUDIO_"] {
        let source = track_file(&imp, prefix);
        let copied = partial.join(source.file_name().unwrap());
        assert_eq!(
            std::fs::read(&source).unwrap(),
            std::fs::read(&copied).unwrap(),
            "{prefix} did not arrive whole"
        );
    }
    assert!(partial.join(format!("CPL_{uuid}.xml")).is_file());
    assert!(partial.join("ASSETMAP.xml").is_file());

    // the new PKL and ASSETMAP cover the copied assets, so the copy is an IMP
    cmd()
        .args(["validate", &partial.to_string_lossy()])
        .assert()
        .success()
        .stdout(predicate::str::contains("IMP validation PASSED"));
}

#[test]
fn partial_version_names_a_cpl_the_imp_does_not_hold() {
    let directory = TempDir::new().unwrap();
    let imp = build_imp(
        directory.path(),
        "whole",
        "Whole",
        SHORT_FRAMES,
        FRAMES_PER_SECOND,
    );
    let missing = "00000000-0000-0000-0000-000000000000";

    cmd()
        .args(["partial-version", "-i", &imp.to_string_lossy()])
        .args(["-o", &directory.path().join("partial").to_string_lossy()])
        .args(["--cpl", missing])
        .assert()
        .failure()
        .stderr(predicate::str::contains(missing));
}

#[test]
fn compliance_checks_the_platform_the_standard_names() {
    let directory = TempDir::new().unwrap();
    let imp = build_imp(
        directory.path(),
        "delivery",
        "Delivery",
        SHORT_FRAMES,
        FRAMES_PER_SECOND,
    );
    let imp = imp.to_string_lossy().into_owned();

    // a 1920x1080 package misses the 4K raster both streaming profiles require
    for (standard, name) in [("netflix", "Netflix IMF"), ("amazon", "Amazon Prime IMF")] {
        cmd()
            .args(["compliance", "-i", &imp, "-s", standard])
            .assert()
            .failure()
            .stdout(predicate::str::contains(format!(
                "Checking compliance against: {name}"
            )))
            .stdout(predicate::str::contains(format!(
                "width 1920 != required {NETFLIX_WIDTH}"
            )))
            .stdout(predicate::str::contains(format!(
                "FAIL: not compliant with {name}"
            )));
    }

    // ST 2067 fixes no raster, so smpte is the structural check on its own
    cmd()
        .args(["compliance", "-i", &imp, "-s", "smpte"])
        .assert()
        .success()
        .stdout(predicate::str::contains("SMPTE ST 2067 structure"))
        .stdout(predicate::str::contains("width 1920").not());

    cmd()
        .args(["compliance", "-i", &imp, "-s", "betamax"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown standard: betamax"))
        .stderr(predicate::str::contains("netflix"));
}

fn directory_size(directory: &Path) -> u64 {
    std::fs::read_dir(directory)
        .expect("the directory")
        .flatten()
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum()
}

// the statistics come back as printed decimals, so they land near the value
fn assert_close(reported: f64, computed: f64) {
    const TOLERANCE_KBPS: f64 = 0.001;
    assert!(
        (reported - computed).abs() < TOLERANCE_KBPS,
        "reported {reported}, the samples give {computed}"
    );
}
