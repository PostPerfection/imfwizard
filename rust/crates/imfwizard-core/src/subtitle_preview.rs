//! The subtitle file the embedded preview hands mpv.
//!
//! libass reads SRT, ASS/SSA and WebVTT and nothing else, so the timed text an
//! IMP packages is written out as SRT before the preview can show it. Times
//! stay source-relative, which is what the preview plays.

use std::path::{Path, PathBuf};

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
        assert_eq!(
            std::fs::read_to_string(&output).unwrap(),
            "1\n00:00:01,500 --> 00:00:03,000\nFirst line\nsecond line\n\n\
             2\n00:00:04,000 --> 00:00:06,250\nA later cue\n\n"
        );
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
