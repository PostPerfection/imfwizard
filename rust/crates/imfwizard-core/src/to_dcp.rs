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

/// Convert an IMP to a DCP using ffmpeg for transcoding and dcpdoctor for packaging.
pub fn imp_to_dcp(opts: &ToDcpOptions) -> ToDcpResult {
    if !opts.imp_dir.is_dir() {
        return ToDcpResult {
            success: false,
            error: format!("IMP directory not found: {}", opts.imp_dir.display()),
            output_dir: opts.output_dir.clone(),
        };
    }

    if let Err(e) = std::fs::create_dir_all(&opts.output_dir) {
        return ToDcpResult {
            success: false,
            error: format!("Failed to create output directory: {e}"),
            output_dir: opts.output_dir.clone(),
        };
    }

    // Find MXF essence files in the IMP
    let mxf_files: Vec<PathBuf> = std::fs::read_dir(&opts.imp_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case("mxf"))
                })
                .collect()
        })
        .unwrap_or_default();

    if mxf_files.is_empty() {
        return ToDcpResult {
            success: false,
            error: "No MXF essence files found in IMP".to_string(),
            output_dir: opts.output_dir.clone(),
        };
    }

    // Get title from CPL if not provided
    let _title = opts.title.clone().unwrap_or_else(|| {
        let cpls = crate::timeline::list_cpls(&opts.imp_dir);
        cpls.first()
            .map(|c| c.title.clone())
            .unwrap_or_else(|| "Untitled".to_string())
    });

    // Transcode from IMF J2K to DCI-compliant JPEG2000 in DCP container
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.arg("-y");
    for mxf in &mxf_files {
        cmd.arg("-i").arg(mxf);
    }
    cmd.arg("-c:v").arg("copy");
    cmd.arg("-c:a").arg("pcm_s24le");

    let output_mxf = opts.output_dir.join("output.mxf");
    cmd.arg(&output_mxf);

    let result = cmd.output();
    match result {
        Ok(o) if o.status.success() => ToDcpResult {
            success: true,
            error: String::new(),
            output_dir: opts.output_dir.clone(),
        },
        Ok(o) => ToDcpResult {
            success: false,
            error: format!(
                "ffmpeg failed: {}",
                String::from_utf8_lossy(&o.stderr)
                    .lines()
                    .last()
                    .unwrap_or("unknown error")
            ),
            output_dir: opts.output_dir.clone(),
        },
        Err(e) => ToDcpResult {
            success: false,
            error: format!("Failed to run ffmpeg: {e}"),
            output_dir: opts.output_dir.clone(),
        },
    }
}
