use std::io::Write;
use std::path::Path;

use crate::MxfTrackFile;

/// Write an ASSETMAP.xml file.
pub fn write_assetmap(
    path: &Path,
    pkl_uuid: &str,
    cpl_uuid: &str,
    track_files: &[MxfTrackFile],
) -> std::io::Result<()> {
    let am_uuid = uuid::Uuid::new_v4();
    let mut f = std::fs::File::create(path)?;
    writeln!(f, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
    writeln!(
        f,
        r#"<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">"#
    )?;
    writeln!(f, "  <Id>urn:uuid:{am_uuid}</Id>")?;
    writeln!(f, "  <Creator>IMF Wizard</Creator>")?;
    writeln!(f, "  <AssetList>")?;

    // PKL reference
    writeln!(f, "    <Asset>")?;
    writeln!(f, "      <Id>urn:uuid:{pkl_uuid}</Id>")?;
    writeln!(f, "      <PackingList>true</PackingList>")?;
    writeln!(f, "      <ChunkList>")?;
    writeln!(f, "        <Chunk>")?;
    writeln!(f, "          <Path>PKL_{pkl_uuid}.xml</Path>")?;
    writeln!(f, "        </Chunk>")?;
    writeln!(f, "      </ChunkList>")?;
    writeln!(f, "    </Asset>")?;

    // CPL reference
    writeln!(f, "    <Asset>")?;
    writeln!(f, "      <Id>urn:uuid:{cpl_uuid}</Id>")?;
    writeln!(f, "      <ChunkList>")?;
    writeln!(f, "        <Chunk>")?;
    writeln!(f, "          <Path>CPL_{cpl_uuid}.xml</Path>")?;
    writeln!(f, "        </Chunk>")?;
    writeln!(f, "      </ChunkList>")?;
    writeln!(f, "    </Asset>")?;

    for track_file in track_files {
        let file_name = track_file
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        writeln!(f, "    <Asset>")?;
        writeln!(f, "      <Id>urn:uuid:{}</Id>", track_file.uuid)?;
        writeln!(f, "      <ChunkList>")?;
        writeln!(f, "        <Chunk>")?;
        writeln!(f, "          <Path>{file_name}</Path>")?;
        writeln!(f, "        </Chunk>")?;
        writeln!(f, "      </ChunkList>")?;
        writeln!(f, "    </Asset>")?;
    }

    writeln!(f, "  </AssetList>")?;
    writeln!(f, "</AssetMap>")?;
    Ok(())
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
