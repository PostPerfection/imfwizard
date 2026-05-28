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
        r#"<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016">"#
    )?;
    writeln!(f, "  <Id>urn:uuid:{cpl_uuid}</Id>")?;
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
