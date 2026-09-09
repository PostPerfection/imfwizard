//! Every still format `create` takes as a picture source, packaged and decoded
//! back out of the track file.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FRAMES: u32 = 3;
const FPS: u32 = 24;

/// A codestream buffer with room for a 2K frame at any ratio these encode at.
const FRAME_BUFFER_BYTES: usize = 16 << 20;

/// Half of a 12-bit component's range, which the mid grey source decodes to.
const MID_GREY_12_BIT: i32 = 2048;
/// The decode is not bit exact through the format conversions, so a component
/// lands within this of the level the source carried.
const LEVEL_TOLERANCE: i32 = 64;

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

/// A frame folder of `extension` stills, mid grey so every format carries the
/// same picture whatever its own bit depth and colour model.
fn frame_folder(dir: &Path, extension: &str, pixel_format: &str) -> PathBuf {
    let frames = dir.join(format!("{extension}_frames"));
    std::fs::create_dir_all(&frames).unwrap();
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "lavfi"])
        .args(["-i", &format!("color=c=gray:s={WIDTH}x{HEIGHT}:r={FPS}")])
        .args(["-frames:v", &FRAMES.to_string()])
        .args(["-pix_fmt", pixel_format])
        .arg(frames.join(format!("frame_%06d.{extension}")))
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{extension}: {}",
        String::from_utf8_lossy(&made.stderr)
    );
    assert_eq!(
        std::fs::read_dir(&frames).unwrap().count(),
        FRAMES as usize,
        "{extension}: ffmpeg wrote the wrong number of stills"
    );
    frames
}

fn picture_track_file(imp: &Path) -> PathBuf {
    std::fs::read_dir(imp)
        .expect("the IMP directory")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(imfwizard_core::imp::PICTURE_PREFIX))
        })
        .expect("a picture track file")
}

/// Decode every frame of the packaged essence, returning the centre pixel of
/// each as 12-bit RGB.
fn decoded_centre_pixels(imp: &Path) -> Vec<[i32; 3]> {
    let picture = picture_track_file(imp);
    let mut reader = asdcplib::as02::jp2k::MxfReader::new();
    reader
        .open_read(&picture.to_string_lossy())
        .expect("the picture MXF opens");

    let pixels = (0..FRAMES)
        .map(|index| {
            let mut buffer = vec![0u8; FRAME_BUFFER_BYTES];
            let size = reader
                .read_frame(index, &mut buffer, None, None)
                .unwrap_or_else(|e| panic!("frame {index} does not read back: {e}"));
            buffer.truncate(size);
            let decoded = postkit::grok_decoder::decode(buffer, 0)
                .unwrap_or_else(|e| panic!("frame {index} does not decode: {e}"));
            assert_eq!(decoded.precision, 12, "App 2E picture is 12 bit");
            assert_eq!((decoded.width, decoded.height), (WIDTH, HEIGHT));
            let centre = (decoded.height / 2 * decoded.width + decoded.width / 2) as usize;
            [
                decoded.components[0][centre] as i32,
                decoded.components[1][centre] as i32,
                decoded.components[2][centre] as i32,
            ]
        })
        .collect();
    reader.close().unwrap();
    pixels
}

/// DPX, EXR, PNG and BMP frame folders each encode and package, and every
/// packaged frame decodes back to the grey the stills carried. TIFF is read by
/// imfwizard itself and the rest go through ffmpeg, so both readers are covered.
#[test]
fn every_still_format_packages_and_decodes_back() {
    for (extension, pixel_format) in [
        ("dpx", "gbrp10le"),
        ("exr", "gbrpf32le"),
        ("png", "rgb48be"),
        ("bmp", "bgr24"),
        ("tif", "rgb48be"),
    ] {
        let dir = TempDir::new().unwrap();
        let frames = frame_folder(dir.path(), extension, pixel_format);
        let imp = dir.path().join("imp");

        cmd()
            .args([
                "create",
                "-o",
                &imp.to_string_lossy(),
                "-t",
                &format!("{extension} sequence"),
                "--video",
                &frames.to_string_lossy(),
                "--fps-num",
                &FPS.to_string(),
                "--fps-den",
                "1",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("IMP created"));

        let pixels = decoded_centre_pixels(&imp);
        assert_eq!(pixels.len(), FRAMES as usize);
        for (index, pixel) in pixels.iter().enumerate() {
            for (component, level) in pixel.iter().enumerate() {
                assert!(
                    (level - MID_GREY_12_BIT).abs() <= LEVEL_TOLERANCE,
                    "{extension} frame {index} component {component} decoded to {level}, \
                     not the {MID_GREY_12_BIT} the grey source carried"
                );
            }
        }
    }
}

/// A folder holding a format `create` has no reader for is refused by name
/// rather than packaged as an empty picture.
#[test]
fn a_folder_of_an_unreadable_format_is_refused() {
    let dir = TempDir::new().unwrap();
    let frames = dir.path().join("frames");
    std::fs::create_dir_all(&frames).unwrap();
    std::fs::write(frames.join("frame_000001.pcx"), b"not a still").unwrap();

    cmd()
        .args([
            "create",
            "-o",
            &dir.path().join("imp").to_string_lossy(),
            "-t",
            "Unreadable",
            "--video",
            &frames.to_string_lossy(),
        ])
        .assert()
        .failure();
}
