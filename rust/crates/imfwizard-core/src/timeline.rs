use quick_xml::events::Event;
use quick_xml::name::QName;
use quick_xml::reader::Reader;
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CplInfo {
    pub id: String,
    pub file_path: String,
    pub title: String,
}

/// The local part of an element name (drops any `cc:` style namespace prefix).
fn local_name(qname: QName) -> String {
    String::from_utf8_lossy(qname.local_name().as_ref()).into_owned()
}

fn strip_urn(s: &str) -> String {
    s.replace("urn:uuid:", "")
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

    let mut reader = Reader::from_str(&content);
    reader.config_mut().trim_text(true);

    // state mirrors the CPL nesting: SegmentList > Segment > SequenceList > *Sequence > ResourceList > Resource
    let mut in_segment = false;
    let mut in_image_seq = false;
    let mut in_audio_seq = false;
    let mut in_resource = false;
    let mut seg = SegmentEntry::default();
    // the element we are currently reading text for
    let mut cur: String = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name());
                cur = name.clone();
                match name.as_str() {
                    "Segment" => {
                        in_segment = true;
                        segment_number += 1;
                        seg = SegmentEntry {
                            segment_number,
                            ..Default::default()
                        };
                    }
                    "MainImageSequence" => {
                        in_image_seq = true;
                        in_audio_seq = false;
                    }
                    "MainAudioSequence" => {
                        in_audio_seq = true;
                        in_image_seq = false;
                    }
                    "Resource" => in_resource = true,
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if !in_segment {
                    continue;
                }
                let text = t.unescape().unwrap_or_default().trim().to_string();
                if text.is_empty() {
                    continue;
                }
                match cur.as_str() {
                    // segment id lives directly under Segment, before any sequence/resource
                    "Id" if !in_image_seq
                        && !in_audio_seq
                        && !in_resource
                        && seg.segment_id.is_empty() =>
                    {
                        seg.segment_id = strip_urn(&text);
                    }
                    "EditRate" if seg.edit_rate.is_empty() => seg.edit_rate = text,
                    "TrackFileId" if in_resource => {
                        let id = strip_urn(&text);
                        if in_image_seq && seg.video_track_file_id.is_empty() {
                            seg.video_track_file_id = id;
                        } else if in_audio_seq && seg.audio_track_file_id.is_empty() {
                            seg.audio_track_file_id = id;
                        }
                    }
                    "SourceDuration" if in_resource => {
                        if let Ok(v) = text.parse::<u64>() {
                            seg.duration_frames = v;
                        }
                    }
                    "IntrinsicDuration" if in_resource && seg.duration_frames == 0 => {
                        if let Ok(v) = text.parse::<u64>() {
                            seg.duration_frames = v;
                        }
                    }
                    "EntryPoint" if in_resource => {
                        if let Ok(v) = text.parse::<u64>() {
                            seg.entry_point = v;
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                cur.clear();
                match local_name(e.name()).as_str() {
                    "Resource" => in_resource = false,
                    "MainImageSequence" => in_image_seq = false,
                    "MainAudioSequence" => in_audio_seq = false,
                    "Segment" => {
                        if in_segment {
                            seg.video_file = asset_map
                                .get(&seg.video_track_file_id)
                                .map(|p| imp_dir.join(p).to_string_lossy().into_owned())
                                .unwrap_or_default();
                            seg.audio_file = asset_map
                                .get(&seg.audio_track_file_id)
                                .map(|p| imp_dir.join(p).to_string_lossy().into_owned())
                                .unwrap_or_default();
                            entries.push(std::mem::take(&mut seg));
                        }
                        in_segment = false;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                tracing::error!("CPL parse error: {e}");
                break;
            }
            _ => {}
        }
    }

    entries
}

/// List CPLs in an IMP directory by scanning ASSETMAP.
pub fn list_cpls(imp_dir: &Path) -> Vec<CplInfo> {
    read_assetmap_assets(imp_dir)
        .into_iter()
        .filter_map(|(id, rel_path)| {
            let full_path = imp_dir.join(&rel_path);
            if !is_xml_file(&full_path) {
                return None;
            }
            let file_content = std::fs::read_to_string(&full_path).ok()?;
            if !file_content.contains("CompositionPlaylist") {
                return None;
            }
            Some(CplInfo {
                id,
                title: first_element_text(&file_content, "ContentTitle").unwrap_or_default(),
                file_path: rel_path,
            })
        })
        .collect()
}

// a picture mxf is an asset too, and reading one whole to look for a cpl tag takes gigabytes
fn is_xml_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
}

/// Asset id to the path it names, relative to the IMP directory.
pub(crate) fn parse_assetmap(imp_dir: &Path) -> HashMap<String, String> {
    read_assetmap_assets(imp_dir).into_iter().collect()
}

/// Read (id, path) pairs from an ASSETMAP, with ids stripped of the `urn:uuid:` prefix.
fn read_assetmap_assets(imp_dir: &Path) -> Vec<(String, String)> {
    let Some(assetmap_path) = find_assetmap(imp_dir) else {
        return Vec::new();
    };
    let Ok(content) = std::fs::read_to_string(&assetmap_path) else {
        return Vec::new();
    };

    let mut reader = Reader::from_str(&content);
    reader.config_mut().trim_text(true);

    let mut assets = Vec::new();
    let mut in_asset = false;
    let mut cur = String::new();
    let mut id = String::new();
    let mut path = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name());
                if name == "Asset" {
                    in_asset = true;
                    id.clear();
                    path.clear();
                }
                cur = name;
            }
            Ok(Event::Text(t)) if in_asset => {
                let text = t.unescape().unwrap_or_default().trim().to_string();
                match cur.as_str() {
                    "Id" if id.is_empty() => id = strip_urn(&text),
                    "Path" if path.is_empty() => path = text,
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                cur.clear();
                if local_name(e.name()) == "Asset" {
                    if in_asset && !id.is_empty() && !path.is_empty() {
                        assets.push((std::mem::take(&mut id), std::mem::take(&mut path)));
                    }
                    in_asset = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    assets
}

/// Read the text of the first occurrence of `<tag>` in an XML string.
fn first_element_text(xml: &str, tag: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut in_tag = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if local_name(e.name()) == tag => in_tag = true,
            Ok(Event::Text(t)) if in_tag => {
                return Some(t.unescape().unwrap_or_default().trim().to_string());
            }
            Ok(Event::End(e)) if local_name(e.name()) == tag => in_tag = false,
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_cpls_reads_assetmap_and_title() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("CPL_abc.xml"),
            r#"<?xml version="1.0"?><CompositionPlaylist><ContentTitle>My &amp; Film</ContentTitle></CompositionPlaylist>"#,
        )
        .unwrap();
        // essence that happens to carry the tag must never be read for it
        std::fs::write(
            dir.path().join("video.mxf"),
            b"\x06\x0e\x2bCompositionPlaylist",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("ASSETMAP.xml"),
            r#"<?xml version="1.0"?><AssetMap><AssetList>
                <Asset><Id>urn:uuid:1111</Id><ChunkList><Chunk><Path>CPL_abc.xml</Path></Chunk></ChunkList></Asset>
                <Asset><Id>urn:uuid:2222</Id><ChunkList><Chunk><Path>video.mxf</Path></Chunk></ChunkList></Asset>
            </AssetList></AssetMap>"#,
        )
        .unwrap();

        let cpls = list_cpls(dir.path());
        assert_eq!(cpls.len(), 1, "{cpls:?}");
        assert_eq!(cpls[0].id, "1111");
        assert_eq!(cpls[0].file_path, "CPL_abc.xml");
        assert_eq!(cpls[0].title, "My & Film");
    }

    #[test]
    fn get_timeline_parses_segment_resources() {
        let dir = tempfile::tempdir().unwrap();
        let cpl = dir.path().join("CPL.xml");
        std::fs::write(
            &cpl,
            r#"<?xml version="1.0"?>
<CompositionPlaylist>
  <SegmentList><Segment>
    <Id>urn:uuid:seg-1</Id>
    <SequenceList>
      <cc:MainImageSequence>
        <ResourceList><Resource>
          <TrackFileId>urn:uuid:video-1</TrackFileId>
          <EditRate>24 1</EditRate>
          <IntrinsicDuration>240</IntrinsicDuration>
          <SourceDuration>200</SourceDuration>
          <EntryPoint>5</EntryPoint>
        </Resource></ResourceList>
      </cc:MainImageSequence>
      <cc:MainAudioSequence>
        <ResourceList><Resource>
          <TrackFileId>urn:uuid:audio-1</TrackFileId>
        </Resource></ResourceList>
      </cc:MainAudioSequence>
    </SequenceList>
  </Segment></SegmentList>
</CompositionPlaylist>"#,
        )
        .unwrap();

        let t = get_timeline(&cpl);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].segment_id, "seg-1");
        assert_eq!(t[0].video_track_file_id, "video-1");
        assert_eq!(t[0].audio_track_file_id, "audio-1");
        assert_eq!(t[0].duration_frames, 200);
        assert_eq!(t[0].entry_point, 5);
        assert_eq!(t[0].edit_rate, "24 1");
    }
}
