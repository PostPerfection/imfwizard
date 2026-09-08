use imfwizard_core::audio_desc::mix_audio_description;
use std::path::{Path, PathBuf};

const SAMPLE_RATE: u32 = 48_000;
const MAIN_HZ: u32 = 200;
const NARRATION_HZ: u32 = 1_000;
const CLIP_SECONDS: u32 = 6;
const NARRATION_START_SECONDS: u32 = 2;
const NARRATION_SECONDS: u32 = 2;

// ffmpeg's sine filter peaks at 0.125, so this puts both tones at -20 dBFS
const SINE_TO_MINUS_20_DBFS: &str = "volume=0.8";

const DUCK_DB: f64 = -12.0;
const THRESHOLD_DB: f64 = -30.0;
const ATTACK_MS: f64 = 20.0;
const RELEASE_MS: f64 = 200.0;

// a second inside the narration's silence, and a second inside the narration
const SILENT_WINDOW: (f64, f64) = (0.5, 1.5);
const NARRATED_WINDOW: (f64, f64) = (2.5, 3.5);

const BANDPASS_Q: f64 = 4.0;

fn sine(directory: &Path, name: &str, hertz: u32, filter: &str) -> PathBuf {
    let path = directory.join(name);
    let source =
        format!("sine=frequency={hertz}:duration={CLIP_SECONDS}:sample_rate={SAMPLE_RATE}");
    let run = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", &source])
        .args(["-af", filter, "-c:a", "pcm_s24le"])
        .arg(&path)
        .output()
        .expect("run ffmpeg");
    assert!(
        run.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    path
}

fn main_audio(directory: &Path) -> PathBuf {
    sine(directory, "main.wav", MAIN_HZ, SINE_TO_MINUS_20_DBFS)
}

fn narration(directory: &Path) -> PathBuf {
    let delay_ms = NARRATION_START_SECONDS * 1000;
    let filter = format!(
        "atrim=duration={NARRATION_SECONDS},{SINE_TO_MINUS_20_DBFS},\
         adelay={delay_ms},apad=whole_dur={CLIP_SECONDS}"
    );
    sine(directory, "narration.wav", NARRATION_HZ, &filter)
}

fn tone_level_db(path: &Path, hertz: u32, window: (f64, f64)) -> f64 {
    let (start, end) = window;
    // one 200 Hz bandpass leaks the 1 kHz narration back in at -20 dB, which reads a 12 dB duck as 11.3
    let bandpass = format!("bandpass=f={hertz}:width_type=q:w={BANDPASS_Q}");
    let filter =
        format!("{bandpass},{bandpass},atrim=start={start}:end={end},astats=metadata=1:reset=0");
    let run = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(path)
        .args(["-af", &filter, "-f", "null", "-"])
        .output()
        .expect("run ffmpeg");
    let report = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "astats failed: {report}");
    let measured = report
        .lines()
        .filter_map(|line| line.split("RMS level dB:").nth(1))
        .next_back()
        .unwrap_or_else(|| panic!("no RMS level in: {report}"));
    measured.trim().parse().expect("RMS level is a number")
}

fn mixed(directory: &Path) -> PathBuf {
    let output = directory.join("mixed.wav");
    mix_audio_description(
        &main_audio(directory),
        &narration(directory),
        &output,
        DUCK_DB,
        THRESHOLD_DB,
        ATTACK_MS,
        RELEASE_MS,
    )
    .expect("mix");
    output
}

#[test]
fn the_main_mix_is_untouched_while_the_narration_is_silent() {
    let directory = tempfile::tempdir().unwrap();
    let output = mixed(directory.path());
    let before = tone_level_db(&main_audio(directory.path()), MAIN_HZ, SILENT_WINDOW);
    let after = tone_level_db(&output, MAIN_HZ, SILENT_WINDOW);
    assert!(
        (after - before).abs() < 0.5,
        "main was {before} dB and came back {after} dB"
    );
}

#[test]
fn the_main_mix_drops_by_the_duck_level_while_the_narration_plays() {
    let directory = tempfile::tempdir().unwrap();
    let output = mixed(directory.path());
    let before = tone_level_db(&main_audio(directory.path()), MAIN_HZ, NARRATED_WINDOW);
    let after = tone_level_db(&output, MAIN_HZ, NARRATED_WINDOW);
    let reduction = before - after;
    assert!(
        (reduction - DUCK_DB.abs()).abs() < 1.5,
        "asked for {DUCK_DB} dB, got {reduction} dB ({before} dB down to {after} dB)"
    );
}

#[test]
fn the_narration_is_mixed_at_unity_gain() {
    let directory = tempfile::tempdir().unwrap();
    let output = mixed(directory.path());
    let before = tone_level_db(&narration(directory.path()), NARRATION_HZ, NARRATED_WINDOW);
    let after = tone_level_db(&output, NARRATION_HZ, NARRATED_WINDOW);
    assert!(
        (after - before).abs() < 1.0,
        "narration was {before} dB and came back {after} dB"
    );
}

#[test]
fn the_mix_is_as_long_as_the_main_audio() {
    let directory = tempfile::tempdir().unwrap();
    let output = mixed(directory.path());
    let main = hound::WavReader::open(main_audio(directory.path())).unwrap();
    let mix = hound::WavReader::open(&output).unwrap();
    assert_eq!(mix.duration(), main.duration());
}

#[test]
fn a_narration_that_is_not_there_is_reported_in_ffmpegs_own_words() {
    let directory = tempfile::tempdir().unwrap();
    let error = mix_audio_description(
        &main_audio(directory.path()),
        &directory.path().join("absent.wav"),
        &directory.path().join("mixed.wav"),
        DUCK_DB,
        THRESHOLD_DB,
        ATTACK_MS,
        RELEASE_MS,
    )
    .unwrap_err();
    assert!(error.contains("No such file"), "got: {error}");
}
