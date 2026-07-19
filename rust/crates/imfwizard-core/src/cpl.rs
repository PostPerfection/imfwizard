use std::io::Write;
use std::path::Path;

use crate::MxfTrackFile;
use crate::imp::ImpOptions;

/// Write a CPL (Composition Playlist) XML file.
pub fn write_cpl(
    path: &Path,
    cpl_uuid: &str,
    opts: &ImpOptions,
    track_files: &[MxfTrackFile],
) -> std::io::Result<()> {
    let mut f = std::fs::File::create(path)?;
    let fps_num = opts.fps_num;
    let fps_den = if opts.fps_den == 0 { 1 } else { opts.fps_den };

    writeln!(f, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
    writeln!(
        f,
        r#"<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016" xmlns:cc="http://www.smpte-ra.org/schemas/2067-2/2016">"#
    )?;
    writeln!(f, "  <Id>urn:uuid:{cpl_uuid}</Id>")?;
    writeln!(f, "  <IssueDate>{}</IssueDate>", crate::issue_date())?;
    writeln!(f, "  <Issuer>IMF Wizard</Issuer>")?;
    writeln!(f, "  <Creator>IMF Wizard</Creator>")?;
    writeln!(
        f,
        "  <ContentTitle>{}</ContentTitle>",
        xml_escape(&opts.title)
    )?;
    writeln!(
        f,
        "  <ContentKind>{}</ContentKind>",
        if opts.content_kind.is_empty() {
            "feature"
        } else {
            &opts.content_kind
        }
    )?;
    writeln!(f, "  <EditRate>{fps_num} {fps_den}</EditRate>")?;
    writeln!(f, "  <ExtensionProperties>")?;
    writeln!(
        f,
        "    <cc:ApplicationIdentification>http://www.smpte-ra.org/schemas/2067-21/2016</cc:ApplicationIdentification>"
    )?;
    writeln!(f, "  </ExtensionProperties>")?;
    writeln!(f, "  <SegmentList>")?;
    writeln!(f, "    <Segment>")?;
    writeln!(f, "      <Id>urn:uuid:{}</Id>", uuid::Uuid::new_v4())?;
    writeln!(f, "      <SequenceList>")?;

    // Video sequences
    for tf in track_files {
        let fname = tf.path.file_name().and_then(|f| f.to_str()).unwrap_or("");
        if fname.starts_with("VIDEO_") {
            writeln!(
                f,
                "        <cc:MainImageSequence xmlns:cc=\"http://www.smpte-ra.org/schemas/2067-2/2016\">"
            )?;
            writeln!(f, "          <Id>urn:uuid:{}</Id>", uuid::Uuid::new_v4())?;
            writeln!(
                f,
                "          <TrackId>urn:uuid:{}</TrackId>",
                uuid::Uuid::new_v4()
            )?;
            writeln!(f, "          <EditRate>{fps_num} {fps_den}</EditRate>")?;
            writeln!(f, "          <ResourceList>")?;
            writeln!(f, "            <Resource>")?;
            writeln!(
                f,
                "              <Id>urn:uuid:{}</Id>",
                uuid::Uuid::new_v4()
            )?;
            writeln!(
                f,
                "              <TrackFileId>urn:uuid:{}</TrackFileId>",
                tf.uuid
            )?;
            writeln!(f, "              <EditRate>{fps_num} {fps_den}</EditRate>")?;
            writeln!(
                f,
                "              <IntrinsicDuration>{}</IntrinsicDuration>",
                tf.duration
            )?;
            writeln!(
                f,
                "              <SourceDuration>{}</SourceDuration>",
                tf.duration
            )?;
            writeln!(f, "            </Resource>")?;
            writeln!(f, "          </ResourceList>")?;
            writeln!(f, "        </cc:MainImageSequence>")?;
        }
    }

    // Audio sequences
    for tf in track_files {
        let fname = tf.path.file_name().and_then(|f| f.to_str()).unwrap_or("");
        if fname.starts_with("AUDIO_") {
            writeln!(
                f,
                "        <cc:MainAudioSequence xmlns:cc=\"http://www.smpte-ra.org/schemas/2067-2/2016\">"
            )?;
            writeln!(f, "          <Id>urn:uuid:{}</Id>", uuid::Uuid::new_v4())?;
            writeln!(
                f,
                "          <TrackId>urn:uuid:{}</TrackId>",
                uuid::Uuid::new_v4()
            )?;
            writeln!(f, "          <EditRate>{fps_num} {fps_den}</EditRate>")?;
            writeln!(f, "          <ResourceList>")?;
            writeln!(f, "            <Resource>")?;
            writeln!(
                f,
                "              <Id>urn:uuid:{}</Id>",
                uuid::Uuid::new_v4()
            )?;
            writeln!(
                f,
                "              <TrackFileId>urn:uuid:{}</TrackFileId>",
                tf.uuid
            )?;
            writeln!(f, "              <EditRate>{fps_num} {fps_den}</EditRate>")?;
            writeln!(
                f,
                "              <IntrinsicDuration>{}</IntrinsicDuration>",
                tf.duration
            )?;
            writeln!(
                f,
                "              <SourceDuration>{}</SourceDuration>",
                tf.duration
            )?;
            writeln!(f, "            </Resource>")?;
            writeln!(f, "          </ResourceList>")?;
            writeln!(f, "        </cc:MainAudioSequence>")?;
        }
    }

    writeln!(f, "      </SequenceList>")?;
    writeln!(f, "    </Segment>")?;
    writeln!(f, "  </SegmentList>")?;
    writeln!(f, "</CompositionPlaylist>")?;
    Ok(())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_cpl_identifies_app_2e() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CPL_test.xml");
        let opts = ImpOptions {
            title: "Test".into(),
            ..ImpOptions::default()
        };

        write_cpl(&path, "cpl", &opts, &[]).unwrap();
        let xml = std::fs::read_to_string(path).unwrap();
        assert!(xml.contains("<IssueDate>"));
        assert!(xml.contains("<cc:ApplicationIdentification>http://www.smpte-ra.org/schemas/2067-21/2016</cc:ApplicationIdentification>"));
    }
}
