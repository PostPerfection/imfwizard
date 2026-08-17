use assert_cmd::Command;
use postkit::certificate::KdmFormulation;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tempfile::TempDir;

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

#[test]
fn version_flag() {
    cmd().arg("--version").assert().success().stdout(
        predicate::str::contains("imfwizard")
            .and(predicate::str::contains(env!("CARGO_PKG_VERSION"))),
    );
}

#[test]
fn help_flag() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("create"))
        .stdout(predicate::str::contains("encode"))
        .stdout(predicate::str::contains("analytics"))
        .stdout(predicate::str::contains("profiles"));
}

#[test]
fn create_subcommand_help() {
    cmd()
        .args(["create", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Create"));
}

#[test]
fn encode_subcommand_help() {
    cmd()
        .args(["encode", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Encode"));
}

#[test]
fn profiles_lists_output() {
    cmd().args(["profiles"]).assert().success();
}

#[test]
fn analyze_missing_directory() {
    let dir = TempDir::new().unwrap();
    let nonexistent = dir.path().join("does_not_exist");

    cmd()
        .args(["analyze", nonexistent.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn analyze_empty_directory() {
    let dir = TempDir::new().unwrap();

    cmd()
        .args(["analyze", dir.path().to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn hash_missing_file() {
    cmd()
        .args(["hash", "/nonexistent/file.mxf"])
        .assert()
        .failure();
}

#[test]
fn hash_existing_file() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("test.bin");
    std::fs::write(&file, b"hello world").unwrap();

    cmd()
        .args(["hash", file.to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn timecode_conversion() {
    cmd()
        .args(["timecode", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("timecode"));
}

#[test]
fn verbose_flag_accepted() {
    cmd().args(["-v", "profiles"]).assert().success();
}

#[test]
fn create_requires_input() {
    // create without arguments should fail
    cmd().args(["create"]).assert().failure();
}

/// Every source treatment flag has to reach `create`, since the GUI's Properties
/// panel offers a control for each and the two are meant to stay one tool.
#[test]
fn create_offers_every_source_treatment_flag() {
    let help = cmd().args(["create", "--help"]).assert().success();
    let mut assertion = help;
    for flag in [
        "--audio-delay",
        "--source-colourspace",
        "--trim-start",
        "--trim-end",
        "--still-length",
        "--burn-subtitle",
        "--burn-subtitle-font",
    ] {
        assertion = assertion.stdout(predicate::str::contains(flag));
    }
}

/// A negative delay is a value, not an unknown flag. Proven by making the delay
/// itself the thing that fails: a one second WAV cannot absorb five seconds.
#[test]
fn a_negative_audio_delay_reaches_the_delay() {
    let dir = TempDir::new().unwrap();
    let wav = dir.path().join("sound.wav");
    write_sine_wav(&wav, 0.5);
    cmd()
        .args([
            "create",
            "-o",
            &dir.path().to_string_lossy(),
            "-t",
            "T",
            "--audio",
            &wav.to_string_lossy(),
            "--audio-delay",
            "-5000",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("audio delay of -5000 ms"));
}

/// A J2K directory goes straight to the wrapper, so a source colour space would
/// have nothing to act on and must not be accepted in silence.
#[test]
fn a_source_colourspace_on_precompressed_picture_is_refused() {
    let dir = TempDir::new().unwrap();
    let frames = dir.path().join("j2k");
    std::fs::create_dir_all(&frames).unwrap();
    std::fs::write(frames.join("frame_00000000.j2c"), b"codestream").unwrap();

    cmd()
        .args([
            "create",
            "-o",
            &dir.path().join("out").to_string_lossy(),
            "-t",
            "T",
            "--video",
            &frames.to_string_lossy(),
            "--source-colourspace",
            "xyz",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already J2K"));
}

#[test]
fn an_unknown_source_colourspace_is_refused_by_name() {
    let dir = TempDir::new().unwrap();
    cmd()
        .args([
            "create",
            "-o",
            &dir.path().to_string_lossy(),
            "-t",
            "Test",
            "--source-colourspace",
            "rec601",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("rec601"));
}

/// postkit models only the Rec.709 to X'Y'Z' transform, so the wide-gamut and log
/// spaces are refused rather than encoded through the wrong matrix.
#[test]
fn a_colourspace_with_no_transform_is_refused() {
    let dir = TempDir::new().unwrap();
    for space in ["p3", "rec2020", "aces", "acescg", "logc"] {
        cmd()
            .args([
                "create",
                "-o",
                &dir.path().to_string_lossy(),
                "-t",
                "Test",
                "--source-colourspace",
                space,
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("X'Y'Z' transform is available"));
    }
}

/// --hdr says nothing transformed the essence, so a source the encoder would
/// transform contradicts it.
#[test]
fn hdr_refuses_a_source_the_encoder_would_transform() {
    let dir = TempDir::new().unwrap();
    let create = |space: &str| {
        cmd()
            .args([
                "create",
                "-o",
                &dir.path().to_string_lossy(),
                "-t",
                "Test",
                "--hdr",
                "pq-bt2020",
                "--source-colourspace",
                space,
            ])
            .assert()
            .failure()
    };
    create("rec709").stderr(predicate::str::contains("--source-colourspace rec709"));
    // xyz leaves the frames alone, so it composes with an HDR label
    create("xyz").stderr(predicate::str::contains("--source-colourspace").not());
    // no flag at all resolves to rec709, which the encoder transforms
    cmd()
        .args([
            "create",
            "-o",
            &dir.path().to_string_lossy(),
            "-t",
            "Test",
            "--hdr",
            "pq-bt2020",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--source-colourspace rec709"));
}

#[test]
fn a_bad_duration_spec_names_both_forms() {
    let dir = TempDir::new().unwrap();
    for flag in ["--trim-start", "--trim-end", "--still-length"] {
        cmd()
            .args([
                "create",
                "-o",
                &dir.path().to_string_lossy(),
                "-t",
                "Test",
                flag,
                "48",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("48f").and(predicate::str::contains("2s")));
    }
}

#[test]
fn a_still_length_without_a_still_input_is_refused() {
    let dir = TempDir::new().unwrap();
    let movie = dir.path().join("clip.mov");
    std::fs::write(&movie, b"not really a movie").unwrap();
    cmd()
        .args([
            "create",
            "-o",
            &dir.path().to_string_lossy(),
            "-t",
            "Test",
            "--video",
            &movie.to_string_lossy(),
            "--still-length",
            "2s",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--still-length"));
}

#[test]
fn a_still_input_without_a_still_length_is_refused() {
    let dir = TempDir::new().unwrap();
    let image = dir.path().join("slate.tif");
    std::fs::write(&image, b"not really a tiff").unwrap();
    cmd()
        .args([
            "create",
            "-o",
            &dir.path().to_string_lossy(),
            "-t",
            "Test",
            "--video",
            &image.to_string_lossy(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("single image"));
}

#[test]
fn transcode_subcommand_help() {
    cmd()
        .args(["transcode", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Transcode"));
}

#[test]
fn subtitle_convert_help() {
    cmd()
        .args(["subtitle-convert", "--help"])
        .assert()
        .success();
}

fn have_ffmpeg() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A one second colour-bars clip with a sine tone, at the raster asked for.
fn synthesize_clip(path: &Path, width: u32, height: u32) {
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc=s={width}x{height}:d=1:r=24"),
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=1000:duration=1",
            "-frames:v",
            "24",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "pcm_s24le",
            "-ar",
            "48000",
        ])
        .arg(path)
        .output()
        .expect("ffmpeg");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The headline feature: a video source has to come out the far end as a package.
/// This is the only test that runs a real J2K encode, and it is the one that
/// would have caught `create` shipping without the postkit `grok-ffi` feature,
/// which made every video encode fail on the first frame.
#[test]
fn a_video_source_encodes_and_packages() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let clip = dir.path().join("clip.mov");
    synthesize_clip(&clip, 1920, 1080);
    let output = dir.path().join("imp");

    cmd()
        .args([
            "create",
            "-o",
            &output.to_string_lossy(),
            "-t",
            "Encode Smoke",
            "--video",
            &clip.to_string_lossy(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("IMP created"));

    let codestreams = std::fs::read_dir(output.join("j2k"))
        .expect("j2k output directory")
        .filter(|entry| {
            entry
                .as_ref()
                .is_ok_and(|e| e.path().extension().is_some_and(|x| x == "j2c"))
        })
        .count();
    assert_eq!(codestreams, 24, "one codestream per source frame");

    let package: Vec<_> = std::fs::read_dir(&output)
        .unwrap()
        .filter_map(|e| Some(e.ok()?.file_name().to_string_lossy().into_owned()))
        .collect();
    assert!(package.iter().any(|n| n == "ASSETMAP.xml"), "{package:?}");
    assert!(package.iter().any(|n| n.starts_with("CPL_")), "{package:?}");
    assert!(
        package.iter().any(|n| n.starts_with("VIDEO_")),
        "{package:?}"
    );
}

/// A fractional rate has to reach the decoder as itself. At `fps=24` a 23.976
/// source gains a frame every 42 seconds, so 500 frames come out as 501 and the
/// composition runs long against a CPL that says 24000/1001.
#[test]
fn a_23_976_source_encodes_one_codestream_per_source_frame() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }
    if postkit::grok::find_grk_compress().is_none() {
        eprintln!("skipping: grk_compress not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let clip = dir.path().join("clip.mov");
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=s=1920x1080:r=24000/1001",
            "-frames:v",
            "500",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&clip)
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    let output = dir.path().join("imp");

    cmd()
        .args([
            "create",
            "-o",
            &output.to_string_lossy(),
            "-t",
            "Fractional Rate",
            "--video",
            &clip.to_string_lossy(),
            "--fps-num",
            "24000",
            "--fps-den",
            "1001",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("IMP created"));

    let codestreams = std::fs::read_dir(output.join("j2k"))
        .expect("j2k output directory")
        .filter(|entry| {
            entry
                .as_ref()
                .is_ok_and(|e| e.path().extension().is_some_and(|x| x == "j2c"))
        })
        .count();
    assert_eq!(
        codestreams, 500,
        "a 23.976 source decoded at 24 fps would gain a frame"
    );
}

/// `create` classifies its picture the way the GUI does, so a directory of
/// stills encodes through postkit instead of reaching the MXF wrapper as
/// codestreams it cannot read.
#[test]
fn an_image_sequence_directory_encodes_and_packages() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }
    if postkit::grok::find_grk_compress().is_none() {
        eprintln!("skipping: grk_compress not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let frames = dir.path().join("frames");
    std::fs::create_dir_all(&frames).unwrap();
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=s=1920x1080:r=24",
            "-frames:v",
            "3",
            // the tiff encoder writes 8 or 16 bits a component and App 2E takes
            // 8, 10 or 12
            "-pix_fmt",
            "rgb24",
        ])
        .arg(frames.join("frame_%06d.tif"))
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    let output = dir.path().join("imp");

    cmd()
        .args([
            "create",
            "-o",
            &output.to_string_lossy(),
            "-t",
            "Sequence Smoke",
            "--video",
            &frames.to_string_lossy(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("IMP created"));

    // the image-sequence encoder names its codestreams .j2k where the video one
    // names them .j2c
    let codestreams = std::fs::read_dir(output.join("j2k"))
        .expect("j2k output directory")
        .filter(|entry| {
            entry.as_ref().is_ok_and(|e| {
                e.path()
                    .extension()
                    .is_some_and(|x| x == "j2c" || x == "j2k")
            })
        })
        .count();
    assert_eq!(codestreams, 3, "one codestream per source frame");

    let package: Vec<_> = std::fs::read_dir(&output)
        .unwrap()
        .filter_map(|e| Some(e.ok()?.file_name().to_string_lossy().into_owned()))
        .collect();
    assert!(package.iter().any(|n| n == "ASSETMAP.xml"), "{package:?}");
    assert!(package.iter().any(|n| n.starts_with("CPL_")), "{package:?}");
    assert!(
        package.iter().any(|n| n.starts_with("VIDEO_")),
        "{package:?}"
    );
}

/// A burn draws display-RGB text onto decoded frames, so every route that hands
/// the encoder X'Y'Z' or nothing to draw on has to be refused before an encode
/// starts.
#[test]
fn a_burn_subtitle_is_refused_wherever_it_would_be_drawn_in_the_wrong_place() {
    let dir = TempDir::new().unwrap();
    let srt = dir.path().join("cues.srt");
    std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:02,000\nhello\n\n").unwrap();
    let clip = dir.path().join("clip.mov");
    std::fs::write(&clip, b"not really a movie").unwrap();
    let codestreams = dir.path().join("j2k");
    std::fs::create_dir_all(&codestreams).unwrap();
    std::fs::write(codestreams.join("frame_00000000.j2c"), b"codestream").unwrap();

    let create = |source: &Path, extra: &[&str]| {
        let mut command = cmd();
        command.args([
            "create",
            "-o",
            &dir.path().join("out").to_string_lossy(),
            "-t",
            "Burn",
            "--video",
            &source.to_string_lossy(),
            "--burn-subtitle",
            &srt.to_string_lossy(),
        ]);
        command.args(extra);
        command
    };

    for (mut command, needle) in [
        (
            create(&clip, &["--source-colourspace", "xyz"]),
            "X'Y'Z' already",
        ),
        (
            create(
                &clip,
                &["--hdr", "pq-bt2020", "--source-colourspace", "xyz"],
            ),
            "X'Y'Z' already",
        ),
        (create(&codestreams, &[]), "already compressed"),
        (
            create(&clip, &["--subtitle", &srt.to_string_lossy()]),
            "pick one",
        ),
    ] {
        command
            .assert()
            .failure()
            .stderr(predicate::str::contains(needle));
    }
}

/// The burn has to reach the encoder, not just pass the pre-encode checks: the
/// same clip encoded with and without cues must not come out the same picture.
#[test]
fn a_burnt_subtitle_changes_the_encoded_picture() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let srt = dir.path().join("cues.srt");
    std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:01,000\nburnt in\n\n").unwrap();
    if imfwizard_core::subtitle_burn::prepare_subtitle_burn(
        &srt,
        None,
        &postkit::subtitle_raster::BurnStyleOverrides::default(),
        postkit::encode::FrameRate::whole(24),
    )
    .is_err()
    {
        eprintln!("skipping: no font available to burn with");
        return;
    }
    let clip = dir.path().join("clip.mov");
    synthesize_clip(&clip, 1920, 1080);

    let first_codestream = |name: &str, burn: bool| -> Vec<u8> {
        let out = dir.path().join(name);
        let mut command = cmd();
        command.args([
            "create",
            "-o",
            &out.to_string_lossy(),
            "-t",
            name,
            "--video",
            &clip.to_string_lossy(),
        ]);
        if burn {
            command.args(["--burn-subtitle", &srt.to_string_lossy()]);
        }
        command.assert().success();
        let mut frames: Vec<PathBuf> = std::fs::read_dir(out.join("j2k"))
            .expect("j2k output directory")
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|x| x == "j2c"))
            .collect();
        frames.sort();
        std::fs::read(frames.first().expect("a codestream")).unwrap()
    };

    assert_ne!(
        first_codestream("plain", false),
        first_codestream("burnt", true),
        "the burn never reached the encoder"
    );
}

/// A held still is the input shape with no decoder of its own, so this is the
/// one that proves the burn is not tied to the video path. The hold costs one
/// encode per run of frames sharing a cue set, not one per frame.
#[test]
fn a_burnt_still_holds_one_codestream_per_cue_change_and_packages() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let card = dir.path().join("card.png");
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=gray:s=1920x1080",
            "-frames:v",
            "1",
        ])
        .arg(&card)
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );

    let srt = dir.path().join("cues.srt");
    std::fs::write(
        &srt,
        "1\n00:00:00,000 --> 00:00:01,000\nfirst line\n\n\
         2\n00:00:02,000 --> 00:00:03,000\nsecond line\n\n",
    )
    .unwrap();
    if imfwizard_core::subtitle_burn::prepare_subtitle_burn(
        &srt,
        None,
        &postkit::subtitle_raster::BurnStyleOverrides::default(),
        postkit::encode::FrameRate::whole(24),
    )
    .is_err()
    {
        eprintln!("skipping: no font available to burn with");
        return;
    }

    let output = dir.path().join("imp");
    cmd()
        .args([
            "create",
            "-o",
            &output.to_string_lossy(),
            "-t",
            "Burnt Still",
            "--video",
            &card.to_string_lossy(),
            "--still-length",
            "3s",
            "--burn-subtitle",
            &srt.to_string_lossy(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("IMP created"));

    // 3 seconds of hold over cues at 0-1s and 2-3s: the picture changes at
    // frames 24, 48 and 72, so four distinct frames are encoded.
    let held = output.join(postkit::still::HELD_PICTURE_DIR);
    let frame = |index: u64| std::fs::read(held.join(format!("frame_{index:08}.j2c"))).unwrap();
    assert_eq!(
        (0..72u64)
            .filter(|i| held.join(format!("frame_{i:08}.j2c")).exists())
            .count(),
        72,
        "every frame of the hold needs a file"
    );
    assert_eq!(frame(0), frame(12), "frames under one cue must be the same");
    assert_ne!(
        frame(0),
        frame(24),
        "the frame where the first cue ends must be a different picture"
    );
    assert_ne!(
        frame(24),
        frame(48),
        "the frame where the second cue starts must be a different picture"
    );

    let package: Vec<_> = std::fs::read_dir(&output)
        .unwrap()
        .filter_map(|e| Some(e.ok()?.file_name().to_string_lossy().into_owned()))
        .collect();
    assert!(package.iter().any(|n| n == "ASSETMAP.xml"), "{package:?}");
    assert!(package.iter().any(|n| n.starts_with("CPL_")), "{package:?}");
}

/// The wrapper refuses an illegal raster, but only once the encode has already
/// run. `create` has to refuse the same source up front, before it spends a pass
/// on essence nothing can wrap.
#[test]
fn an_illegal_raster_is_refused_before_any_encode() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let clip = dir.path().join("clip.mov");
    // the raster off a Sintel master, which App 2E has no place for
    synthesize_clip(&clip, 2048, 872);
    let output = dir.path().join("imp");

    cmd()
        .args([
            "create",
            "-o",
            &output.to_string_lossy(),
            "-t",
            "Illegal Raster",
            "--video",
            &clip.to_string_lossy(),
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("2048x872").and(predicate::str::contains("target-convert")),
        );

    assert!(
        !output.join("j2k").exists(),
        "nothing should be encoded before the raster is checked"
    );
}

// write a mono 16-bit 48k WAV of a 1 kHz sine at the given peak amplitude (0..1)
fn write_sine_wav(path: &std::path::Path, amplitude: f64) {
    let sample_rate = 48_000u32;
    let samples: Vec<i16> = (0..sample_rate)
        .map(|n| {
            let t = n as f64 / sample_rate as f64;
            let v = amplitude * (2.0 * std::f64::consts::PI * 1000.0 * t).sin();
            (v * i16::MAX as f64).round() as i16
        })
        .collect();
    let data_bytes = samples.len() * 2;
    let mut buf = Vec::with_capacity(44 + data_bytes);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&((36 + data_bytes) as u32).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // mono
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    buf.extend_from_slice(&2u16.to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    for s in samples {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, buf).unwrap();
}

#[test]
fn loudness_adjust_to_target_writes_output() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("in.wav");
    let output = dir.path().join("out.wav");
    write_sine_wav(&input, 0.25);
    cmd()
        .args([
            "loudness",
            input.to_str().unwrap(),
            "--adjust-to",
            "-24",
            "-o",
            output.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Gain applied"))
        .stdout(predicate::str::contains("Adjusted audio written"));
    assert!(output.exists(), "adjusted wav should exist");
}

#[test]
fn loudness_adjust_refuses_when_true_peak_would_clip() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("quiet.wav");
    let output = dir.path().join("loud.wav");
    // quiet source + loud target forces a large positive gain that breaches the ceiling
    write_sine_wav(&input, 0.05);
    cmd()
        .args([
            "loudness",
            input.to_str().unwrap(),
            "--adjust-to",
            "-3",
            "-o",
            output.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("true-peak ceiling exceeded"));
    assert!(!output.exists(), "clip-safe: nothing written on breach");
}

/// Any valid UUID identifies the CPL a test KDM targets: nothing reads the
/// composition itself.
const TEST_CPL_ID: &str = "1a2b3c4d-5e6f-4a8b-9c0d-1e2f3a4b5c6d";
const TEST_CONTENT_TITLE: &str = "Formulation Test";
/// The KDMRequiredExtensions element that only the dci- formulations emit.
const CONTENT_AUTHENTICATOR_ELEMENT: &str = "ContentAuthenticator";

/// A signer chain plus a separate device leaf, generated once per test binary
/// because each certificate costs an RSA key generation.
struct KdmCerts {
    _dir: TempDir,
    signer_cert: PathBuf,
    signer_key: PathBuf,
    /// Intermediate then root: postkit walks the signer chain to a self-issued
    /// certificate before it will issue a KDM, so the leaf alone is refused.
    signer_chain: Vec<PathBuf>,
    device_cert: PathBuf,
}

fn kdm_certs() -> &'static KdmCerts {
    static CERTS: OnceLock<KdmCerts> = OnceLock::new();
    CERTS.get_or_init(|| {
        let dir = TempDir::new().unwrap();
        assert_eq!(
            postkit::certificate::generate_chain("IMF Wizard Test", dir.path()),
            0,
            "signer chain generation should succeed"
        );
        let device_cert = dir.path().join("device.pem");
        let device_opts = postkit::certificate::CertOptions {
            cert_type: postkit::certificate::CertType::Leaf,
            common_name: "IMF Wizard Test Device".to_string(),
            output_cert: device_cert.clone(),
            output_key: dir.path().join("device.key"),
            issuer_cert: dir.path().join("intermediate.pem"),
            issuer_key: dir.path().join("intermediate.key"),
            ..Default::default()
        };
        assert_eq!(
            postkit::certificate::generate_certificate(&device_opts),
            0,
            "device certificate generation should succeed"
        );
        KdmCerts {
            signer_cert: dir.path().join("signer.pem"),
            signer_key: dir.path().join("signer.key"),
            signer_chain: vec![
                dir.path().join("intermediate.pem"),
                dir.path().join("root.pem"),
            ],
            device_cert,
            _dir: dir,
        }
    })
}

/// A KDM validity start one day out, as an ST 430-1 timestamp. postkit refuses
/// a KDM whose window starts on the day its signer certificate does, and the
/// fixture chain is minted now, so a window starting "now" would be rejected.
fn kdm_valid_from() -> String {
    let start = time::OffsetDateTime::now_utc() + time::Duration::days(1);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+00:00",
        start.year(),
        u8::from(start.month()),
        start.day(),
        start.hour(),
        start.minute(),
        start.second()
    )
}

/// A `kdm` invocation with every required argument filled in, so a test only
/// adds the formulation and device arguments it is about.
fn kdm_cmd(certs: &KdmCerts, output: &Path) -> Command {
    let mut command = cmd();
    command.args([
        "kdm",
        "--cpl-id",
        TEST_CPL_ID,
        "--content-title",
        TEST_CONTENT_TITLE,
        "--cert",
        certs.signer_cert.to_str().unwrap(),
        "--signer-cert",
        certs.signer_cert.to_str().unwrap(),
        "--signer-key",
        certs.signer_key.to_str().unwrap(),
        "--valid-from",
        &kdm_valid_from(),
        "-o",
        output.to_str().unwrap(),
    ]);
    for link in &certs.signer_chain {
        command.args(["--signer-chain", link.to_str().unwrap()]);
    }
    command
}

#[test]
fn every_formulation_reaches_the_kdm() {
    let certs = kdm_certs();
    let device_thumbprint = postkit::certificate::read_certificate(&certs.device_cert).thumbprint;
    assert!(
        !device_thumbprint.is_empty(),
        "device certificate should parse"
    );
    let dir = TempDir::new().unwrap();

    // (formulation, lists the supplied device, carries a ContentAuthenticator)
    let cases = [
        (KdmFormulation::ModifiedTransitional1, false, false),
        (KdmFormulation::MultipleModifiedTransitional1, true, false),
        (KdmFormulation::DciAny, false, true),
        (KdmFormulation::DciSpecific, true, true),
    ];

    for (formulation, lists_device, content_authenticator) in cases {
        let output = dir.path().join(format!("{formulation}.kdm.xml"));
        let mut command = kdm_cmd(certs, &output);
        command.args(["--formulation", &formulation.to_string()]);
        if lists_device {
            command.args(["--device-cert", certs.device_cert.to_str().unwrap()]);
        }
        command.assert().success();

        let xml = std::fs::read_to_string(&output).unwrap();
        assert_eq!(
            xml.contains(&device_thumbprint),
            lists_device,
            "{formulation} device list"
        );
        assert_eq!(
            xml.contains(CONTENT_AUTHENTICATOR_ELEMENT),
            content_authenticator,
            "{formulation} ContentAuthenticator"
        );
    }
}

#[test]
fn unknown_formulation_is_rejected() {
    let dir = TempDir::new().unwrap();
    let mut command = kdm_cmd(kdm_certs(), &dir.path().join("unused.kdm.xml"));
    let mut assertion = command
        .args(["--formulation", "no-such-formulation"])
        .assert()
        .failure();
    // the error has to name every spelling the user could have meant
    for formulation in [
        KdmFormulation::ModifiedTransitional1,
        KdmFormulation::MultipleModifiedTransitional1,
        KdmFormulation::DciAny,
        KdmFormulation::DciSpecific,
    ] {
        assertion = assertion.stderr(predicate::str::contains(formulation.to_string()));
    }
}

#[test]
fn device_certificates_must_agree_with_the_formulation() {
    let certs = kdm_certs();
    let dir = TempDir::new().unwrap();

    // a device-listing formulation with nothing to list
    let output = dir.path().join("no-devices.kdm.xml");
    kdm_cmd(certs, &output)
        .args(["--formulation", &KdmFormulation::DciSpecific.to_string()])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("device certificate")
                .and(predicate::str::contains(KdmFormulation::DciAny.to_string())),
        );
    assert!(
        !output.exists(),
        "nothing written on a rejected formulation"
    );

    // devices named under the default formulation, which lists none
    let output = dir.path().join("unlisted-devices.kdm.xml");
    kdm_cmd(certs, &output)
        .args(["--device-cert", certs.device_cert.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            KdmFormulation::MultipleModifiedTransitional1.to_string(),
        ));
    assert!(
        !output.exists(),
        "nothing written on a rejected formulation"
    );
}

/// ST 430-1 Annex C ForensicMarkFlag URIs. A KDM carries one per essence type
/// whose marking is off, and none at all while marking stays on.
const FORENSIC_MARK_PICTURE_DISABLE: &str =
    "http://www.smpte-ra.org/430-1/2006/KDM#mrkflg-picture-disable";
const FORENSIC_MARK_AUDIO_DISABLE: &str =
    "http://www.smpte-ra.org/430-1/2006/KDM#mrkflg-audio-disable";
/// Appended to the audio URI, with a channel number, when marking stops above a
/// channel rather than on all of them.
const FORENSIC_MARK_ABOVE_CHANNEL_SUFFIX: &str = "-above-channel-";
/// The channel a 7.1 + HI/VI order names: the HI and VI tracks sit above it.
const HI_VI_CHANNEL: u32 = 12;

#[test]
fn every_forensic_marking_state_reaches_the_kdm() {
    let certs = kdm_certs();
    let dir = TempDir::new().unwrap();
    let audio_above_channel =
        format!("{FORENSIC_MARK_AUDIO_DISABLE}{FORENSIC_MARK_ABOVE_CHANNEL_SUFFIX}{HI_VI_CHANNEL}");

    // (label, extra arguments, picture marking off, the audio flag expected)
    let cases: [(&str, Vec<String>, bool, Option<&str>); 5] = [
        ("marking-on", vec![], false, None),
        (
            "picture-off",
            vec!["--disable-forensic-marking-picture".to_string()],
            true,
            None,
        ),
        (
            "audio-off",
            vec!["--disable-forensic-marking-audio".to_string()],
            false,
            Some(FORENSIC_MARK_AUDIO_DISABLE),
        ),
        (
            "audio-off-above-channel",
            vec![
                "--disable-forensic-marking-audio".to_string(),
                HI_VI_CHANNEL.to_string(),
            ],
            false,
            Some(&audio_above_channel),
        ),
        (
            "both-off",
            vec![
                "-p".to_string(),
                "-a".to_string(),
                HI_VI_CHANNEL.to_string(),
            ],
            true,
            Some(&audio_above_channel),
        ),
    ];

    for (label, arguments, picture_disabled, audio_flag) in cases {
        let output = dir.path().join(format!("{label}.kdm.xml"));
        kdm_cmd(certs, &output).args(&arguments).assert().success();

        let xml = std::fs::read_to_string(&output).unwrap();
        assert_eq!(
            xml.contains(FORENSIC_MARK_PICTURE_DISABLE),
            picture_disabled,
            "{label} picture flag"
        );
        assert_eq!(
            xml.contains(FORENSIC_MARK_AUDIO_DISABLE),
            audio_flag.is_some(),
            "{label} audio flag"
        );
        if let Some(flag) = audio_flag {
            assert!(xml.contains(flag), "{label} audio flag spelling");
        }
        // the bare form must not smuggle in an above-channel limit
        assert_eq!(
            xml.contains(FORENSIC_MARK_ABOVE_CHANNEL_SUFFIX),
            audio_flag.is_some_and(|flag| flag.contains(FORENSIC_MARK_ABOVE_CHANNEL_SUFFIX)),
            "{label} above-channel limit"
        );
    }
}

/// The KeyIdList wrapper that only SMPTE output carries: Interop lists bare
/// KeyId elements instead.
const TYPED_KEY_ID_ELEMENT: &str = "TypedKeyId";

#[test]
fn the_kdm_format_chooses_the_key_id_layout() {
    let certs = kdm_certs();
    let dir = TempDir::new().unwrap();

    // (format argument, carries the SMPTE TypedKeyId wrapper)
    for (format, typed_key_id) in [("smpte", true), ("interop", false)] {
        let output = dir.path().join(format!("{format}.kdm.xml"));
        kdm_cmd(certs, &output)
            .args(["--format", format])
            .assert()
            .success();

        let xml = std::fs::read_to_string(&output).unwrap();
        assert_eq!(
            xml.contains(TYPED_KEY_ID_ELEMENT),
            typed_key_id,
            "{format} key id layout"
        );
    }
}

#[test]
fn the_annotation_override_reaches_the_kdm() {
    let certs = kdm_certs();
    let dir = TempDir::new().unwrap();
    let annotation = "Press screening, no marking";

    let output = dir.path().join("annotated.kdm.xml");
    kdm_cmd(certs, &output)
        .args(["--annotation", annotation])
        .assert()
        .success();
    let xml = std::fs::read_to_string(&output).unwrap();
    assert!(xml.contains(annotation), "annotation override");

    // the default still derives its text from the content title
    let output = dir.path().join("default-annotation.kdm.xml");
    kdm_cmd(certs, &output).assert().success();
    let xml = std::fs::read_to_string(&output).unwrap();
    assert!(!xml.contains(annotation), "override is not the default");
    assert!(xml.contains(TEST_CONTENT_TITLE), "derived annotation");
}

#[test]
fn a_kdm_without_content_keys_warns_that_it_unlocks_nothing() {
    let dir = TempDir::new().unwrap();
    kdm_cmd(kdm_certs(), &dir.path().join("minted.kdm.xml"))
        .assert()
        .success()
        .stderr(
            predicate::str::contains("freshly minted")
                .and(predicate::str::contains("will not unlock")),
        );
}

/// Every picture and audio-map flag has to reach `create`, since the GUI's
/// Properties panel offers a control for each and the two are one tool.
#[test]
fn create_offers_every_picture_and_audio_map_flag() {
    let help = cmd().args(["create", "--help"]).assert().success();
    let mut assertion = help;
    for flag in [
        "--crop-left",
        "--crop-right",
        "--crop-top",
        "--crop-bottom",
        "--auto-crop",
        "--auto-crop-threshold",
        "--fill-crop",
        "--deinterlace",
        "--denoise",
        "--rotate",
        "--flip",
        "--raster",
        "--audio-map",
    ] {
        assertion = assertion.stdout(predicate::str::contains(flag));
    }
}

/// The picture flags are refused before anything is encoded, since each of them
/// would otherwise be dropped without a word.
#[test]
fn the_picture_flags_are_refused_where_they_cannot_act() {
    let dir = TempDir::new().unwrap();
    let clip = dir.path().join("clip.mov");
    std::fs::write(&clip, b"not really a movie").unwrap();
    let codestreams = dir.path().join("j2k");
    std::fs::create_dir_all(&codestreams).unwrap();
    std::fs::write(codestreams.join("frame_00000000.j2c"), b"codestream").unwrap();

    let create = |source: &Path, extra: &[&str]| {
        let mut command = cmd();
        command.args([
            "create",
            "-o",
            &dir.path().join("out").to_string_lossy(),
            "-t",
            "Picture",
            "--video",
            &source.to_string_lossy(),
        ]);
        command.args(extra);
        command
    };

    for (mut command, needle) in [
        (create(&codestreams, &["--deinterlace"]), "already J2K"),
        (create(&clip, &["--raster", "1998x1080"]), "1998x1080"),
        (create(&clip, &["--rotate", "45"]), "45"),
        (create(&clip, &["--flip", "sideways"]), "sideways"),
        (
            create(&clip, &["--auto-crop-threshold", "0.2"]),
            "requires --auto-crop",
        ),
        (
            create(&clip, &["--fill-crop", "--crop-left", "10"]),
            "only one of them",
        ),
        (
            create(&clip, &["--fill-crop", "--auto-crop"]),
            "only one of them",
        ),
    ] {
        command
            .assert()
            .failure()
            .stderr(predicate::str::contains(needle));
    }
}

/// The auto-demuxed track is written after the map would have run, so a map with
/// no --audio has to say so rather than doing nothing.
#[test]
fn an_audio_map_without_an_audio_file_is_refused() {
    let dir = TempDir::new().unwrap();
    cmd()
        .args([
            "create",
            "-o",
            &dir.path().join("out").to_string_lossy(),
            "-t",
            "Map",
            "--audio-map",
            "1:L,2:R",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--audio-map needs --audio"));
}

#[test]
fn an_unreadable_audio_map_is_refused_by_name() {
    let dir = TempDir::new().unwrap();
    let wav = dir.path().join("sound.wav");
    write_sine_wav(&wav, 0.5);
    for (spec, needle) in [("1:Middle", "Middle"), ("banana", "not IN:OUT")] {
        cmd()
            .args([
                "create",
                "-o",
                &dir.path().join("out").to_string_lossy(),
                "-t",
                "Map",
                "--audio",
                &wav.to_string_lossy(),
                "--audio-map",
                spec,
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(needle));
    }
}

/// The picture plan has to reach the encoder, not just the log: a turned source
/// fitted into a named raster comes out on that raster, not the source's own.
#[test]
fn a_rotated_source_encodes_onto_the_named_raster() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let clip = dir.path().join("clip.mov");
    synthesize_clip(&clip, 2048, 1080);
    let output = dir.path().join("imp");

    cmd()
        .args([
            "create",
            "-o",
            &output.to_string_lossy(),
            "-t",
            "Rotated",
            "--video",
            &clip.to_string_lossy(),
            "--rotate",
            "90",
            "--raster",
            "1920x1080",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("IMP created"));

    let mut frames: Vec<PathBuf> = std::fs::read_dir(output.join("j2k"))
        .expect("j2k output directory")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|x| x == "j2c"))
        .collect();
    frames.sort();
    let codestream = std::fs::read(frames.first().expect("a codestream")).unwrap();
    let header = postkit::j2k::parse_j2k_header(&codestream).expect("a J2K header");
    assert_eq!(
        (header.width, header.height),
        (1920, 1080),
        "the encode landed on {}x{}",
        header.width,
        header.height
    );
}

/// A stereo ramp mapped to L/R plus a -6 dB centre: the centre lane has to carry
/// the left channel at half amplitude, which is what -6 dB is.
#[test]
fn an_audio_map_writes_the_gained_lane_into_the_package() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let wav = dir.path().join("stereo.wav");
    write_stereo_ramp_wav(&wav);
    let clip = dir.path().join("clip.mov");
    synthesize_clip(&clip, 1920, 1080);
    let output = dir.path().join("imp");

    cmd()
        .args([
            "create",
            "-o",
            &output.to_string_lossy(),
            "-t",
            "Mapped",
            "--video",
            &clip.to_string_lossy(),
            "--audio",
            &wav.to_string_lossy(),
            "--audio-map",
            "1:L,2:R,1:C@-6",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("IMP created"));

    let mapped = output.join("audio_mapped.wav");
    let mut reader = hound::WavReader::open(&mapped).expect("the mapped wav");
    assert_eq!(reader.spec().channels, 3);
    let samples: Vec<i32> = reader.samples::<i32>().map(|s| s.unwrap()).collect();
    let half = 10f64.powf(-6.0 / 20.0);
    for frame in 1..samples.len() / 3 {
        let left = samples[frame * 3] as f64;
        let centre = samples[frame * 3 + 2] as f64;
        assert!(
            (centre - left * half).abs() <= 1.0,
            "frame {frame}: centre {centre} is not {half} of {left}"
        );
    }
}

/// A rising 16-bit stereo ramp, so a gained lane is checked sample by sample.
fn write_stereo_ramp_wav(path: &std::path::Path) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for frame in 0..48_000i32 {
        let sample = (frame % 30_000) as i16;
        writer.write_sample(sample).unwrap();
        writer.write_sample(-sample).unwrap();
    }
    writer.finalize().unwrap();
}

/// The appearance flags reach postkit's parsers, so a value it cannot read has
/// to name the flag that carried it, and a flag with no burn to style has to
/// name itself rather than the group.
#[test]
fn a_burn_appearance_flag_is_refused_by_name() {
    let dir = TempDir::new().unwrap();
    let srt = dir.path().join("cues.srt");
    std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:02,000\nhello\n\n").unwrap();
    let clip = dir.path().join("clip.mov");
    std::fs::write(&clip, b"not really a movie").unwrap();

    let create = |extra: &[&str], with_burn: bool| {
        let mut command = cmd();
        command.args([
            "create",
            "-o",
            &dir.path().join("out").to_string_lossy(),
            "-t",
            "Burn",
            "--video",
            &clip.to_string_lossy(),
        ]);
        if with_burn {
            command.args(["--burn-subtitle", &srt.to_string_lossy()]);
        }
        command.args(extra);
        command
    };

    for (mut command, needle) in [
        (
            create(&["--burn-colour", "octarine"], true),
            "--burn-colour: octarine is not a colour",
        ),
        (
            create(&["--burn-effect-colour", "12345"], true),
            "--burn-effect-colour: 12345 is not a colour",
        ),
        (
            create(&["--burn-effect", "glow"], true),
            "--burn-effect: glow is not an effect",
        ),
        (
            create(&["--burn-font-size", "0"], true),
            "burn-in appearance:",
        ),
        (
            create(&["--burn-font-size", "8"], false),
            "--burn-font-size needs --burn-subtitle",
        ),
        (
            create(&["--burn-fade-up", "250"], false),
            "--burn-fade-up needs --burn-subtitle",
        ),
        (
            create(&["--burn-margin", "10"], false),
            "--burn-margin needs --burn-subtitle",
        ),
        (
            create(&["--burn-line-height", "0.5"], true),
            "burn-in appearance: a line height",
        ),
    ] {
        command
            .assert()
            .failure()
            .stderr(predicate::str::contains(needle));
    }
}

/// `subtitle-convert` writes the named size and colour as the document's own
/// default style, so a cue that says nothing for itself lands at that look.
#[test]
fn subtitle_convert_writes_a_default_size_and_colour() {
    let dir = TempDir::new().unwrap();
    let srt = dir.path().join("cues.srt");
    std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:02,000\nhello\n\n").unwrap();
    let ttml = dir.path().join("cues.ttml");

    cmd()
        .args([
            "subtitle-convert",
            "-i",
            &srt.to_string_lossy(),
            "-o",
            &ttml.to_string_lossy(),
            "--font-size",
            "6",
            "--colour",
            "FFFF00",
        ])
        .assert()
        .success();

    // a bare percent would be read against the parent's size, so 6% of the frame
    // height goes out as 6% of the 15-row cell grid, 0.9 cells
    let written = std::fs::read_to_string(&ttml).unwrap();
    assert!(
        written.contains(r#"tts:fontSize="0.900c""#),
        "ttml: {written}"
    );
    assert!(
        written.contains(r#"ttp:cellResolution="32 15""#),
        "a cell-relative size needs the grid it is against: {written}"
    );
    assert!(
        written.contains(r##"tts:color="#FFFF00FF""##),
        "ttml: {written}"
    );
    assert!(
        written.contains(r#"style="default""#),
        "the default style has to reach the cues: {written}"
    );

    cmd()
        .args([
            "subtitle-convert",
            "-i",
            &srt.to_string_lossy(),
            "-o",
            &ttml.to_string_lossy(),
            "--colour",
            "octarine",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--colour: octarine is not"));
}

/// `create --check` is the pre-build pass: it has to refuse a job that cannot
/// finish and leave the output folder untouched, rather than encoding first.
#[test]
fn the_pre_build_check_refuses_a_trim_longer_than_the_clip() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let clip = dir.path().join("clip.mov");
    synthesize_clip(&clip, 1920, 1080);
    let output = dir.path().join("imp");

    cmd()
        .args([
            "create",
            "--check",
            "-o",
            &output.to_string_lossy(),
            "-t",
            "Trimmed",
            "--video",
            &clip.to_string_lossy(),
            "--trim-start",
            "200f",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("leaves nothing of the 24 picture"));

    assert!(!output.exists(), "the check must write nothing");
}

/// A first cue a second in is the DoM four-second rule, and `--check` is where a
/// caller sees it without spending an encode.
#[test]
fn the_pre_build_check_prints_the_first_cue_hint() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let clip = dir.path().join("clip.mov");
    synthesize_clip(&clip, 1920, 1080);
    let srt = dir.path().join("cues.srt");
    std::fs::write(
        &srt,
        "1\n00:00:01,000 --> 00:00:03,000\nA cue that starts early\n\n",
    )
    .unwrap();
    let output = dir.path().join("imp");

    cmd()
        .args([
            "create",
            "--check",
            "-o",
            &output.to_string_lossy(),
            "-t",
            "Early",
            "--video",
            &clip.to_string_lossy(),
            "--burn-subtitle",
            &srt.to_string_lossy(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("The first subtitle in cues.srt"))
        .stdout(predicate::str::contains("at least 4 seconds"));

    assert!(!output.exists(), "the check must write nothing");
}

/// Nothing to say is the common case, and it has to say nothing rather than
/// inventing a hint.
#[test]
fn the_pre_build_check_is_quiet_on_a_clean_job() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let clip = dir.path().join("clip.mov");
    synthesize_clip(&clip, 1920, 1080);
    let output = dir.path().join("imp");

    cmd()
        .args([
            "create",
            "--check",
            "-o",
            &output.to_string_lossy(),
            "-t",
            "Clean",
            "--video",
            &clip.to_string_lossy(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("0 hint(s)"))
        .stdout(predicate::str::contains("hint:").not());

    assert!(!output.exists(), "the check must write nothing");
}
