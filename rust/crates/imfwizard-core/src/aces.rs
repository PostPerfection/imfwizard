use std::path::Path;

/// ACES CTL transform type.
#[derive(Debug, Clone)]
pub enum AcesTransform {
    /// Input Device Transform (camera → ACES)
    Idt(String),
    /// Reference Rendering Transform (ACES → OCES)
    Rrt,
    /// Output Device Transform (OCES → display)
    Odt(String),
}

/// Options for the full ACES colour pipeline.
pub struct AcesPipelineOptions<'a> {
    pub input: &'a Path,
    pub output: &'a Path,
    pub idt: Option<&'a str>,
    pub odt: Option<&'a str>,
    pub ctl_dir: Option<&'a Path>,
}

/// Run the full IDT → RRT → ODT pipeline using ctlrender.
///
/// Falls back to ffmpeg colorspace conversion if ctlrender is not available.
pub fn run_aces_pipeline(opts: &AcesPipelineOptions) -> Result<(), String> {
    if !crate::tools::has_ctlrender() {
        return run_aces_ffmpeg_fallback(opts);
    }

    let ctl_dir = opts
        .ctl_dir
        .map(|p| p.to_path_buf())
        .or_else(find_ctl_dir)
        .ok_or_else(|| {
            "CTL transforms directory not found. Set --ctl-dir or install ampas-ctl.".to_string()
        })?;

    // Build the ctlrender command with IDT → RRT → ODT chain
    let mut cmd = std::process::Command::new("ctlrender");

    // IDT (Input Device Transform)
    if let Some(idt_name) = opts.idt {
        let idt_path = resolve_ctl_file(&ctl_dir, "idt", idt_name)?;
        cmd.args(["-ctl", &idt_path]);
    }

    // RRT (Reference Rendering Transform) — always applied
    let rrt_path = find_rrt_ctl(&ctl_dir)?;
    cmd.args(["-ctl", &rrt_path]);

    // ODT (Output Device Transform)
    if let Some(odt_name) = opts.odt {
        let odt_path = resolve_ctl_file(&ctl_dir, "odt", odt_name)?;
        cmd.args(["-ctl", &odt_path]);
    }

    cmd.args([
        "-format",
        "exr",
        "-input_scale",
        "1.0",
        "-output_scale",
        "1.0",
    ]);
    cmd.arg(opts.input);
    cmd.arg(opts.output);

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run ctlrender: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ctlrender failed: {stderr}"));
    }

    Ok(())
}

/// Find the CTL transforms directory (ampas-ctl standard locations).
fn find_ctl_dir() -> Option<std::path::PathBuf> {
    let candidates = [
        "/usr/share/ctl/transforms",
        "/usr/local/share/ctl/transforms",
        "/opt/aces/transforms/ctl",
    ];
    for path in &candidates {
        let p = std::path::PathBuf::from(path);
        if p.is_dir() {
            return Some(p);
        }
    }
    // Check ACES_CTL_DIR env var
    std::env::var("ACES_CTL_DIR")
        .ok()
        .map(std::path::PathBuf::from)
}

/// Resolve a CTL file by category and name.
fn resolve_ctl_file(ctl_dir: &Path, category: &str, name: &str) -> Result<String, String> {
    // Try exact path first
    let exact = ctl_dir.join(category).join(name);
    if exact.is_file() {
        return Ok(exact.to_string_lossy().to_string());
    }

    // Try with .ctl extension
    let with_ext = ctl_dir.join(category).join(format!("{name}.ctl"));
    if with_ext.is_file() {
        return Ok(with_ext.to_string_lossy().to_string());
    }

    // Search recursively
    if let Ok(entries) = std::fs::read_dir(ctl_dir.join(category)) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file()
                && p.file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|stem| stem.to_lowercase().contains(&name.to_lowercase()))
            {
                return Ok(p.to_string_lossy().to_string());
            }
        }
    }

    Err(format!(
        "CTL file not found: {category}/{name} in {}",
        ctl_dir.display()
    ))
}

/// Find the RRT CTL file.
fn find_rrt_ctl(ctl_dir: &Path) -> Result<String, String> {
    let rrt = ctl_dir.join("rrt").join("RRT.ctl");
    if rrt.is_file() {
        return Ok(rrt.to_string_lossy().to_string());
    }

    // Search for any RRT file
    if let Ok(entries) = std::fs::read_dir(ctl_dir.join("rrt")) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.to_uppercase().contains("RRT") && name.ends_with(".ctl"))
            {
                return Ok(p.to_string_lossy().to_string());
            }
        }
    }

    Err(format!("RRT.ctl not found in {}/rrt", ctl_dir.display()))
}

/// Fallback: use ffmpeg colorspace conversion when ctlrender is not available.
fn run_aces_ffmpeg_fallback(opts: &AcesPipelineOptions) -> Result<(), String> {
    tracing::warn!("ctlrender not available, falling back to ffmpeg colorspace conversion");

    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-i"])
        .arg(opts.input)
        .args([
            "-vf",
            "colorspace=all=bt709:iall=bt2020:itrc=arib-std-b67:fast=1",
            "-c:v",
            "tiff",
        ])
        .arg(opts.output)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .status()
        .map_err(|e| format!("Failed to run ffmpeg: {e}"))?;

    if !status.success() {
        return Err("ffmpeg ACES fallback conversion failed".to_string());
    }

    Ok(())
}
