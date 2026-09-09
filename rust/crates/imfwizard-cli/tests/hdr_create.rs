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

/// A composition with no sound, which these picture fixtures are. The 2016
/// edition's validator requires a main audio sequence and the 2020 one does not.
const NO_SOUND_TRACK: &str = "does not contain a single main audio sequence";

/// Photon has to find nothing but `allowed`: the CPL, the picture MXF, the PKL
/// and the ASSETMAP. The CPL's EssenceDescriptor is the MXF's own descriptor, so
/// Photon compares the two and checks the App 2E colour against it.
fn assert_photon_finds_only(imp: &Path, preset: &str, allowed: &[&str]) {
    let photon = match imfwizard_core::photon::run_photon(imp, None) {
        Ok(photon) => photon,
        Err(e) => panic!("{preset}: Photon failed to analyse the IMP: {e}"),
    };
    for finding in &photon.details {
        assert!(
            allowed.iter().any(|known| finding.contains(known)),
            "{preset}: Photon reports {finding}"
        );
    }
    if allowed.is_empty() {
        assert!(
            photon.errors.is_empty() && photon.warnings.is_empty(),
            "{preset}: Photon errors {:?}, warnings {:?}",
            photon.errors,
            photon.warnings
        );
    }
    for counted in photon.errors.iter().chain(photon.warnings.iter()) {
        assert!(
            counted.contains("CPL_"),
            "{preset}: only the CPL may carry a finding, got {counted}"
        );
    }
}

fn assert_photon_is_clean(imp: &Path, preset: &str) {
    assert_photon_finds_only(imp, preset, &[]);
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

    assert_photon_finds_only(&imp, "pq-bt2020", &[NO_SOUND_TRACK]);
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

/// An SDR App 2E picture is COLOR.3, so the wrap declares the Rec.709 ULs and
/// the CPL repeats them: a picture that declared nothing would read as an
/// unknown colour system.
#[test]
fn an_sdr_imp_declares_rec709_and_passes_photon() {
    let dir = TempDir::new().unwrap();
    let clip = dir.path().join("sdr.mkv");
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi"])
        .args([
            "-i",
            &format!("color=c=gray:s={SOURCE_WIDTH}x{SOURCE_HEIGHT}:r={FPS}"),
        ])
        .args(["-frames:v", &FRAMES.to_string()])
        .args(["-c:v", "ffv1", "-pix_fmt", "gbrp"])
        .arg(&clip)
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );

    let imp = dir.path().join("imp_sdr");
    cmd()
        .args([
            "create",
            "-o",
            &imp.to_string_lossy(),
            "-t",
            "SDR",
            "--video",
            &clip.to_string_lossy(),
            "--raster",
            RASTER,
            "--fps-num",
            &FPS.to_string(),
            "--fps-den",
            "1",
        ])
        .assert()
        .success();

    let (transfer, primaries) = picture_transfer_and_primaries(&imp);
    assert_eq!(transfer, asdcplib::jp2k::TRANSFER_CHARACTERISTIC_BT709);
    assert_eq!(primaries, asdcplib::jp2k::COLOR_PRIMARIES_BT709);
    assert_photon_finds_only(&imp, "sdr", &[NO_SOUND_TRACK]);
}

/// The colour in the CPL is checked, not taken on trust: BT.709 primaries under
/// the HLG transfer is no App 2E colour system, and Photon says so.
#[test]
fn photon_rejects_a_colour_the_cpl_invents() {
    let dir = TempDir::new().unwrap();
    let clip = tagged_clip(dir.path(), HLG_TRANSFER_TAG);
    let imp = build_imp(dir.path(), &clip, "hlg-bt2020");
    assert_photon_is_clean(&imp, "hlg-bt2020");

    let cpl_path = file_starting_with(&imp, "CPL_");
    let cpl = std::fs::read_to_string(&cpl_path).unwrap();
    let rewritten = cpl.replace(
        "<r1:ColorPrimaries>urn:smpte:ul:060e2b34.0401010d.04010101.03040000",
        "<r1:ColorPrimaries>urn:smpte:ul:060e2b34.04010106.04010101.03030000",
    );
    assert_ne!(rewritten, cpl, "the CPL has to carry the BT.2020 primaries");
    std::fs::write(&cpl_path, rewritten).unwrap();

    let photon = imfwizard_core::photon::run_photon(&imp, None).expect("Photon runs");
    assert!(
        photon.details.iter().any(|finding| finding
            .contains("invalid ColorPrimaries(ITU709)-TransferCharacteristic(HLG)")),
        "Photon did not check the colour: {:?}",
        photon.details
    );
}

// the x265 master-display string of the ST 2086 block, and the values it holds:
// G,B,R and the white point in 0.00002 units, the luminances in 0.0001 cd/m²
const MASTERING_DISPLAY: &str =
    "G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(40000000,50)";
// the x265 string lists green, blue, red and ST 2086 stores red, green, blue
const MASTERING_DISPLAY_PRIMARIES: [[u16; 2]; 3] = [[34000, 16000], [13250, 34500], [7500, 3000]];
const MASTERING_DISPLAY_WHITE_POINT: [u16; 2] = [15635, 16450];
const MASTERING_DISPLAY_MAX_LUMINANCE: u32 = 40_000_000;
const MASTERING_DISPLAY_MIN_LUMINANCE: u32 = 50;
const MEASURED_MAX_CLL: u16 = 993;
const MEASURED_MAX_FALL: u16 = 362;

/// ST 2086 goes on the MXF RGBA descriptor, and ST 2067-21 clause 7.5 puts the
/// content light levels in the CPL instead. Both are read back off what was
/// written: the descriptor out of the track file, the light levels out of the CPL.
#[test]
fn the_mastering_display_lands_on_the_descriptor_and_the_light_levels_in_the_cpl() {
    let dir = TempDir::new().unwrap();
    let clip = tagged_clip(dir.path(), PQ_TRANSFER_TAG);

    let imp = dir.path().join("imp_st2086");
    cmd()
        .args([
            "create",
            "-o",
            &imp.to_string_lossy(),
            "-t",
            "Mastered",
            "--video",
            &clip.to_string_lossy(),
            "--raster",
            RASTER,
            "--hdr",
            "pq-bt2020",
            "--mastering-display",
            MASTERING_DISPLAY,
            "--max-cll",
            &MEASURED_MAX_CLL.to_string(),
            "--max-fall",
            &MEASURED_MAX_FALL.to_string(),
            "--fps-num",
            &FPS.to_string(),
            "--fps-den",
            "1",
        ])
        .assert()
        .success();

    let picture = file_starting_with(&imp, imfwizard_core::imp::PICTURE_PREFIX);
    let mut reader = asdcplib::as02::jp2k::MxfReader::new();
    reader
        .open_read(&picture.to_string_lossy())
        .expect("the picture MXF opens");
    let hdr = reader.hdr_metadata().expect("the RGBA descriptor's colour");
    reader.close().unwrap();

    assert_eq!(
        hdr.mastering_display_primaries,
        Some(MASTERING_DISPLAY_PRIMARIES)
    );
    assert_eq!(
        hdr.mastering_display_white_point,
        Some(MASTERING_DISPLAY_WHITE_POINT)
    );
    assert_eq!(
        hdr.mastering_display_max_luminance,
        Some(MASTERING_DISPLAY_MAX_LUMINANCE)
    );
    assert_eq!(
        hdr.mastering_display_min_luminance,
        Some(MASTERING_DISPLAY_MIN_LUMINANCE)
    );

    let cpl = std::fs::read_to_string(file_starting_with(&imp, "CPL_")).unwrap();
    assert_eq!(
        extension_property(&cpl, "MaxCLL").as_deref(),
        Some(MEASURED_MAX_CLL.to_string().as_str())
    );
    assert_eq!(
        extension_property(&cpl, "MaxFALL").as_deref(),
        Some(MEASURED_MAX_FALL.to_string().as_str())
    );
    // the descriptor in the CPL is the track file's own, so it repeats the block
    assert!(
        cpl.contains(&MASTERING_DISPLAY_MAX_LUMINANCE.to_string()),
        "the CPL EssenceDescriptor carries no mastering display maximum: {cpl}"
    );

    assert_photon_finds_only(&imp, "pq-bt2020", &[NO_SOUND_TRACK]);
}

// the light levels the fixture RPU carries in its level 6 block
const RPU_MAX_CONTENT_LIGHT_LEVEL: u16 = 993;
const RPU_MAX_FRAME_AVERAGE_LIGHT_LEVEL: u16 = 362;
const RPU_MASTERING_DISPLAY_MAX_NITS: u16 = 1000;
const RPU_MASTERING_DISPLAY_MIN_STEPS: u16 = 1;

/// A profile 8.1 master: an HEVC base layer with an RPU after every slice,
/// remuxed to MP4 so `create --video` takes it.
fn dolby_vision_master(dir: &Path) -> PathBuf {
    let annex_b = postkit::dolby_vision::write_dolby_vision_fixture(
        dir,
        "dv81.hevc",
        postkit::dolby_vision::DolbyVisionFixtureProfile::Profile81,
        Some(
            dolby_vision::rpu::extension_metadata::blocks::ExtMetadataBlockLevel6 {
                max_display_mastering_luminance: RPU_MASTERING_DISPLAY_MAX_NITS,
                min_display_mastering_luminance: RPU_MASTERING_DISPLAY_MIN_STEPS,
                max_content_light_level: RPU_MAX_CONTENT_LIGHT_LEVEL,
                max_frame_average_light_level: RPU_MAX_FRAME_AVERAGE_LIGHT_LEVEL,
            },
        ),
        None,
    )
    .expect("the Dolby Vision fixture");

    let mp4 = dir.join("dv81.mp4");
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-i"])
        .arg(&annex_b)
        .args(["-c", "copy"])
        .arg(&mp4)
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    mp4
}

/// The text of an App 2E ExtensionProperty in a CPL, by its local name. The
/// element carries the 2067-21 namespace declaration, so the open tag is not
/// just the name.
fn extension_property(cpl: &str, name: &str) -> Option<String> {
    let start = cpl.find(&format!("<app2e:{name}"))?;
    let (_, after_tag) = cpl[start..].split_once('>')?;
    let (value, _) = after_tag.split_once('<')?;
    Some(value.trim().to_string())
}

/// ST 2067-21 clause 7.5 carries MaxCLL and MaxFALL in the CPL, and a Dolby
/// Vision 8.1 master already measured them into its RPU, so `--hdr` alone is
/// enough: the level 6 block reaches the CPL without either flag being passed.
#[test]
fn a_dolby_vision_rpu_fills_the_light_levels_no_flag_passed() {
    let dir = TempDir::new().unwrap();
    let master = dolby_vision_master(dir.path());

    let imp = dir.path().join("imp_dv");
    cmd()
        .args([
            "create",
            "-o",
            &imp.to_string_lossy(),
            "-t",
            "Dolby Vision 8.1",
            "--video",
            &master.to_string_lossy(),
            "--raster",
            RASTER,
            "--hdr",
            "pq-bt2020",
            "--fps-num",
            "25",
            "--fps-den",
            "1",
        ])
        .assert()
        .success();

    let cpl = std::fs::read_to_string(file_starting_with(&imp, "CPL_")).unwrap();
    assert_eq!(
        extension_property(&cpl, "MaxCLL").as_deref(),
        Some(RPU_MAX_CONTENT_LIGHT_LEVEL.to_string().as_str()),
        "the RPU's MaxCLL did not reach the CPL: {cpl}"
    );
    assert_eq!(
        extension_property(&cpl, "MaxFALL").as_deref(),
        Some(RPU_MAX_FRAME_AVERAGE_LIGHT_LEVEL.to_string().as_str()),
        "the RPU's MaxFALL did not reach the CPL: {cpl}"
    );
}

/// The flags are the operator's own measurement, so a passed value stands
/// instead of the RPU's.
#[test]
fn a_passed_light_level_beats_the_rpu() {
    let dir = TempDir::new().unwrap();
    let master = dolby_vision_master(dir.path());
    let measured = RPU_MAX_CONTENT_LIGHT_LEVEL + 100;

    let imp = dir.path().join("imp_measured");
    cmd()
        .args([
            "create",
            "-o",
            &imp.to_string_lossy(),
            "-t",
            "Dolby Vision 8.1",
            "--video",
            &master.to_string_lossy(),
            "--raster",
            RASTER,
            "--hdr",
            "pq-bt2020",
            "--max-cll",
            &measured.to_string(),
            "--fps-num",
            "25",
            "--fps-den",
            "1",
        ])
        .assert()
        .success();

    let cpl = std::fs::read_to_string(file_starting_with(&imp, "CPL_")).unwrap();
    assert_eq!(
        extension_property(&cpl, "MaxCLL").as_deref(),
        Some(measured.to_string().as_str())
    );
    assert!(
        extension_property(&cpl, "MaxFALL").is_none(),
        "MaxFALL was neither passed nor taken from the RPU: {cpl}"
    );
}

/// Profile 5 carries no HDR10 base layer, so it is refused rather than packaged
/// as if its light levels applied.
#[test]
fn a_profile_5_master_is_refused_by_name() {
    let dir = TempDir::new().unwrap();
    let annex_b = postkit::dolby_vision::write_dolby_vision_fixture(
        dir.path(),
        "dv5.hevc",
        postkit::dolby_vision::DolbyVisionFixtureProfile::Profile5,
        None,
        None,
    )
    .expect("the Dolby Vision fixture");
    let mp4 = dir.path().join("dv5.mp4");
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-i"])
        .arg(&annex_b)
        .args(["-c", "copy"])
        .arg(&mp4)
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );

    cmd()
        .args([
            "create",
            "-o",
            &dir.path().join("imp").to_string_lossy(),
            "-t",
            "Profile 5",
            "--video",
            &mp4.to_string_lossy(),
            "--raster",
            RASTER,
            "--hdr",
            "pq-bt2020",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("5"));
}

/// A sine WAV of exactly `sample_frames` stereo sample frames at 48 kHz.
fn sine_wav(dir: &Path, name: &str, sample_frames: u32) -> PathBuf {
    let wav = dir.join(format!("{name}.wav"));
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi"])
        .args(["-i", "sine=frequency=1000:duration=10:sample_rate=48000"])
        .args(["-af", &format!("atrim=end_sample={sample_frames}")])
        .args(["-ac", "2", "-c:a", "pcm_s24le"])
        .arg(&wav)
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    wav
}

fn build_sound_imp(dir: &Path, clip: &Path, name: &str, title: &str, wav: &Path) -> PathBuf {
    let imp = dir.join(name);
    cmd()
        .args([
            "create",
            "-o",
            &imp.to_string_lossy(),
            "-t",
            title,
            "--video",
            &clip.to_string_lossy(),
            "--raster",
            RASTER,
            "--hdr",
            "hlg-bt2020",
            "--audio",
            &wav.to_string_lossy(),
            "--audio-lang",
            "en-US",
            "--fps-num",
            &FPS.to_string(),
            "--fps-den",
            "1",
        ])
        .assert()
        .success();
    imp
}

/// Every resource's IntrinsicDuration in the IMP's CPL, in CPL order.
fn intrinsic_durations(imp: &Path) -> Vec<u64> {
    let cpl = file_starting_with(imp, "CPL_");
    let xml = std::fs::read_to_string(cpl).expect("the CPL");
    xml.split("<IntrinsicDuration>")
        .skip(1)
        .map(|rest| {
            rest.split("</IntrinsicDuration>")
                .next()
                .expect("a closing tag")
                .parse()
                .expect("a frame count")
        })
        .collect()
}

/// Sound shorter than the picture is padded with silence and sound longer than
/// it is cut, so the segment's two sequences run the same length. Photon rejects
/// a segment whose sequences differ.
#[test]
fn sound_is_fitted_to_the_picture() {
    let dir = TempDir::new().unwrap();
    let clip = tagged_clip(dir.path(), HLG_TRANSFER_TAG);
    let frame_samples = 48000 / FPS;

    for (name, sample_frames) in [("short", frame_samples), ("long", frame_samples * 3)] {
        let wav = sine_wav(dir.path(), name, sample_frames);
        let imp = build_sound_imp(
            dir.path(),
            &clip,
            &format!("imp_{name}"),
            "Fitted sound",
            &wav,
        );
        let durations = intrinsic_durations(&imp);
        assert_eq!(
            durations,
            vec![FRAMES as u64; 2],
            "{name} sound must run the picture's {FRAMES} frames"
        );
        assert_photon_finds_only(&imp, name, &[]);
    }
}

/// A sound track file is wrapped with the MCA labels ST 2067-2 asks for, which
/// Photon checks on the MXF, and its CPL EssenceDescriptorList entry is that
/// same descriptor read back off the track file.
#[test]
fn a_sound_track_carries_its_mca_labels() {
    let dir = TempDir::new().unwrap();
    let clip = tagged_clip(dir.path(), HLG_TRANSFER_TAG);
    let wav = sine_wav(dir.path(), "stereo", FRAMES as u32 * 48000 / FPS);
    let imp = build_sound_imp(dir.path(), &clip, "imp_sound", "HLG with sound", &wav);

    let sound = file_starting_with(&imp, "AUDIO_");
    let mut reader = asdcplib::as02::pcm::MxfReader::new();
    reader
        .open_read(&sound.to_string_lossy(), asdcplib::Rational::new(24, 1))
        .expect("the sound MXF opens");
    assert_eq!(
        reader.channel_assignment().expect("channel assignment"),
        Some(asdcplib::as02::pcm::IMF_CHANNEL_ASSIGNMENT_MCA)
    );
    let labels = reader.mca_label_subdescriptors().expect("mca labels");
    assert_eq!(labels.len(), 3, "two channels plus the soundfield group");
    let group = labels
        .iter()
        .find(|label| label.kind == asdcplib::pcm::McaLabelKind::SoundfieldGroup)
        .expect("a soundfield group");
    assert_eq!(group.spoken_language.as_deref(), Some("en-US"));
    assert_eq!(group.title.as_deref(), Some("HLG with sound"));
    assert_eq!(group.title_version.as_deref(), Some("Original Version"));
    assert_eq!(group.audio_content_kind.as_deref(), Some("PRM"));
    assert_eq!(group.audio_element_kind.as_deref(), Some("FCMP"));

    assert_photon_finds_only(&imp, "sound", &[]);
}
