//! Centralized external tool dependency checking.
//!
//! Provides the `doctor` functionality — probing external tools for
//! availability, version, and generating human/JSON reports.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// Tool availability status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolStatus {
    Available,
    Missing,
    VersionMismatch,
}

/// Information about a single external tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    pub status: ToolStatus,
    pub purpose: String,
    pub required: bool,
}

/// Result of checking all tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCheckResult {
    pub tools: Vec<ToolInfo>,
    pub available: usize,
    pub missing: usize,
    pub required_missing: usize,
}

/// Tool definition used for probing.
struct ToolDef {
    name: &'static str,
    purpose: &'static str,
    required: bool,
    version_args: &'static [&'static str],
}

const TOOL_DEFS: &[ToolDef] = &[
    ToolDef {
        name: "ffmpeg",
        purpose: "Transcoding, burn-in, loudness, LUT, ACES fallback, audio description, slate",
        required: true,
        version_args: &["-version"],
    },
    ToolDef {
        name: "ffprobe",
        purpose: "Media probing, codec detection, duration/resolution extraction",
        required: true,
        version_args: &["-version"],
    },
    ToolDef {
        name: "grk_compress",
        purpose: "JPEG 2000 encoding (Grok codec)",
        required: true,
        version_args: &["--version"],
    },
    ToolDef {
        name: "dovi_tool",
        purpose: "Dolby Vision RPU injection/extraction",
        required: false,
        version_args: &["--version"],
    },
    ToolDef {
        name: "ctlrender",
        purpose: "ACES CTL transforms (IDT/RRT/ODT)",
        required: false,
        version_args: &["--version"],
    },
    ToolDef {
        name: "xmllint",
        purpose: "XSD schema validation of IMP XML files",
        required: false,
        version_args: &["--version"],
    },
    ToolDef {
        name: "wkhtmltopdf",
        purpose: "PDF report generation",
        required: false,
        version_args: &["--version"],
    },
    ToolDef {
        name: "weasyprint",
        purpose: "PDF report generation (alternative)",
        required: false,
        version_args: &["--version"],
    },
    ToolDef {
        name: "ascp",
        purpose: "Aspera FASP high-speed transfer",
        required: false,
        version_args: &["--version"],
    },
];

fn which(name: &str) -> Option<PathBuf> {
    Command::new("which")
        .arg(name)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(PathBuf::from(s))
            }
        })
}

fn extract_version(name: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(name).args(args).output().ok()?;

    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);

    // Try common version patterns
    let re_patterns = [r"(\d+\.\d+\.\d+)", r"(\d+\.\d+)", r"version\s+(\d+[\d.]+)"];

    for pat in &re_patterns {
        if let Some(caps) = regex_lite::Regex::new(pat)
            .ok()
            .and_then(|re| re.captures(&text))
        {
            return Some(caps[1].to_string());
        }
    }
    None
}

fn probe_tool(def: &ToolDef) -> ToolInfo {
    let path = which(def.name);
    let (status, version) = if path.is_some() {
        let ver = extract_version(def.name, def.version_args);
        (ToolStatus::Available, ver)
    } else {
        (ToolStatus::Missing, None)
    };

    ToolInfo {
        name: def.name.to_string(),
        path,
        version,
        status,
        purpose: def.purpose.to_string(),
        required: def.required,
    }
}

/// Check all known external tools and return a summary.
pub fn check_all_tools() -> ToolCheckResult {
    let tools: Vec<ToolInfo> = TOOL_DEFS.iter().map(probe_tool).collect();
    let available = tools
        .iter()
        .filter(|t| t.status == ToolStatus::Available)
        .count();
    let missing = tools
        .iter()
        .filter(|t| t.status == ToolStatus::Missing)
        .count();
    let required_missing = tools
        .iter()
        .filter(|t| t.required && t.status == ToolStatus::Missing)
        .count();
    ToolCheckResult {
        tools,
        available,
        missing,
        required_missing,
    }
}

/// Format a human-readable doctor report.
pub fn format_doctor_report(result: &ToolCheckResult) -> String {
    let mut out = String::new();
    out.push_str("IMF Wizard — Dependency Check\n");
    out.push_str("==============================\n\n");

    for tool in &result.tools {
        match tool.status {
            ToolStatus::Available => {
                let ver = tool.version.as_deref().unwrap_or("");
                let ver_str = if ver.is_empty() {
                    String::new()
                } else {
                    format!(" ({ver})")
                };
                out.push_str(&format!("  [OK] {}{}\n", tool.name, ver_str));
                out.push_str(&format!("       {}\n", tool.purpose));
                if let Some(ref p) = tool.path {
                    out.push_str(&format!("       Path: {}\n\n", p.display()));
                } else {
                    out.push('\n');
                }
            }
            ToolStatus::Missing | ToolStatus::VersionMismatch => {
                let tag = if tool.required { " [REQUIRED]" } else { "" };
                out.push_str(&format!("  [--] {} — NOT FOUND{}\n", tool.name, tag));
                out.push_str(&format!("       {}\n\n", tool.purpose));
            }
        }
    }

    let total = result.tools.len();
    out.push_str("-------------------------------\n");
    out.push_str(&format!("Available: {}/{total}\n", result.available));
    if result.required_missing > 0 {
        out.push_str(&format!(
            "WARNING: {} required tool(s) missing — core features will not work!\n",
            result.required_missing
        ));
    }
    out
}

// Cached tool checks for has_* functions.
static TOOL_RESULT: OnceLock<ToolCheckResult> = OnceLock::new();

fn cached_result() -> &'static ToolCheckResult {
    TOOL_RESULT.get_or_init(check_all_tools)
}

fn tool_available(name: &str) -> bool {
    cached_result()
        .tools
        .iter()
        .any(|t| t.name == name && t.status == ToolStatus::Available)
}

pub fn has_ffmpeg() -> bool {
    tool_available("ffmpeg")
}
pub fn has_ffprobe() -> bool {
    tool_available("ffprobe")
}
pub fn has_grk_compress() -> bool {
    tool_available("grk_compress")
}
pub fn has_dovi_tool() -> bool {
    tool_available("dovi_tool")
}
pub fn has_ctlrender() -> bool {
    tool_available("ctlrender")
}
pub fn has_xmllint() -> bool {
    tool_available("xmllint")
}
pub fn has_wkhtmltopdf() -> bool {
    tool_available("wkhtmltopdf")
}
pub fn has_weasyprint() -> bool {
    tool_available("weasyprint")
}
pub fn has_ascp() -> bool {
    tool_available("ascp")
}

/// Get the path to a tool if available.
pub fn tool_path(name: &str) -> Option<&'static PathBuf> {
    cached_result()
        .tools
        .iter()
        .find(|t| t.name == name && t.status == ToolStatus::Available)
        .and_then(|t| t.path.as_ref())
}
