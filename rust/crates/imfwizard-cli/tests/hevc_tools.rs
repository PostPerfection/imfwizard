use assert_cmd::Command;
use postkit::dolby_vision::{
    DOLBY_VISION_FIXTURE_FRAMES, DolbyVisionFixtureProfile, write_dolby_vision_fixture,
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
const DOVI_PROFILE_8: u64 = 8;

const MAX_CLL: u64 = 1000;
const MAX_FALL: u64 = 400;
const PQ_TRANSFER_TAG: &str = "smpte2084";

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

// postkit's parse_single_rpu leaves the emulation prevention bytes in place, so
// the RPUs are read back with dovi_tool's own parser
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

    let rpus = exported_rpus(&rpu);
    assert_eq!(rpus.len(), DOLBY_VISION_FIXTURE_FRAMES);
    for parsed in &rpus {
        assert_eq!(parsed["dovi_profile"], DOVI_PROFILE_8);
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
fn hdr10_inject_writes_the_content_light_level() {
    let directory = TempDir::new().unwrap();
    let base_layer = plain_hevc(directory.path(), "base.hevc", HDR10PLUS_FRAMES);
    let injected = directory.path().join("hdr10.hevc");

    cmd()
        .arg("hdr10-inject")
        .arg("-i")
        .arg(&base_layer)
        .arg("-o")
        .arg(&injected)
        .args(["--max-cll", &MAX_CLL.to_string()])
        .args(["--max-fall", &MAX_FALL.to_string()])
        .assert()
        .success();

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
    assert!(
        side_data
            .iter()
            .any(|entry| entry["side_data_type"] == "Mastering display metadata"),
        "no mastering display in the injected stream"
    );

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
