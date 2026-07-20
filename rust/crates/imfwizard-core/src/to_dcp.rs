use std::path::PathBuf;

/// Options for IMP to DCP conversion.
pub struct ToDcpOptions {
    pub imp_dir: PathBuf,
    pub output_dir: PathBuf,
    pub title: Option<String>,
    pub content_kind: String,
}

/// Result of IMP to DCP conversion.
pub struct ToDcpResult {
    pub success: bool,
    pub error: String,
    pub output_dir: PathBuf,
}

/// Convert an IMP to a DCP.
///
/// Not implemented: a real conversion must transcode IMF (App#2E broadcast-profile) J2K to
/// DCI 2K/4K J2K, rewrap essence as AS-DCP (ST 429) MXF, and build DCP-flavour CPL/PKL/ASSETMAP
/// (ST 429-7/8/9), which are different schemas than the IMF (ST 2067) writers here. The previous
/// implementation only ran `ffmpeg -c copy` and reported success without producing a DCP, so it
/// fails loud instead of emitting a non-DCP.
pub fn imp_to_dcp(opts: &ToDcpOptions) -> ToDcpResult {
    ToDcpResult {
        success: false,
        error: "IMF to DCP conversion is not implemented".to_string(),
        output_dir: opts.output_dir.clone(),
    }
}
