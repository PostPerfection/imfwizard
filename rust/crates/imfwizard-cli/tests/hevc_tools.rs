use assert_cmd::Command;
use postkit::dolby_vision::{
    DOLBY_VISION_FIXTURE_FRAMES, DolbyVisionFixtureProfile, parse_single_rpu,
    write_dolby_vision_fixture,
};
use predicates::prelude::*;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const FIXTURE_WIDTH: u32 = 320;
const FIXTURE_HEIGHT: u32 = 180;
const FIXTURE_FPS: u32 = 25;
// 12 bit PQ code for 600 cd/m², the level 1 peak the fixture RPUs carry
const PQ_CODE_600_NITS: u16 = 2851;
// an 8.1 RPU reads back as profile 8, the RPU carries no sub profile
const DOVI_PROFILE_8: u8 = 8;
const DOVI_PROFILE_5: u8 = 5;
const NAL_START_CODE: [u8; 4] = [0, 0, 0, 1];
// the reshaping curve mode 5 writes, an eight segment polynomial over the luma
const PROFILE_84_LUMA_PIVOTS: [u16; 9] = [63, 69, 230, 256, 256, 37, 16, 8, 7];
// profile 8.1 maps the whole range in one segment
const PROFILE_81_LUMA_PIVOTS: [u16; 2] = [0, 1023];

const MAX_CLL: u64 = 1000;
const MAX_FALL: u64 = 400;
const PQ_TRANSFER_TAG: &str = "smpte2084";

struct MasteringDisplayField {
    flag: &'static str,
    probe_key: &'static str,
    value: u32,
    // ffprobe reports the ST 2086 value over the unit it counts in
    denominator: u32,
}

const CHROMATICITY_UNIT: u32 = 50000;
const LUMINANCE_UNIT: u32 = 10000;

// none of these is the flag's default, so a default reaching the SEI fails the test
const MASTERING_DISPLAY: [MasteringDisplayField; 10] = [
    field(
        "--display-primaries-gx",
        "green_x",
        13250,
        CHROMATICITY_UNIT,
    ),
    field(
        "--display-primaries-gy",
        "green_y",
        34500,
        CHROMATICITY_UNIT,
    ),
    field("--display-primaries-bx", "blue_x", 7500, CHROMATICITY_UNIT),
    field("--display-primaries-by", "blue_y", 3000, CHROMATICITY_UNIT),
    field("--display-primaries-rx", "red_x", 34000, CHROMATICITY_UNIT),
    field("--display-primaries-ry", "red_y", 16000, CHROMATICITY_UNIT),
    field("--white-point-x", "white_point_x", 15600, CHROMATICITY_UNIT),
    field("--white-point-y", "white_point_y", 16400, CHROMATICITY_UNIT),
    field(
        "--max-luminance",
        "max_luminance",
        40_000_000,
        LUMINANCE_UNIT,
    ),
    field("--min-luminance", "min_luminance", 50, LUMINANCE_UNIT),
];

const fn field(
    flag: &'static str,
    probe_key: &'static str,
    value: u32,
    denominator: u32,
) -> MasteringDisplayField {
    MasteringDisplayField {
        flag,
        probe_key,
        value,
        denominator,
    }
}

const HDR10PLUS_FRAMES: usize = 4;
const HDR10PLUS_SCENE_LENGTHS: [usize; 2] = [2, 2];
const FIRST_SCENE_MAX_SCL: [u32; 3] = [17000, 18000, 19000];
const SECOND_SCENE_MAX_SCL: [u32; 3] = [3000, 3100, 3200];
const FIRST_SCENE_AVERAGE_RGB: u32 = 1024;
const SECOND_SCENE_AVERAGE_RGB: u32 = 256;
const TARGET_DISPLAY_LUMINANCE: u32 = 500;
// the histogram and tone mapping curve of an HDR10+ profile B frame, taken from
// what `hdr10plus_tool extract` writes
const DISTRIBUTION_INDEX: [u32; 9] = [1, 5, 10, 25, 50, 75, 90, 95, 99];
const DISTRIBUTION_VALUES: [u32; 9] = [0, 0, 100, 3, 4, 5, 6, 7, 8];
const BEZIER_ANCHORS: [u32; 9] = [102, 205, 307, 410, 512, 614, 717, 819, 922];

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

// the base layer the Dolby Vision fixture encodes, without any RPU in it
fn plain_hevc(directory: &Path, name: &str, frames: usize) -> PathBuf {
    let output = directory.join(name);
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c=gray:s={FIXTURE_WIDTH}x{FIXTURE_HEIGHT}:r={FIXTURE_FPS}"),
            "-frames:v",
        ])
        .arg(frames.to_string())
        .args([
            "-pix_fmt",
            "yuv420p10le",
            "-c:v",
            "libx265",
            "-x265-params",
            "log-level=none",
            "-f",
            "hevc",
        ])
        .arg(&output)
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    output
}

fn profile81_fixture(directory: &Path) -> PathBuf {
    write_dolby_vision_fixture(
        directory,
        "profile81.hevc",
        DolbyVisionFixtureProfile::Profile81,
        None,
        Some(PQ_CODE_600_NITS),
    )
    .unwrap()
}

fn profile5_fixture(directory: &Path) -> PathBuf {
    write_dolby_vision_fixture(
        directory,
        "profile5.hevc",
        DolbyVisionFixtureProfile::Profile5,
        None,
        Some(PQ_CODE_600_NITS),
    )
    .unwrap()
}

fn extract_rpu(hevc: &Path, bin: &Path) {
    cmd()
        .arg("dv-extract")
        .arg("-i")
        .arg(hevc)
        .arg("-o")
        .arg(bin)
        .assert()
        .success();
}

// an escaped RPU cannot hold a start code, so the NALUs split on one
fn rpu_nalus(bin: &Path) -> Vec<Vec<u8>> {
    let data = std::fs::read(bin).unwrap_or_else(|e| panic!("read {}: {e}", bin.display()));
    let starts: Vec<usize> = (0..data.len().saturating_sub(NAL_START_CODE.len() - 1))
        .filter(|&start| data[start..start + NAL_START_CODE.len()] == NAL_START_CODE)
        .collect();
    assert!(!starts.is_empty(), "{} holds no RPU", bin.display());

    starts
        .iter()
        .enumerate()
        .map(|(index, &start)| {
            let end = starts.get(index + 1).copied().unwrap_or(data.len());
            data[start..end].to_vec()
        })
        .collect()
}

fn luma_pivots(nalu: &[u8]) -> Vec<u16> {
    parse_single_rpu(nalu)
        .expect("parse the RPU")
        .rpu_data_mapping
        .expect("the RPU carries no mapping")
        .curves[0]
        .pivots
        .clone()
}

// dovi_tool reading the file back proves the RPUs are written as it writes them
fn exported_rpus(bin: &Path) -> Vec<Value> {
    let json_path = bin.with_extension("export.json");
    let exported = std::process::Command::new("dovi_tool")
        .arg("export")
        .arg("-i")
        .arg(bin)
        .arg("-d")
        .arg(format!("all={}", json_path.display()))
        .output()
        .expect("dovi_tool");
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    serde_json::from_slice(&std::fs::read(&json_path).unwrap()).expect("the exported RPU list")
}

#[test]
fn dv_extract_writes_the_rpu_of_every_frame() {
    let directory = TempDir::new().unwrap();
    let fixture = profile81_fixture(directory.path());

    let rpu = directory.path().join("extracted.bin");
    extract_rpu(&fixture, &rpu);

    let nalus = rpu_nalus(&rpu);
    assert_eq!(nalus.len(), DOLBY_VISION_FIXTURE_FRAMES);
    for nalu in &nalus {
        assert_eq!(
            parse_single_rpu(nalu).expect("parse the RPU").dovi_profile,
            DOVI_PROFILE_8
        );
    }
}

#[test]
fn dv_convert_retargets_profile_5() {
    let directory = TempDir::new().unwrap();
    let fixture = profile5_fixture(directory.path());
    let rpu = directory.path().join("profile5.bin");
    extract_rpu(&fixture, &rpu);
    for nalu in rpu_nalus(&rpu) {
        assert_eq!(
            parse_single_rpu(&nalu).expect("parse the RPU").dovi_profile,
            DOVI_PROFILE_5
        );
    }

    for (target_profile, pivots) in [
        ("8.1", PROFILE_81_LUMA_PIVOTS.to_vec()),
        ("8.4", PROFILE_84_LUMA_PIVOTS.to_vec()),
    ] {
        let converted = directory
            .path()
            .join(format!("profile{target_profile}.bin"));
        cmd()
            .arg("dv-convert")
            .arg("-i")
            .arg(&rpu)
            .arg("-o")
            .arg(&converted)
            .args(["--target-profile", target_profile])
            .assert()
            .success();

        let exported = exported_rpus(&converted);
        assert_eq!(exported.len(), DOLBY_VISION_FIXTURE_FRAMES);
        for parsed in &exported {
            assert_eq!(parsed["dovi_profile"], DOVI_PROFILE_8);
        }
        for nalu in rpu_nalus(&converted) {
            assert_eq!(luma_pivots(&nalu), pivots);
        }
    }
}

#[test]
fn dv_inject_puts_the_extracted_rpu_back() {
    let directory = TempDir::new().unwrap();
    let fixture = profile81_fixture(directory.path());
    let rpu = directory.path().join("extracted.bin");
    extract_rpu(&fixture, &rpu);

    let base_layer = plain_hevc(directory.path(), "base.hevc", DOLBY_VISION_FIXTURE_FRAMES);
    let no_rpu = directory.path().join("no_rpu.bin");
    cmd()
        .arg("dv-extract")
        .arg("-i")
        .arg(&base_layer)
        .arg("-o")
        .arg(&no_rpu)
        .assert()
        .failure();

    let injected = directory.path().join("injected.hevc");
    cmd()
        .arg("dv-inject")
        .arg("-i")
        .arg(&base_layer)
        .arg("-r")
        .arg(&rpu)
        .arg("-o")
        .arg(&injected)
        .assert()
        .success();

    let round_tripped = directory.path().join("round_tripped.bin");
    extract_rpu(&injected, &round_tripped);
    assert_eq!(
        std::fs::read(&rpu).unwrap(),
        std::fs::read(&round_tripped).unwrap(),
        "the injected stream gives back different RPU bytes"
    );
}

fn hdr10plus_frame(
    sequence_frame_index: usize,
    scene_id: usize,
    scene_frame_index: usize,
    max_scl: [u32; 3],
    average_rgb: u32,
) -> Value {
    json!({
        "BezierCurveData": {
            "Anchors": BEZIER_ANCHORS,
            "KneePointX": 0,
            "KneePointY": 0
        },
        "LuminanceParameters": {
            "AverageRGB": average_rgb,
            "LuminanceDistributions": {
                "DistributionIndex": DISTRIBUTION_INDEX,
                "DistributionValues": DISTRIBUTION_VALUES
            },
            "MaxScl": max_scl
        },
        "NumberOfWindows": 1,
        "TargetedSystemDisplayMaximumLuminance": TARGET_DISPLAY_LUMINANCE,
        "SceneFrameIndex": scene_frame_index,
        "SceneId": scene_id,
        "SequenceFrameIndex": sequence_frame_index
    })
}

// two scenes of two frames, injected with hdr10plus_tool because nothing writes
// HDR10+ SEI in process
fn hdr10plus_fixture(directory: &Path) -> PathBuf {
    let metadata = json!({
        "JSONInfo": {"HDR10plusProfile": "B", "Version": "1.0"},
        "SceneInfo": [
            hdr10plus_frame(0, 0, 0, FIRST_SCENE_MAX_SCL, FIRST_SCENE_AVERAGE_RGB),
            hdr10plus_frame(1, 0, 1, FIRST_SCENE_MAX_SCL, FIRST_SCENE_AVERAGE_RGB),
            hdr10plus_frame(2, 1, 0, SECOND_SCENE_MAX_SCL, SECOND_SCENE_AVERAGE_RGB),
            hdr10plus_frame(3, 1, 1, SECOND_SCENE_MAX_SCL, SECOND_SCENE_AVERAGE_RGB),
        ],
        "SceneInfoSummary": {
            "SceneFirstFrameIndex": [0, 2],
            "SceneFrameNumbers": HDR10PLUS_SCENE_LENGTHS
        },
        "ToolInfo": {"Tool": "imfwizard test fixture", "Version": "1.0"}
    });
    let json_path = directory.join("injected.json");
    std::fs::write(&json_path, serde_json::to_vec_pretty(&metadata).unwrap()).unwrap();

    let base_layer = plain_hevc(directory, "base.hevc", HDR10PLUS_FRAMES);
    let injected = directory.join("hdr10plus.hevc");
    let made = std::process::Command::new("hdr10plus_tool")
        .arg("inject")
        .arg("-i")
        .arg(&base_layer)
        .arg("-j")
        .arg(&json_path)
        .arg("-o")
        .arg(&injected)
        .output()
        .expect("hdr10plus_tool");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    injected
}

#[test]
fn hdr10plus_extract_returns_the_injected_metadata() {
    let directory = TempDir::new().unwrap();
    let fixture = hdr10plus_fixture(directory.path());
    let extracted = directory.path().join("extracted.json");

    cmd()
        .arg("hdr10plus-extract")
        .arg("-i")
        .arg(&fixture)
        .arg("-o")
        .arg(&extracted)
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "{} scenes",
            HDR10PLUS_SCENE_LENGTHS.len()
        )));

    let parsed: Value = serde_json::from_slice(&std::fs::read(&extracted).unwrap()).unwrap();
    assert_eq!(
        parsed["SceneInfoSummary"]["SceneFrameNumbers"],
        json!(HDR10PLUS_SCENE_LENGTHS)
    );

    let frames = parsed["SceneInfo"].as_array().expect("SceneInfo");
    assert_eq!(frames.len(), HDR10PLUS_FRAMES);
    for (index, frame) in frames.iter().enumerate() {
        let first_scene = index < HDR10PLUS_SCENE_LENGTHS[0];
        let luminance = &frame["LuminanceParameters"];
        assert_eq!(
            luminance["MaxScl"],
            json!(if first_scene {
                FIRST_SCENE_MAX_SCL
            } else {
                SECOND_SCENE_MAX_SCL
            })
        );
        assert_eq!(
            luminance["AverageRGB"],
            json!(if first_scene {
                FIRST_SCENE_AVERAGE_RGB
            } else {
                SECOND_SCENE_AVERAGE_RGB
            })
        );
        assert_eq!(
            frame["TargetedSystemDisplayMaximumLuminance"],
            json!(TARGET_DISPLAY_LUMINANCE)
        );
    }
}

fn ffprobe_json(arguments: &[&str], input: &Path) -> Value {
    let probed = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-of", "json"])
        .args(arguments)
        .arg(input)
        .output()
        .expect("ffprobe");
    assert!(
        probed.status.success(),
        "{}",
        String::from_utf8_lossy(&probed.stderr)
    );
    serde_json::from_slice(&probed.stdout).expect("ffprobe json")
}

#[test]
fn hdr10_inject_writes_the_mastering_display_and_the_light_levels() {
    let directory = TempDir::new().unwrap();
    let base_layer = plain_hevc(directory.path(), "base.hevc", HDR10PLUS_FRAMES);
    let injected = directory.path().join("hdr10.hevc");

    let mut command = cmd();
    command
        .arg("hdr10-inject")
        .arg("-i")
        .arg(&base_layer)
        .arg("-o")
        .arg(&injected)
        .args(["--max-cll", &MAX_CLL.to_string()])
        .args(["--max-fall", &MAX_FALL.to_string()]);
    for field in &MASTERING_DISPLAY {
        command.args([field.flag, &field.value.to_string()]);
    }
    command
        .assert()
        .success()
        .stdout(predicate::str::contains("master-display=G(13250,34500)"))
        .stdout(predicate::str::contains("L(40000000,50)"));

    let frames = ffprobe_json(&["-show_frames", "-read_intervals", "%+#1"], &injected);
    let side_data = frames["frames"][0]["side_data_list"]
        .as_array()
        .expect("side data")
        .clone();
    let light_level = side_data
        .iter()
        .find(|entry| entry["side_data_type"] == "Content light level metadata")
        .expect("no content light level in the injected stream");
    assert_eq!(light_level["max_content"].as_u64(), Some(MAX_CLL));
    assert_eq!(light_level["max_average"].as_u64(), Some(MAX_FALL));

    let mastering = side_data
        .iter()
        .find(|entry| entry["side_data_type"] == "Mastering display metadata")
        .expect("no mastering display in the injected stream");
    for field in &MASTERING_DISPLAY {
        assert_eq!(
            mastering[field.probe_key],
            json!(format!("{}/{}", field.value, field.denominator)),
            "{} did not reach the stream",
            field.flag
        );
    }

    let streams = ffprobe_json(&["-show_streams"], &injected);
    assert_eq!(streams["streams"][0]["color_transfer"], PQ_TRANSFER_TAG);
}

#[test]
fn every_subcommand_names_a_missing_input() {
    let directory = TempDir::new().unwrap();
    let missing_hevc = directory.path().join("missing.hevc");
    let missing_rpu = directory.path().join("missing.bin");
    let output = directory.path().join("output.hevc");

    let invocations: [(&str, Vec<&Path>); 5] = [
        ("dv-extract", vec![&missing_hevc, &output]),
        ("dv-inject", vec![&missing_hevc, &missing_rpu, &output]),
        ("dv-convert", vec![&missing_rpu, &output]),
        ("hdr10plus-extract", vec![&missing_hevc, &output]),
        ("hdr10-inject", vec![&missing_hevc, &output]),
    ];

    for (subcommand, paths) in invocations {
        let mut command = cmd();
        command.arg(subcommand).arg("-i").arg(paths[0]);
        if subcommand == "dv-inject" {
            command.arg("-r").arg(paths[1]);
        }
        let named = paths[0].display().to_string();
        command
            .arg("-o")
            .arg(paths.last().unwrap())
            .assert()
            .failure()
            .stderr(predicate::str::contains(named));
    }
}
