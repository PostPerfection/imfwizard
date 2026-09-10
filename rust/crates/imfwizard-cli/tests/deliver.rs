// the transfers run rsync
#![cfg(unix)]

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FRAMES: u32 = 1;
const FRAMES_PER_SECOND: u32 = 24;
const TITLE: &str = "Delivery Feature";

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

fn create_imp(work: &Path) -> PathBuf {
    let clip = work.join("source.mkv");
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c=blue:s={WIDTH}x{HEIGHT}:r={FRAMES_PER_SECOND}"),
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
            "--fps-num",
            &FRAMES_PER_SECOND.to_string(),
            "--fps-den",
            "1",
        ])
        .assert()
        .success();
    imp
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

fn digest(path: &Path) -> String {
    postkit::hash::hash_file(path, postkit::hash::HashAlgorithm::Sha1)
        .unwrap_or_else(|error| panic!("cannot hash {}: {error}", path.display()))
        .base64
}

// rsync is a unix transport; Windows has no rsync and would deliver another way.
#[test]
fn an_rsync_delivery_copies_every_file_and_lands_in_the_tracker() {
    let work = TempDir::new().unwrap();
    let imp = create_imp(work.path());
    let destination = work.path().join("delivered");
    let db = work.path().join("deliveries.db");

    let composition = imfwizard_core::timeline::list_cpls(&imp)
        .into_iter()
        .next()
        .expect("the created package holds a CPL");
    assert_eq!(composition.title, TITLE);

    cmd()
        .args([
            "deliver",
            "-i",
            &imp.to_string_lossy(),
            "-d",
            &destination.to_string_lossy(),
            "--db",
            &db.to_string_lossy(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Delivered to"));

    let delivered = file_names(&destination);
    assert_eq!(
        delivered,
        file_names(&imp),
        "the destination does not hold the same files"
    );
    assert!(
        delivered.iter().any(|name| name == "ASSETMAP.xml"),
        "no ASSETMAP arrived: {delivered:?}"
    );
    for name in &delivered {
        assert_eq!(
            digest(&destination.join(name)),
            digest(&imp.join(name)),
            "{name} arrived with different bytes"
        );
    }

    cmd()
        .args(["version", "list", "--db", &db.to_string_lossy()])
        .assert()
        .success()
        .stdout(
            predicate::str::contains(&composition.id)
                .and(predicate::str::contains(TITLE))
                .and(predicate::str::contains("rsync")),
        );
}

#[test]
fn a_failed_transfer_records_nothing() {
    let work = TempDir::new().unwrap();
    let imp = create_imp(work.path());
    let db = work.path().join("deliveries.db");
    // a destination under a regular file cannot be created (ENOTDIR), which fails
    // the same way under GNU rsync and BSD rsync
    let blocker = work.path().join("blocker");
    std::fs::write(&blocker, b"not a directory").unwrap();
    let destination = blocker.join("nowhere");

    cmd()
        .args([
            "deliver",
            "-i",
            &imp.to_string_lossy(),
            "-d",
            &destination.to_string_lossy(),
            "--db",
            &db.to_string_lossy(),
        ])
        .assert()
        .failure();

    assert!(
        !db.exists(),
        "a delivery that never landed was written into the tracker"
    );
    assert!(
        !imp.join(".imfwizard_deliveries.db").exists(),
        "the source package was written into"
    );
}

const S3_DESTINATION: &str = "s3://deliveries/delivery-feature";
const ASPERA_DESTINATION: &str = "aspera://ops@fasp.example.test:/incoming";

// a stand-in transport on PATH that logs its argv and copies the directory it was handed
fn fake_transport(bin: &Path, name: &str, log: &Path, landed: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nfor a in \"$@\"; do [ -d \"$a\" ] && cp -R \"$a\"/. '{}'; done\nexit 0\n",
        log.display(),
        landed.display()
    );
    let path = bin.join(name);
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn deliver_through(binary: &str, destination: &str) -> (Vec<String>, Vec<String>, String) {
    let work = TempDir::new().unwrap();
    let imp = create_imp(work.path());
    let bin = work.path().join("bin");
    let landed = work.path().join("landed");
    let log = work.path().join("argv.log");
    let db = work.path().join("deliveries.db");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&landed).unwrap();
    fake_transport(&bin, binary, &log, &landed);
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    cmd()
        .env("PATH", &path)
        .args([
            "deliver",
            "-i",
            &imp.to_string_lossy(),
            "-d",
            destination,
            "--db",
            &db.to_string_lossy(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Delivered to"));

    let argv: Vec<String> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(
        file_names(&landed),
        file_names(&imp),
        "the transport was not handed the package"
    );
    let listed = cmd()
        .args(["version", "list", "--db", &db.to_string_lossy()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    (argv, file_names(&imp), String::from_utf8(listed).unwrap())
}

#[test]
fn an_s3_delivery_runs_aws_s3_sync_and_lands_in_the_tracker() {
    let (argv, _, listed) = deliver_through("aws", S3_DESTINATION);
    assert_eq!(&argv[..2], ["s3", "sync"], "{argv:?}");
    assert_eq!(argv[3], S3_DESTINATION, "{argv:?}");
    assert_eq!(argv[4], "--no-progress", "{argv:?}");
    assert!(listed.contains(TITLE) && listed.contains("s3"), "{listed}");
}

#[test]
fn an_aspera_delivery_runs_ascp_against_the_remote_and_lands_in_the_tracker() {
    let (argv, _, listed) = deliver_through("ascp", ASPERA_DESTINATION);
    assert_eq!(&argv[..4], ["-QT", "-l", "1000m", "-r"], "{argv:?}");
    assert_eq!(argv[5], "ops@fasp.example.test:/incoming", "{argv:?}");
    assert!(
        listed.contains(TITLE) && listed.contains("aspera"),
        "{listed}"
    );
}
