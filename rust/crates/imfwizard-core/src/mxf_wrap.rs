use base64::Engine;
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
    /// HDR/WCG picture metadata for a J2K wrap (ST 2067-21); None is SDR.
    #[serde(skip)]
    pub hdr: Option<asdcplib::jp2k::HdrMetadata>,
    /// The id written into the MXF as its AssetUUID and returned as the track
    /// file uuid. A caller that names the output file after an id must pass that
    /// id here, or the MXF carries one the package never mentions. None mints one.
    #[serde(default)]
    pub asset_uuid: Option<[u8; 16]>,
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
/// IMF track files use the AS-02 (ST 2067) writers. Every essence type delegates to
/// postkit's asdcplib-backed wrappers; postkit's PCM wrapper now parses the real WAV
/// header, so there is one WAV-parsing implementation.
pub fn wrap_mxf(opts: &MxfWrapOptions) -> MxfWrapResult {
    match opts.essence_type {
        crate::EssenceType::Wav => delegate(
            opts,
            postkit::mxf_wrap::EssenceType::Pcm,
            postkit::mxf_wrap::MxfStandard::As02,
        ),
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
        encryption: None,
        mca_config: None,
        resource_ids: vec![],
        hdr: opts.hdr.clone(),
        asset_uuid: opts.asset_uuid,
    });

    if !pk.success {
        return MxfWrapResult {
            error: pk.error,
            ..Default::default()
        };
    }

    let hash = match base64_digest_from_hex(&pk.hash) {
        Ok(h) => h,
        Err(e) => {
            return MxfWrapResult {
                error: format!("unusable MXF hash from postkit: {e}"),
                ..Default::default()
            };
        }
    };

    MxfWrapResult {
        success: true,
        error: String::new(),
        track_file: crate::MxfTrackFile {
            path: pk.path,
            uuid: pk.uuid,
            hash,
            size: pk.size,
            duration: pk.duration,
        },
    }
}

/// SHA-1 digest length. ST 2067-2 fixes the PKL Hash field to a SHA-1 digest.
const SHA1_DIGEST_BYTES: usize = 20;

/// postkit returns a hex SHA-1, but the PKL Hash field is xs:base64Binary.
/// This decodes the hex to the raw digest and re-encodes, so a postkit that
/// starts returning base64 fails here instead of double-encoding into a string
/// that still looks like a plausible hash.
fn base64_digest_from_hex(hex: &str) -> Result<String, String> {
    if hex.len() != SHA1_DIGEST_BYTES * 2 {
        return Err(format!(
            "expected {} hex characters, got {}: {hex}",
            SHA1_DIGEST_BYTES * 2,
            hex.len()
        ));
    }
    let mut digest = [0u8; SHA1_DIGEST_BYTES];
    for (byte, pair) in digest.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
        let (Some(high), Some(low)) = (hex_nibble(pair[0]), hex_nibble(pair[1])) else {
            return Err(format!("not hexadecimal: {hex}"));
        };
        *byte = (high << 4) | low;
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(digest))
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
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
    fn hex_digest_becomes_base64() {
        assert_eq!(
            base64_digest_from_hex("37a15bc80b0243dde57f7469865579a5700c472c").unwrap(),
            "N6FbyAsCQ93lf3RphlV5pXAMRyw="
        );
    }

    #[test]
    fn already_base64_is_rejected_not_re_encoded() {
        assert!(base64_digest_from_hex("N6FbyAsCQ93lf3RphlV5pXAMRyw=").is_err());
    }

    #[test]
    fn non_hex_of_the_right_length_is_rejected() {
        assert!(base64_digest_from_hex(&"z".repeat(40)).is_err());
    }

    /// A non-default (2ch/16-bit) WAV wraps into an AS-02 PCM MXF via postkit,
    /// and the wrapped descriptor reflects the real header, not a 5.1 default.
    #[test]
    fn wrap_wav_delegates_and_preserves_channel_count() {
        let dir = tempfile::tempdir().unwrap();
        let wav_path = dir.path().join("stereo.wav");
        std::fs::write(&wav_path, make_wav(2, 48000, 16, 48000)).unwrap();
        let out = dir.path().join("AUDIO.mxf");

        let opts = MxfWrapOptions {
            input_dir: wav_path,
            output_file: out.clone(),
            essence_type: crate::EssenceType::Wav,
            edit_rate_num: 24,
            edit_rate_den: 1,
            duration: 0,
            hdr: None,
            asset_uuid: None,
        };
        let result = wrap_mxf(&opts);
        assert!(result.success, "wrap failed: {}", result.error);

        let mut reader = asdcplib::as02::pcm::MxfReader::new();
        reader
            .open_read(&out.to_string_lossy(), asdcplib::Rational::new(24, 1))
            .expect("open wrapped MXF");
        let desc = reader.audio_descriptor().expect("audio descriptor");
        assert_eq!(desc.channel_count, 2);
        assert_eq!(desc.quantization_bits, 16);
        assert_eq!(desc.audio_sampling_rate.numerator, 48000);
    }

    /// Build a minimal PCM WAV (fmt + data chunks) with the given parameters.
    fn make_wav(channels: u16, sample_rate: u32, bits: u16, sample_frames: u32) -> Vec<u8> {
        let block_align = (bits / 8) as u32 * channels as u32;
        let data_len = block_align * sample_frames;
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data_len).to_le_bytes());
        w.extend_from_slice(b"WAVE");
        w.extend_from_slice(b"fmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes()); // WAVE_FORMAT_PCM
        w.extend_from_slice(&channels.to_le_bytes());
        w.extend_from_slice(&sample_rate.to_le_bytes());
        w.extend_from_slice(&(sample_rate * block_align).to_le_bytes()); // byte rate
        w.extend_from_slice(&(block_align as u16).to_le_bytes());
        w.extend_from_slice(&bits.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&data_len.to_le_bytes());
        w.resize(w.len() + data_len as usize, 0);
        w
    }
}
