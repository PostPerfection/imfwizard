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
}
