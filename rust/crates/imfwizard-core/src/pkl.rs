use std::path::Path;

use postkit::packaging::{PackingList, PklAsset, ns};

use crate::MxfTrackFile;

/// Write an IMF PKL (ST 2067-2) using the shared postkit writer.
pub fn write_pkl(
    path: &Path,
    pkl_uuid: &str,
    cpl_uuid: &str,
    cpl_path: &Path,
    track_files: &[MxfTrackFile],
) -> std::io::Result<()> {
    let cpl_hash = postkit::hash::hash_file(cpl_path, postkit::hash::HashAlgorithm::Sha1)
        .map(|h| h.base64)
        .unwrap_or_default();
    let cpl_size = std::fs::metadata(cpl_path).map(|m| m.len()).unwrap_or(0);

    let mut assets = vec![PklAsset {
        id: cpl_uuid.to_string(),
        hash: cpl_hash,
        size: cpl_size,
        asset_type: "text/xml".to_string(),
    }];
    for tf in track_files {
        assets.push(PklAsset {
            id: tf.uuid.clone(),
            hash: tf.hash.clone(),
            size: tf.size,
            asset_type: "application/mxf".to_string(),
        });
    }

    let pkl = PackingList {
        uuid: pkl_uuid.to_string(),
        namespace: ns::PKL_IMF.to_string(),
        issuer: "IMF Wizard".to_string(),
        creator: "IMF Wizard".to_string(),
        issue_date: crate::issue_date(),
        assets,
    };
    std::fs::write(path, pkl.to_xml())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_pkl_includes_track_files() {
        let dir = tempfile::tempdir().unwrap();
        let cpl_path = dir.path().join("CPL_cpl.xml");
        std::fs::write(&cpl_path, "cpl").unwrap();
        let pkl_path = dir.path().join("PKL_pkl.xml");
        let tracks = [MxfTrackFile {
            path: dir.path().join("VIDEO_track.mxf"),
            uuid: "track".into(),
            hash: "base64-hash".into(),
            size: 42,
            duration: 24,
        }];

        write_pkl(&pkl_path, "pkl", "cpl", &cpl_path, &tracks).unwrap();
        let xml = std::fs::read_to_string(pkl_path).unwrap();
        assert!(xml.contains("http://www.smpte-ra.org/schemas/2067-2/2016/PKL"));
        assert!(xml.contains("<IssueDate>"));
        assert!(xml.contains("<Id>urn:uuid:track</Id>"));
        assert!(xml.contains("<Hash>base64-hash</Hash>"));
        assert!(xml.contains("<Size>42</Size>"));
    }
}
