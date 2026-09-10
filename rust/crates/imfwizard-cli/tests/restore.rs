use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const RASTER: &str = "1920x1080";
const FRAMES_PER_SECOND: u32 = 24;
const FRAMES: u32 = 3;
const AUDIO_SAMPLE_RATE: u32 = 48_000;
const AUDIO_CHANNELS: u16 = 2;
const SAMPLES_PER_FRAME: u32 = AUDIO_SAMPLE_RATE / FRAMES_PER_SECOND;

// SOC then SIZ, the two markers every JPEG 2000 codestream opens with
const CODESTREAM_HEAD: [u8; 4] = [0xFF, 0x4F, 0xFF, 0x51];

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
        &AUDIO_CHANNELS.to_string(),
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

fn restore(imp: &Path, output: &Path, extra: &[&str]) {
    cmd()
        .args(["restore", "-i", &imp.to_string_lossy()])
        .args(["-o", &output.to_string_lossy()])
        .args(extra)
        .assert()
        .success();
}

fn files_with_extension(root: &Path, extension: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|found| found.eq_ignore_ascii_case(extension))
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[test]
fn restore_writes_a_codestream_per_frame_and_a_wav_per_sound_track() {
    let directory = TempDir::new().unwrap();
    let imp = build_imp(
        directory.path(),
        "restore",
        "Restore",
        FRAMES,
        FRAMES_PER_SECOND,
    );
    let output = directory.path().join("restored");

    restore(&imp, &output, &[]);

    let codestreams = files_with_extension(&output, "j2c");
    assert_eq!(
        codestreams.len(),
        FRAMES as usize,
        "one codestream a frame, found {codestreams:?}"
    );
    for codestream in &codestreams {
        let bytes = std::fs::read(codestream).unwrap();
        assert_eq!(
            &bytes[..CODESTREAM_HEAD.len()],
            &CODESTREAM_HEAD,
            "{} does not open a JPEG 2000 codestream",
            codestream.display()
        );
    }

    let wavs = files_with_extension(&output, "wav");
    assert_eq!(wavs.len(), 1, "one wav a sound track, found {wavs:?}");
    let reader = hound::WavReader::open(&wavs[0]).expect("the restored wav");
    assert_eq!(reader.spec().channels, AUDIO_CHANNELS);
    assert_eq!(reader.spec().sample_rate, AUDIO_SAMPLE_RATE);
    assert_eq!(reader.duration(), FRAMES * SAMPLES_PER_FRAME);
}

#[test]
fn video_only_writes_no_wav_and_audio_only_writes_no_codestream() {
    let directory = TempDir::new().unwrap();
    let imp = build_imp(
        directory.path(),
        "one-kind",
        "One Kind",
        FRAMES,
        FRAMES_PER_SECOND,
    );

    let picture = directory.path().join("picture-only");
    restore(&imp, &picture, &["--video-only"]);
    assert_eq!(files_with_extension(&picture, "j2c").len(), FRAMES as usize);
    assert!(files_with_extension(&picture, "wav").is_empty());

    let sound = directory.path().join("sound-only");
    restore(&imp, &sound, &["--audio-only"]);
    assert_eq!(files_with_extension(&sound, "wav").len(), 1);
    assert!(files_with_extension(&sound, "j2c").is_empty());
}

#[test]
fn asking_for_both_kinds_at_once_is_refused() {
    let directory = TempDir::new().unwrap();
    cmd()
        .args(["restore", "-i", &directory.path().to_string_lossy()])
        .args(["-o", &directory.path().join("out").to_string_lossy()])
        .args(["--video-only", "--audio-only"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "--video-only and --audio-only ask for different tracks",
        ));
}
