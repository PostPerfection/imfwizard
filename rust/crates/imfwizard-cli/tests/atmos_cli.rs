//! `imfwizard atmos` over a BW64 ADM master.

use assert_cmd::Command;
use imfwizard_core::atmos::{BwfForm, synthetic_adm_bwf, synthetic_ia_bitstream_frame};
use predicates::prelude::*;
use std::path::{Path, PathBuf};
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

const IAB_PICTURE_FRAMES: u32 = 4;
const IAB_EDIT_RATE: u32 = 24;
const IAB_SAMPLE_RATE: u32 = 48_000;
const IAB_SEQUENCE_OPEN: &str =
    r#"<iab:IABSequence xmlns:iab="http://www.smpte-ra.org/ns/2067-201/2019">"#;
const IAB_SEQUENCE_CLOSE: &str = "</iab:IABSequence>";
const PASSED: &str = "IMP validation PASSED";

fn codestream_directory(work: &Path, frames: u32) -> PathBuf {
    let directory = work.join("j2k");
    std::fs::create_dir_all(&directory).unwrap();
    for frame in 0..frames {
        std::fs::write(
            directory.join(format!("frame_{frame:08}.j2c")),
            imfwizard_core::mxf_wrap::synthetic_j2k_codestream(1920, 1080, 12),
        )
        .unwrap();
    }
    directory
}

fn stereo_wav(work: &Path, picture_frames: u32) -> PathBuf {
    let path = work.join("sound.wav");
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: IAB_SAMPLE_RATE,
        bits_per_sample: 24,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec).unwrap();
    for _ in 0..picture_frames * IAB_SAMPLE_RATE / IAB_EDIT_RATE * 2 {
        writer.write_sample(0i32).unwrap();
    }
    writer.finalize().unwrap();
    path
}

fn ia_bitstream_directory(work: &Path, frames: u32) -> (PathBuf, Vec<Vec<u8>>) {
    let directory = work.join("iab");
    std::fs::create_dir_all(&directory).unwrap();
    let written: Vec<Vec<u8>> = (0..frames as u8)
        .map(|seed| {
            synthetic_ia_bitstream_frame(seed, 8 + seed as usize, 1_000 * (seed as usize + 1))
        })
        .collect();
    for (index, frame) in written.iter().enumerate() {
        std::fs::write(directory.join(format!("frame_{index:05}.iab")), frame).unwrap();
    }
    (directory, written)
}

fn create_with_atmos(work: &Path, atmos: &Path, extra: &[&str]) -> (PathBuf, Command) {
    let picture = codestream_directory(work, IAB_PICTURE_FRAMES);
    let sound = stereo_wav(work, IAB_PICTURE_FRAMES);
    let imp = work.join("imp");
    let mut command = Command::cargo_bin("imfwizard").unwrap();
    command.args([
        "create",
        "-o",
        &imp.to_string_lossy(),
        "-t",
        "Immersive",
        "--video",
        &picture.to_string_lossy(),
        "--audio",
        &sound.to_string_lossy(),
        "--audio-lang",
        "de-DE",
        "--atmos",
        &atmos.to_string_lossy(),
        "--fps-num",
        &IAB_EDIT_RATE.to_string(),
        "--fps-den",
        "1",
    ]);
    command.args(extra);
    (imp, command)
}

fn package_file(imp: &Path, prefix: &str) -> PathBuf {
    let matching: Vec<PathBuf> = std::fs::read_dir(imp)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix))
        })
        .collect();
    assert_eq!(matching.len(), 1, "{prefix} files: {matching:?}");
    matching.into_iter().next().unwrap()
}

fn between<'a>(text: &'a str, open: &str, close: &str) -> &'a str {
    let start = text.find(open).unwrap_or_else(|| panic!("no {open}")) + open.len();
    let end = start
        + text[start..]
            .find(close)
            .unwrap_or_else(|| panic!("no {close}"));
    &text[start..end]
}

fn urn_uuid(bytes: [u8; 16]) -> String {
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "urn:uuid:{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

#[test]
fn create_wraps_an_ia_bitstream_directory_as_an_iab_sequence() {
    let work = TempDir::new().unwrap();
    let (atmos, frames) = ia_bitstream_directory(work.path(), IAB_PICTURE_FRAMES);
    let (imp, mut command) = create_with_atmos(work.path(), &atmos, &[]);
    command
        .assert()
        .success()
        .stdout(predicate::str::contains(PASSED));

    let track_file = package_file(&imp, "IAB_");
    let track_file_id = track_file
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .trim_start_matches("IAB_")
        .to_string();
    let mut reader = asdcplib::as02::iab::MxfReader::new();
    reader.open_read(&track_file.to_string_lossy()).unwrap();
    assert_eq!(reader.frame_count().unwrap(), IAB_PICTURE_FRAMES);
    for (index, expected) in frames.iter().enumerate() {
        assert_eq!(
            reader.read_frame(index as u32).unwrap(),
            expected.as_slice(),
            "frame {index}"
        );
    }
    let descriptor = reader.iab_essence_descriptor().unwrap();
    let label = reader.soundfield_label().unwrap();
    assert_eq!(label.spoken_language.as_deref(), Some("de-DE"));
    assert_eq!(label.title.as_deref(), Some("Immersive"));

    let cpl = std::fs::read_to_string(package_file(&imp, "CPL_")).unwrap();
    assert_eq!(cpl.matches(IAB_SEQUENCE_OPEN).count(), 1, "{cpl}");
    let sequence = between(&cpl, IAB_SEQUENCE_OPEN, IAB_SEQUENCE_CLOSE);
    assert!(sequence.contains("<EditRate>24 1</EditRate>"), "{sequence}");
    assert!(
        sequence.contains(&format!(
            "<IntrinsicDuration>{IAB_PICTURE_FRAMES}</IntrinsicDuration>"
        )),
        "{sequence}"
    );
    assert!(
        sequence.contains(&format!(
            "<TrackFileId>urn:uuid:{track_file_id}</TrackFileId>"
        )),
        "{sequence}"
    );

    let source_encoding = between(sequence, "<SourceEncoding>", "</SourceEncoding>");
    let entry = cpl
        .split("<EssenceDescriptor>")
        .skip(1)
        .find(|entry| entry.contains(&format!("<Id>{source_encoding}</Id>")))
        .unwrap_or_else(|| panic!("no EssenceDescriptor {source_encoding} in\n{cpl}"));
    let body = between(
        entry,
        "<r0:IABEssenceDescriptor",
        "</r0:IABEssenceDescriptor>",
    );
    for expected in [
        format!(
            "<r1:InstanceID>{}</r1:InstanceID>",
            urn_uuid(descriptor.instance_id)
        ),
        format!("<r1:EssenceLength>{IAB_PICTURE_FRAMES}</r1:EssenceLength>"),
        format!(
            "<r1:InstanceID>{}</r1:InstanceID>",
            urn_uuid(label.instance_id)
        ),
        "<r1:MCATagSymbol>IAB</r1:MCATagSymbol>".to_string(),
    ] {
        assert!(body.contains(&expected), "no {expected} in\n{body}");
    }

    for listing in [package_file(&imp, "PKL_"), imp.join("ASSETMAP.xml")] {
        let xml = std::fs::read_to_string(&listing).unwrap();
        assert!(
            xml.contains(&format!("<Id>urn:uuid:{track_file_id}</Id>")),
            "{} does not list the IAB track file",
            listing.display()
        );
    }

    let photon = imfwizard_core::photon::run_photon(&imp, None)
        .unwrap_or_else(|e| panic!("Photon did not analyse the IAB package: {e}"));
    assert!(
        photon.errors.is_empty(),
        "Photon errors {:?}, findings {:?}",
        photon.errors,
        photon.details
    );
    assert!(
        photon.details.is_empty(),
        "Photon findings {:?}",
        photon.details
    );
}

#[test]
fn check_refuses_an_ia_bitstream_that_does_not_cover_the_picture() {
    let work = TempDir::new().unwrap();
    let (atmos, _) = ia_bitstream_directory(work.path(), IAB_PICTURE_FRAMES - 1);
    let (imp, mut command) = create_with_atmos(work.path(), &atmos, &["--check"]);
    command
        .assert()
        .failure()
        .stderr(predicate::str::contains(format!(
            "the Atmos IA bitstream is {} frames but the picture is {IAB_PICTURE_FRAMES}",
            IAB_PICTURE_FRAMES - 1
        )));
    assert!(!imp.exists(), "the check must write nothing");
}

#[test]
fn check_refuses_an_empty_or_missing_ia_bitstream_directory() {
    let work = TempDir::new().unwrap();
    let empty = work.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let (imp, mut command) = create_with_atmos(work.path(), &empty, &["--check"]);
    command
        .assert()
        .failure()
        .stderr(predicate::str::contains("no IA bitstream frame files in"));
    assert!(!imp.exists(), "the check must write nothing");

    let missing = work.path().join("no_such_directory");
    let (_, mut command) = create_with_atmos(work.path(), &missing, &["--check"]);
    command
        .assert()
        .failure()
        .stderr(predicate::str::contains(format!(
            "Atmos IA bitstream directory not found: {}",
            missing.display()
        )));
}
