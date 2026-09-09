use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FRAMES: u32 = 1;
const FRAMES_PER_SECOND: u32 = 24;

const PASSED: &str = "IMP validation PASSED";
const SKIPPED: &str = "--no-verify: the package was not validated";

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

fn source_clip(work: &Path) -> PathBuf {
    let clip = work.join("source.mkv");
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c=green:s={WIDTH}x{HEIGHT}:r={FRAMES_PER_SECOND}"),
            "-frames:v",
            &FRAMES.to_string(),
            "-c:v",
            "ffv1",
            "-pix_fmt",
            "gbrp",
        ])
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

fn create(work: &Path, extra: &[&str]) -> (PathBuf, Command) {
    let clip = source_clip(work);
    let imp = work.join("imp");
    let mut command = cmd();
    command.args([
        "create",
        "-o",
        &imp.to_string_lossy(),
        "-t",
        "Verified",
        "--video",
        &clip.to_string_lossy(),
        "--fps-num",
        &FRAMES_PER_SECOND.to_string(),
        "--fps-den",
        "1",
    ]);
    command.args(extra);
    (imp, command)
}

#[test]
fn a_created_package_is_validated_before_the_command_returns() {
    let work = TempDir::new().unwrap();
    let (imp, mut command) = create(work.path(), &[]);

    command
        .assert()
        .success()
        .stdout(predicate::str::contains("IMP created at").and(predicate::str::contains(PASSED)));

    assert!(imp.join("ASSETMAP.xml").is_file(), "no package was written");
}

#[test]
fn no_verify_says_it_skipped_and_leaves_the_package() {
    let work = TempDir::new().unwrap();
    let (imp, mut command) = create(work.path(), &["--no-verify"]);

    command
        .assert()
        .success()
        .stdout(
            predicate::str::contains("IMP created at").and(predicate::str::contains(PASSED).not()),
        )
        .stderr(predicate::str::contains(SKIPPED));

    assert!(imp.join("ASSETMAP.xml").is_file(), "no package was written");
}

// create and its verify pass run in one process, so nothing can break the
// package between the wrap and the check. This runs the same call on the same
// package instead, which is where the non-zero exit comes from.
#[test]
fn the_check_create_runs_exits_non_zero_on_a_broken_package() {
    let work = TempDir::new().unwrap();
    let (imp, mut command) = create(work.path(), &["--no-verify"]);
    command.assert().success();

    // the PKL names a hash for the CPL, so an edited CPL no longer matches it
    let cpl = std::fs::read_dir(&imp)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("CPL_"))
        })
        .expect("the package holds a CPL");
    let xml = std::fs::read_to_string(&cpl).unwrap();
    std::fs::write(&cpl, xml.replacen("<ContentTitle>", "<ContentTitle> ", 1)).unwrap();

    cmd()
        .args(["validate", &imp.to_string_lossy()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("IMP validation FAILED"));
}
