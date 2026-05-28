// IMF packaging
pub mod assetmap;
pub mod cpl;
pub mod cpl_annotation;
pub mod edl_import;
pub mod imp;
pub mod mxf_wrap;
pub mod otioz_import;
pub mod pkl;

// Encoding/transcoding
pub mod encode;
pub mod probe;
pub mod transcode;

// HDR/Color
pub mod dolby_vision;
pub mod hdr;

// Audio
pub mod audio;
pub mod channel_map;
pub mod mca;

// Subtitles/Captions
pub mod burnin;
pub mod captions;
pub mod subtitle_convert;
pub mod subtitle_retime;

// Infrastructure
pub mod job_queue;
pub mod plugin;
pub mod preferences;
pub mod rest_api;
pub mod watch;
pub mod webhook;

// Delivery
pub mod delivery;
pub mod profiles;

// Tools
pub mod analytics;
pub mod imp_diff;
pub mod prores;
pub mod report;
pub mod timecode;

// Crypto
pub mod hash;
pub mod signature;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    pub hash: String,
    pub size: u64,
    pub duration: u64,
}
