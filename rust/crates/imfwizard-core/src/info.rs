use serde::{Deserialize, Serialize};
use std::path::Path;

/// IMP metadata info (returned as JSON).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImpInfo {
    pub title: String,
    pub annotation: String,
    pub issuer: String,
    pub cpl_count: usize,
    pub edit_rate: String,
    pub duration_frames: u64,
}

/// Inspect an IMP directory and return metadata.
pub fn inspect_imp(imp_dir: &Path) -> Result<ImpInfo, String> {
    if !imp_dir.is_dir() {
        return Err(format!("Not a directory: {}", imp_dir.display()));
    }

    let cpls = crate::timeline::list_cpls(imp_dir);
    let mut info = ImpInfo {
        cpl_count: cpls.len(),
        ..Default::default()
    };

    // Read metadata from first CPL
    if let Some(cpl) = cpls.first() {
        let cpl_path = imp_dir.join(&cpl.file_path);
        if let Ok(content) = std::fs::read_to_string(&cpl_path) {
            info.title = extract_xml_text(&content, "ContentTitle").unwrap_or_default();
            info.annotation = extract_xml_text(&content, "AnnotationText").unwrap_or_default();
            info.issuer = extract_xml_text(&content, "Issuer").unwrap_or_default();
            info.edit_rate = extract_xml_text(&content, "EditRate").unwrap_or_default();

            // Sum segment durations
            let mut total_duration = 0u64;
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("<IntrinsicDuration>")
                    && let Some(val) = extract_xml_text(trimmed, "IntrinsicDuration")
                {
                    total_duration += val.parse::<u64>().unwrap_or(0);
                }
            }
            info.duration_frames = total_duration;
        }
    }

    Ok(info)
}

fn extract_xml_text(content: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = content.find(&open)?;
    let after = &content[start + open.len()..];
    let end = after.find(&close)?;
    let val = after[..end].trim().to_string();
    if val.is_empty() { None } else { Some(val) }
}
