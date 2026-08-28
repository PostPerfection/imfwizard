//! What `imfwizard create` leaves on disk has to be an IMF App 2E picture track
//! and not a DCP picture in IMF clothing: an IMF JPEG 2000 profile, 12-bit RGB
//! samples, and the colour signalled on the RGBA essence descriptor.
//!
//! Every assertion opens the MXF the CLI wrote and reads it back through
//! asdcplib, decoding the frame the reader returns with grok, so the encoder
//! agreeing with itself cannot pass this.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// App 2E's smallest legal raster.
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FRAMES: u32 = 3;
const FPS: u32 = 24;

/// The largest 12-bit sample, what pure red decodes back to.
const FULL_SCALE_12BIT: u32 = 4095;
/// Red has to come back close to full scale and green and blue close to zero,
/// with room for what a lossy encode moves.
const RED_FLOOR: i32 = 3900;
const GREEN_AND_BLUE_CEILING: i32 = 200;

/// The 12-bit RGB pixel layout of SMPTE 377's RGBAValue_RGB_12, zero-terminated.
const PIXEL_LAYOUT_RGB_12: [u8; 16] = [b'R', 12, b'G', 12, b'B', 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// The CPL namespace ST 2067-3 fixes, and the App 2E identifier ST 2067-21 puts
/// in ApplicationIdentification.
const CPL_NAMESPACE_2067_3: &str = "http://www.smpte-ra.org/schemas/2067-3/2016";
const APP2E_IDENTIFIER: &str = "http://www.smpte-ra.org/schemas/2067-21/2016";

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

/// A lossless RGB clip of pure red, so the samples that reach the encoder are
/// exactly 255,0,0 and any spread across the three components is the encode's.
fn make_red_clip(path: &Path) {
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c=red:s={WIDTH}x{HEIGHT}:r={FPS}"),
            "-frames:v",
            &FRAMES.to_string(),
            "-c:v",
            "ffv1",
            "-pix_fmt",
            "gbrp",
        ])
        .arg(path)
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
}

/// The one picture track file in a written IMP.
fn picture_track_file(imp_dir: &Path) -> PathBuf {
    let mut pictures: Vec<PathBuf> = std::fs::read_dir(imp_dir)
        .expect("the IMP directory")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(imfwizard_core::imp::PICTURE_PREFIX))
        })
        .collect();
    assert_eq!(pictures.len(), 1, "expected one picture track file");
    pictures.pop().unwrap()
}

/// The centre pixel of a decoded frame, as (red, green, blue).
fn centre_pixel(decoded: &postkit::grok_decoder::DecodedFrame) -> (i32, i32, i32) {
    let centre =
        (decoded.height as usize / 2) * decoded.width as usize + decoded.width as usize / 2;
    (
        decoded.components[0][centre],
        decoded.components[1][centre],
        decoded.components[2][centre],
    )
}

#[test]
fn create_writes_an_app2e_picture_track() {
    let dir = TempDir::new().unwrap();
    let clip = dir.path().join("red.mkv");
    make_red_clip(&clip);
    let imp = dir.path().join("imp");

    cmd()
        .args([
            "create",
            "-o",
            &imp.to_string_lossy(),
            "-t",
            "App 2E Picture",
            "--video",
            &clip.to_string_lossy(),
            "--fps-num",
            &FPS.to_string(),
            "--fps-den",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("IMP created"));

    let picture = picture_track_file(&imp);
    assert_eq!(
        asdcplib::essence_type(&picture.to_string_lossy()).unwrap(),
        asdcplib::EssenceType::As02Jpeg2000,
        "the picture is not AS-02 JPEG 2000 essence"
    );

    let mut reader = asdcplib::as02::jp2k::MxfReader::new();
    reader
        .open_read(&picture.to_string_lossy())
        .expect("the picture MXF opens");

    // the codestream the writer described in the sub-descriptor
    let codestream = reader.picture_descriptor().unwrap().codestream;
    let profile = postkit::j2k::J2kProfile::from(codestream.rsize);
    assert_eq!(
        profile,
        postkit::j2k::J2kProfile::Imf,
        "rsiz {:#06x} is not an IMF profile: a DCP encode of this source would have \
         declared the cinema 2K profile 0x0003",
        codestream.rsize
    );
    assert!(
        !profile.is_dci_cinema(),
        "rsiz {:#06x} is a cinema profile, so the samples would be X'Y'Z'",
        codestream.rsize
    );
    assert_eq!(codestream.components.len(), 3);
    for component in &codestream.components {
        assert_eq!(component.bit_depth(), 12);
        // 4:4:4, no chroma subsampling
        assert_eq!(component.x_rsize, 1);
        assert_eq!(component.y_rsize, 1);
    }

    // the RGBA essence descriptor, which is what a reader trusts over the essence
    let rgba = reader.rgba_descriptor().unwrap();
    assert_eq!(
        rgba.pixel_layout, PIXEL_LAYOUT_RGB_12,
        "the RGBA essence descriptor has to describe 12-bit RGB"
    );
    assert_eq!(rgba.component_max_ref, Some(FULL_SCALE_12BIT));
    assert_eq!(rgba.component_min_ref, Some(0));

    // an SDR create names no colour of its own, so the wrap has to supply Rec.709
    let hdr = reader.hdr_metadata().unwrap();
    assert_eq!(
        hdr.color_primaries,
        Some(asdcplib::jp2k::COLOR_PRIMARIES_BT709)
    );
    assert_eq!(
        hdr.transfer_characteristic,
        Some(asdcplib::jp2k::TRANSFER_CHARACTERISTIC_BT709)
    );

    // the samples themselves, decoded out of the essence the reader returns
    let mut buf = vec![0u8; 16 << 20];
    let size = reader.read_frame(0, &mut buf, None, None).unwrap();
    buf.truncate(size);
    let decoded = postkit::grok_decoder::decode(buf, 0).expect("the wrapped frame has to decode");
    assert_eq!(decoded.precision, 12);
    let (red, green, blue) = centre_pixel(&decoded);
    assert!(
        red > RED_FLOOR && green < GREEN_AND_BLUE_CEILING && blue < GREEN_AND_BLUE_CEILING,
        "the centre pixel read back as {red},{green},{blue}: pure red in RGB is near \
         {FULL_SCALE_12BIT},0,0, while an X'Y'Z' encode of the same frame leaves all three \
         components large"
    );

    reader.close().unwrap();

    // the CPL over that essence has to be the IMF one, identified as App 2E
    let cpl_path = std::fs::read_dir(&imp)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("CPL_"))
        })
        .expect("the IMP has a CPL");
    let cpl = std::fs::read_to_string(&cpl_path).unwrap();
    assert!(
        cpl.contains(CPL_NAMESPACE_2067_3),
        "the CPL is not in the ST 2067-3 namespace: {cpl_path:?}"
    );
    assert!(
        cpl.contains(&format!(
            "<cc:ApplicationIdentification>{APP2E_IDENTIFIER}</cc:ApplicationIdentification>"
        )),
        "the CPL does not identify itself as App 2E"
    );
}
