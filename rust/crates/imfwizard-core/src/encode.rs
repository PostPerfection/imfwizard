pub use postkit::encode::{
    EncodeResult, FrameRate, ImageFormat, detect_image_format, find_source_frames,
};

/// The bitrate the standalone `encode` command and the encode job compress at
/// when none is asked for.
pub const DEFAULT_ENCODE_BITRATE_MBPS: f64 = 250.0;

/// Encode a directory of stills to App 2E codestreams at `bitrate_mbps`, the
/// way `create` does for its picture, without packaging them. The codestreams
/// land in a `j2k` directory under `output_dir`.
pub fn encode_image_sequence(
    input_dir: &std::path::Path,
    output_dir: &std::path::Path,
    bitrate_mbps: f64,
    fps: FrameRate,
) -> Result<postkit::pipeline::EncodeResult, String> {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    let (width, height) = postkit::encode::source_raster(input_dir)?;
    let rsiz = imf_rsiz_for_encode(width, height, fps.as_f64(), bitrate_mbps)?;
    postkit::pipeline::run_encode_with_options(
        input_dir,
        output_dir,
        &postkit::pipeline::EncodeRunOptions {
            compression_ratio: DEFAULT_COMPRESSION_RATIO,
            target_codestream_bytes: Some(codestream_byte_cap_for_bitrate(
                fps.as_f64(),
                bitrate_mbps,
            )),
            fps,
            source_colour: postkit::encode::SourceColour::KeepRgb,
            rsiz,
            ..postkit::pipeline::EncodeRunOptions::default()
        },
        &Arc::new(AtomicBool::new(false)),
        &Arc::new(AtomicBool::new(false)),
        |progress| tracing::info!("{}", progress.message),
        |line| tracing::info!("{line}"),
    )
}

/// A J2K frame carries 12-bit RGB, three components a pixel, so an uncompressed
/// frame is this many bits per pixel.
const RAW_BITS_PER_PIXEL: f64 = 36.0;

/// What the encoder compresses at when no bitrate is asked for: 10:1.
pub const DEFAULT_COMPRESSION_RATIO: f64 = 10.0;

/// Below this a PSNR target costs less than the 10:1 default ratio already
/// gives, so grok's quality allocation has nothing to do.
pub const MINIMUM_QUALITY_PSNR_DB: f64 = 20.0;
/// A 12-bit component tops out near this, so a higher target only spends bits
/// without changing the picture.
pub const MAXIMUM_QUALITY_PSNR_DB: f64 = 80.0;

/// The per-frame codestream size a target bitrate allows, in bytes: what the
/// allocation aims at, and the cap the encoder holds to under a PSNR target.
pub fn codestream_byte_cap_for_bitrate(fps: f64, bitrate_mbps: f64) -> u64 {
    let fps = fps.max(1.0);
    (bitrate_mbps * 1_000_000.0 / 8.0 / fps) as u64
}

pub fn target_codestream_bytes_for_job(
    explicit_bitrate_mbps: Option<f64>,
    profile_bitrate_mbps: Option<f64>,
    fps: f64,
) -> Option<u64> {
    explicit_bitrate_mbps
        .or(profile_bitrate_mbps)
        .map(|bitrate_mbps| codestream_byte_cap_for_bitrate(fps, bitrate_mbps))
}

/// The bits a second a `create` job's picture is allowed. Named the same way
/// [`target_codestream_bytes_for_job`] picks its target, so the two describe one
/// encode: without a bitrate it is what the default ratio leaves of the raw frame.
pub fn bitrate_mbps_for_job(
    explicit_bitrate_mbps: Option<f64>,
    profile_bitrate_mbps: Option<f64>,
    width: u32,
    height: u32,
    fps: f64,
) -> f64 {
    if let Some(bitrate_mbps) = explicit_bitrate_mbps.or(profile_bitrate_mbps) {
        return bitrate_mbps;
    }
    let raw_bits_per_second = width as f64 * height as f64 * RAW_BITS_PER_PIXEL * fps.max(1.0);
    raw_bits_per_second / DEFAULT_COMPRESSION_RATIO / 1_000_000.0
}

/// The Rsiz an App 2E encode declares: the IMF profile this raster picks, with
/// the main level its sample rate asks for and the sub level its bit rate asks
/// for. Errs for a picture past IMF 8K or past main level 11 / sub level 9.
pub fn imf_rsiz_for_encode(
    width: u32,
    height: u32,
    fps: f64,
    bitrate_mbps: f64,
) -> Result<u16, String> {
    let profile = postkit::j2k::ImfProfile::for_raster(width, height)?;
    let bits_per_second = (bitrate_mbps * 1_000_000.0) as u64;
    let levels = postkit::j2k::imf_levels(width, height, fps, bits_per_second)?;
    Ok(postkit::j2k::imf_rsiz(profile, levels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bitrate_becomes_the_bytes_one_frame_may_take() {
        assert_eq!(codestream_byte_cap_for_bitrate(24.0, 250.0), 1_302_083);
        assert_eq!(codestream_byte_cap_for_bitrate(24.0, 50.0), 260_416);
        assert_eq!(codestream_byte_cap_for_bitrate(48.0, 250.0), 651_041);
    }

    #[test]
    fn an_explicit_bitrate_wins_over_the_delivery_preset() {
        assert_eq!(
            target_codestream_bytes_for_job(Some(100.0), Some(250.0), 24.0),
            Some(520_833)
        );
        assert_eq!(
            target_codestream_bytes_for_job(None, Some(250.0), 24.0),
            Some(1_302_083)
        );
        // no bitrate anywhere, so the encode keeps the default ratio and aims
        // at no target
        assert_eq!(target_codestream_bytes_for_job(None, None, 24.0), None);
    }

    /// The bitrate the Rsiz sub level is composed from and the bitrate the
    /// frames are allocated to have to be one bitrate, or the Rsiz declares a
    /// sub level the essence does not hold to.
    #[test]
    fn the_job_bitrate_and_the_byte_target_come_from_one_bitrate() {
        for (explicit, profile) in [(Some(100.0), Some(250.0)), (None, Some(250.0))] {
            let bitrate = bitrate_mbps_for_job(explicit, profile, 1920, 1080, 24.0);
            assert_eq!(
                target_codestream_bytes_for_job(explicit, profile, 24.0),
                Some(codestream_byte_cap_for_bitrate(24.0, bitrate))
            );
        }

        let default = bitrate_mbps_for_job(None, None, 1920, 1080, 24.0);
        let raw_mbps = 1920.0 * 1080.0 * RAW_BITS_PER_PIXEL * 24.0 / 1_000_000.0;
        assert!((default - raw_mbps / DEFAULT_COMPRESSION_RATIO).abs() < 1e-9);
    }

    /// Netflix's Sol Levante picture is Rsiz 0x0536, which is what 3840x2160 at
    /// 24 fps and 800 Mbps has to compose to.
    #[test]
    fn a_raster_and_a_bitrate_compose_an_imf_rsiz() {
        assert_eq!(
            imf_rsiz_for_encode(3840, 2160, 24.0, 800.0).unwrap(),
            0x0536
        );

        let rsiz = imf_rsiz_for_encode(1920, 1080, 24.0, 250.0).unwrap();
        assert_eq!(
            postkit::j2k::J2kProfile::from(rsiz),
            postkit::j2k::J2kProfile::Imf
        );
        assert!(!postkit::j2k::J2kProfile::from(rsiz).is_dci_cinema());
    }

    /// The default ratio at every App 2E raster has to land on a level pair IMF
    /// allows, or `create` refuses its own default.
    #[test]
    fn every_app2e_raster_composes_an_rsiz_at_the_default_ratio() {
        for (width, height) in crate::mxf_wrap::APP2E_RASTERS {
            for fps in [24.0, 60.0] {
                let bitrate = bitrate_mbps_for_job(None, None, width, height, fps);
                let rsiz = imf_rsiz_for_encode(width, height, fps, bitrate)
                    .unwrap_or_else(|e| panic!("{width}x{height} at {fps} fps: {e}"));
                assert_eq!(
                    postkit::j2k::J2kProfile::from(rsiz),
                    postkit::j2k::J2kProfile::Imf
                );
            }
        }
    }
}
