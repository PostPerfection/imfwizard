use std::path::Path;

use postkit::packaging::{AssetMap, AssetMapAsset, ns};

use crate::MxfTrackFile;
use crate::imp::CplEntry;

/// Write an ASSETMAP.xml (ST 429-9, shared DCP/IMF) using the postkit writer.
/// Covers the PKL, every CPL, and all track files. IMF omits `<VolumeCount>`.
pub fn write_assetmap(
    path: &Path,
    pkl_uuid: &str,
    cpls: &[CplEntry],
    track_files: &[MxfTrackFile],
) -> std::io::Result<()> {
    let mut assets = vec![AssetMapAsset {
        id: pkl_uuid.to_string(),
        path: format!("PKL_{pkl_uuid}.xml"),
        packing_list: true,
    }];
    for cpl in cpls {
        let file_name = cpl
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();
        assets.push(AssetMapAsset {
            id: cpl.uuid.clone(),
            path: file_name,
            packing_list: false,
        });
    }
    for tf in track_files {
        let file_name = tf
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();
        assets.push(AssetMapAsset {
            id: tf.uuid.clone(),
            path: file_name,
            packing_list: false,
        });
    }

    let am = AssetMap {
        uuid: uuid::Uuid::new_v4().to_string(),
        namespace: ns::AM_SMPTE.to_string(),
        issuer: "IMF Wizard".to_string(),
        creator: "IMF Wizard".to_string(),
        issue_date: crate::issue_date(),
        assets,
    };
    std::fs::write(path, am.to_xml())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_assetmap_includes_track_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ASSETMAP.xml");
        let cpls = [CplEntry {
            uuid: "cpl".into(),
            path: dir.path().join("CPL_cpl.xml"),
        }];
        let tracks = [MxfTrackFile {
            path: dir.path().join("VIDEO_track.mxf"),
            uuid: "track".into(),
            ..Default::default()
        }];

        write_assetmap(&path, "pkl", &cpls, &tracks).unwrap();
        let xml = std::fs::read_to_string(path).unwrap();
        assert!(xml.contains("<PackingList>true</PackingList>"));
        assert!(xml.contains("<Path>CPL_cpl.xml</Path>"));
        assert!(xml.contains("<Id>urn:uuid:track</Id>"));
        assert!(xml.contains("<Path>VIDEO_track.mxf</Path>"));
    }
}
