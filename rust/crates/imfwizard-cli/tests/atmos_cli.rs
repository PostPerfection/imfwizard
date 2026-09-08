//! `imfwizard atmos` over a BW64 ADM master.

use assert_cmd::Command;
use imfwizard_core::atmos::{BwfForm, synthetic_adm_bwf};
use predicates::prelude::*;
use tempfile::TempDir;

/// Half a second, a whole number of frames at 25 fps.
const SAMPLE_FRAMES: u32 = 24_000;

#[test]
fn atmos_imports_a_bw64_master_at_the_given_edit_rate() {
    let directory = TempDir::new().unwrap();
    let master = directory.path().join("master.wav");
    std::fs::write(&master, synthetic_adm_bwf(BwfForm::Bw64, SAMPLE_FRAMES)).unwrap();
    let output = directory.path().join("out");

    Command::cargo_bin("imfwizard")
        .unwrap()
        .args([
            "atmos",
            "-i",
            &master.to_string_lossy(),
            "-o",
            &output.to_string_lossy(),
            "--fps-num",
            "25",
            "--fps-den",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("10 beds, 2 objects, 12 channels"));

    assert!(output.join("atmos.mxf").is_file());
    assert!(output.join("adm_metadata.xml").is_file());
}

#[test]
fn a_missing_master_is_refused_by_name() {
    let directory = TempDir::new().unwrap();
    let missing = directory.path().join("no_such_master.wav");

    Command::cargo_bin("imfwizard")
        .unwrap()
        .args([
            "atmos",
            "-i",
            &missing.to_string_lossy(),
            "-o",
            &directory.path().join("out").to_string_lossy(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no_such_master.wav"));
}
