//! `create --audio-role` labels an accessibility sound track, and the CPL links
//! the resource to that descriptor by SourceEncoding.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const RASTER: &str = "1920x1080";
const FRAMES: u32 = 4;
const FPS: u32 = 24;
const SAMPLE_RATE: u32 = 48_000;
const LANGUAGE: &str = "en-US";

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

fn ffmpeg(arguments: &[&str]) {
    let out = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(arguments)
        .output()
        .expect("ffmpeg");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// One mono narration track exactly as long as the picture.
fn narration(dir: &Path) -> PathBuf {
    let wav = dir.join("narration.wav");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("sine=frequency=440:duration=5:sample_rate={SAMPLE_RATE}"),
        "-af",
        &format!("atrim=end_sample={}", FRAMES * SAMPLE_RATE / FPS),
        "-ac",
        "1",
        "-c:a",
        "pcm_s24le",
        &wav.to_string_lossy(),
    ]);
    wav
}

fn grey_clip(dir: &Path) -> PathBuf {
    let clip = dir.join("picture.mkv");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("color=c=gray:s={RASTER}:r={FPS}"),
        "-frames:v",
        &FRAMES.to_string(),
        "-c:v",
        "ffv1",
        "-pix_fmt",
        "gbrp",
        &clip.to_string_lossy(),
    ]);
    clip
}

fn file_starting_with(imp: &Path, prefix: &str) -> PathBuf {
    std::fs::read_dir(imp)
        .expect("the IMP directory")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix))
        })
        .unwrap_or_else(|| panic!("the IMP has no {prefix} file"))
}

fn build(dir: &Path, role: &str) -> PathBuf {
    let clip = grey_clip(dir);
    let wav = narration(dir);
    let imp = dir.join(format!("imp_{role}"));
    cmd()
        .args(["create", "-o", &imp.to_string_lossy()])
        .args(["-t", "Accessible"])
        .args(["--video", &clip.to_string_lossy()])
        .args(["--raster", RASTER])
        .args(["--audio", &wav.to_string_lossy()])
        .args(["--audio-lang", LANGUAGE])
        .args(["--audio-role", role])
        .args(["--fps-num", &FPS.to_string(), "--fps-den", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("IMP created"));
    imp
}

/// Every text between `<tag>` and its close, in document order.
fn values_of(xml: &str, tag: &str) -> Vec<String> {
    xml.split(&format!("<{tag}>"))
        .skip(1)
        .filter_map(|rest| rest.split_once('<').map(|(value, _)| value.to_string()))
        .collect()
}

/// The Id of every EssenceDescriptor in the CPL's descriptor list.
fn descriptor_ids(xml: &str) -> Vec<String> {
    let (_, list) = xml
        .split_once("<EssenceDescriptorList>")
        .expect("a descriptor list");
    values_of(list, "Id")
}

/// The MCA tag symbols the sound descriptor carries, soundfield group first.
fn mca_tag_symbols(xml: &str) -> Vec<String> {
    values_of(xml, "r1:MCATagSymbol")
}

/// `ad` and `hi` each write their own MCA channel label, and the sound
/// resource's SourceEncoding names a descriptor the CPL actually lists, which
/// is the link ST 2067-3 makes mandatory.
#[test]
fn each_accessibility_role_writes_its_own_label_linked_by_source_encoding() {
    // sgVA is Visual Accessibility and sgHA Hearing Accessibility
    for (role, channel, soundfield) in [("ad", "chVIN", "sgVA"), ("hi", "chHI", "sgHA")] {
        let dir = TempDir::new().unwrap();
        let imp = build(dir.path(), role);
        let cpl = std::fs::read_to_string(file_starting_with(&imp, "CPL_")).unwrap();

        let symbols = mca_tag_symbols(&cpl);
        assert!(
            symbols.iter().any(|symbol| symbol == channel),
            "--audio-role {role} wrote no {channel} channel label, got {symbols:?}"
        );
        assert!(
            symbols.iter().any(|symbol| symbol == soundfield),
            "--audio-role {role} wrote no {soundfield} soundfield group, got {symbols:?}"
        );
        assert!(
            cpl.contains(&format!("<r1:RFC5646SpokenLanguage>{LANGUAGE}<")),
            "--audio-role {role} lost the spoken language"
        );

        // one SourceEncoding a resource, each naming a listed descriptor
        let ids = descriptor_ids(&cpl);
        let links = values_of(&cpl, "SourceEncoding");
        assert_eq!(links.len(), 2, "one picture resource and one sound one");
        for link in &links {
            assert!(
                ids.contains(link),
                "SourceEncoding {link} names no descriptor in {ids:?}"
            );
        }
        assert_ne!(links[0], links[1], "the two tracks share a descriptor");
    }
}

/// The labels are on the track file too, not only in the CPL: the CPL entry is
/// the descriptor read back off the MXF, so the two have to agree.
#[test]
fn the_role_labels_are_on_the_sound_track_file() {
    for (role, channel) in [("ad", "chVIN"), ("hi", "chHI")] {
        let dir = TempDir::new().unwrap();
        let imp = build(dir.path(), role);
        let sound = file_starting_with(&imp, "AUDIO_");

        let mut reader = asdcplib::as02::pcm::MxfReader::new();
        reader
            .open_read(
                &sound.to_string_lossy(),
                asdcplib::Rational::new(FPS as i32, 1),
            )
            .expect("the sound MXF opens");
        assert_eq!(
            reader.channel_assignment().expect("channel assignment"),
            Some(asdcplib::as02::pcm::IMF_CHANNEL_ASSIGNMENT_MCA)
        );
        let labels = reader.mca_label_subdescriptors().expect("mca labels");
        reader.close().unwrap();

        assert!(
            labels
                .iter()
                .any(|label| label.tag_symbol == channel),
            "--audio-role {role}: the track file carries no {channel}, got {:?}",
            labels
                .iter()
                .map(|label| label.tag_symbol.clone())
                .collect::<Vec<_>>()
        );
        let group = labels
            .iter()
            .find(|label| label.kind == asdcplib::pcm::McaLabelKind::SoundfieldGroup)
            .expect("a soundfield group");
        assert_eq!(group.spoken_language.as_deref(), Some(LANGUAGE));
    }
}

/// A role the flag does not know is refused naming the two that work.
#[test]
fn an_unknown_role_is_refused_naming_the_ones_that_work() {
    let dir = TempDir::new().unwrap();
    let clip = grey_clip(dir.path());
    let wav = narration(dir.path());

    cmd()
        .args(["create", "-o", &dir.path().join("imp").to_string_lossy()])
        .args(["-t", "Bad role"])
        .args(["--video", &clip.to_string_lossy()])
        .args(["--raster", RASTER])
        .args(["--audio", &wav.to_string_lossy()])
        .args(["--audio-role", "commentary"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("commentary"))
        .stderr(predicate::str::contains("ad"))
        .stderr(predicate::str::contains("hi"));
}
