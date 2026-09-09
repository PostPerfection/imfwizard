use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FPS: u32 = 24;
// each reel is eight frames and each event keeps six of them, from a different
// place in the two reels, so a resource playing the whole source would show
const MEDIA_FRAMES: u32 = 8;
const EVENT_FRAMES: u64 = 6;
const SECOND_EVENT_SOURCE_IN: u64 = 2;
const EVENTS: usize = 2;

const EDL_TITLE: &str = "Edl Cut";
const XMEML_TITLE: &str = "Xmeml Cut";

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

fn write_media(dir: &Path) -> PathBuf {
    let media = dir.join("media");
    std::fs::create_dir_all(&media).unwrap();
    for reel in 1..=EVENTS {
        let made = std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error"])
            .args([
                "-f",
                "lavfi",
                "-i",
                &format!("testsrc2=size={WIDTH}x{HEIGHT}:rate={FPS}:duration=1"),
            ])
            .args([
                "-f",
                "lavfi",
                "-i",
                &format!("sine=frequency={}:sample_rate=48000:duration=1", 440 * reel),
            ])
            .args(["-frames:v", &MEDIA_FRAMES.to_string()])
            .args(["-c:v", "ffv1", "-c:a", "pcm_s24le", "-shortest"])
            .arg(media.join(format!("REEL00{reel}.mov")))
            .output()
            .expect("ffmpeg");
        assert!(
            made.status.success(),
            "{}",
            String::from_utf8_lossy(&made.stderr)
        );
    }
    media
}

fn write_edl(dir: &Path) -> PathBuf {
    let path = dir.join("cut.edl");
    std::fs::write(
        &path,
        format!(
            "TITLE: {EDL_TITLE}\nFCM: NON-DROP FRAME\n\n\
             001  REEL001  V     C        00:00:00:00 00:00:00:06 00:00:00:00 00:00:00:06\n\
             002  REEL002  V     C        00:00:00:02 00:00:00:08 00:00:00:06 00:00:00:12\n"
        ),
    )
    .unwrap();
    path
}

fn write_xmeml(dir: &Path) -> PathBuf {
    let path = dir.join("cut.xml");
    std::fs::write(
        &path,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<xmeml version="5">
  <sequence>
    <name>{XMEML_TITLE}</name>
    <rate><timebase>{FPS}</timebase><ntsc>FALSE</ntsc></rate>
    <media>
      <video>
        <track>
          <clipitem>
            <name>REEL001</name>
            <start>0</start><end>6</end><in>0</in><out>6</out>
            <file><name>REEL001.mov</name></file>
          </clipitem>
          <clipitem>
            <name>REEL002</name>
            <start>6</start><end>12</end><in>2</in><out>8</out>
            <file><name>REEL002.mov</name></file>
          </clipitem>
        </track>
      </video>
    </media>
  </sequence>
</xmeml>
"#
        ),
    )
    .unwrap();
    path
}

fn conform(timeline: &Path, media: &Path, imp: &Path) {
    cmd()
        .args(["conform", "--input", &timeline.to_string_lossy()])
        .args(["--media-dir", &media.to_string_lossy()])
        .args(["--output", &imp.to_string_lossy()])
        .assert()
        .success();
}

fn files_starting_with(imp: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(imp)
        .expect("the IMP directory")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix))
        })
        .collect();
    found.sort();
    found
}

/// The text of every `<tag>` inside the first sequence of `element`.
fn sequence_values(cpl: &str, element: &str, tag: &str) -> Vec<String> {
    let open = format!("<cc:{element}");
    let close = format!("</cc:{element}>");
    let start = cpl.find(&open).unwrap_or_else(|| panic!("no {element}"));
    let end = cpl[start..].find(&close).expect("the sequence closes") + start;
    cpl[start..end]
        .split(&format!("<{tag}>"))
        .skip(1)
        .map(|rest| rest.split(&format!("</{tag}>")).next().unwrap().to_string())
        .collect()
}

fn assert_conformed(imp: &Path, title: &str) {
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(imp.join("conform_manifest.json")).unwrap())
            .expect("the conform manifest is JSON");
    assert_eq!(manifest["title"], title);
    let events = manifest["events"].as_array().expect("the manifest events");
    assert_eq!(events.len(), EVENTS);
    assert_eq!(events[1]["source_in"], SECOND_EVENT_SOURCE_IN);
    for event in events {
        assert!(
            event["picture_track_file"].is_string() && event["sound_track_file"].is_string(),
            "the manifest does not say what event {} became: {event}",
            event["event_number"]
        );
    }

    let cpl_path = files_starting_with(imp, "CPL_")
        .into_iter()
        .next()
        .expect("the IMP has a CPL");
    let cpl = std::fs::read_to_string(&cpl_path).unwrap();
    // one virtual track a kind: two image sequences would be two main image tracks
    assert_eq!(cpl.matches("<cc:MainImageSequence").count(), 1, "{cpl}");
    assert_eq!(cpl.matches("<cc:MainAudioSequence").count(), 1, "{cpl}");

    for element in ["MainImageSequence", "MainAudioSequence"] {
        let durations = sequence_values(&cpl, element, "SourceDuration");
        assert_eq!(
            durations,
            vec![EVENT_FRAMES.to_string(); EVENTS],
            "{element} does not play {EVENTS} trimmed resources"
        );
        let track_files = sequence_values(&cpl, element, "TrackFileId");
        assert_eq!(track_files.len(), EVENTS);
        assert_ne!(track_files[0], track_files[1], "{element} plays one file");
    }

    let pictures = files_starting_with(imp, imfwizard_core::imp::PICTURE_PREFIX);
    assert_eq!(pictures.len(), EVENTS);
    for picture in &pictures {
        let mut reader = asdcplib::as02::jp2k::MxfReader::new();
        reader
            .open_read(&picture.to_string_lossy())
            .expect("the picture MXF opens");
        let descriptor = reader.rgba_essence_descriptor().unwrap();
        assert_eq!(descriptor.container_duration, Some(EVENT_FRAMES));
        let mut buf = vec![0u8; 16 << 20];
        let size = reader.read_frame(0, &mut buf, None, None).unwrap();
        buf.truncate(size);
        reader.close().unwrap();
        let decoded = postkit::grok_decoder::decode(buf, 0).expect("the wrapped frame decodes");
        assert_eq!((decoded.width, decoded.height), (WIDTH, HEIGHT));
    }

    let photon = imfwizard_core::photon::run_photon(imp, None).expect("Photon analyses the IMP");
    assert!(
        photon.errors.is_empty() && photon.warnings.is_empty(),
        "Photon errors {:?}, warnings {:?}",
        photon.errors,
        photon.warnings
    );
}

#[test]
fn an_edl_timeline_conforms_to_an_imp() {
    let dir = TempDir::new().unwrap();
    let media = write_media(dir.path());
    let imp = dir.path().join("imp_edl");
    conform(&write_edl(dir.path()), &media, &imp);
    assert_conformed(&imp, EDL_TITLE);
}

#[test]
fn an_xmeml_timeline_conforms_to_an_imp() {
    let dir = TempDir::new().unwrap();
    let media = write_media(dir.path());
    let imp = dir.path().join("imp_xmeml");
    conform(&write_xmeml(dir.path()), &media, &imp);
    assert_conformed(&imp, XMEML_TITLE);
}

#[test]
fn conforming_needs_both_a_media_directory_and_an_output() {
    let dir = TempDir::new().unwrap();
    cmd()
        .args([
            "conform",
            "--input",
            &write_edl(dir.path()).to_string_lossy(),
        ])
        .args(["--media-dir", &dir.path().to_string_lossy()])
        .assert()
        .failure()
        .stderr(predicates::str::contains("--media-dir and --output"));
}

#[test]
fn a_reel_with_no_media_is_named() {
    let dir = TempDir::new().unwrap();
    let empty = dir.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    cmd()
        .args([
            "conform",
            "--input",
            &write_edl(dir.path()).to_string_lossy(),
        ])
        .args(["--media-dir", &empty.to_string_lossy()])
        .args(["--output", &dir.path().join("imp").to_string_lossy()])
        .assert()
        .failure()
        .stderr(predicates::str::contains("REEL001"));
}
