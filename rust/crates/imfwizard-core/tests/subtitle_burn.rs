//! `create --burn-subtitle`: the refusal matrix and the burn sources there is a
//! cue reader for.
//!
//! The picture assertions live in postkit (`tests/subtitle_burn_e2e.rs`), which
//! can decode a codestream back to pixels. What matters here is that the flag
//! combinations fail before an encode starts.

use imfwizard_core::subtitle_burn::{
    BurnTarget, check_burn_supported, prepare_subtitle_burn, resolve_burn_style,
};
use postkit::subtitle_raster::{BurnEffect, BurnStyleOverrides};
use std::path::{Path, PathBuf};

const FPS: u32 = 24;

const SRT: &str = "1\n00:00:00,000 --> 00:00:01,000\nfirst line\n\n\
                   2\n00:00:02,000 --> 00:00:03,000\nsecond line\n\n";

fn write_srt(dir: &Path) -> PathBuf {
    let path = dir.join("cues.srt");
    std::fs::write(&path, SRT).unwrap();
    path
}

/// Everything a plain display-RGB video encode looks like.
fn video_source(timed_text: &[PathBuf]) -> BurnTarget<'_> {
    BurnTarget {
        timed_text,
        frames_already_xyz: false,
        input_is_codestreams: false,
    }
}

#[test]
fn a_burn_is_refused_wherever_it_would_be_drawn_in_the_wrong_place() {
    let dir = tempfile::tempdir().unwrap();
    let srt = write_srt(dir.path());
    let elsewhere = vec![dir.path().join("other.ttml")];

    check_burn_supported(&srt, &video_source(&[])).expect("a plain display-RGB burn is fine");
    check_burn_supported(&srt, &video_source(&elsewhere))
        .expect("a different timed-text file is fine");

    let missing = dir.path().join("nope.srt");
    let same = vec![srt.clone()];
    for (label, result, needle) in [
        (
            "missing file",
            check_burn_supported(&missing, &video_source(&[])),
            "not found",
        ),
        (
            "same file as --subtitle",
            check_burn_supported(&srt, &video_source(&same)),
            "pick one",
        ),
        (
            "J2K input",
            check_burn_supported(
                &srt,
                &BurnTarget {
                    input_is_codestreams: true,
                    ..video_source(&[])
                },
            ),
            "already compressed",
        ),
        (
            "frames already X'Y'Z'",
            check_burn_supported(
                &srt,
                &BurnTarget {
                    frames_already_xyz: true,
                    ..video_source(&[])
                },
            ),
            "X'Y'Z' already",
        ),
    ] {
        let err = result.expect_err(label);
        assert!(err.contains(needle), "{label}: got {err}");
    }
}

/// TTML/IMSC is what imfwizard packages, not what it reads back to cues, so the
/// refusal has to name a way out rather than leave the user at a dead end.
#[test]
fn ttml_is_refused_with_a_message_naming_what_to_pass_instead() {
    let dir = tempfile::tempdir().unwrap();
    let ttml = dir.path().join("subs.ttml");
    std::fs::write(
        &ttml,
        r#"<?xml version="1.0"?><tt xmlns="http://www.w3.org/ns/ttml"><body><div><p begin="0s" end="1s">hi</p></div></body></tt>"#,
    )
    .unwrap();
    let err = prepare_subtitle_burn(&ttml, None, &BurnStyleOverrides::default(), FPS).unwrap_err();
    assert!(err.contains("TTML/IMSC"), "got: {err}");
    assert!(
        err.contains("SRT"),
        "the message must name a way out: {err}"
    );
}

#[test]
fn a_format_with_no_cue_reader_is_refused_by_extension() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["subs.vtt", "subs.stl", "subs.pac"] {
        let path = dir.path().join(name);
        std::fs::write(&path, "whatever").unwrap();
        let err =
            prepare_subtitle_burn(&path, None, &BurnStyleOverrides::default(), FPS).unwrap_err();
        assert!(err.contains("SRT"), "{name}: {err}");
    }
}

#[test]
fn a_missing_burn_in_font_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let srt = write_srt(dir.path());
    let font = dir.path().join("nothere.ttf");
    let err =
        prepare_subtitle_burn(&srt, Some(&font), &BurnStyleOverrides::default(), FPS).unwrap_err();
    assert!(err.contains("font not found"), "got: {err}");
}

/// A burn source with no cues would encode a subtitle-free picture, which looks
/// exactly like a burn that silently did nothing.
#[test]
fn an_empty_cue_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let srt = dir.path().join("empty.srt");
    std::fs::write(&srt, "").unwrap();
    let err = prepare_subtitle_burn(&srt, None, &BurnStyleOverrides::default(), FPS).unwrap_err();
    assert!(err.contains("no subtitle cues"), "got: {err}");
}

/// The styled formats reach the burn through postkit's parsers, so a cue set
/// built from ASS has to come out as cues rather than an unsupported format.
#[test]
fn ass_is_a_burn_source() {
    let dir = tempfile::tempdir().unwrap();
    let ass = dir.path().join("subs.ass");
    std::fs::write(
        &ass,
        "[Script Info]\nTitle: t\n\n\
[V4+ Styles]\n\
Format: Name, Fontname, Fontsize, PrimaryColour, Bold, Italic, Underline, Alignment\n\
Style: Default,Arial,40,&H00FFFFFF,0,0,0,2\n\n\
[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:03.50,Default,,0,0,0,,hello\n",
    )
    .unwrap();
    let cues = imfwizard_core::subtitle_burn::load_styled_cues(&ass).unwrap();
    assert_eq!(cues.len(), 1);
    assert_eq!(cues[0].plain_text(), "hello");
}

/// A named appearance has to reach the built burn, and a value outside the
/// rasteriser's range has to stop before an encode does anything.
#[test]
fn an_appearance_override_builds_a_burn_and_a_bad_one_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let srt = write_srt(dir.path());
    let appearance = BurnStyleOverrides {
        font_size_percent: Some(8.0),
        effect: Some(BurnEffect::Outline),
        colour: Some(postkit::subtitle_formats::Rgba {
            r: 255,
            g: 255,
            b: 0,
            a: 255,
        }),
        ..BurnStyleOverrides::default()
    };

    let style = resolve_burn_style(&appearance).expect("a drawable appearance");
    assert_eq!(style.font_size_ratio, 0.08);
    assert_eq!(style.effect, BurnEffect::Outline);
    assert_eq!(style.default_colour.g, 255);
    assert_eq!(style.default_colour.b, 0);

    match prepare_subtitle_burn(&srt, None, &appearance, FPS) {
        Ok(_) => {}
        Err(error) => assert!(
            error.contains("cannot burn"),
            "an override must not be what fails: {error}"
        ),
    }

    let err = resolve_burn_style(&BurnStyleOverrides {
        x_scale: Some(0.0),
        ..BurnStyleOverrides::default()
    })
    .unwrap_err();
    assert!(err.contains("burn-in appearance"), "got: {err}");
}
