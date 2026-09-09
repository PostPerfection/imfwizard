use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FRAMES: u32 = 1;
const FRAMES_PER_SECOND: u32 = 24;
const SAMPLE_RATE: u32 = 48_000;

// the only depth App 2E sound may carry
const APP2E_SOUND_BITS: u32 = 24;

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

fn run_ffmpeg(arguments: &[&str]) {
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(arguments)
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
}

fn source_clip(work: &Path) -> PathBuf {
    let clip = work.join("source.mkv");
    run_ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("color=c=blue:s={WIDTH}x{HEIGHT}:r={FRAMES_PER_SECOND}"),
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

// a rising ramp so the widening can be checked sample by sample, not just by depth
fn ramp_wav(path: &Path, bits: u16, format: hound::SampleFormat) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: bits,
        sample_format: format,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for frame in 0..SAMPLE_RATE as i32 {
        match format {
            hound::SampleFormat::Int => {
                let sample = frame % 30_000;
                writer.write_sample(sample).unwrap();
                writer.write_sample(-sample).unwrap();
            }
            hound::SampleFormat::Float => {
                let sample = (frame % 30_000) as f32 / 30_000.0;
                writer.write_sample(sample).unwrap();
                writer.write_sample(-sample).unwrap();
            }
        }
    }
    writer.finalize().unwrap();
}

fn create(work: &Path, sound: &Path) -> (PathBuf, Command) {
    let clip = source_clip(work);
    let imp = work.join("imp");
    let mut command = cmd();
    command.args([
        "create",
        "-o",
        &imp.to_string_lossy(),
        "-t",
        "Sound Depth",
        "--video",
        &clip.to_string_lossy(),
        "--audio",
        &sound.to_string_lossy(),
        "--fps-num",
        &FRAMES_PER_SECOND.to_string(),
        "--fps-den",
        "1",
    ]);
    (imp, command)
}

fn only_file_starting_with(dir: &Path, prefix: &str) -> PathBuf {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix))
        })
        .collect();
    assert_eq!(found.len(), 1, "expected one {prefix} file in {dir:?}");
    found.pop().unwrap()
}

/// A 16-bit master is the ordinary case, since ffmpeg writes a WAV at 16 bits
/// unless asked otherwise. It widens losslessly, so the wrap carries 24-bit and
/// Photon, which refuses any other depth, passes the package.
#[test]
fn a_16_bit_master_is_packaged_as_24_bit() {
    let work = TempDir::new().unwrap();
    let sound = work.path().join("s16.wav");
    ramp_wav(&sound, 16, hound::SampleFormat::Int);

    let (imp, mut command) = create(work.path(), &sound);
    command
        .assert()
        .success()
        .stdout(predicate::str::contains("IMP validation PASSED"));

    let audio = only_file_starting_with(&imp, "AUDIO_");
    let mut reader = asdcplib::as02::pcm::MxfReader::new();
    reader
        .open_read(
            &audio.to_string_lossy(),
            asdcplib::Rational::new(FRAMES_PER_SECOND as i32, 1),
        )
        .expect("the sound MXF opens");
    let descriptor = reader
        .wave_audio_descriptor()
        .expect("the wave audio descriptor");
    assert_eq!(
        descriptor.quantization_bits, APP2E_SOUND_BITS,
        "the track file was wrapped at the master's depth"
    );
    assert_eq!(
        descriptor.block_align,
        (APP2E_SOUND_BITS as u16 / 8) * descriptor.channel_count as u16
    );

    assert!(
        !imp.join("widened_audio_0.wav").exists(),
        "the widened scratch was left in the package"
    );
}

/// The widening is a shift, so every sample of the master survives it: the
/// 24-bit file holds each 16-bit value scaled by 256 and nothing else.
#[test]
fn widening_scales_every_sample_and_loses_none() {
    let work = TempDir::new().unwrap();
    let sound = work.path().join("s16.wav");
    ramp_wav(&sound, 16, hound::SampleFormat::Int);
    let widened = work.path().join("widened.wav");

    let from_bits = imfwizard_core::source_edits::widen_sound_depth(&sound, &widened)
        .expect("the widening runs")
        .expect("a 16-bit master is widened");
    assert_eq!(from_bits, 16);

    let master: Vec<i32> = hound::WavReader::open(&sound)
        .unwrap()
        .into_samples::<i32>()
        .map(Result::unwrap)
        .collect();
    let mut reader = hound::WavReader::open(&widened).unwrap();
    assert_eq!(reader.spec().bits_per_sample, APP2E_SOUND_BITS as u16);
    let wide: Vec<i32> = reader.samples::<i32>().map(Result::unwrap).collect();

    assert_eq!(wide.len(), master.len());
    let scale = 1 << (APP2E_SOUND_BITS - 16);
    assert!(
        wide.iter().zip(&master).all(|(w, m)| *w == m * scale),
        "a sample did not survive the widening"
    );
}

/// A float master would have to be scaled and clamped to reach 24-bit, which is
/// a mastering decision, so it is refused before anything is encoded.
#[test]
fn a_float_master_is_refused_before_the_encode() {
    let work = TempDir::new().unwrap();
    let sound = work.path().join("f32.wav");
    ramp_wav(&sound, 32, hound::SampleFormat::Float);

    let (imp, mut command) = create(work.path(), &sound);
    command.assert().failure().stderr(
        predicate::str::contains("32-bit float PCM")
            .and(predicate::str::contains("App 2E sound is 24-bit"))
            .and(predicate::str::contains("-c:a pcm_s24le")),
    );

    assert!(
        !imp.join("j2k").exists(),
        "the refusal came after the encode had started"
    );
    assert!(
        !imp.join("ASSETMAP.xml").exists(),
        "a package was written for a master that cannot be packaged"
    );
}

/// A 32-bit integer master is refused the same way: the low 8 bits have nowhere
/// to go in a 24-bit wrap.
#[test]
fn a_32_bit_integer_master_is_refused_before_the_encode() {
    let work = TempDir::new().unwrap();
    let sound = work.path().join("s32.wav");
    ramp_wav(&sound, 32, hound::SampleFormat::Int);

    let (imp, mut command) = create(work.path(), &sound);
    command.assert().failure().stderr(
        predicate::str::contains("32-bit PCM")
            .and(predicate::str::contains("App 2E sound is 24-bit"))
            .and(predicate::str::contains("-c:a pcm_s24le")),
    );

    assert!(
        !imp.join("j2k").exists(),
        "the refusal came after the encode had started"
    );
}
