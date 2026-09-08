use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

/// HDR metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HdrMetadata {
    pub max_cll: u32,
    pub max_fall: u32,
    pub mastering_display: String,
    pub color_primaries: String,
    pub transfer_characteristics: String,
}

/// HDR10+ metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Hdr10PlusMetadata {
    pub json_path: PathBuf,
    pub scene_count: usize,
}

/// Extract HDR10+ metadata using hdr10plus_tool.
pub fn extract_hdr10plus(
    input: &std::path::Path,
    output_json: &std::path::Path,
) -> Result<Hdr10PlusMetadata, String> {
    let out = Command::new("hdr10plus_tool")
        .arg("extract")
        .arg("-i")
        .arg(input)
        .arg("-o")
        .arg(output_json)
        .output()
        .map_err(|e| format!("Failed to run hdr10plus_tool: {e}"))?;

    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }

    Ok(Hdr10PlusMetadata {
        json_path: output_json.to_path_buf(),
        scene_count: count_scenes(output_json)?,
    })
}

// one entry a scene, which is how hdr10plus_tool summarises what it extracted
#[derive(Deserialize)]
struct SceneInfoSummary {
    #[serde(rename = "SceneFirstFrameIndex")]
    scene_first_frame_index: Vec<u32>,
}

#[derive(Deserialize)]
struct Hdr10PlusJson {
    #[serde(rename = "SceneInfoSummary")]
    scene_info_summary: SceneInfoSummary,
}

fn count_scenes(json_path: &std::path::Path) -> Result<usize, String> {
    let file = std::fs::File::open(json_path)
        .map_err(|e| format!("Failed to open {}: {e}", json_path.display()))?;
    let parsed: Hdr10PlusJson = serde_json::from_reader(std::io::BufReader::new(file))
        .map_err(|e| format!("Failed to read {}: {e}", json_path.display()))?;
    Ok(parsed.scene_info_summary.scene_first_frame_index.len())
}

/// Inject HDR10+ metadata using hdr10plus_tool.
pub fn inject_hdr10plus(
    input: &std::path::Path,
    json: &std::path::Path,
    output: &std::path::Path,
) -> Result<(), String> {
    let out = Command::new("hdr10plus_tool")
        .arg("inject")
        .arg("-i")
        .arg(input)
        .arg("-j")
        .arg(json)
        .arg("-o")
        .arg(output)
        .output()
        .map_err(|e| format!("Failed to run hdr10plus_tool: {e}"))?;

    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(())
}
