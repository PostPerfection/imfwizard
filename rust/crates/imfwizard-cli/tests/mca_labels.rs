use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FRAMES: u32 = 1;
const FRAMES_PER_SECOND: u32 = 24;
const SAMPLE_RATE: u32 = 48000;
const CHANNELS: u32 = 6;
const TITLE: &str = "Labelled Feature";
const LANGUAGE: &str = "fr-CA";

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

// a 5.1 IMP, whose sound track file `create` already labelled in English
fn create_imp(work: &Path) -> PathBuf {
    let clip = work.join("source.mkv");
    run_ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("color=c=red:s={WIDTH}x{HEIGHT}:r={FRAMES_PER_SECOND}"),
        "-frames:v",
        &FRAMES.to_string(),
        "-c:v",
        "ffv1",
        "-pix_fmt",
        "gbrp",
        &clip.to_string_lossy(),
    ]);
    let sound = work.join("sound.wav");
    run_ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("anullsrc=r={SAMPLE_RATE}:cl=5.1"),
        "-t",
        "0.5",
        "-c:a",
        "pcm_s24le",
        &sound.to_string_lossy(),
    ]);

    let imp = work.join("imp");
    cmd()
        .args([
            "create",
            "-o",
            &imp.to_string_lossy(),
            "-t",
            TITLE,
            "--video",
            &clip.to_string_lossy(),
            "--audio",
            &sound.to_string_lossy(),
            "--audio-lang",
            "en",
            "--fps-num",
            &FRAMES_PER_SECOND.to_string(),
            "--fps-den",
            "1",
        ])
        .assert()
        .success();
    imp
}

fn only_file_starting_with(dir: &Path, prefix: &str) -> PathBuf {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix))
        })
        .collect();
    assert_eq!(found.len(), 1, "expected one {prefix} file in {dir:?}");
    found.pop().unwrap()
}

fn read_labels(mxf: &Path) -> Vec<asdcplib::pcm::McaLabelSubDescriptor> {
    let mut reader = asdcplib::as02::pcm::MxfReader::new();
    reader
        .open_read(
            &mxf.to_string_lossy(),
            asdcplib::Rational::new(FRAMES_PER_SECOND as i32, 1),
        )
        .expect("the sound MXF opens");
    assert_eq!(
        reader.channel_assignment().expect("channel assignment"),
        Some(asdcplib::as02::pcm::IMF_CHANNEL_ASSIGNMENT_MCA),
        "the track file no longer signals MCA channel assignment"
    );
    reader
        .mca_label_subdescriptors()
        .expect("the mca subdescriptors")
}

#[test]
fn mca_writes_the_labels_into_the_sound_mxf_and_the_package_still_validates() {
    let work = TempDir::new().unwrap();
    let imp = create_imp(work.path());
    let audio = only_file_starting_with(&imp, "AUDIO_");

    let before = read_labels(&audio);
    let language_before = before
        .iter()
        .find(|label| label.kind == asdcplib::pcm::McaLabelKind::SoundfieldGroup)
        .and_then(|label| label.spoken_language.clone());
    assert_eq!(
        language_before.as_deref(),
        Some("en"),
        "create did not label the sound in the language it was given"
    );

    cmd()
        .args([
            "mca",
            "-i",
            &audio.to_string_lossy(),
            "-l",
            "51",
            "-L",
            LANGUAGE,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("MCA labels written into"));

    let labels = read_labels(&audio);
    assert_eq!(
        labels.len(),
        CHANNELS as usize + 1,
        "one label a channel plus the soundfield group: {labels:?}"
    );
    let group = labels
        .iter()
        .find(|label| label.kind == asdcplib::pcm::McaLabelKind::SoundfieldGroup)
        .expect("a soundfield group label");
    assert_eq!(group.spoken_language.as_deref(), Some(LANGUAGE));
    assert_eq!(
        group.title.as_deref(),
        Some(TITLE),
        "the rewrap dropped what the group said about the work"
    );
    let channels: Vec<u32> = labels.iter().filter_map(|label| label.channel_id).collect();
    assert_eq!(channels, vec![1, 2, 3, 4, 5, 6]);

    // the package documents describe the track file the rewrap wrote
    cmd()
        .args(["validate", &imp.to_string_lossy()])
        .assert()
        .success()
        .stdout(predicate::str::contains("IMP validation PASSED"));
}

#[test]
fn a_layout_the_track_file_cannot_carry_is_refused_naming_both() {
    let work = TempDir::new().unwrap();
    let imp = create_imp(work.path());
    let audio = only_file_starting_with(&imp, "AUDIO_");
    let before = std::fs::metadata(&audio).unwrap().len();

    cmd()
        .args(["mca", "-i", &audio.to_string_lossy(), "-l", "stereo"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("carries 6 channels")
                .and(predicate::str::contains("--layout stereo names 2")),
        );

    assert_eq!(
        std::fs::metadata(&audio).unwrap().len(),
        before,
        "the refused run rewrote the track file"
    );
}

#[test]
fn an_unknown_layout_lists_the_ones_there_are() {
    let work = TempDir::new().unwrap();
    let imp = create_imp(work.path());
    let audio = only_file_starting_with(&imp, "AUDIO_");

    cmd()
        .args(["mca", "-i", &audio.to_string_lossy(), "-l", "51+HI+VI"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("Unknown layout: 51+HI+VI")
                .and(predicate::str::contains("mono, 10, stereo, 20, 51, 71")),
        );
}

#[test]
fn a_sound_mxf_outside_a_package_is_labelled_on_its_own() {
    let work = TempDir::new().unwrap();
    let imp = create_imp(work.path());
    let loose = work.path().join("loose.mxf");
    std::fs::copy(only_file_starting_with(&imp, "AUDIO_"), &loose).unwrap();

    cmd()
        .args([
            "mca",
            "-i",
            &loose.to_string_lossy(),
            "-l",
            "51",
            "-L",
            LANGUAGE,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Rewritten:").not());

    let labels = read_labels(&loose);
    let group = labels
        .iter()
        .find(|label| label.kind == asdcplib::pcm::McaLabelKind::SoundfieldGroup)
        .expect("a soundfield group label");
    assert_eq!(group.spoken_language.as_deref(), Some(LANGUAGE));
}
