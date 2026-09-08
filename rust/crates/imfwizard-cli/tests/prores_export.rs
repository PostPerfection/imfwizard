use assert_cmd::Command;
use predicates::prelude::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const RASTER: &str = "1920x1080";
const FRAMES: u32 = 6;
const FPS: u32 = 24;

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

fn testsrc_clip(dir: &Path) -> PathBuf {
    let clip = dir.join("testsrc.mkv");
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi"])
        .args(["-i", &format!("testsrc=size={RASTER}:rate={FPS}")])
        .args(["-frames:v", &FRAMES.to_string()])
        .args(["-c:v", "ffv1", "-pix_fmt", "yuv420p10le"])
        .arg(&clip)
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    clip
}

fn sine_wav(dir: &Path) -> PathBuf {
    let wav = dir.join("sine.wav");
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi"])
        .args(["-i", "sine=frequency=1000:duration=10:sample_rate=48000"])
        .args(["-af", &format!("atrim=end_sample={}", FRAMES * 48000 / FPS)])
        .args(["-ac", "2", "-c:a", "pcm_s24le"])
        .arg(&wav)
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    wav
}

fn build_sound_imp(dir: &Path, name: &str) -> PathBuf {
    let clip = testsrc_clip(dir);
    let wav = sine_wav(dir);
    let imp = dir.join(name);
    cmd()
        .args([
            "create",
            "-o",
            &imp.to_string_lossy(),
            "-t",
            "ProRes export",
            "--video",
            &clip.to_string_lossy(),
            "--raster",
            RASTER,
            "--audio",
            &wav.to_string_lossy(),
            "--audio-lang",
            "en-US",
            "--fps-num",
            &FPS.to_string(),
            "--fps-den",
            "1",
        ])
        .assert()
        .success();
    imp
}

fn imp_digests(imp: &Path) -> BTreeMap<String, String> {
    std::fs::read_dir(imp)
        .expect("the IMP directory")
        .map(|entry| {
            let path = entry.expect("a directory entry").path();
            let digest = std::process::Command::new("sha1sum")
                .arg(&path)
                .output()
                .expect("sha1sum");
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let sum = String::from_utf8_lossy(&digest.stdout)
                .split_whitespace()
                .next()
                .expect("a digest")
                .to_string();
            (name, sum)
        })
        .collect()
}

fn probe(movie: &Path, kind: &str, entry: &str) -> String {
    let output = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", kind])
        .args(["-show_entries", &format!("stream={entry}")])
        .args(["-of", "csv=p=0"])
        .arg(movie)
        .output()
        .expect("ffprobe");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn assert_is_prores_4444_with_sound(movie: &Path) {
    assert_eq!(probe(movie, "v:0", "codec_name"), "prores");
    assert_eq!(probe(movie, "v:0", "profile"), "4444");
    assert_eq!(probe(movie, "v:0", "nb_frames"), FRAMES.to_string());
    assert_eq!(probe(movie, "a:0", "codec_name"), "pcm_s24le");
    assert_eq!(probe(movie, "a:0", "channels"), "2");
}

#[test]
fn an_imp_exports_into_the_2k_container_without_being_touched() {
    let dir = TempDir::new().unwrap();
    let imp = build_sound_imp(dir.path(), "imp_2k");
    let before = imp_digests(&imp);
    let movie = dir.path().join("out2k.mov");

    cmd()
        .args(["prores", "-i", &imp.to_string_lossy()])
        .args(["-o", &movie.to_string_lossy()])
        .args(["--container", "2k"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ProRes 4444 exported"));

    assert_is_prores_4444_with_sound(&movie);
    assert_eq!(probe(&movie, "v:0", "width"), "2048");
    assert_eq!(probe(&movie, "v:0", "height"), "1080");
    assert_eq!(imp_digests(&imp), before, "the export changed the IMP");
}

#[test]
fn the_4k_container_pads_the_picture_to_4096x2160() {
    let dir = TempDir::new().unwrap();
    let imp = build_sound_imp(dir.path(), "imp_4k");
    let movie = dir.path().join("out4k.mov");

    cmd()
        .args(["prores", "-i", &imp.to_string_lossy()])
        .args(["-o", &movie.to_string_lossy()])
        .args(["--container", "4k"])
        .assert()
        .success();

    assert_is_prores_4444_with_sound(&movie);
    assert_eq!(probe(&movie, "v:0", "width"), "4096");
    assert_eq!(probe(&movie, "v:0", "height"), "2160");
}

#[test]
fn without_a_container_the_picture_keeps_its_own_raster() {
    let dir = TempDir::new().unwrap();
    let imp = build_sound_imp(dir.path(), "imp_raster");
    let movie = dir.path().join("out.mov");

    cmd()
        .args(["prores", "-i", &imp.to_string_lossy()])
        .args(["-o", &movie.to_string_lossy()])
        .assert()
        .success();

    assert_is_prores_4444_with_sound(&movie);
    assert_eq!(probe(&movie, "v:0", "width"), "1920");
    assert_eq!(probe(&movie, "v:0", "height"), "1080");
}

#[test]
fn a_file_input_still_encodes_at_the_default_profile() {
    let dir = TempDir::new().unwrap();
    let clip = testsrc_clip(dir.path());
    let movie = dir.path().join("file.mov");

    cmd()
        .args(["prores", "-i", &clip.to_string_lossy()])
        .args(["-o", &movie.to_string_lossy()])
        .assert()
        .success()
        .stdout(predicate::str::contains("ProRes encoded"));

    assert_eq!(probe(&movie, "v:0", "codec_name"), "prores");
    assert_eq!(probe(&movie, "v:0", "profile"), "HQ");
    assert_eq!(probe(&movie, "v:0", "width"), "1920");
}

#[test]
fn a_directory_without_a_cpl_is_refused_by_name() {
    let dir = TempDir::new().unwrap();
    let empty = dir.path().join("not_an_imp");
    std::fs::create_dir(&empty).unwrap();

    cmd()
        .args(["prores", "-i", &empty.to_string_lossy()])
        .args(["-o", &dir.path().join("out.mov").to_string_lossy()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not_an_imp"))
        .stderr(predicate::str::contains("holds no CPL"));
}
