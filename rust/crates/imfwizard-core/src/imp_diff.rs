//! Compare two IMF packages (OV vs supplemental) and report differences.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DiffError {
    #[error("IMP directory does not exist: {0}")]
    NotFound(PathBuf),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DiffStatus {
    Added,
    Removed,
    Modified,
    Unchanged,
    Replaced,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrackDiff {
    pub track_id: String,
    pub essence_type: String,
    pub status: DiffStatus,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SegmentDiff {
    pub cpl_id: String,
    pub entry_point: u64,
    pub duration: u64,
    pub status: DiffStatus,
    pub old_track_id: String,
    pub new_track_id: String,
}

#[derive(Debug, Clone)]
pub struct DiffOptions {
    pub imp_a: PathBuf,
    pub imp_b: PathBuf,
    pub include_hashes: bool,
    pub show_unchanged: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffResult {
    pub tracks_added: u32,
    pub tracks_removed: u32,
    pub tracks_modified: u32,
    pub segments_changed: u32,
    pub track_diffs: Vec<TrackDiff>,
    pub segment_diffs: Vec<SegmentDiff>,
    pub cpl_title_changed: bool,
    pub cpl_annotation_changed: bool,
    pub edit_rate_changed: bool,
}

#[derive(Debug, Clone)]
struct AssetInfo {
    #[allow(dead_code)]
    id: String,
    asset_type: String,
    size: u64,
    path: PathBuf,
}

#[derive(Debug, Clone, Default)]
struct CplInfo {
    id: String,
    title: String,
    annotation: String,
    edit_rate: String,
    track_file_ids: Vec<String>,
}

fn parse_assetmap(imp_dir: &Path) -> HashMap<String, AssetInfo> {
    let mut assets = HashMap::new();

    let assetmap = if imp_dir.join("ASSETMAP.xml").exists() {
        imp_dir.join("ASSETMAP.xml")
    } else if imp_dir.join("ASSETMAP").exists() {
        imp_dir.join("ASSETMAP")
    } else {
        return assets;
    };

    let Ok(content) = fs::read_to_string(&assetmap) else {
        return assets;
    };

    // Parse asset entries using simple regex-like scanning
    // Look for <Asset>...<Id>urn:uuid:XXX</Id>...<Path>YYY</Path>...</Asset>
    for asset_block in content.split("<Asset>").skip(1) {
        let Some(end) = asset_block.find("</Asset>") else {
            continue;
        };
        let block = &asset_block[..end];

        let id = extract_tag(block, "Id")
            .unwrap_or_default()
            .trim_start_matches("urn:uuid:")
            .to_string();

        let rel_path = extract_tag(block, "Path").unwrap_or_default();

        if id.is_empty() || rel_path.is_empty() {
            continue;
        }

        let full_path = imp_dir.join(&rel_path);
        let size = fs::metadata(&full_path).map(|m| m.len()).unwrap_or(0);

        let ext = full_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let asset_type = match ext.as_str() {
            "mxf" => "mxf",
            "xml" => "xml",
            _ => "other",
        }
        .to_string();

        assets.insert(
            id.clone(),
            AssetInfo {
                id,
                asset_type,
                size,
                path: full_path,
            },
        );
    }

    assets
}

fn parse_cpl(cpl_path: &Path) -> CplInfo {
    let mut info = CplInfo::default();

    let Ok(content) = fs::read_to_string(cpl_path) else {
        return info;
    };

    info.id = extract_tag(&content, "Id")
        .unwrap_or_default()
        .trim_start_matches("urn:uuid:")
        .to_string();
    info.title = extract_tag(&content, "ContentTitle").unwrap_or_default();
    info.annotation = extract_tag(&content, "Annotation").unwrap_or_default();
    info.edit_rate = extract_tag(&content, "EditRate").unwrap_or_default();

    // Extract all TrackFileId references
    for segment in content.split("<TrackFileId>").skip(1) {
        if let Some(end) = segment.find("</TrackFileId>") {
            let raw = segment[..end].trim();
            let id = raw.trim_start_matches("urn:uuid:").to_string();
            info.track_file_ids.push(id);
        }
    }

    info
}

fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim().to_string())
}

fn find_cpl(assets: &HashMap<String, AssetInfo>) -> Option<&AssetInfo> {
    assets.values().find(|a| {
        a.asset_type == "xml"
            && a.path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.contains("CPL"))
                .unwrap_or(false)
    })
}

/// Compare two IMF packages and return a detailed diff.
pub fn diff_packages(opts: &DiffOptions) -> Result<DiffResult, DiffError> {
    if !opts.imp_a.exists() {
        return Err(DiffError::NotFound(opts.imp_a.clone()));
    }
    if !opts.imp_b.exists() {
        return Err(DiffError::NotFound(opts.imp_b.clone()));
    }

    let assets_a = parse_assetmap(&opts.imp_a);
    let assets_b = parse_assetmap(&opts.imp_b);

    // Find CPL in each
    let cpl_info_a = find_cpl(&assets_a)
        .map(|a| parse_cpl(&a.path))
        .unwrap_or_default();
    let cpl_info_b = find_cpl(&assets_b)
        .map(|a| parse_cpl(&a.path))
        .unwrap_or_default();

    let mut result = DiffResult {
        tracks_added: 0,
        tracks_removed: 0,
        tracks_modified: 0,
        segments_changed: 0,
        track_diffs: Vec::new(),
        segment_diffs: Vec::new(),
        cpl_title_changed: cpl_info_a.title != cpl_info_b.title,
        cpl_annotation_changed: cpl_info_a.annotation != cpl_info_b.annotation,
        edit_rate_changed: cpl_info_a.edit_rate != cpl_info_b.edit_rate,
    };

    // Compare MXF tracks: find removed/modified
    for (id, asset) in &assets_a {
        if asset.asset_type != "mxf" {
            continue;
        }

        let status;
        let detail;

        if let Some(other) = assets_b.get(id) {
            if opts.include_hashes && asset.size != other.size {
                status = DiffStatus::Modified;
                detail = format!("Track {id} size changed ({} → {})", asset.size, other.size);
                result.tracks_modified += 1;
            } else {
                status = DiffStatus::Unchanged;
                detail = String::new();
                if !opts.show_unchanged {
                    continue;
                }
            }
        } else {
            status = DiffStatus::Removed;
            detail = format!("Track {id} removed in B");
            result.tracks_removed += 1;
        }

        result.track_diffs.push(TrackDiff {
            track_id: id.clone(),
            essence_type: "video".to_string(),
            status,
            detail,
        });
    }

    // Find added tracks
    for (id, asset) in &assets_b {
        if asset.asset_type != "mxf" {
            continue;
        }
        if !assets_a.contains_key(id) {
            result.tracks_added += 1;
            result.track_diffs.push(TrackDiff {
                track_id: id.clone(),
                essence_type: "video".to_string(),
                status: DiffStatus::Added,
                detail: format!("New track {id} in B"),
            });
        }
    }

    // Compare segments (track file references in CPL)
    let seg_a: HashSet<&str> = cpl_info_a
        .track_file_ids
        .iter()
        .map(|s| s.as_str())
        .collect();
    let seg_b: HashSet<&str> = cpl_info_b
        .track_file_ids
        .iter()
        .map(|s| s.as_str())
        .collect();

    for tid in &seg_a {
        if !seg_b.contains(tid) {
            result.segments_changed += 1;
            result.segment_diffs.push(SegmentDiff {
                cpl_id: cpl_info_a.id.clone(),
                entry_point: 0,
                duration: 0,
                status: DiffStatus::Removed,
                old_track_id: tid.to_string(),
                new_track_id: String::new(),
            });
        }
    }
    for tid in &seg_b {
        if !seg_a.contains(tid) {
            result.segments_changed += 1;
            result.segment_diffs.push(SegmentDiff {
                cpl_id: cpl_info_b.id.clone(),
                entry_point: 0,
                duration: 0,
                status: DiffStatus::Added,
                old_track_id: String::new(),
                new_track_id: tid.to_string(),
            });
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_imp(dir: &Path, cpl_title: &str, track_ids: &[&str]) {
        // Create ASSETMAP.xml
        let mut asset_entries = String::new();
        let cpl_filename = "CPL_test.xml";

        // Add CPL asset entry
        asset_entries.push_str(&format!(
            r#"<Asset><Id>urn:uuid:cpl-id-1234</Id><ChunkList><Chunk><Path>{}</Path></Chunk></ChunkList></Asset>"#,
            cpl_filename
        ));

        // Add MXF track entries
        for tid in track_ids {
            let filename = format!("{}.mxf", tid);
            asset_entries.push_str(&format!(
                r#"<Asset><Id>urn:uuid:{}</Id><ChunkList><Chunk><Path>{}</Path></Chunk></ChunkList></Asset>"#,
                tid, filename
            ));
            // Create the MXF file with some content
            fs::write(dir.join(&filename), vec![0u8; 1024]).unwrap();
        }

        let assetmap = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:am-1234</Id>
  <AssetList>{}</AssetList>
</AssetMap>"#,
            asset_entries
        );
        fs::write(dir.join("ASSETMAP.xml"), assetmap).unwrap();

        // Create CPL XML
        let track_refs: String = track_ids
            .iter()
            .map(|tid| format!("<TrackFileId>urn:uuid:{}</TrackFileId>", tid))
            .collect::<Vec<_>>()
            .join("\n");

        let cpl = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016">
  <Id>urn:uuid:cpl-id-1234</Id>
  <ContentTitle>{}</ContentTitle>
  <Annotation>Test annotation</Annotation>
  <EditRate>24 1</EditRate>
  <SegmentList>
    <Segment>
      <SequenceList>
        <Sequence>
          <ResourceList>
            <Resource>
              {}
            </Resource>
          </ResourceList>
        </Sequence>
      </SequenceList>
    </Segment>
  </SegmentList>
</CompositionPlaylist>"#,
            cpl_title, track_refs
        );
        fs::write(dir.join(cpl_filename), cpl).unwrap();
    }

    #[test]
    fn test_identical_packages() {
        let tmp_a = TempDir::new().unwrap();
        let tmp_b = TempDir::new().unwrap();

        create_test_imp(tmp_a.path(), "Test Film", &["track-001", "track-002"]);
        create_test_imp(tmp_b.path(), "Test Film", &["track-001", "track-002"]);

        let opts = DiffOptions {
            imp_a: tmp_a.path().to_path_buf(),
            imp_b: tmp_b.path().to_path_buf(),
            include_hashes: false,
            show_unchanged: false,
        };

        let result = diff_packages(&opts).unwrap();
        assert_eq!(result.tracks_added, 0);
        assert_eq!(result.tracks_removed, 0);
        assert_eq!(result.tracks_modified, 0);
        assert_eq!(result.segments_changed, 0);
        assert!(!result.cpl_title_changed);
        assert!(!result.edit_rate_changed);
    }

    #[test]
    fn test_track_added() {
        let tmp_a = TempDir::new().unwrap();
        let tmp_b = TempDir::new().unwrap();

        create_test_imp(tmp_a.path(), "Test Film", &["track-001"]);
        create_test_imp(tmp_b.path(), "Test Film", &["track-001", "track-002"]);

        let opts = DiffOptions {
            imp_a: tmp_a.path().to_path_buf(),
            imp_b: tmp_b.path().to_path_buf(),
            include_hashes: false,
            show_unchanged: false,
        };

        let result = diff_packages(&opts).unwrap();
        assert_eq!(result.tracks_added, 1);
        assert_eq!(result.tracks_removed, 0);
        assert_eq!(result.segments_changed, 1);
    }

    #[test]
    fn test_track_removed() {
        let tmp_a = TempDir::new().unwrap();
        let tmp_b = TempDir::new().unwrap();

        create_test_imp(tmp_a.path(), "Test Film", &["track-001", "track-002"]);
        create_test_imp(tmp_b.path(), "Test Film", &["track-001"]);

        let opts = DiffOptions {
            imp_a: tmp_a.path().to_path_buf(),
            imp_b: tmp_b.path().to_path_buf(),
            include_hashes: false,
            show_unchanged: false,
        };

        let result = diff_packages(&opts).unwrap();
        assert_eq!(result.tracks_removed, 1);
        assert_eq!(result.tracks_added, 0);
        assert_eq!(result.segments_changed, 1);
    }

    #[test]
    fn test_title_changed() {
        let tmp_a = TempDir::new().unwrap();
        let tmp_b = TempDir::new().unwrap();

        create_test_imp(tmp_a.path(), "Version 1", &["track-001"]);
        create_test_imp(tmp_b.path(), "Version 2", &["track-001"]);

        let opts = DiffOptions {
            imp_a: tmp_a.path().to_path_buf(),
            imp_b: tmp_b.path().to_path_buf(),
            include_hashes: false,
            show_unchanged: false,
        };

        let result = diff_packages(&opts).unwrap();
        assert!(result.cpl_title_changed);
        assert_eq!(result.tracks_added, 0);
        assert_eq!(result.tracks_removed, 0);
    }

    #[test]
    fn test_size_change_detected_with_hashes() {
        let tmp_a = TempDir::new().unwrap();
        let tmp_b = TempDir::new().unwrap();

        create_test_imp(tmp_a.path(), "Test Film", &["track-001"]);
        create_test_imp(tmp_b.path(), "Test Film", &["track-001"]);

        // Modify file size in B
        fs::write(tmp_b.path().join("track-001.mxf"), vec![0u8; 2048]).unwrap();

        let opts = DiffOptions {
            imp_a: tmp_a.path().to_path_buf(),
            imp_b: tmp_b.path().to_path_buf(),
            include_hashes: true,
            show_unchanged: false,
        };

        let result = diff_packages(&opts).unwrap();
        assert_eq!(result.tracks_modified, 1);
    }

    #[test]
    fn test_missing_imp_returns_error() {
        let tmp = TempDir::new().unwrap();
        create_test_imp(tmp.path(), "Test", &["t1"]);

        let opts = DiffOptions {
            imp_a: tmp.path().to_path_buf(),
            imp_b: PathBuf::from("/nonexistent/path"),
            include_hashes: false,
            show_unchanged: false,
        };

        let result = diff_packages(&opts);
        assert!(result.is_err());
    }

    #[test]
    fn test_show_unchanged() {
        let tmp_a = TempDir::new().unwrap();
        let tmp_b = TempDir::new().unwrap();

        create_test_imp(tmp_a.path(), "Test", &["track-001", "track-002"]);
        create_test_imp(tmp_b.path(), "Test", &["track-001", "track-002"]);

        let opts = DiffOptions {
            imp_a: tmp_a.path().to_path_buf(),
            imp_b: tmp_b.path().to_path_buf(),
            include_hashes: false,
            show_unchanged: true,
        };

        let result = diff_packages(&opts).unwrap();
        assert_eq!(result.track_diffs.len(), 2);
        assert!(
            result
                .track_diffs
                .iter()
                .all(|d| d.status == DiffStatus::Unchanged)
        );
    }
}
