use serde::{Deserialize, Serialize};

/// MXF wrapping options.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MxfWrapOptions {
    pub input_dir: std::path::PathBuf,
    pub output_file: std::path::PathBuf,
    pub essence_type: crate::EssenceType,
    pub edit_rate_num: u32,
    pub edit_rate_den: u32,
    pub duration: u64,
}

/// MXF wrapping result.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MxfWrapResult {
    pub success: bool,
    pub error: String,
    pub track_file: crate::MxfTrackFile,
}

fn collect_input_files(path: &std::path::Path) -> Result<Vec<std::path::PathBuf>, String> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }

    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(path)
        .map_err(|e| format!("cannot read input {}: {e}", path.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("no files found in {}", path.display()));
    }
    Ok(files)
}

/// Wrap essence into an MXF track file.
///
/// IMF track files use the AS-02 (ST 2067) writers. J2K/TimedText/Atmos delegate to
/// postkit's asdcplib-backed wrappers. PCM is handled here because postkit's PCM wrapper
/// hardcodes 5.1/24-bit/48k instead of reading the WAV header, so we parse it ourselves.
pub fn wrap_mxf(opts: &MxfWrapOptions) -> MxfWrapResult {
    match opts.essence_type {
        crate::EssenceType::Wav => wrap_pcm(opts),
        crate::EssenceType::J2k => {
            if let Err(error) = precheck_j2k(&opts.input_dir) {
                return MxfWrapResult {
                    error,
                    ..Default::default()
                };
            }
            delegate(
                opts,
                postkit::mxf_wrap::EssenceType::J2k,
                postkit::mxf_wrap::MxfStandard::As02,
            )
        }
        crate::EssenceType::TimedText => delegate(
            opts,
            postkit::mxf_wrap::EssenceType::TimedText,
            postkit::mxf_wrap::MxfStandard::As02,
        ),
        // asdcplib provides AS-02 writers only for J2K/PCM/TimedText, so Atmos/IAB uses AS-DCP.
        crate::EssenceType::Atmos => delegate(
            opts,
            postkit::mxf_wrap::EssenceType::Atmos,
            postkit::mxf_wrap::MxfStandard::AsDcp,
        ),
    }
}

/// Delegate wrapping to postkit and adapt its result to our types.
fn delegate(
    opts: &MxfWrapOptions,
    essence_type: postkit::mxf_wrap::EssenceType,
    standard: postkit::mxf_wrap::MxfStandard,
) -> MxfWrapResult {
    let input_files = match collect_input_files(&opts.input_dir) {
        Ok(f) => f,
        Err(e) => {
            return MxfWrapResult {
                error: e,
                ..Default::default()
            };
        }
    };

    let pk = postkit::mxf_wrap::mxf_wrap(&postkit::mxf_wrap::MxfWrapOptions {
        input_files,
        output: opts.output_file.clone(),
        essence_type,
        standard,
        fps_num: opts.edit_rate_num,
        fps_den: opts.edit_rate_den,
        partition_size: 0,
    });

    if !pk.success {
        return MxfWrapResult {
            error: pk.error,
            ..Default::default()
        };
    }

    MxfWrapResult {
        success: true,
        error: String::new(),
        track_file: crate::MxfTrackFile {
            path: pk.path,
            uuid: pk.uuid,
            hash: pk.hash,
            size: pk.size,
            duration: pk.duration,
        },
    }
}

fn precheck_j2k(input_dir: &std::path::Path) -> Result<(), String> {
    let files = collect_input_files(input_dir)?;
    let first = std::fs::read(&files[0])
        .map_err(|e| format!("failed to read {}: {e}", files[0].display()))?;
    let Some(header) = postkit::j2k::parse_j2k_header(&first) else {
        return Err(format!(
            "invalid JPEG 2000 codestream: {}",
            files[0].display()
        ));
    };
    validate_app2e_picture(header.width, header.height, header.bit_depth)
}

pub fn validate_app2e_picture(width: u32, height: u32, bit_depth: u8) -> Result<(), String> {
    let valid_resolutions = [(1920, 1080), (2048, 1080), (3840, 2160), (4096, 2160)];
    if !valid_resolutions.contains(&(width, height)) {
        return Err(format!(
            "App 2E requires 1920x1080, 2048x1080, 3840x2160, or 4096x2160 picture essence, got {width}x{height}"
        ));
    }
    if !matches!(bit_depth, 8 | 10 | 12) {
        return Err(format!(
            "App 2E requires 8, 10, or 12-bit picture essence, got {bit_depth}-bit"
        ));
    }
    Ok(())
}

/// Parsed WAV `fmt ` fields plus the byte range of the `data` chunk.
struct WavInfo {
    channels: u32,
    bits: u32,
    sample_rate: u32,
    data_offset: usize,
    data_len: usize,
}

/// Parse a canonical/extensible RIFF-WAVE header by walking its chunks.
fn parse_wav(data: &[u8]) -> Result<WavInfo, String> {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let mut pos = 12;
    let mut fmt: Option<(u32, u32, u32)> = None; // channels, bits, sample_rate
    let mut data_range: Option<(usize, usize)> = None;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]])
            as usize;
        let body = pos + 8;
        if id == b"fmt " {
            if body + 16 > data.len() {
                return Err("truncated fmt chunk".into());
            }
            let channels = u16::from_le_bytes([data[body + 2], data[body + 3]]) as u32;
            let sample_rate = u32::from_le_bytes([
                data[body + 4],
                data[body + 5],
                data[body + 6],
                data[body + 7],
            ]);
            let bits = u16::from_le_bytes([data[body + 14], data[body + 15]]) as u32;
            fmt = Some((channels, bits, sample_rate));
        } else if id == b"data" {
            let end = (body + size).min(data.len());
            data_range = Some((body, end));
        }
        // chunks are word-aligned
        pos = body + size + (size & 1);
    }
    let (channels, bits, sample_rate) = fmt.ok_or("missing fmt chunk")?;
    let (data_offset, data_end) = data_range.ok_or("missing data chunk")?;
    if channels == 0 || bits == 0 || sample_rate == 0 {
        return Err("invalid fmt chunk (zero channels/bits/rate)".into());
    }
    Ok(WavInfo {
        channels,
        bits,
        sample_rate,
        data_offset,
        data_len: data_end.saturating_sub(data_offset),
    })
}

fn wrap_pcm(opts: &MxfWrapOptions) -> MxfWrapResult {
    let files = match collect_input_files(&opts.input_dir) {
        Ok(f) => f,
        Err(e) => {
            return MxfWrapResult {
                error: e,
                ..Default::default()
            };
        }
    };

    let wav_data = match std::fs::read(&files[0]) {
        Ok(d) => d,
        Err(e) => {
            return MxfWrapResult {
                error: format!("failed to read WAV: {e}"),
                ..Default::default()
            };
        }
    };

    let wav = match parse_wav(&wav_data) {
        Ok(w) => w,
        Err(e) => {
            return MxfWrapResult {
                error: format!("WAV parse ({}): {e}", files[0].display()),
                ..Default::default()
            };
        }
    };

    let asset_uuid = *uuid::Uuid::new_v4().as_bytes();
    let info = asdcplib::WriterInfo {
        asset_uuid,
        context_id: *uuid::Uuid::new_v4().as_bytes(),
        label_set: asdcplib::LabelSet::Smpte,
        ..Default::default()
    };

    let block_align = (wav.bits / 8) * wav.channels;
    let samples_per_frame = (wav.sample_rate as f64
        / (opts.edit_rate_num as f64 / opts.edit_rate_den as f64))
        .ceil() as u32;
    let frame_size = samples_per_frame * block_align;
    if frame_size == 0 {
        return MxfWrapResult {
            error: "computed zero PCM frame size".into(),
            ..Default::default()
        };
    }

    let pcm_data = &wav_data[wav.data_offset..wav.data_offset + wav.data_len];
    let num_frames = (pcm_data.len() as u32) / frame_size;

    let desc = asdcplib::pcm::AudioDescriptor {
        edit_rate: asdcplib::Rational::new(opts.edit_rate_num as i32, opts.edit_rate_den as i32),
        audio_sampling_rate: asdcplib::Rational::new(wav.sample_rate as i32, 1),
        locked: true,
        channel_count: wav.channels,
        quantization_bits: wav.bits,
        block_align,
        avg_bps: wav.sample_rate * block_align,
        linked_track_id: 0,
        container_duration: num_frames,
        channel_format: if wav.channels == 6 {
            asdcplib::pcm::ChannelFormat::Cfg1
        } else {
            asdcplib::pcm::ChannelFormat::None
        },
    };

    let mut writer = asdcplib::as02::pcm::MxfWriter::new();
    let output_str = opts.output_file.to_string_lossy().to_string();
    if let Err(e) = writer.open_write(&output_str, &info, &desc, 16384) {
        return MxfWrapResult {
            error: format!("PCM open_write: {e}"),
            ..Default::default()
        };
    }

    for i in 0..num_frames {
        let start = (i * frame_size) as usize;
        let end = start + frame_size as usize;
        if end > pcm_data.len() {
            break;
        }
        if let Err(e) = writer.write_frame(&pcm_data[start..end], None, None) {
            return MxfWrapResult {
                error: format!("PCM write_frame: {e}"),
                ..Default::default()
            };
        }
    }

    if let Err(e) = writer.finalize() {
        return MxfWrapResult {
            error: format!("PCM finalize: {e}"),
            ..Default::default()
        };
    }

    let (hash, size) =
        match postkit::hash::hash_file(&opts.output_file, postkit::hash::HashAlgorithm::Sha1) {
            Ok(h) => (
                h.base64,
                std::fs::metadata(&opts.output_file)
                    .map(|m| m.len())
                    .unwrap_or(0),
            ),
            Err(_) => (String::new(), 0),
        };

    MxfWrapResult {
        success: true,
        error: String::new(),
        track_file: crate::MxfTrackFile {
            path: opts.output_file.clone(),
            uuid: uuid::Uuid::from_bytes(asset_uuid).hyphenated().to_string(),
            hash,
            size,
            duration: num_frames as u64,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app2e_picture_constraints_reject_invalid_essence() {
        assert!(validate_app2e_picture(1920, 1080, 12).is_ok());
        assert!(validate_app2e_picture(2048, 872, 12).is_err());
        assert!(validate_app2e_picture(2048, 1080, 16).is_err());
    }

    #[test]
    fn parse_wav_reads_real_header() {
        // minimal 2ch/16-bit/44100 WAV with 4 sample frames of silence
        let mut d = Vec::new();
        d.extend_from_slice(b"RIFF");
        d.extend_from_slice(&[0, 0, 0, 0]); // riff size (ignored)
        d.extend_from_slice(b"WAVE");
        d.extend_from_slice(b"fmt ");
        d.extend_from_slice(&16u32.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes()); // pcm
        d.extend_from_slice(&2u16.to_le_bytes()); // channels
        d.extend_from_slice(&44100u32.to_le_bytes());
        d.extend_from_slice(&(44100u32 * 4).to_le_bytes()); // byte rate
        d.extend_from_slice(&4u16.to_le_bytes()); // block align
        d.extend_from_slice(&16u16.to_le_bytes()); // bits
        d.extend_from_slice(b"data");
        let payload = [0u8; 16];
        d.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        d.extend_from_slice(&payload);

        let w = parse_wav(&d).unwrap();
        assert_eq!(w.channels, 2);
        assert_eq!(w.bits, 16);
        assert_eq!(w.sample_rate, 44100);
        assert_eq!(w.data_len, 16);
    }

    #[test]
    fn parse_wav_rejects_non_riff() {
        assert!(parse_wav(b"not a wav file at all").is_err());
    }
}
