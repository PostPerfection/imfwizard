use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

// the source is small and `--raster` pads it into an App 2E raster
const SOURCE_WIDTH: u32 = 256;
const SOURCE_HEIGHT: u32 = 144;
const RASTER: &str = "1920x1080";
const FRAMES: usize = 2;
const FPS: u32 = 24;

// ffprobe's spellings of the two HDR transfer characteristics
const PQ_TRANSFER_TAG: &str = "smpte2084";
const HLG_TRANSFER_TAG: &str = "arib-std-b67";

const TEN_BIT_MID_GREY: [u8; 2] = [0x00, 0x02];

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

// rawvideo carries no tags of its own, so they go on the input and the output
fn tagged_clip(dir: &Path, transfer: &str) -> PathBuf {
    let raw = dir.join(format!("{transfer}.yuv"));
    // yuv420p10le: one 16-bit sample a pixel of luma, half that again of chroma
    let samples = (SOURCE_WIDTH * SOURCE_HEIGHT) as usize * 3 / 2;
    std::fs::write(&raw, TEN_BIT_MID_GREY.repeat(samples * FRAMES)).unwrap();

    let colour_tags = [
        "-color_primaries",
        "bt2020",
        "-color_trc",
        transfer,
        "-colorspace",
        "bt2020nc",
        "-color_range",
        "tv",
    ];
    let clip = dir.join(format!("{transfer}.mkv"));
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "rawvideo"])
        .args(["-pix_fmt", "yuv420p10le"])
        .args(["-s", &format!("{SOURCE_WIDTH}x{SOURCE_HEIGHT}")])
        .args(["-r", &FPS.to_string()])
        .args(colour_tags)
        .arg("-i")
        .arg(&raw)
        .args(["-c:v", "ffv1"])
        .args(colour_tags)
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

fn build_imp(dir: &Path, clip: &Path, preset: &str) -> PathBuf {
    let imp = dir.join(format!("imp_{preset}"));
    cmd()
        .args([
            "create",
            "-o",
            &imp.to_string_lossy(),
            "-t",
            preset,
            "--video",
            &clip.to_string_lossy(),
            "--raster",
            RASTER,
            "--hdr",
            preset,
            "--fps-num",
            &FPS.to_string(),
            "--fps-den",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("IMP created"));
    imp
}

fn file_starting_with(imp: &Path, prefix: &str) -> PathBuf {
    std::fs::read_dir(imp)
        .expect("the IMP directory")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix))
        })
        .unwrap_or_else(|| panic!("the IMP has no {prefix} file"))
}

fn picture_transfer_and_primaries(imp: &Path) -> ([u8; 16], [u8; 16]) {
    let picture = file_starting_with(imp, imfwizard_core::imp::PICTURE_PREFIX);
    let mut reader = asdcplib::as02::jp2k::MxfReader::new();
    reader
        .open_read(&picture.to_string_lossy())
        .expect("the picture MXF opens");
    let hdr = reader.hdr_metadata().expect("the RGBA descriptor's colour");
    reader.close().unwrap();
    (
        hdr.transfer_characteristic.expect("a transfer UL"),
        hdr.color_primaries.expect("a colour primaries UL"),
    )
}

fn assert_photon_is_clean(imp: &Path, preset: &str) {
    match imfwizard_core::photon::run_photon(imp, None) {
        Ok(photon) => assert!(
            photon.errors.is_empty() && photon.warnings.is_empty(),
            "{preset}: Photon errors {:?}, warnings {:?}",
            photon.errors,
            photon.warnings
        ),
        Err(e) => panic!("{preset}: Photon failed to analyse the IMP: {e}"),
    }
}

#[test]
fn a_pq_imp_carries_st2084_and_passes_photon() {
    let dir = TempDir::new().unwrap();
    let clip = tagged_clip(dir.path(), PQ_TRANSFER_TAG);
    let imp = build_imp(dir.path(), &clip, "pq-bt2020");

    let (transfer, primaries) = picture_transfer_and_primaries(&imp);
    assert_eq!(transfer, asdcplib::jp2k::TRANSFER_CHARACTERISTIC_ST2084);
    assert_eq!(primaries, asdcplib::jp2k::COLOR_PRIMARIES_BT2020);

    let cpl = std::fs::read_to_string(file_starting_with(&imp, "CPL_")).unwrap();
    assert!(
        cpl.contains("urn:smpte:ul:060e2b34.0401010d.04010101.010a0000"),
        "the CPL descriptor does not carry the ST 2084 transfer UL"
    );

    assert_photon_is_clean(&imp, "pq-bt2020");
}

// COLOR.8, the BT.2020 primaries with the HLG OETF
#[test]
fn an_hlg_imp_carries_the_hlg_transfer_and_passes_photon() {
    let dir = TempDir::new().unwrap();
    let clip = tagged_clip(dir.path(), HLG_TRANSFER_TAG);
    let imp = build_imp(dir.path(), &clip, "hlg-bt2020");

    let (transfer, primaries) = picture_transfer_and_primaries(&imp);
    assert_eq!(
        transfer,
        imfwizard_core::hdr_wcg::TRANSFER_CHARACTERISTIC_HLG
    );
    assert_eq!(primaries, asdcplib::jp2k::COLOR_PRIMARIES_BT2020);

    let cpl = std::fs::read_to_string(file_starting_with(&imp, "CPL_")).unwrap();
    assert!(
        cpl.contains("urn:smpte:ul:060e2b34.0401010d.04010101.010b0000"),
        "the CPL descriptor does not carry the HLG transfer UL"
    );

    assert_photon_is_clean(&imp, "hlg-bt2020");
}

#[test]
fn content_light_levels_are_refused_on_an_hlg_package() {
    let dir = TempDir::new().unwrap();
    let clip = tagged_clip(dir.path(), HLG_TRANSFER_TAG);
    cmd()
        .args([
            "create",
            "-o",
            &dir.path().join("imp").to_string_lossy(),
            "-t",
            "HLG",
            "--video",
            &clip.to_string_lossy(),
            "--raster",
            RASTER,
            "--hdr",
            "hlg-bt2020",
            "--max-cll",
            "993",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--max-cll"))
        .stderr(predicate::str::contains("COLOR.8"));
}

#[test]
fn a_pq_source_under_the_hlg_preset_is_refused() {
    let dir = TempDir::new().unwrap();
    let clip = tagged_clip(dir.path(), PQ_TRANSFER_TAG);
    cmd()
        .args([
            "create",
            "-o",
            &dir.path().join("imp").to_string_lossy(),
            "-t",
            "PQ",
            "--video",
            &clip.to_string_lossy(),
            "--raster",
            RASTER,
            "--hdr",
            "hlg-bt2020",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(PQ_TRANSFER_TAG))
        .stderr(predicate::str::contains("hlg-bt2020"));
}

#[test]
fn an_hdr_source_without_a_preset_is_refused() {
    let dir = TempDir::new().unwrap();
    let clip = tagged_clip(dir.path(), HLG_TRANSFER_TAG);
    cmd()
        .args([
            "create",
            "-o",
            &dir.path().join("imp").to_string_lossy(),
            "-t",
            "HLG",
            "--video",
            &clip.to_string_lossy(),
            "--raster",
            RASTER,
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--hdr hlg-bt2020"))
        .stderr(predicate::str::contains("Rec.709 SDR"));
}
