//! What `to-dcp` carries from the IMP to the DCP, and what it refuses.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const RASTER: &str = "1920x1080";
const FRAMES: u32 = 4;
/// A DCI rate that is not the 24 every other fixture here uses, so a hard coded
/// 24 anywhere in the conversion shows up.
const DCI_RATE: u32 = 25;

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

fn solid_clip(dir: &Path, rate: &str) -> PathBuf {
    let clip = dir.join("source.mov");
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "lavfi"])
        .args(["-i", &format!("color=c=red:s={RASTER}:r={rate}")])
        .args(["-frames:v", &FRAMES.to_string()])
        .args(["-pix_fmt", "yuv420p", "-an"])
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

fn create_imp(dir: &Path, name: &str, rate: u32) -> PathBuf {
    let clip = solid_clip(dir, &rate.to_string());
    let imp = dir.join(name);
    cmd()
        .args(["create", "-o", &imp.to_string_lossy()])
        .args(["-t", name])
        .args(["--video", &clip.to_string_lossy()])
        .args(["--raster", RASTER])
        .args(["--fps-num", &rate.to_string()])
        .args(["--fps-den", "1"])
        .assert()
        .success();
    imp
}

fn dcp_cpl(dcp: &Path) -> String {
    let cpl = std::fs::read_dir(dcp)
        .expect("the DCP directory")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("CPL_"))
        })
        .expect("the DCP has a CPL");
    std::fs::read_to_string(cpl).unwrap()
}

/// The IMP's edit rate is what the DCP runs at: the CPL says so and the picture
/// track file's own descriptor agrees, so a projector and a validator read the
/// same rate.
#[test]
fn the_imp_edit_rate_reaches_the_dcp_cpl_and_its_picture_descriptor() {
    let dir = TempDir::new().unwrap();
    let imp = create_imp(dir.path(), "pal", DCI_RATE);
    let dcp = dir.path().join("dcp");

    cmd()
        .args([
            "to-dcp",
            "-i",
            &imp.to_string_lossy(),
            "-o",
            &dcp.to_string_lossy(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("transcoded"));

    assert!(
        dcp_cpl(&dcp).contains(&format!("<EditRate>{DCI_RATE} 1</EditRate>")),
        "the DCP CPL does not carry the IMP's {DCI_RATE} fps"
    );

    let mut reader = asdcplib::jp2k::MxfReader::new();
    reader
        .open_read(&dcp.join("picture.mxf").to_string_lossy())
        .expect("the DCP picture opens");
    let descriptor = reader.picture_descriptor().expect("a picture descriptor");
    reader.close().unwrap();
    assert_eq!(
        descriptor.edit_rate,
        asdcplib::Rational::new(DCI_RATE as i32, 1)
    );
    assert_eq!(descriptor.container_duration, FRAMES);
}

/// DCI caps a distribution codestream at 250 Mb/s, and `--bitrate` over that is
/// refused naming both figures rather than clamped.
#[test]
fn a_bitrate_over_the_dci_ceiling_is_refused_naming_the_limit() {
    let dir = TempDir::new().unwrap();
    let imp = create_imp(dir.path(), "over", 24);
    let over = postkit::j2k::DCI_MAX_BITRATE_MBPS + 50.0;

    cmd()
        .args([
            "to-dcp",
            "-i",
            &imp.to_string_lossy(),
            "-o",
            &dir.path().join("dcp").to_string_lossy(),
            "--bitrate",
            &over.to_string(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(over.to_string()))
        .stderr(predicate::str::contains(format!(
            "DCI limit of {:.0}",
            postkit::j2k::DCI_MAX_BITRATE_MBPS
        )));
}

/// The cap's own value is taken, so 250 itself converts.
#[test]
fn the_dci_ceiling_itself_is_accepted() {
    let dir = TempDir::new().unwrap();
    let imp = create_imp(dir.path(), "at_limit", 24);
    let dcp = dir.path().join("dcp");

    cmd()
        .args([
            "to-dcp",
            "-i",
            &imp.to_string_lossy(),
            "-o",
            &dcp.to_string_lossy(),
            "--bitrate",
            &format!("{:.0}", postkit::j2k::DCI_MAX_BITRATE_MBPS),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("transcoded"));
    assert!(dcp.join("picture.mxf").is_file());
}

/// An edit rate outside the DCI set is refused naming the rate and the set.
#[test]
fn an_edit_rate_outside_the_dci_set_is_refused_naming_both() {
    let dir = TempDir::new().unwrap();
    let clip = solid_clip(dir.path(), "23");
    let imp = dir.path().join("odd");
    cmd()
        .args(["create", "-o", &imp.to_string_lossy()])
        .args(["-t", "Odd rate"])
        .args(["--video", &clip.to_string_lossy()])
        .args(["--raster", RASTER])
        .args(["--fps-num", "23", "--fps-den", "1"])
        .assert()
        .success();

    cmd()
        .args([
            "to-dcp",
            "-i",
            &imp.to_string_lossy(),
            "-o",
            &dir.path().join("dcp").to_string_lossy(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("23/1"))
        .stderr(predicate::str::contains(
            "24, 25, 30, 48, 50, 60, 96, 100, 120",
        ));
}
