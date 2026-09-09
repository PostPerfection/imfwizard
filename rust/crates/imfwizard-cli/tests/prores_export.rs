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
fn an_imp_exports_into_a_container_without_being_touched() {
    let dir = TempDir::new().unwrap();
    let imp = build_sound_imp(dir.path(), "imp_2k");
    let before = imp_digests(&imp);
    let movie = dir.path().join("out2k.mov");

    cmd()
        .args(["prores", "-i", &imp.to_string_lossy()])
        .args(["-o", &movie.to_string_lossy()])
        .args(["--container", "2k-full"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ProRes 4444 exported"));

    assert_is_prores_4444_with_sound(&movie);
    assert_eq!(probe(&movie, "v:0", "width"), "2048");
    assert_eq!(probe(&movie, "v:0", "height"), "1080");
    assert_eq!(imp_digests(&imp), before, "the export changed the IMP");
}

#[test]
fn every_container_name_pads_the_picture_to_its_own_raster() {
    let containers = [
        ("2k-scope", "2048", "858"),
        ("2k-flat", "1998", "1080"),
        ("2k-full", "2048", "1080"),
        ("4k-scope", "4096", "1716"),
        ("4k-flat", "3996", "2160"),
        ("4k-full", "4096", "2160"),
    ];
    let dir = TempDir::new().unwrap();
    let imp = build_sound_imp(dir.path(), "imp_containers");

    for (container, width, height) in containers {
        let movie = dir.path().join(format!("{container}.mov"));
        cmd()
            .args(["prores", "-i", &imp.to_string_lossy()])
            .args(["-o", &movie.to_string_lossy()])
            .args(["--container", container])
            .assert()
            .success();

        assert_is_prores_4444_with_sound(&movie);
        assert_eq!(probe(&movie, "v:0", "width"), width, "{container}");
        assert_eq!(probe(&movie, "v:0", "height"), height, "{container}");
    }
}

#[test]
fn an_unknown_container_is_refused_with_the_names_that_work() {
    let dir = TempDir::new().unwrap();
    let empty = dir.path().join("empty_imp");
    std::fs::create_dir(&empty).unwrap();

    cmd()
        .args(["prores", "-i", &empty.to_string_lossy()])
        .args(["-o", &dir.path().join("out.mov").to_string_lossy()])
        .args(["--container", "2k"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown container '2k'"))
        .stderr(predicate::str::contains("2k-full, 4k-scope"));
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

fn only_cpl(imp: &Path) -> PathBuf {
    let mut found: Vec<PathBuf> = std::fs::read_dir(imp)
        .expect("the IMP directory")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("CPL_"))
        })
        .collect();
    assert_eq!(found.len(), 1, "expected one CPL in {imp:?}");
    found.pop().unwrap()
}

fn counted_frames(movie: &Path) -> u32 {
    let output = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-count_frames", "-select_streams", "v:0"])
        .args(["-show_entries", "stream=nb_read_frames"])
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
        .trim()
        .parse()
        .expect("a frame count")
}

// the created CPL holds one segment over the whole track file, so cutting it in
// two is the only way to get a multi-segment composition out of `create`
fn cut_into_two_segments(imp: &Path, first_frames: u32, second_frames: u32) {
    const SEGMENT_OPEN: &str = "    <Segment>\n";
    const SEGMENT_CLOSE: &str = "    </Segment>\n";
    let cpl = only_cpl(imp);
    let xml = std::fs::read_to_string(&cpl).expect("the CPL");
    let start = xml.find(SEGMENT_OPEN).expect("a segment");
    let end = xml.find(SEGMENT_CLOSE).expect("a closed segment") + SEGMENT_CLOSE.len();
    let segment = &xml[start..end];

    let source_duration = format!("<SourceDuration>{FRAMES}</SourceDuration>");
    let cut = |entry_point: u32, frames: u32| {
        let cut_resource = format!(
            "<EntryPoint>{entry_point}</EntryPoint><SourceDuration>{frames}</SourceDuration>"
        );
        segment.replace(&source_duration, &cut_resource)
    };
    let two = format!(
        "{}{}",
        cut(0, first_frames),
        cut(first_frames, second_frames)
    );
    std::fs::write(&cpl, format!("{}{two}{}", &xml[..start], &xml[end..])).expect("the edited CPL");
}

#[test]
fn every_segment_of_a_composition_reaches_the_export() {
    const FIRST_SEGMENT_FRAMES: u32 = 2;
    const SECOND_SEGMENT_FRAMES: u32 = 3;
    let dir = TempDir::new().unwrap();
    let imp = build_sound_imp(dir.path(), "imp_segments");
    cut_into_two_segments(&imp, FIRST_SEGMENT_FRAMES, SECOND_SEGMENT_FRAMES);
    let movie = dir.path().join("segments.mov");

    cmd()
        .args(["prores", "-i", &imp.to_string_lossy()])
        .args(["-o", &movie.to_string_lossy()])
        .assert()
        .success();

    assert_eq!(probe(&movie, "v:0", "codec_name"), "prores");
    assert_eq!(probe(&movie, "a:0", "codec_name"), "pcm_s24le");
    assert_eq!(
        counted_frames(&movie),
        FIRST_SEGMENT_FRAMES + SECOND_SEGMENT_FRAMES,
        "the export must hold both segments and nothing else"
    );
}

#[test]
fn a_supplemental_imp_exports_against_the_ov_it_names() {
    let dir = TempDir::new().unwrap();
    let ov = build_sound_imp(dir.path(), "ov_imp");
    let dub = sine_wav(dir.path());
    let supplement = dir.path().join("supplemental_imp");
    cmd()
        .args(["supplement", "--ov", &ov.to_string_lossy()])
        .args(["-t", "French dub", "-o", &supplement.to_string_lossy()])
        .args(["--replace", &format!("{}@audio", dub.display())])
        .assert()
        .success();

    let movie = dir.path().join("supplemental.mov");
    cmd()
        .args(["prores", "-i", &supplement.to_string_lossy()])
        .args(["-o", &movie.to_string_lossy()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("names no picture track file"))
        .stderr(predicate::str::contains("--ov"));

    cmd()
        .args(["prores", "-i", &supplement.to_string_lossy()])
        .args(["-o", &movie.to_string_lossy()])
        .args(["--ov", &ov.to_string_lossy()])
        .assert()
        .success();

    assert_is_prores_4444_with_sound(&movie);
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
