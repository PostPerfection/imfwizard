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
