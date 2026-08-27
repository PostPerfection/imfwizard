pub use postkit::encode::{
    EncodeOptions, EncodeResult, FrameRate, ImageFormat, detect_image_format, encode,
    find_source_frames,
};

/// A J2K frame carries 12-bit RGB, three components a pixel, so an uncompressed
/// frame is this many bits per pixel.
const RAW_BITS_PER_PIXEL: f64 = 36.0;

/// What the encoder compresses at when no bitrate is asked for: 10:1.
pub const DEFAULT_COMPRESSION_RATIO: f64 = 10.0;

/// The J2K compression ratio that lands a picture on a target bitrate. The
/// raster is the one that is encoded, which a picture plan may have changed.
pub fn compression_ratio_for_bitrate(width: u32, height: u32, fps: f64, bitrate_mbps: f64) -> f64 {
    let fps = fps.max(1.0);
    let raw_bits = width as f64 * height as f64 * RAW_BITS_PER_PIXEL;
    let target_bits = (bitrate_mbps * 1_000_000.0) / fps;
    (raw_bits / target_bits).max(1.0)
}

/// Below this a PSNR target costs less than the 10:1 default ratio already
/// gives, so grok's quality allocation has nothing to do.
pub const MINIMUM_QUALITY_PSNR_DB: f64 = 20.0;
/// A 12-bit component tops out near this, so a higher target only spends bits
/// without changing the picture.
pub const MAXIMUM_QUALITY_PSNR_DB: f64 = 80.0;

/// The per-frame codestream size a target bitrate allows, in bytes. Under a
/// PSNR target this is the cap the encoder holds to instead of a ratio.
pub fn codestream_byte_cap_for_bitrate(fps: f64, bitrate_mbps: f64) -> u64 {
    let fps = fps.max(1.0);
    (bitrate_mbps * 1_000_000.0 / 8.0 / fps) as u64
}

/// The ratio a `create` job encodes at: an explicit bitrate wins over the
/// delivery preset's, and neither leaves the default ratio.
pub fn compression_ratio_for_job(
    explicit_bitrate_mbps: Option<f64>,
    profile_bitrate_mbps: Option<f64>,
    width: u32,
    height: u32,
    fps: f64,
) -> f64 {
    match explicit_bitrate_mbps.or(profile_bitrate_mbps) {
        Some(bitrate_mbps) => compression_ratio_for_bitrate(width, height, fps, bitrate_mbps),
        None => DEFAULT_COMPRESSION_RATIO,
    }
}

/// The bits a second a `create` job's picture is allowed. Named the same way
/// [`compression_ratio_for_job`] picks its ratio, so the two describe one encode:
/// without a bitrate it is what the default ratio leaves of the raw frame.
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
    fn a_bitrate_becomes_the_ratio_the_raster_needs() {
        // 1920 * 1080 * 36 bits a frame against 250 Mbps spread over 24 frames
        let ratio = compression_ratio_for_bitrate(1920, 1080, 24.0, 250.0);
        assert!((ratio - 7.166_361_6).abs() < 1e-9, "{ratio}");
    }

    #[test]
    fn a_bitrate_becomes_the_bytes_one_frame_may_take() {
        assert_eq!(codestream_byte_cap_for_bitrate(24.0, 250.0), 1_302_083);
        assert_eq!(codestream_byte_cap_for_bitrate(24.0, 50.0), 260_416);
        assert_eq!(codestream_byte_cap_for_bitrate(48.0, 250.0), 651_041);
    }

    #[test]
    fn an_explicit_bitrate_wins_over_the_delivery_preset() {
        let explicit = compression_ratio_for_job(Some(100.0), Some(250.0), 1920, 1080, 24.0);
        assert_eq!(
            explicit,
            compression_ratio_for_bitrate(1920, 1080, 24.0, 100.0)
        );

        let from_preset = compression_ratio_for_job(None, Some(250.0), 1920, 1080, 24.0);
        assert_eq!(
            from_preset,
            compression_ratio_for_bitrate(1920, 1080, 24.0, 250.0)
        );

        let neither = compression_ratio_for_job(None, None, 1920, 1080, 24.0);
        assert_eq!(neither, DEFAULT_COMPRESSION_RATIO);
    }

    /// The bitrate and the ratio have to describe the same encode, or the Rsiz
    /// declares a sub level the essence does not hold to.
    #[test]
    fn the_job_bitrate_is_the_ratio_the_job_encodes_at() {
        assert_eq!(
            bitrate_mbps_for_job(Some(100.0), Some(250.0), 1920, 1080, 24.0),
            100.0
        );
        assert_eq!(
            bitrate_mbps_for_job(None, Some(250.0), 1920, 1080, 24.0),
            250.0
        );

        let default = bitrate_mbps_for_job(None, None, 1920, 1080, 24.0);
        assert_eq!(
            compression_ratio_for_bitrate(1920, 1080, 24.0, default),
            DEFAULT_COMPRESSION_RATIO
        );
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
