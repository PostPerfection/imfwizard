use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const SOURCE_WIDTH: u32 = 320;
const SOURCE_HEIGHT: u32 = 180;
const FRAMES: u32 = 3;
const DCI_FLAT_WIDTH: u32 = 1998;
const DCI_FLAT_HEIGHT: u32 = 1080;

// Rec.709 white through the ST 428-1 matrix, 48/52.37 scale and 2.6 gamma at 12 bits
const WHITE_XYZ_CODES: [i32; 3] = [3883, 3960, 4092];
// the same white through a LUT that halves every code
const HALVED_WHITE_XYZ_CODES: [i32; 3] = [2160, 2203, 2276];
const CODE_TOLERANCE: i32 = 2;
const MAX_CODE_12_BIT: i32 = 4095;

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

fn white_frames(dir: &Path) -> PathBuf {
    let frames = dir.join("frames");
    std::fs::create_dir_all(&frames).unwrap();
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg(format!(
            "color=c=white:s={SOURCE_WIDTH}x{SOURCE_HEIGHT},\
             format=rgb48be,lutrgb=r=maxval:g=maxval:b=maxval"
        ))
        .args(["-frames:v", &FRAMES.to_string()])
        .args(["-pix_fmt", "rgb48be"])
        .arg(frames.join("frame_%06d.tif"))
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    frames
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

fn stream_entry(path: &Path, entry: &str) -> String {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .arg(format!("stream={entry}"))
        .args(["-of", "default=nw=1:nk=1"])
        .arg(path)
        .output()
        .expect("ffprobe");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn centre_codes(frame: &Path, width: u32, height: u32) -> [i32; 3] {
    let out = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(frame)
        .args(["-vf", &format!("crop=1:1:{}:{}", width / 2, height / 2)])
        .args([
            "-frames:v",
            "1",
            "-pix_fmt",
            "rgb48le",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .expect("ffmpeg");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout.len(), 6, "one 16-bit RGB pixel");
    let codes = out.stdout.as_chunks::<2>().0;
    [
        i32::from(u16::from_le_bytes(codes[0])),
        i32::from(u16::from_le_bytes(codes[1])),
        i32::from(u16::from_le_bytes(codes[2])),
    ]
}

fn assert_codes(measured: [i32; 3], expected: [i32; 3], what: &str) {
    for (component, (got, want)) in measured.iter().zip(&expected).enumerate() {
        assert!(
            (got - want).abs() <= CODE_TOLERANCE,
            "{what} component {component} came out at {got}, not the {want} the transform gives"
        );
        assert!(
            *got <= MAX_CODE_12_BIT,
            "{what} component {component} is {got}, past the 12-bit range"
        );
    }
}

#[test]
fn dcdm_writes_xyz_tiffs_at_the_raster_it_was_given() {
    let dir = TempDir::new().unwrap();
    let frames = white_frames(dir.path());
    let output = dir.path().join("dcdm");

    cmd()
        .args([
            "dcdm",
            "-i",
            &frames.to_string_lossy(),
            "-o",
            &output.to_string_lossy(),
            "--colour-space",
            "rec709",
            "--width",
            &DCI_FLAT_WIDTH.to_string(),
            "--height",
            &DCI_FLAT_HEIGHT.to_string(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "DCDM created: {FRAMES} frames"
        )));

    assert_eq!(
        file_names(&output),
        vec!["dcdm_000000.tif", "dcdm_000001.tif", "dcdm_000002.tif"]
    );

    for name in file_names(&output) {
        let tiff = output.join(&name);
        assert_eq!(stream_entry(&tiff, "width"), DCI_FLAT_WIDTH.to_string());
        assert_eq!(stream_entry(&tiff, "height"), DCI_FLAT_HEIGHT.to_string());
        assert_eq!(
            stream_entry(&tiff, "pix_fmt"),
            "rgb48le",
            "{name} is not a 16-bit RGB TIFF"
        );
        assert_codes(
            centre_codes(&tiff, DCI_FLAT_WIDTH, DCI_FLAT_HEIGHT),
            WHITE_XYZ_CODES,
            &name,
        );
    }
}

#[test]
fn a_lut_transforms_the_source_before_the_xyz_encode() {
    let dir = TempDir::new().unwrap();
    let frames = white_frames(dir.path());
    let output = dir.path().join("dcdm");
    let lut = dir.path().join("half.cube");
    std::fs::write(
        &lut,
        "LUT_3D_SIZE 2\n\
         0 0 0\n0.5 0 0\n0 0.5 0\n0.5 0.5 0\n\
         0 0 0.5\n0.5 0 0.5\n0 0.5 0.5\n0.5 0.5 0.5\n",
    )
    .unwrap();

    cmd()
        .args([
            "dcdm",
            "-i",
            &frames.to_string_lossy(),
            "-o",
            &output.to_string_lossy(),
            "--colour-space",
            "rec709",
            "--lut",
            &lut.to_string_lossy(),
            "--width",
            &SOURCE_WIDTH.to_string(),
            "--height",
            &SOURCE_HEIGHT.to_string(),
        ])
        .assert()
        .success();

    let first = output.join("dcdm_000000.tif");
    assert_codes(
        centre_codes(&first, SOURCE_WIDTH, SOURCE_HEIGHT),
        HALVED_WHITE_XYZ_CODES,
        "the LUT run",
    );
}
