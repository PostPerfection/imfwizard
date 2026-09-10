//! What `create` aims the encoder at: a delivery preset's bitrate, a named
//! raster, and a PSNR quality target.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const SOURCE_WIDTH: u32 = 1920;
const SOURCE_HEIGHT: u32 = 1080;
const FRAMES: usize = 4;
const FPS: u32 = 24;
// how far the achieved size ratio may sit from the ratio of the two bitrates
const BITRATE_RATIO_TOLERANCE: f64 = 0.4;

// the App 2E 4K raster, the widest `create` writes
const RASTER_4K: &str = "4096x2160";
const RASTER_4K_WIDTH: u32 = 4096;
const RASTER_4K_HEIGHT: u32 = 2160;
const RASTER_2K: &str = "1920x1080";

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

// noise at the raster the encoder writes: it compresses worse than the leanest
// bitrate here allows, so every preset's byte cap actually bites
fn detailed_clip(dir: &Path) -> PathBuf {
    let clip = dir.join("source.mkv");
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi"])
        .args([
            "-i",
            &format!("testsrc2=s={SOURCE_WIDTH}x{SOURCE_HEIGHT}:r={FPS}"),
        ])
        .args(["-vf", "noise=alls=90:allf=t+u"])
        .args(["-frames:v", &FRAMES.to_string()])
        .args(["-c:v", "ffv1", "-pix_fmt", "yuv444p10le"])
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

/// Every codestream `create` kept under `--keep-intermediates`, in name order.
fn codestream_sizes(imp: &Path) -> Vec<u64> {
    let mut frames: Vec<_> = std::fs::read_dir(imp.join("j2k"))
        .expect("the j2k directory")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "j2c" || extension == "j2k")
        })
        .collect();
    frames.sort();
    assert_eq!(frames.len(), FRAMES, "one codestream a source frame");
    frames
        .iter()
        .map(|path| std::fs::metadata(path).expect("a codestream").len())
        .collect()
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

fn picture_descriptor(imp: &Path) -> asdcplib::jp2k::PictureDescriptor {
    let mut reader = asdcplib::as02::jp2k::MxfReader::new();
    reader
        .open_read(&picture_track_file(imp).to_string_lossy())
        .expect("the picture MXF opens");
    let descriptor = reader.picture_descriptor().expect("a picture descriptor");
    reader.close().unwrap();
    descriptor
}

fn build(dir: &Path, clip: &Path, name: &str, raster: &str, extra: &[&str]) -> PathBuf {
    let imp = dir.join(name);
    cmd()
        .args([
            "create",
            "--keep-intermediates",
            "-o",
            &imp.to_string_lossy(),
            "-t",
            name,
            "--video",
            &clip.to_string_lossy(),
            "--raster",
            raster,
            "--fps-num",
            &FPS.to_string(),
            "--fps-den",
            "1",
        ])
        .args(extra)
        .assert()
        .success()
        .stdout(predicate::str::contains("IMP created"));
    imp
}

/// `--profile <name>` aims the encoder at the preset's bitrate and nothing
/// else: the broadcast preset is 200 Mb/s and the Netflix one 400, so the same
/// source under the two leaves codestreams whose sizes are two to one.
#[test]
fn two_presets_leave_codestreams_in_the_ratio_of_their_bitrates() {
    let dir = TempDir::new().unwrap();
    let clip = detailed_clip(dir.path());

    let broadcast = imfwizard_core::profiles::profile_for(postkit::profiles::Platform::Broadcast);
    let netflix = imfwizard_core::profiles::profile_for(postkit::profiles::Platform::Netflix);
    let asked = netflix.bitrate_mbps / broadcast.bitrate_mbps;
    assert!(
        asked > 1.0,
        "the two presets have to disagree for this to prove anything"
    );

    let lean = build(
        dir.path(),
        &clip,
        "broadcast",
        RASTER_2K,
        &["--profile", "broadcast"],
    );
    let rich = build(
        dir.path(),
        &clip,
        "netflix",
        RASTER_2K,
        &["--profile", "netflix"],
    );

    let lean_total: u64 = codestream_sizes(&lean).iter().sum();
    let rich_total: u64 = codestream_sizes(&rich).iter().sum();
    // an encode that ignored the preset would leave the two the same size
    let got = rich_total as f64 / lean_total as f64;
    assert!(
        (got - asked).abs() <= BITRATE_RATIO_TOLERANCE,
        "the two presets are {got:.2} apart and their bitrates ask for {asked:.2} \
         ({rich_total} bytes against {lean_total})"
    );
}

/// The preset carries a raster, a colour space, a frame rate and an audio
/// layout as well, and `create` applies none of them: the picture keeps the
/// `--raster` it was given and declares Rec.709 rather than the preset's
/// Rec.2020. Anything the preset says beyond the bitrate is documentation.
#[test]
fn a_preset_sets_the_bitrate_and_leaves_the_raster_alone() {
    let dir = TempDir::new().unwrap();
    let clip = detailed_clip(dir.path());
    let netflix = imfwizard_core::profiles::profile_for(postkit::profiles::Platform::Netflix);
    assert_eq!(netflix.width, 3840, "the preset names a UHD raster");

    let imp = build(
        dir.path(),
        &clip,
        "netflix_2k",
        RASTER_2K,
        &["--profile", "netflix"],
    );
    let descriptor = picture_descriptor(&imp);
    assert_eq!(
        (descriptor.stored_width, descriptor.stored_height),
        (SOURCE_WIDTH, SOURCE_HEIGHT),
        "--raster decided the raster, not the preset"
    );
}

/// `--bitrate` names the target directly, wins over a preset, and says so.
#[test]
fn an_explicit_bitrate_overrides_the_preset_it_is_given_with() {
    let dir = TempDir::new().unwrap();
    let clip = detailed_clip(dir.path());
    let netflix = imfwizard_core::profiles::profile_for(postkit::profiles::Platform::Netflix);
    let named_mbps = netflix.bitrate_mbps / 2.0;

    let preset_only = build(
        dir.path(),
        &clip,
        "preset_only",
        RASTER_2K,
        &["--profile", "netflix"],
    );

    let overridden = dir.path().join("override");
    cmd()
        .args([
            "create",
            "--keep-intermediates",
            "-o",
            &overridden.to_string_lossy(),
            "-t",
            "Override",
            "--video",
            &clip.to_string_lossy(),
            "--raster",
            RASTER_2K,
            "--profile",
            "netflix",
            "--bitrate",
            &named_mbps.to_string(),
            "--fps-num",
            &FPS.to_string(),
            "--fps-den",
            "1",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("overrides preset"));

    let preset_total: u64 = codestream_sizes(&preset_only).iter().sum();
    let named_total: u64 = codestream_sizes(&overridden).iter().sum();
    let got = preset_total as f64 / named_total as f64;
    let asked = netflix.bitrate_mbps / named_mbps;
    assert!(
        (got - asked).abs() <= BITRATE_RATIO_TOLERANCE,
        "the flag left {named_total} bytes and the preset {preset_total}, {got:.2} apart \
         where the two bitrates ask for {asked:.2}"
    );
}

/// "Up to 4K": `create --raster 4096x2160` writes a track file whose descriptor
/// stores that raster, and the wrapped codestream is that size too.
#[test]
fn a_4k_raster_reaches_the_picture_descriptor() {
    let dir = TempDir::new().unwrap();
    let clip = detailed_clip(dir.path());
    let imp = build(dir.path(), &clip, "uhd", RASTER_4K, &[]);

    let descriptor = picture_descriptor(&imp);
    assert_eq!(descriptor.stored_width, RASTER_4K_WIDTH);
    assert_eq!(descriptor.stored_height, RASTER_4K_HEIGHT);
    assert_eq!(descriptor.container_duration, FRAMES as u32);
    assert_eq!(descriptor.edit_rate, asdcplib::Rational::new(FPS as i32, 1));

    // the sub-descriptor is parsed off the first codestream, so it says what the
    // encoder wrote rather than what the wrap was told
    assert_eq!(descriptor.codestream.xsize, RASTER_4K_WIDTH);
    assert_eq!(descriptor.codestream.ysize, RASTER_4K_HEIGHT);
}

/// A gradient with a little movement: the encoder can reach a high PSNR on it
/// without leaving a frame the byte cap has to refuse.
fn smooth_clip(dir: &Path) -> PathBuf {
    let clip = dir.join("smooth.mkv");
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi"])
        .args([
            "-i",
            &format!("gradients=s={SOURCE_WIDTH}x{SOURCE_HEIGHT}:r={FPS}"),
        ])
        .args(["-frames:v", &FRAMES.to_string()])
        .args(["-c:v", "ffv1", "-pix_fmt", "yuv444p10le"])
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

/// `--quality-psnr` allocates to a quality target rather than to a ratio, so a
/// higher target spends more bytes on the same source.
#[test]
fn a_higher_psnr_target_leaves_larger_codestreams() {
    let dir = TempDir::new().unwrap();
    let clip = smooth_clip(dir.path());

    let coarse = build(
        dir.path(),
        &clip,
        "psnr30",
        RASTER_2K,
        &["--quality-psnr", "30"],
    );
    let fine = build(
        dir.path(),
        &clip,
        "psnr60",
        RASTER_2K,
        &["--quality-psnr", "60"],
    );

    let coarse_total: u64 = codestream_sizes(&coarse).iter().sum();
    let fine_total: u64 = codestream_sizes(&fine).iter().sum();
    assert!(
        fine_total > coarse_total,
        "the 60 dB target wrote {fine_total} bytes and the 30 dB one {coarse_total}"
    );
}

/// Under a quality target the bitrate becomes a per-frame byte ceiling, and a
/// frame the target pushes over it is encoded again by ratio to fit.
#[test]
fn a_psnr_target_over_the_byte_cap_is_held_to_the_cap() {
    let dir = TempDir::new().unwrap();
    let clip = detailed_clip(dir.path());
    let cap_mbps = 1.0;
    let cap_bytes =
        imfwizard_core::encode::codestream_byte_cap_for_bitrate(f64::from(FPS), cap_mbps);

    let capped = dir.path().join("capped");
    cmd()
        .args([
            "create",
            "-o",
            &capped.to_string_lossy(),
            "-t",
            "Capped",
            "--video",
            &clip.to_string_lossy(),
            "--raster",
            RASTER_2K,
            "--bitrate",
            &cap_mbps.to_string(),
            "--quality-psnr",
            "60",
            "--fps-num",
            &FPS.to_string(),
            "--fps-den",
            "1",
            "--keep-intermediates",
        ])
        .assert()
        .success();
    let largest = codestream_sizes(&capped).into_iter().max().unwrap();
    assert!(
        largest <= cap_bytes,
        "a frame reached {largest} bytes over the {cap_bytes} byte cap"
    );
}

/// A cap below what a codestream's headers alone take cannot be met by any
/// allocation, so the frame is refused by name rather than shipped over it.
#[test]
fn a_byte_cap_no_frame_can_meet_is_refused_naming_the_frame() {
    let dir = TempDir::new().unwrap();
    let clip = detailed_clip(dir.path());
    let cap_mbps = 0.001;
    let cap_bytes =
        imfwizard_core::encode::codestream_byte_cap_for_bitrate(f64::from(FPS), cap_mbps);

    cmd()
        .args([
            "create",
            "-o",
            &dir.path().join("capped").to_string_lossy(),
            "-t",
            "Capped",
            "--video",
            &clip.to_string_lossy(),
            "--raster",
            RASTER_2K,
            "--bitrate",
            &cap_mbps.to_string(),
            "--quality-psnr",
            "60",
            "--fps-num",
            &FPS.to_string(),
            "--fps-den",
            "1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(cap_bytes.to_string()))
        .stderr(predicate::str::contains("per-frame cap"));
}

/// The target is a dB figure with a range, and a value outside it is refused by
/// name rather than clamped into something the operator did not ask for.
#[test]
fn a_psnr_target_outside_the_range_is_refused_by_name() {
    let dir = TempDir::new().unwrap();
    let clip = smooth_clip(dir.path());

    let below = (imfwizard_core::encode::MINIMUM_QUALITY_PSNR_DB - 1.0).to_string();
    let above = (imfwizard_core::encode::MAXIMUM_QUALITY_PSNR_DB + 1.0).to_string();
    for target in [below.as_str(), above.as_str()] {
        cmd()
            .args([
                "create",
                "-o",
                &dir.path().join("rejected").to_string_lossy(),
                "-t",
                "Out of range",
                "--video",
                &clip.to_string_lossy(),
                "--raster",
                RASTER_2K,
                "--quality-psnr",
                target,
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("--quality-psnr"));
    }
}
