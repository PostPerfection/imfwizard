use std::path::Path;

use postkit::packaging::{AssetMap, AssetMapAsset, ns};

use crate::MxfTrackFile;

/// Write an ASSETMAP.xml (ST 429-9, shared DCP/IMF) using the postkit writer.
/// IMF omits `<VolumeCount>`.
pub fn write_assetmap(
    path: &Path,
    pkl_uuid: &str,
    cpl_uuid: &str,
    track_files: &[MxfTrackFile],
) -> std::io::Result<()> {
    let mut assets = vec![
        AssetMapAsset {
            id: pkl_uuid.to_string(),
            path: format!("PKL_{pkl_uuid}.xml"),
            packing_list: true,
        },
        AssetMapAsset {
            id: cpl_uuid.to_string(),
            path: format!("CPL_{cpl_uuid}.xml"),
            packing_list: false,
        },
    ];
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
        let tracks = [MxfTrackFile {
            path: dir.path().join("VIDEO_track.mxf"),
            uuid: "track".into(),
            ..Default::default()
        }];

        write_assetmap(&path, "pkl", "cpl", &tracks).unwrap();
        let xml = std::fs::read_to_string(path).unwrap();
        assert!(xml.contains("<PackingList>true</PackingList>"));
        assert!(xml.contains("<Id>urn:uuid:track</Id>"));
        assert!(xml.contains("<Path>VIDEO_track.mxf</Path>"));
    }
}
