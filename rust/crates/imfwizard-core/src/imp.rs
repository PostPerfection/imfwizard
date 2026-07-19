use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// IMP creation options.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImpOptions {
    pub output_dir: PathBuf,
    pub title: String,
    pub content_kind: String,
    /// J2K codestream directory
    pub j2k_dir: Option<PathBuf>,
    /// WAV audio files
    pub audio_files: Vec<PathBuf>,
    /// Timed text files
    pub timed_text_files: Vec<PathBuf>,
    /// Frame rate
    pub fps_num: u32,
    pub fps_den: u32,
    /// Edit rate
    pub edit_rate: String,
    /// Duration in frames
    pub duration: u64,
}

/// IMP creation result.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImpResult {
    pub success: bool,
    pub error: String,
    pub output_dir: PathBuf,
    pub cpl_path: PathBuf,
    pub pkl_path: PathBuf,
    pub assetmap_path: PathBuf,
    pub track_files: Vec<crate::MxfTrackFile>,
}

/// Create an IMP (Interoperable Master Package).
pub fn create_imp(opts: &ImpOptions) -> ImpResult {
    let Some(j2k_dir) = opts.j2k_dir.as_ref() else {
        return ImpResult {
            error: "A J2K input directory is required".into(),
            ..Default::default()
        };
    };
    if !j2k_dir.is_dir() {
        return ImpResult {
            error: format!("J2K input directory not found: {}", j2k_dir.display()),
            ..Default::default()
        };
    }

    // 1. Create output directory
    if let Err(e) = std::fs::create_dir_all(&opts.output_dir) {
        return ImpResult {
            success: false,
            error: format!("Failed to create output directory: {e}"),
            ..Default::default()
        };
    }

    // 2. Wrap J2K codestreams into MXF track file
    let mut track_files = Vec::new();
    {
        let video_uuid = uuid::Uuid::new_v4().to_string();
        let mxf_path = opts.output_dir.join(format!("VIDEO_{video_uuid}.mxf"));
        let wrap_opts = crate::mxf_wrap::MxfWrapOptions {
            input_dir: j2k_dir.clone(),
            output_file: mxf_path,
            essence_type: crate::EssenceType::J2k,
            edit_rate_num: opts.fps_num,
            edit_rate_den: opts.fps_den,
            duration: opts.duration,
        };
        let wrap_result = crate::mxf_wrap::wrap_mxf(&wrap_opts);
        if !wrap_result.success {
            return ImpResult {
                success: false,
                error: format!("MXF wrapping failed: {}", wrap_result.error),
                ..Default::default()
            };
        }
        track_files.push(wrap_result.track_file);
    }

    // 2b. Wrap audio files into MXF track files
    for audio_path in &opts.audio_files {
        if !audio_path.exists() {
            continue;
        }
        let audio_uuid = uuid::Uuid::new_v4().to_string();
        let mxf_path = opts.output_dir.join(format!("AUDIO_{audio_uuid}.mxf"));
        let wrap_opts = crate::mxf_wrap::MxfWrapOptions {
            input_dir: audio_path.clone(),
            output_file: mxf_path,
            essence_type: crate::EssenceType::Wav,
            edit_rate_num: opts.fps_num,
            edit_rate_den: opts.fps_den,
            duration: opts.duration,
        };
        let wrap_result = crate::mxf_wrap::wrap_mxf(&wrap_opts);
        if !wrap_result.success {
            return ImpResult {
                error: format!("Audio wrap failed: {}", wrap_result.error),
                ..Default::default()
            };
        }
        track_files.push(wrap_result.track_file);
    }

    // 3. Generate UUIDs
    let cpl_uuid = uuid::Uuid::new_v4().to_string();
    let pkl_uuid = uuid::Uuid::new_v4().to_string();

    // 4. Write CPL
    let cpl_path = opts.output_dir.join(format!("CPL_{cpl_uuid}.xml"));
    if let Err(e) = crate::cpl::write_cpl(&cpl_path, &cpl_uuid, opts, &track_files) {
        return ImpResult {
            success: false,
            error: format!("Failed to write CPL: {e}"),
            ..Default::default()
        };
    }

    // 5. Write PKL
    let pkl_path = opts.output_dir.join(format!("PKL_{pkl_uuid}.xml"));
    if let Err(e) = crate::pkl::write_pkl(&pkl_path, &pkl_uuid, &cpl_uuid, &cpl_path, &track_files)
    {
        return ImpResult {
            success: false,
            error: format!("Failed to write PKL: {e}"),
            ..Default::default()
        };
    }

    // 6. Write ASSETMAP
    let am_path = opts.output_dir.join("ASSETMAP.xml");
    if let Err(e) = crate::assetmap::write_assetmap(&am_path, &pkl_uuid, &cpl_uuid, &track_files) {
        return ImpResult {
            success: false,
            error: format!("Failed to write ASSETMAP: {e}"),
            ..Default::default()
        };
    }

    ImpResult {
        success: true,
        error: String::new(),
        output_dir: opts.output_dir.clone(),
        cpl_path,
        pkl_path,
        assetmap_path: am_path,
        track_files,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_imp_requires_picture_input() {
        let dir = tempfile::tempdir().unwrap();
        let opts = ImpOptions {
            output_dir: dir.path().to_path_buf(),
            title: "Test Feature".into(),
            content_kind: "feature".into(),
            fps_num: 24,
            fps_den: 1,
            ..Default::default()
        };
        let result = create_imp(&opts);
        assert!(!result.success);
        assert!(result.error.contains("J2K input directory is required"));
    }

    #[test]
    fn test_create_imp_xml_escape() {
        let dir = tempfile::tempdir().unwrap();
        let cpl_path = dir.path().join("CPL_test.xml");
        let opts = ImpOptions {
            output_dir: dir.path().to_path_buf(),
            title: "Test & <Special> \"Film\"".into(),
            fps_num: 25,
            fps_den: 1,
            ..Default::default()
        };
        crate::cpl::write_cpl(&cpl_path, "test", &opts, &[]).unwrap();
        let cpl_xml = std::fs::read_to_string(cpl_path).unwrap();
        assert!(cpl_xml.contains("Test &amp; &lt;Special&gt; &quot;Film&quot;"));
    }
}
