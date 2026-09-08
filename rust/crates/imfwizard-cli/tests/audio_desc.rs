use assert_cmd::Command;
use std::path::{Path, PathBuf};

const SAMPLE_RATE: u32 = 48_000;
const CLIP_SECONDS: u32 = 4;

fn tone(directory: &Path, name: &str, hertz: u32) -> PathBuf {
    let path = directory.join(name);
    let source =
        format!("sine=frequency={hertz}:duration={CLIP_SECONDS}:sample_rate={SAMPLE_RATE}");
    let run = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", &source])
        .args(["-c:a", "pcm_s24le"])
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

#[test]
fn audio_desc_writes_the_mix_it_was_asked_for() {
    let directory = tempfile::TempDir::new().unwrap();
    let main = tone(directory.path(), "main.wav", 200);
    let narration = tone(directory.path(), "narration.wav", 1000);
    let output = directory.path().join("mixed.wav");

    Command::cargo_bin("imfwizard")
        .unwrap()
        .arg("audio-desc")
        .arg("-i")
        .arg(&main)
        .arg("--narration")
        .arg(&narration)
        .arg("-o")
        .arg(&output)
        .assert()
        .success();

    let mix = hound::WavReader::open(&output).unwrap();
    assert_eq!(
        mix.duration(),
        hound::WavReader::open(&main).unwrap().duration()
    );
}
