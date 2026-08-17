//! The subtitle file the embedded preview hands mpv.
//!
//! libass reads SRT, ASS/SSA and WebVTT and nothing else, so the timed text an
//! IMP packages is written out as SRT before the preview can show it. Times
//! stay source-relative, which is what the preview plays.

use std::path::{Path, PathBuf};

use postkit::packaging::ImfTrackKind;

use crate::source_edits::{TimedTextCue, read_timed_text_cues};

/// What libass reads as it stands, so the preview opens the file the job
/// packages rather than a copy of it.
const PLAYABLE_EXTENSIONS: [&str; 5] = ["srt", "ass", "ssa", "vtt", "webvtt"];

/// What `read_timed_text_cues` reads, which is TTML and the IMSC profile of it,
/// spelled the three ways the GUI takes a subtitle asset.
const TIMED_TEXT_EXTENSIONS: [&str; 3] = ["ttml", "xml", "imsc"];

const MILLISECONDS_PER_SECOND: u64 = 1000;
const SECONDS_PER_MINUTE: u64 = 60;
const MINUTES_PER_HOUR: u64 = 60;

/// A subtitle file the preview player can render, writing the packaged timed
/// text out as SRT under `work_dir` when mpv cannot read it as it stands.
///
/// `fps` reads the timed text's frame-form times; clock times need none.
pub fn playable_subtitle_file(input: &Path, fps: f64, work_dir: &Path) -> Result<PathBuf, String> {
    let extension = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if PLAYABLE_EXTENSIONS.contains(&extension.as_str()) {
        return Ok(input.to_path_buf());
    }
    if !TIMED_TEXT_EXTENSIONS.contains(&extension.as_str()) {
        return Err(format!(
            "{} is none of the subtitle formats the preview shows: {} or TTML/IMSC",
            input.display(),
            PLAYABLE_EXTENSIONS.join(", ")
        ));
    }

    let cues = read_timed_text_cues(input, fps)?;
    if cues.is_empty() {
        return Err(format!("no cues in {}", input.display()));
    }
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("{} has no readable file name", input.display()))?;
    std::fs::create_dir_all(work_dir)
        .map_err(|e| format!("cannot create {}: {e}", work_dir.display()))?;
    let output = work_dir.join(format!("{stem}.srt"));
    std::fs::write(&output, srt_document(&cues))
        .map_err(|e| format!("cannot write {}: {e}", output.display()))?;
    Ok(output)
}

/// A subtitle file the preview player can render for a built IMP, unwrapping the
/// timed text of its first CPL out of the AS-02 track file and writing it out as
/// SRT under `work_dir`. `Ok(None)` when the package carries no timed text.
///
/// The cue times are the document's own, which is what the package plays at.
pub fn packaged_subtitle_file(imp_dir: &Path, work_dir: &Path) -> Result<Option<PathBuf>, String> {
    let cpl = crate::timeline::list_cpls(imp_dir)
        .into_iter()
        .next()
        .ok_or_else(|| format!("no CPL in {}", imp_dir.display()))?;
    let composition = crate::supplement::parse_cpl_resources(&imp_dir.join(&cpl.file_path))?;
    let Some(subtitles) = composition
        .resources
        .iter()
        .find(|resource| resource.kind == ImfTrackKind::Subtitle)
    else {
        return Ok(None);
    };
    let relative_path = crate::timeline::parse_assetmap(imp_dir)
        .remove(&subtitles.uuid)
        .ok_or_else(|| {
            format!(
                "{} lists no asset for the timed text track {}",
                imp_dir.display(),
                subtitles.uuid
            )
        })?;
    let track_file = imp_dir.join(relative_path);

    let stem = track_file
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("{} has no readable file name", track_file.display()))?;
    std::fs::create_dir_all(work_dir)
        .map_err(|e| format!("cannot create {}: {e}", work_dir.display()))?;
    let document_path = work_dir.join(format!("{stem}.ttml"));
    std::fs::write(&document_path, timed_text_document(&track_file)?)
        .map_err(|e| format!("cannot write {}: {e}", document_path.display()))?;

    let fps = f64::from(composition.fps_num) / f64::from(composition.fps_den);
    playable_subtitle_file(&document_path, fps, work_dir).map(Some)
}

/// The timed text document inside an AS-02 track file. The ancillary resources a
/// subtitle can carry, its fonts and PNGs, are left alone: the preview renders
/// the text on its own.
fn timed_text_document(track_file: &Path) -> Result<Vec<u8>, String> {
    let path = track_file.to_string_lossy().into_owned();
    let mut reader = asdcplib::as02::timed_text::MxfReader::new();
    reader
        .open_read(&path)
        .map_err(|e| format!("cannot open the timed text track {path}: {e}"))?;
    // the document cannot be bigger than the MXF wrapping it, so one read is enough
    let wrapped_bytes = std::fs::metadata(track_file)
        .map_err(|e| format!("cannot read {path}: {e}"))?
        .len() as usize;
    let mut document = vec![0u8; wrapped_bytes];
    let read = reader
        .read_timed_text_resource(&mut document, None, None)
        .map_err(|e| format!("cannot read the timed text of {path}: {e}"))?;
    document.truncate(read);
    Ok(document)
}

/// The cues as a SubRip document, numbered from one.
fn srt_document(cues: &[TimedTextCue]) -> String {
    let mut document = String::new();
    for (index, cue) in cues.iter().enumerate() {
        document.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            index + 1,
            format_srt_time(cue.start_ms),
            format_srt_time(cue.end_ms),
            cue.lines.join("\n")
        ));
    }
    document
}

/// SubRip's own timestamp, `HH:MM:SS,mmm`.
fn format_srt_time(milliseconds: u64) -> String {
    let seconds = milliseconds / MILLISECONDS_PER_SECOND;
    format!(
        "{:02}:{:02}:{:02},{:03}",
        seconds / (SECONDS_PER_MINUTE * MINUTES_PER_HOUR),
        (seconds / SECONDS_PER_MINUTE) % MINUTES_PER_HOUR,
        seconds % SECONDS_PER_MINUTE,
        milliseconds % MILLISECONDS_PER_SECOND
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const FPS: f64 = 24.0;

    const EXPECTED_SRT: &str = "1\n00:00:01,500 --> 00:00:03,000\nFirst line\nsecond line\n\n\
         2\n00:00:04,000 --> 00:00:06,250\nA later cue\n\n";

    fn timed_text(dir: &Path) -> PathBuf {
        let path = dir.join("subs.ttml");
        std::fs::write(
            &path,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xmlns="http://www.w3.org/ns/ttml">
  <body>
    <div>
      <p begin="00:00:01.500" end="00:00:03.000">First line<br/>second line</p>
      <p begin="00:00:04.000" end="00:00:06.250">A later cue</p>
    </div>
  </body>
</tt>"#,
        )
        .unwrap();
        path
    }

    #[test]
    fn timed_text_becomes_srt_the_preview_can_show() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("preview-subtitles");
        let output = playable_subtitle_file(&timed_text(dir.path()), FPS, &work).unwrap();

        assert_eq!(output, work.join("subs.srt"));
        assert_eq!(std::fs::read_to_string(&output).unwrap(), EXPECTED_SRT);
    }

    /// Build a one-frame IMP into `dir`, with the timed text as its subtitle
    /// track when one is given, and return the package directory.
    fn built_package(dir: &Path, subtitle: Option<PathBuf>) -> PathBuf {
        let j2k_dir = dir.join("j2k");
        std::fs::create_dir_all(&j2k_dir).unwrap();
        std::fs::write(
            j2k_dir.join("0001.j2c"),
            crate::mxf_wrap::synthetic_j2k_codestream(2048, 1080, 12),
        )
        .unwrap();
        let output_dir = dir.join("imp");
        let result = crate::imp::create_imp(&crate::imp::ImpOptions {
            output_dir: output_dir.clone(),
            compositions: vec![crate::imp::Composition {
                title: "Subtitled".into(),
                content_kind: "feature".into(),
                j2k_dir: Some(j2k_dir),
                timed_text_files: subtitle.into_iter().collect(),
                ..Default::default()
            }],
            fps_num: 24,
            fps_den: 1,
            ..Default::default()
        });
        assert!(result.success, "create failed: {}", result.error);
        output_dir
    }

    #[test]
    fn a_built_package_gives_back_the_cues_it_wrapped() {
        let dir = tempfile::tempdir().unwrap();
        let package = built_package(dir.path(), Some(timed_text(dir.path())));
        let work = dir.path().join("preview-subtitles");

        let output = packaged_subtitle_file(&package, &work)
            .unwrap()
            .expect("a timed text track");

        assert_eq!(output.parent(), Some(work.as_path()));
        assert_eq!(std::fs::read_to_string(&output).unwrap(), EXPECTED_SRT);
    }

    #[test]
    fn a_package_with_no_timed_text_track_has_no_subtitle_file() {
        let dir = tempfile::tempdir().unwrap();
        let package = built_package(dir.path(), None);

        let output = packaged_subtitle_file(&package, &dir.path().join("work")).unwrap();

        assert_eq!(output, None);
    }

    #[test]
    fn a_directory_that_is_no_package_says_so() {
        let dir = tempfile::tempdir().unwrap();

        let error = packaged_subtitle_file(dir.path(), &dir.path().join("work"))
            .expect_err("no CPL to read");

        assert!(error.contains("no CPL"), "got: {error}");
    }

    #[test]
    fn a_file_mpv_reads_itself_is_handed_over_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let srt = dir.path().join("cues.SRT");
        std::fs::write(&srt, "1\n00:00:01,000 --> 00:00:02,000\nhi\n").unwrap();

        let output = playable_subtitle_file(&srt, FPS, &dir.path().join("work")).unwrap();

        assert_eq!(output, srt);
    }

    #[test]
    fn a_format_with_no_cue_reader_names_what_the_preview_shows() {
        let dir = tempfile::tempdir().unwrap();
        let scc = dir.path().join("captions.scc");
        std::fs::write(&scc, "Scenarist_SCC V1.0\n").unwrap();

        let error =
            playable_subtitle_file(&scc, FPS, &dir.path().join("work")).expect_err("no reader");

        assert!(error.contains("TTML/IMSC"), "got: {error}");
    }
}
