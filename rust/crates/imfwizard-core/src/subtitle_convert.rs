use postkit::subtitle_retime::{SrtCue, parse_srt};
use serde::{Deserialize, Serialize};

/// Subtitle format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubtitleFormat {
    Srt,
    Vtt,
    Stl,
    Scc,
    Ttml,
    ImscTtml,
}

impl SubtitleFormat {
    pub fn extension(&self) -> &str {
        match self {
            Self::Srt => "srt",
            Self::Vtt => "vtt",
            Self::Stl => "stl",
            Self::Scc => "scc",
            Self::Ttml | Self::ImscTtml => "ttml",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "srt" => Some(Self::Srt),
            "vtt" | "webvtt" => Some(Self::Vtt),
            "stl" => Some(Self::Stl),
            "scc" => Some(Self::Scc),
            "ttml" | "xml" => Some(Self::Ttml),
            _ => None,
        }
    }
}

/// Convert subtitles between formats.
pub fn convert_subtitles(
    input: &std::path::Path,
    output: &std::path::Path,
    _target_format: SubtitleFormat,
) -> Result<(), String> {
    // Parse input
    let content =
        std::fs::read_to_string(input).map_err(|e| format!("Failed to read input: {e}"))?;

    let ext = input.extension().and_then(|e| e.to_str()).unwrap_or("");
    let source_format = SubtitleFormat::from_extension(ext)
        .ok_or_else(|| format!("Unknown subtitle format: {ext}"))?;

    let cues = match source_format {
        SubtitleFormat::Scc => crate::scc::parse_scc(&content)?,
        _ => parse_srt(&content),
    };

    // Write output as TTML (IMF standard)
    write_ttml(&cues, output)
}

fn write_ttml(cues: &[SrtCue], output: &std::path::Path) -> Result<(), String> {
    use std::io::Write;
    let mut f =
        std::fs::File::create(output).map_err(|e| format!("Failed to create output: {e}"))?;

    writeln!(f, r#"<?xml version="1.0" encoding="UTF-8"?>"#).map_err(|e| e.to_string())?;
    writeln!(
        f,
        r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttp="http://www.w3.org/ns/ttml#parameter">"#
    )
    .map_err(|e| e.to_string())?;
    writeln!(f, "  <body>").map_err(|e| e.to_string())?;
    writeln!(f, "    <div>").map_err(|e| e.to_string())?;

    for cue in cues {
        let start = format_ttml_time(cue.start_ms);
        let end = format_ttml_time(cue.end_ms);
        let escaped = cue
            .text
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        writeln!(f, r#"      <p begin="{start}" end="{end}">{escaped}</p>"#)
            .map_err(|e| e.to_string())?;
    }

    writeln!(f, "    </div>").map_err(|e| e.to_string())?;
    writeln!(f, "  </body>").map_err(|e| e.to_string())?;
    writeln!(f, "</tt>").map_err(|e| e.to_string())?;
    Ok(())
}

fn format_ttml_time(ms: u64) -> String {
    let h = ms / 3_600_000;
    let m = (ms % 3_600_000) / 60_000;
    let s = (ms % 60_000) / 1000;
    let f = ms % 1000;
    format!("{h:02}:{m:02}:{s:02}.{f:03}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_ttml_time() {
        assert_eq!(format_ttml_time(3_723_456), "01:02:03.456");
    }

    #[test]
    fn test_convert_srt_to_ttml() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.srt");
        let output = dir.path().join("out.ttml");
        std::fs::write(
            &input,
            "1\n00:00:01,000 --> 00:00:04,000\nHello world\n\n2\n00:00:05,000 --> 00:00:08,000\nSecond cue\n",
        )
        .unwrap();
        convert_subtitles(&input, &output, SubtitleFormat::ImscTtml).unwrap();
        let ttml = std::fs::read_to_string(output).unwrap();
        assert!(ttml.contains(r#"<p begin="00:00:01.000" end="00:00:04.000">Hello world</p>"#));
        assert!(ttml.contains("Second cue"));
    }

    #[test]
    fn test_convert_scc_to_ttml() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.scc");
        let output = dir.path().join("out.ttml");
        std::fs::write(
            &input,
            "Scenarist_SCC V1.0\n\n\
00:00:01:00\t9420 9420 942e 942e 9470 9470 4845 4c4c 4f80 942f 942f\n\n\
00:00:03:00\t942c 942c\n",
        )
        .unwrap();
        convert_subtitles(&input, &output, SubtitleFormat::ImscTtml).unwrap();
        let ttml = std::fs::read_to_string(output).unwrap();
        assert!(ttml.contains("HELLO"), "ttml: {ttml}");
        assert!(ttml.contains(r#"begin="00:00:01.001""#), "ttml: {ttml}");
    }
}
