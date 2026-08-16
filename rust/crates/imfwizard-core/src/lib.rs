// IMF packaging
pub mod assetmap;
pub mod cpl;
pub mod cpl_annotation;
pub mod edl_import;
pub mod imp;
pub mod mxf_wrap;
pub mod pkl;
pub mod timeline;

// Encoding/transcoding
pub mod duration_spec;
pub mod encode;
pub mod hints;
pub mod preflight;
pub mod probe;
pub mod source_edits;
pub mod source_picture;
pub mod still;
pub mod transcode;

// HDR/Color
pub mod aces;
pub mod dolby_vision;
pub mod hdr;
pub mod hdr_wcg;
pub mod source_colourspace;

// Audio
pub mod atmos;
pub mod audio;
pub mod audio_desc;
pub mod audio_map;
pub mod channel_map;
pub mod mca;

// Subtitles/Captions
pub mod burnin;
pub mod scc;
pub mod subtitle_burn;
pub mod subtitle_convert;

// Infrastructure
pub mod executor;
pub mod job_queue;
pub mod preferences;
pub mod rest_api;
pub mod tools;
pub mod watch;

// Delivery
pub mod delivery;
pub mod profiles;

// Tools
pub mod analytics;
pub mod export_frames;
pub use postkit::frame_compare;
pub mod info;
pub mod photon;
pub mod prores;
pub mod report;
pub mod supplement;
pub use postkit::timecode;
pub mod to_dcp;
pub mod validate;
pub mod xsd_validate;

// Crypto
pub mod hash;
pub mod signature;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub(crate) fn issue_date() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .expect("RFC 3339 formatting supports UTC timestamps")
}

/// Essence type for MXF wrapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EssenceType {
    #[default]
    J2k,
    Wav,
    TimedText,
    Atmos,
}

/// Image format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFormat {
    Dpx,
    Tiff,
    Exr,
    Png,
    Jpeg,
    Bmp,
    J2k,
}

/// Color space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorSpace {
    Bt709,
    Bt2020,
    P3D65,
    P3Dci,
    Aces,
}

/// Transfer function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferFunction {
    SdrBt1886,
    Pq,
    Hlg,
    Linear,
}

/// MXF track file result.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MxfTrackFile {
    pub path: PathBuf,
    pub uuid: String,
    /// Base64 SHA-1 digest, the encoding the PKL Hash field takes.
    pub hash: String,
    pub size: u64,
    pub duration: u64,
}
