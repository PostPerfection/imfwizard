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

fn samples(path: &Path) -> Vec<i32> {
    hound::WavReader::open(path)
        .unwrap()
        .samples::<i32>()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn a_negative_duck_level_parses_and_reaches_the_mix() {
    let directory = tempfile::TempDir::new().unwrap();
    let main = tone(directory.path(), "main.wav", 200);
    let narration = tone(directory.path(), "narration.wav", 1000);
    let default_duck = directory.path().join("default_duck.wav");
    let light_duck = directory.path().join("light_duck.wav");

    for (output, duck_level) in [(&default_duck, None), (&light_duck, Some("-6"))] {
        let mut run = Command::cargo_bin("imfwizard").unwrap();
        run.arg("audio-desc")
            .arg("-i")
            .arg(&main)
            .arg("--narration")
            .arg(&narration)
            .arg("-o")
            .arg(output);
        if let Some(duck_level) = duck_level {
            run.args(["--duck-level", duck_level]);
        }
        run.assert().success();
    }

    assert_ne!(
        samples(&default_duck),
        samples(&light_duck),
        "--duck-level -6 mixed the same as the -12 default"
    );
}
