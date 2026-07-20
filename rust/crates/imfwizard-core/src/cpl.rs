use std::path::Path;

use postkit::packaging::{ImfCpl, ImfResource, ImfTrackKind};

use crate::MxfTrackFile;
use crate::imp::ImpOptions;

/// Write an IMF CPL (ST 2067-3) using the shared postkit writer.
///
/// Track files are classified by their filename prefix (VIDEO_/AUDIO_/SUBTITLE_),
/// the same convention the MXF wrapper writes.
pub fn write_cpl(
    path: &Path,
    cpl_uuid: &str,
    opts: &ImpOptions,
    track_files: &[MxfTrackFile],
) -> std::io::Result<()> {
    let resources = track_files
        .iter()
        .filter_map(|tf| {
            let fname = tf.path.file_name().and_then(|f| f.to_str()).unwrap_or("");
            let kind = if fname.starts_with("VIDEO_") {
                ImfTrackKind::Image
            } else if fname.starts_with("AUDIO_") {
                ImfTrackKind::Audio
            } else if fname.starts_with("SUBTITLE_") {
                ImfTrackKind::Subtitle
            } else {
                return None;
            };
            Some(ImfResource {
                track_file_uuid: tf.uuid.clone(),
                duration: tf.duration,
                kind,
            })
        })
        .collect();

    let cpl = ImfCpl {
        uuid: cpl_uuid.to_string(),
        title: opts.title.clone(),
        content_kind: opts.content_kind.clone(),
        issuer: "IMF Wizard".to_string(),
        creator: "IMF Wizard".to_string(),
        issue_date: crate::issue_date(),
        fps_num: opts.fps_num,
        fps_den: opts.fps_den,
        resources,
    };
    std::fs::write(path, cpl.to_xml())
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
