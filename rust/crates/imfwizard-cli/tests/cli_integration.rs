use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

#[test]
fn version_flag() {
    cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("imfwizard").and(predicate::str::contains("0.1.0")));
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
