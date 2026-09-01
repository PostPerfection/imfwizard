//! The audio mix matrix as `create` and the GUI spell it.
//!
//! postkit's grammar is `IN:OUT[@GAIN]` over 1-based channel numbers. Here the
//! destination may also be a channel name, taken from `channel_map` so the
//! spellings a mapped file is later MCA-labelled with are the ones a caller can
//! type. The highest destination lane decides how many channels come out, so
//! `1:L,2:R,1:C` writes a three channel file.

use std::path::{Path, PathBuf};

use postkit::audio_mix_matrix::{MixMatrix, MixReport, mix_wav_files};

use crate::channel_map::channel_map_7_1;

/// Prefix the MCA tag symbols carry, which a caller may leave off: `chLs` and
/// `Ls` both name the left surround.
const MCA_SYMBOL_PREFIX: &str = "ch";

/// What the mapped WAV is called under the job's work directory.
pub const MAPPED_AUDIO_NAME: &str = "audio_mapped.wav";

const SPEC_ENTRY_SEPARATOR: char = ',';
const SPEC_CHANNEL_SEPARATOR: char = ':';
const SPEC_GAIN_SEPARATOR: char = '@';
const FIRST_CHANNEL_NUMBER: usize = 1;

/// Read a map whose destinations may be channel names, against an input of
/// `input_channels` channels. The output carries as many channels as the highest
/// destination lane.
pub fn parse_audio_map(spec: &str, input_channels: usize) -> Result<MixMatrix, String> {
    if spec.trim().is_empty() {
        return Err("audio map is empty".to_string());
    }
    let mut numbered = Vec::new();
    let mut output_channels = 0;
    for entry in spec.split(SPEC_ENTRY_SEPARATOR) {
        let entry = entry.trim();
        if entry.is_empty() {
            return Err("audio map has an empty entry".to_string());
        }
        let (channels, gain) = match entry.split_once(SPEC_GAIN_SEPARATOR) {
            Some((channels, gain)) => (channels, Some(gain)),
            None => (entry, None),
        };
        let (input, output) = channels
            .split_once(SPEC_CHANNEL_SEPARATOR)
            .ok_or_else(|| format!("audio map entry \"{entry}\" is not IN:OUT or IN:OUT@GAIN"))?;
        let lane = destination_lane(output.trim(), entry)?;
        output_channels = output_channels.max(lane + 1);
        let numbered_entry = match gain {
            Some(gain) => format!(
                "{}{SPEC_CHANNEL_SEPARATOR}{}{SPEC_GAIN_SEPARATOR}{gain}",
                input.trim(),
                lane + FIRST_CHANNEL_NUMBER
            ),
            None => format!(
                "{}{SPEC_CHANNEL_SEPARATOR}{}",
                input.trim(),
                lane + FIRST_CHANNEL_NUMBER
            ),
        };
        numbered.push(numbered_entry);
    }
    MixMatrix::parse(
        &numbered.join(&SPEC_ENTRY_SEPARATOR.to_string()),
        input_channels,
        output_channels,
    )
}

/// Every destination name, in lane order, as a GUI matrix labels its columns.
pub fn destination_names() -> Vec<String> {
    channel_map_7_1()
        .channels
        .into_iter()
        .map(|channel| short_name(&channel.mca_tag_symbol))
        .collect()
}

/// The 0-based lane a destination names, whether it is a number or a name.
fn destination_lane(output: &str, entry: &str) -> Result<usize, String> {
    if let Ok(number) = output.parse::<usize>() {
        if number < FIRST_CHANNEL_NUMBER {
            return Err(format!(
                "audio map entry \"{entry}\" names output channel {number}, and channels count from {FIRST_CHANNEL_NUMBER}"
            ));
        }
        return Ok(number - FIRST_CHANNEL_NUMBER);
    }
    let map = channel_map_7_1();
    let found = map.channels.iter().find(|channel| {
        channel.label.eq_ignore_ascii_case(output)
            || channel.mca_tag_symbol.eq_ignore_ascii_case(output)
            || short_name(&channel.mca_tag_symbol).eq_ignore_ascii_case(output)
    });
    match found {
        Some(channel) => Ok(channel.index as usize),
        None => Err(format!(
            "audio map entry \"{entry}\" names output channel \"{output}\", which is neither a channel number nor one of {}",
            destination_names().join(", ")
        )),
    }
}

fn short_name(mca_tag_symbol: &str) -> String {
    mca_tag_symbol
        .strip_prefix(MCA_SYMBOL_PREFIX)
        .unwrap_or(mca_tag_symbol)
        .to_string()
}

/// One line for a log: what the map did and whether it changed any sample.
fn describe_mix(matrix: &MixMatrix, report: &MixReport) -> String {
    let routing = if matrix.is_pure_routing() {
        "bit-exact routing"
    } else {
        "mixed"
    };
    format!(
        "audio map: {} in, {} out, {} frames, {routing}",
        report.input_channels, report.output_channels, report.frames
    )
}

/// Map `input` into `work_dir` and return the mapped file, logging what it did
/// through `on_log`. This is the whole `--audio-map` step both callers run.
pub fn map_audio_file(
    spec: &str,
    input: &Path,
    work_dir: &Path,
    on_log: impl Fn(&str),
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(work_dir)
        .map_err(|error| format!("cannot create {}: {error}", work_dir.display()))?;
    let channels = postkit::wav_io::channel_count(input)?;
    let matrix = parse_audio_map(spec, channels)?;
    let output = work_dir.join(MAPPED_AUDIO_NAME);
    let report = mix_wav_files(&matrix, &[input.to_path_buf()], &output)?;
    on_log(&describe_mix(&matrix, &report));
    if report.clipped_samples > 0 {
        on_log(&format!(
            "audio map clipped {} samples: lower the gains to fit",
            report.clipped_samples
        ));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_numbers_reach_the_same_lanes() {
        let by_name = parse_audio_map("1:L,2:R,1:C@-6", 2).unwrap();
        let by_number = parse_audio_map("1:1,2:2,1:3@-6", 2).unwrap();
        assert_eq!(by_name, by_number);
        assert_eq!(by_name.output_channels(), 3);
        assert_eq!(by_name.gain_db(0, 0), Some(0.0));
        assert!((by_name.gain_db(0, 2).unwrap() - -6.0).abs() < 1e-9);
    }

    /// Both spellings channel_map carries have to work, since one is what the
    /// MCA labels say and the other is what a person types.
    #[test]
    fn every_channel_name_channel_map_spells_resolves() {
        for (name, lane) in [
            ("L", 0),
            ("Left", 0),
            ("chL", 0),
            ("r", 1),
            ("C", 2),
            ("LFE", 3),
            ("Ls", 4),
            ("Left Surround", 4),
            ("Rs", 5),
            ("Lrs", 6),
            ("Left Back", 6),
            ("Rrs", 7),
        ] {
            let matrix = parse_audio_map(&format!("1:{name}"), 1).unwrap();
            assert_eq!(matrix.output_channels(), lane + 1, "{name}");
            assert_eq!(matrix.gain_db(0, lane), Some(0.0), "{name}");
        }
        assert_eq!(
            destination_names(),
            ["L", "R", "C", "LFE", "Ls", "Rs", "Lrs", "Rrs"]
        );
    }

    #[test]
    fn the_output_is_as_wide_as_the_highest_lane() {
        assert_eq!(parse_audio_map("1:1", 2).unwrap().output_channels(), 1);
        assert_eq!(parse_audio_map("1:Rrs", 2).unwrap().output_channels(), 8);
        assert_eq!(parse_audio_map("1:6,2:2", 2).unwrap().output_channels(), 6);
    }

    #[test]
    fn a_bad_map_fails_by_name() {
        for (spec, wanted) in [
            ("", "empty"),
            ("1:1,,2:2", "empty entry"),
            ("banana", "not IN:OUT"),
            ("1:Middle", "Middle"),
            ("1:0", "count from 1"),
            ("3:1", "outside 1..=2"),
            ("x:1", "non-numeric input"),
            ("1:1,1:1", "twice"),
            ("1:1@loud", "unknown gain unit"),
        ] {
            let error = parse_audio_map(spec, 2).unwrap_err();
            assert!(error.contains(wanted), "spec {spec:?} said {error:?}");
        }
    }

    #[test]
    fn a_pure_routing_is_named_as_one() {
        let routing = parse_audio_map("1:L,2:R", 2).unwrap();
        let report = MixReport {
            input_channels: 2,
            output_channels: 2,
            frames: 48_000,
            clipped_samples: 0,
        };
        assert!(describe_mix(&routing, &report).contains("bit-exact routing"));

        let mixed = parse_audio_map("1:L,2:R,1:C@-6", 2).unwrap();
        assert!(describe_mix(&mixed, &report).contains("mixed"));
    }

    /// The end of the chain: the mapped file has to carry the lanes the spec
    /// asked for, with the gain applied.
    #[test]
    fn a_mapped_wav_carries_the_gained_lane() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("stereo.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 24,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&input, spec).unwrap();
        for frame in 0..480 {
            writer.write_sample(frame * 1000).unwrap();
            writer.write_sample(-frame * 1000).unwrap();
        }
        writer.finalize().unwrap();

        let mapped = map_audio_file("1:L,2:R,1:C@-6.0206", &input, dir.path(), |_| {}).unwrap();
        let mut reader = hound::WavReader::open(&mapped).unwrap();
        assert_eq!(reader.spec().channels, 3);
        let samples: Vec<i32> = reader.samples::<i32>().map(|s| s.unwrap()).collect();
        assert_eq!(samples.len(), 480 * 3);
        for frame in 1..480i32 {
            let left = samples[frame as usize * 3];
            let centre = samples[frame as usize * 3 + 2];
            assert_eq!(left, frame * 1000);
            assert!(
                (centre as f64 - left as f64 / 2.0).abs() <= 1.0,
                "frame {frame}: centre {centre} is not half of {left}"
            );
        }
    }
}
