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

// the ACES AP0 to AP1 and AP1 to Rec.709 matrices, each carrying the Bradford
// adaptation to D65. colorchannelmixer refuses a coefficient outside ±2, which
// the single AP0 to Rec.709 matrix needs, so the two run in series
const AP0_TO_AP1: [[f64; 3]; 3] = [
    [1.4514393161, -0.2365107469, -0.2149285693],
    [-0.0765537733, 1.1762296998, -0.0996759265],
    [0.0083161484, -0.0060324498, 0.9977163014],
];
const AP1_TO_REC709: [[f64; 3]; 3] = [
    [1.70505, -0.62179, -0.08326],
    [-0.13026, 1.14080, -0.01055],
    [-0.02400, -0.12897, 1.15297],
];

fn channel_mixer(matrix: &[[f64; 3]; 3]) -> String {
    let coefficients: Vec<String> = matrix
        .iter()
        // the fourth coefficient of each row is the alpha the output ignores
        .flat_map(|row| {
            row.iter()
                .map(|value| value.to_string())
                .chain(["0".into()])
        })
        .collect();
    format!("colorchannelmixer={}", coefficients.join(":"))
}

/// Fallback: convert ACES AP0 to Rec.709 with ffmpeg when ctlrender is absent.
fn run_aces_ffmpeg_fallback(opts: &AcesPipelineOptions) -> Result<(), String> {
    tracing::warn!(
        "ctlrender is not installed, so this falls back to the ffmpeg AP0 to Rec.709 matrix and the Rec.709 transfer, ignoring the named transforms and applying no RRT tone mapping"
    );

    let filter = format!(
        "format=gbrpf32le,{},{},zscale=transferin=linear:transfer=bt709,format=gbrp16le",
        channel_mixer(&AP0_TO_AP1),
        channel_mixer(&AP1_TO_REC709)
    );
    let output = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-i"])
        .arg(opts.input)
        .args(["-vf", &filter, "-c:v", "tiff"])
        .arg(opts.output)
        .output()
        .map_err(|e| format!("Failed to run ffmpeg: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "the ffmpeg ACES fallback failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}
