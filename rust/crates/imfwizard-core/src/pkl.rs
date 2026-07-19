use std::io::Write;
use std::path::Path;

use crate::MxfTrackFile;

/// Write a PKL (Packing List) XML file.
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

    let mut f = std::fs::File::create(path)?;
    writeln!(f, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
    writeln!(
        f,
        r#"<PackingList xmlns="http://www.smpte-ra.org/schemas/2067-2/2016/PKL">"#
    )?;
    writeln!(f, "  <Id>urn:uuid:{pkl_uuid}</Id>")?;
    writeln!(f, "  <IssueDate>{}</IssueDate>", crate::issue_date())?;
    writeln!(f, "  <Issuer>IMF Wizard</Issuer>")?;
    writeln!(f, "  <Creator>IMF Wizard</Creator>")?;
    writeln!(f, "  <AssetList>")?;
    writeln!(f, "    <Asset>")?;
    writeln!(f, "      <Id>urn:uuid:{cpl_uuid}</Id>")?;
    writeln!(f, "      <Hash>{cpl_hash}</Hash>")?;
    writeln!(f, "      <Size>{cpl_size}</Size>")?;
    writeln!(f, "      <Type>text/xml</Type>")?;
    writeln!(f, "    </Asset>")?;
    for track_file in track_files {
        writeln!(f, "    <Asset>")?;
        writeln!(f, "      <Id>urn:uuid:{}</Id>", track_file.uuid)?;
        writeln!(f, "      <Hash>{}</Hash>", track_file.hash)?;
        writeln!(f, "      <Size>{}</Size>", track_file.size)?;
        writeln!(f, "      <Type>application/mxf</Type>")?;
        writeln!(f, "    </Asset>")?;
    }
    writeln!(f, "  </AssetList>")?;
    writeln!(f, "</PackingList>")?;
    Ok(())
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
