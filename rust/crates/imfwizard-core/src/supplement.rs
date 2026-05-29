use std::path::PathBuf;

/// Options for creating a supplemental IMP.
pub struct SupplementOptions {
    pub ov_dir: PathBuf,
    pub title: String,
    pub output_dir: PathBuf,
    pub video: Option<PathBuf>,
    pub entry_point: u64,
    pub duration: Option<String>,
}

/// Result of supplemental IMP creation.
pub struct SupplementResult {
    pub success: bool,
    pub error: String,
    pub output_dir: PathBuf,
}

/// Create a supplemental IMP referencing an Original Version (OV).
pub fn create_supplement(opts: &SupplementOptions) -> SupplementResult {
    // Validate OV exists
    if !opts.ov_dir.is_dir() {
        return SupplementResult {
            success: false,
            error: format!("OV directory not found: {}", opts.ov_dir.display()),
            output_dir: opts.output_dir.clone(),
        };
    }

    // Validate OV has an ASSETMAP
    let ov_has_assetmap =
        opts.ov_dir.join("ASSETMAP.xml").exists() || opts.ov_dir.join("ASSETMAP").exists();
    if !ov_has_assetmap {
        return SupplementResult {
            success: false,
            error: "OV directory does not contain a valid IMP (no ASSETMAP)".to_string(),
            output_dir: opts.output_dir.clone(),
        };
    }

    // Get OV CPLs
    let ov_cpls = crate::timeline::list_cpls(&opts.ov_dir);
    if ov_cpls.is_empty() {
        return SupplementResult {
            success: false,
            error: "OV directory does not contain any CPL".to_string(),
            output_dir: opts.output_dir.clone(),
        };
    }

    // Create output directory
    if let Err(e) = std::fs::create_dir_all(&opts.output_dir) {
        return SupplementResult {
            success: false,
            error: format!("Failed to create output directory: {e}"),
            output_dir: opts.output_dir.clone(),
        };
    }

    // Build supplemental IMP using the OV as a base
    let mut imp_opts = crate::imp::ImpOptions {
        output_dir: opts.output_dir.clone(),
        title: opts.title.clone(),
        content_kind: "feature".to_string(),
        fps_num: 24,
        fps_den: 1,
        ..Default::default()
    };

    // If video is provided, use it
    if let Some(ref video) = opts.video
        && video.is_dir()
    {
        imp_opts.j2k_dir = Some(video.clone());
    }

    // Read edit rate from OV CPL
    if let Some(cpl) = ov_cpls.first() {
        let cpl_path = opts.ov_dir.join(&cpl.file_path);
        if let Ok(content) = std::fs::read_to_string(&cpl_path)
            && let Some(edit_rate) = extract_edit_rate(&content)
        {
            imp_opts.fps_num = edit_rate.0;
            imp_opts.fps_den = edit_rate.1;
        }
    }

    let result = crate::imp::create_imp(&imp_opts);
    SupplementResult {
        success: result.success,
        error: result.error,
        output_dir: result.output_dir,
    }
}

fn extract_edit_rate(content: &str) -> Option<(u32, u32)> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("<EditRate>") && trimmed.ends_with("</EditRate>") {
            let val = &trimmed[10..trimmed.len() - 11];
            let parts: Vec<&str> = val.split_whitespace().collect();
            if parts.len() == 2 {
                let num = parts[0].parse::<u32>().ok()?;
                let den = parts[1].parse::<u32>().ok()?;
                return Some((num, den));
            }
        }
    }
    None
}
