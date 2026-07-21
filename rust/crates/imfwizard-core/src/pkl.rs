use std::path::Path;

use postkit::packaging::{PackingList, PklAsset, ns};

use crate::MxfTrackFile;
use crate::imp::CplEntry;

/// Write an IMF PKL (ST 2067-2) covering one or more CPLs plus track files.
pub fn write_pkl(
    path: &Path,
    pkl_uuid: &str,
    cpls: &[CplEntry],
    track_files: &[MxfTrackFile],
) -> std::io::Result<()> {
    let mut assets = Vec::new();
    for cpl in cpls {
        let hash = postkit::hash::hash_file(&cpl.path, postkit::hash::HashAlgorithm::Sha1)
            .map(|h| h.base64)
            .unwrap_or_default();
        let size = std::fs::metadata(&cpl.path).map(|m| m.len()).unwrap_or(0);
        assets.push(PklAsset {
            id: cpl.uuid.clone(),
            hash,
            size,
            asset_type: "text/xml".to_string(),
        });
    }
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
        let cpls = [CplEntry {
            uuid: "cpl".into(),
            path: cpl_path,
        }];
        let tracks = [MxfTrackFile {
            path: dir.path().join("VIDEO_track.mxf"),
            uuid: "track".into(),
            hash: "base64-hash".into(),
            size: 42,
            duration: 24,
        }];

        write_pkl(&pkl_path, "pkl", &cpls, &tracks).unwrap();
        let xml = std::fs::read_to_string(pkl_path).unwrap();
        assert!(xml.contains("http://www.smpte-ra.org/schemas/2067-2/2016/PKL"));
        assert!(xml.contains("<IssueDate>"));
        assert!(xml.contains("<Id>urn:uuid:track</Id>"));
        assert!(xml.contains("<Hash>base64-hash</Hash>"));
        assert!(xml.contains("<Size>42</Size>"));
    }

    #[test]
    fn write_pkl_covers_multiple_cpls() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("CPL_a.xml");
        let b = dir.path().join("CPL_b.xml");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let pkl_path = dir.path().join("PKL_pkl.xml");
        let cpls = [
            CplEntry {
                uuid: "aaa".into(),
                path: a,
            },
            CplEntry {
                uuid: "bbb".into(),
                path: b,
            },
        ];

        write_pkl(&pkl_path, "pkl", &cpls, &[]).unwrap();
        let xml = std::fs::read_to_string(pkl_path).unwrap();
        assert!(xml.contains("<Id>urn:uuid:aaa</Id>"));
        assert!(xml.contains("<Id>urn:uuid:bbb</Id>"));
    }
}
