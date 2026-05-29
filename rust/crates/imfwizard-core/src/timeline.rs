use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// A segment entry from an IMF CPL (equivalent to DCP reel).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SegmentEntry {
    pub segment_id: String,
    pub segment_number: u32,
    pub duration_frames: u64,
    pub entry_point: u64,
    pub edit_rate: String,
    pub video_track_file_id: String,
    pub audio_track_file_id: String,
    pub video_file: String,
    pub audio_file: String,
}

/// Parse an IMF CPL and return its segment/resource timeline.
pub fn get_timeline(cpl_path: &Path) -> Vec<SegmentEntry> {
    let imp_dir = cpl_path.parent().unwrap_or(Path::new("."));
    let content = match std::fs::read_to_string(cpl_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to read CPL: {e}");
            return Vec::new();
        }
    };

    let asset_map = parse_assetmap(imp_dir);
    let mut entries = Vec::new();
    let mut segment_number = 0u32;

    // IMF CPL structure: SegmentList > Segment > SequenceList > MainImageSequence/MainAudioSequence > ResourceList > Resource
    let mut in_segment = false;
    let mut segment_id = String::new();
    let mut in_image_seq = false;
    let mut in_audio_seq = false;
    let mut in_resource = false;
    let mut edit_rate = String::new();
    let mut duration = 0u64;
    let mut entry_point = 0u64;
    let mut video_track_file_id = String::new();
    let mut audio_track_file_id = String::new();

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.contains("<Segment>") || trimmed.contains("<Segment ") {
            in_segment = true;
            segment_number += 1;
            segment_id.clear();
            video_track_file_id.clear();
            audio_track_file_id.clear();
            duration = 0;
            entry_point = 0;
            edit_rate.clear();
        } else if trimmed.contains("</Segment>") {
            if in_segment {
                let video_file = asset_map
                    .get(&video_track_file_id)
                    .map(|p| imp_dir.join(p).to_string_lossy().into_owned())
                    .unwrap_or_default();
                let audio_file = asset_map
                    .get(&audio_track_file_id)
                    .map(|p| imp_dir.join(p).to_string_lossy().into_owned())
                    .unwrap_or_default();
                entries.push(SegmentEntry {
                    segment_id: segment_id.clone(),
                    segment_number,
                    duration_frames: duration,
                    entry_point,
                    edit_rate: edit_rate.clone(),
                    video_track_file_id: video_track_file_id.clone(),
                    audio_track_file_id: audio_track_file_id.clone(),
                    video_file,
                    audio_file,
                });
            }
            in_segment = false;
        } else if in_segment {
            if trimmed.contains("MainImageSequence") && !trimmed.contains('/') {
                in_image_seq = true;
                in_audio_seq = false;
            } else if trimmed.contains("MainAudioSequence") && !trimmed.contains('/') {
                in_audio_seq = true;
                in_image_seq = false;
            } else if trimmed.contains("</MainImageSequence")
                || trimmed.contains("/>") && trimmed.contains("MainImageSequence")
            {
                in_image_seq = false;
            } else if trimmed.contains("</MainAudioSequence")
                || trimmed.contains("/>") && trimmed.contains("MainAudioSequence")
            {
                in_audio_seq = false;
            }

            if trimmed.contains("<Resource>") || trimmed.contains("<Resource ") {
                in_resource = true;
            } else if trimmed.contains("</Resource>") {
                in_resource = false;
            }

            if in_segment && !in_image_seq && !in_audio_seq {
                if let Some(id) = extract_xml_value(trimmed, "Id") {
                    if segment_id.is_empty() {
                        segment_id = id.replace("urn:uuid:", "");
                    }
                }
            }

            if in_resource {
                if let Some(id) = extract_xml_value(trimmed, "TrackFileId") {
                    let clean_id = id.replace("urn:uuid:", "");
                    if in_image_seq && video_track_file_id.is_empty() {
                        video_track_file_id = clean_id;
                    } else if in_audio_seq && audio_track_file_id.is_empty() {
                        audio_track_file_id = clean_id;
                    }
                }
                if let Some(d) = extract_xml_value(trimmed, "SourceDuration") {
                    if let Ok(v) = d.parse::<u64>() {
                        duration = v;
                    }
                } else if duration == 0 {
                    if let Some(d) = extract_xml_value(trimmed, "IntrinsicDuration") {
                        if let Ok(v) = d.parse::<u64>() {
                            duration = v;
                        }
                    }
                }
                if let Some(ep) = extract_xml_value(trimmed, "EntryPoint") {
                    if let Ok(v) = ep.parse::<u64>() {
                        entry_point = v;
                    }
                }
            }

            if let Some(er) = extract_xml_value(trimmed, "EditRate") {
                if edit_rate.is_empty() {
                    edit_rate = er;
                }
            }
        }
    }

    entries
}

/// List CPLs in an IMP directory by scanning ASSETMAP.
pub fn list_cpls(imp_dir: &Path) -> Vec<CplInfo> {
    let assetmap_path = find_assetmap(imp_dir);
    let assetmap_path = match assetmap_path {
        Some(p) => p,
        None => return Vec::new(),
    };
    let content = match std::fs::read_to_string(&assetmap_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut cpls = Vec::new();
    let mut in_asset = false;
    let mut current_id = String::new();
    let mut current_path = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("<Asset>") || trimmed.starts_with("<Asset ") {
            in_asset = true;
            current_id.clear();
            current_path.clear();
        } else if trimmed == "</Asset>" {
            if in_asset && !current_id.is_empty() && !current_path.is_empty() {
                let full_path = imp_dir.join(&current_path);
                if full_path.exists() {
                    if let Ok(file_content) = std::fs::read_to_string(&full_path) {
                        if file_content.contains("CompositionPlaylist") {
                            let title = extract_xml_value(&file_content, "ContentTitle")
                                .unwrap_or_default();
                            cpls.push(CplInfo {
                                id: current_id.clone(),
                                file_path: current_path.clone(),
                                title,
                            });
                        }
                    }
                }
            }
            in_asset = false;
        } else if in_asset {
            if let Some(id) = extract_xml_value(trimmed, "Id") {
                current_id = id.replace("urn:uuid:", "");
            }
            if let Some(path) = extract_xml_value(trimmed, "Path") {
                current_path = path;
            }
        }
    }

    cpls
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CplInfo {
    pub id: String,
    pub file_path: String,
    pub title: String,
}

fn parse_assetmap(imp_dir: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let assetmap_path = match find_assetmap(imp_dir) {
        Some(p) => p,
        None => return map,
    };
    let content = match std::fs::read_to_string(&assetmap_path) {
        Ok(c) => c,
        Err(_) => return map,
    };

    let mut in_asset = false;
    let mut current_id = String::new();
    let mut current_path = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("<Asset>") || trimmed.starts_with("<Asset ") {
            in_asset = true;
            current_id.clear();
            current_path.clear();
        } else if trimmed == "</Asset>" {
            if in_asset && !current_id.is_empty() && !current_path.is_empty() {
                map.insert(current_id.clone(), current_path.clone());
            }
            in_asset = false;
        } else if in_asset {
            if let Some(id) = extract_xml_value(trimmed, "Id") {
                current_id = id.replace("urn:uuid:", "");
            }
            if let Some(path) = extract_xml_value(trimmed, "Path") {
                current_path = path;
            }
        }
    }

    map
}

fn find_assetmap(dir: &Path) -> Option<std::path::PathBuf> {
    for name in &["ASSETMAP", "ASSETMAP.xml"] {
        let path = dir.join(name);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn extract_xml_value(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let start_pos = text.find(&open)?;
    let after_open = &text[start_pos + open.len()..];
    let content_start = after_open.find('>')?;
    let content = &after_open[content_start + 1..];
    let end_pos = content.find(&close)?;
    let value = content[..end_pos].trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}
