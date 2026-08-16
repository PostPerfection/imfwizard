use postkit::packaging::escape_xml;
use postkit::subtitle_formats::{HAlign, Rgba, StyledCue, StyledRun, VAlign, to_srt_cues};
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
    Ass,
    Fcpxml,
    Mks,
}

impl SubtitleFormat {
    pub fn extension(&self) -> &str {
        match self {
            Self::Srt => "srt",
            Self::Vtt => "vtt",
            Self::Stl => "stl",
            Self::Scc => "scc",
            Self::Ttml | Self::ImscTtml => "ttml",
            Self::Ass => "ass",
            Self::Fcpxml => "fcpxml",
            Self::Mks => "mks",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "srt" => Some(Self::Srt),
            "vtt" | "webvtt" => Some(Self::Vtt),
            "stl" => Some(Self::Stl),
            "scc" => Some(Self::Scc),
            "ttml" | "xml" => Some(Self::Ttml),
            "ass" | "ssa" => Some(Self::Ass),
            "fcpxml" => Some(Self::Fcpxml),
            "mks" | "mkv" => Some(Self::Mks),
            _ => None,
        }
    }
}

/// The `xml:id` of the style every `<p>` the writers emit points at, so a cue
/// with no styling of its own lands at the named look and a run's own
/// `tts:color` still wins.
const DEFAULT_STYLE_ID: &str = "default";

/// Rows and columns the cell grid is divided into, written out because the
/// default text size is a cell-relative length and a cell is a row of this grid.
/// These are TTML's own defaults, so declaring them changes nothing else.
const CELL_ROWS: u32 = 15;
const CELL_COLUMNS: u32 = 32;

/// A whole, as the size flag spells its fraction.
const PERCENT_OF_A_WHOLE: f32 = 100.0;

/// How the written TTML looks where neither the source nor a run says for
/// itself. Each field stays `None` until the caller names it, and a TTML with
/// nothing named carries no styling of its own.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TextAppearance {
    /// Text height as a percent of the frame height.
    pub font_size_percent: Option<f32>,
    pub colour: Option<Rgba>,
}

impl TextAppearance {
    /// Refuse a size the TTML could not carry. The range is the rasteriser's,
    /// which measures the same percent of the frame height.
    pub fn check(&self) -> Result<(), String> {
        match self.font_size_percent {
            Some(percent) => postkit::subtitle_raster::check_font_size_percent(percent),
            None => Ok(()),
        }
    }

    /// The `tts:` attributes for the default style, or None when the caller
    /// named nothing.
    fn style_attributes(&self) -> Option<String> {
        let mut attributes: Vec<String> = Vec::new();
        if let Some(percent) = self.font_size_percent {
            // a bare percent is read against the parent element's size, not the
            // frame, so the height goes out in cells, which are frame-relative
            let cells = percent / PERCENT_OF_A_WHOLE * CELL_ROWS as f32;
            attributes.push(format!(r#"tts:fontSize="{cells:.3}c""#));
        }
        if let Some(colour) = self.colour {
            attributes.push(format!(
                r##"tts:color="#{:02X}{:02X}{:02X}{:02X}""##,
                colour.r, colour.g, colour.b, colour.a
            ));
        }
        (!attributes.is_empty()).then(|| attributes.join(" "))
    }

    /// The cell grid declaration for the `<tt>` element, needed only where a
    /// cell-relative size is written against it.
    fn cell_resolution(&self) -> String {
        match self.font_size_percent {
            Some(_) => format!(r#" ttp:cellResolution="{CELL_COLUMNS} {CELL_ROWS}""#),
            None => String::new(),
        }
    }
}

/// Convert subtitles between formats.
///
/// TTML/IMSC targets keep the styling and placement the source supplies (ASS,
/// FCPXML, MKS carry it via `StyledCue`); plain timed-text targets flatten to
/// text-only. Authored TTML passes through unchanged, and SCC keeps its
/// existing plain-cue path. `appearance` is written as the document's default
/// style, which anything the source styled for itself overrides.
pub fn convert_subtitles(
    input: &std::path::Path,
    output: &std::path::Path,
    target_format: SubtitleFormat,
    appearance: &TextAppearance,
) -> Result<(), String> {
    appearance.check()?;
    let ext = input.extension().and_then(|e| e.to_str()).unwrap_or("");
    let source_format = SubtitleFormat::from_extension(ext)
        .ok_or_else(|| format!("Unknown subtitle format: {ext}"))?;

    if source_format == SubtitleFormat::Ttml
        && matches!(
            target_format,
            SubtitleFormat::Ttml | SubtitleFormat::ImscTtml
        )
    {
        if appearance.style_attributes().is_some() {
            return Err(
                "authored TTML is copied unchanged, so a default size or colour asked for here \
                 would be dropped: edit the TTML's own styling instead"
                    .into(),
            );
        }
        std::fs::copy(input, output).map_err(|e| format!("Failed to copy TTML: {e}"))?;
        return Ok(());
    }

    let target_styled = matches!(
        target_format,
        SubtitleFormat::Ttml | SubtitleFormat::ImscTtml
    );

    // styled source formats
    match source_format {
        SubtitleFormat::Ass => {
            let content =
                std::fs::read_to_string(input).map_err(|e| format!("Failed to read input: {e}"))?;
            let parsed = postkit::subtitle_formats::ass::parse_ass(&content)
                .map_err(|e| format!("ASS parse: {e}"))?;
            for w in &parsed.warnings {
                eprintln!("warning: unsupported ASS override tag {w}");
            }
            return write_styled_or_flat(&parsed.cues, output, target_styled, appearance);
        }
        SubtitleFormat::Fcpxml => {
            let content =
                std::fs::read_to_string(input).map_err(|e| format!("Failed to read input: {e}"))?;
            let cues = postkit::subtitle_formats::fcpxml::parse_fcpxml(&content)
                .map_err(|e| format!("FCPXML parse: {e}"))?;
            return write_styled_or_flat(&cues, output, target_styled, appearance);
        }
        SubtitleFormat::Mks => {
            let cues = postkit::subtitle_formats::mks::parse_mks(input, None)
                .map_err(|e| format!("MKS parse: {e}"))?;
            return write_styled_or_flat(&cues, output, target_styled, appearance);
        }
        _ => {}
    }

    // plain source formats
    let content =
        std::fs::read_to_string(input).map_err(|e| format!("Failed to read input: {e}"))?;
    let cues = match source_format {
        SubtitleFormat::Scc => crate::scc::parse_scc(&content)?,
        _ => parse_srt(&content),
    };

    // Write output as TTML (IMF standard)
    write_ttml(&cues, output, appearance)
}

fn write_styled_or_flat(
    cues: &[StyledCue],
    output: &std::path::Path,
    target_styled: bool,
    appearance: &TextAppearance,
) -> Result<(), String> {
    if target_styled {
        write_ttml_styled(cues, output, appearance)
    } else {
        write_ttml(&to_srt_cues(cues), output, appearance)
    }
}

/// The `<styling>` block carrying the document's default style.
fn write_styling(f: &mut std::fs::File, attributes: &str) -> Result<(), String> {
    use std::io::Write;
    writeln!(f, "    <styling>").map_err(|e| e.to_string())?;
    writeln!(
        f,
        r#"      <style xml:id="{DEFAULT_STYLE_ID}" {attributes}/>"#
    )
    .map_err(|e| e.to_string())?;
    writeln!(f, "    </styling>").map_err(|e| e.to_string())
}

/// The `style` attribute a `<p>` carries when there is a default style, spelled
/// with its leading space so it drops out cleanly when there is none.
fn paragraph_style(style: Option<&String>) -> String {
    match style {
        Some(_) => format!(r#" style="{DEFAULT_STYLE_ID}""#),
        None => String::new(),
    }
}

fn write_ttml(
    cues: &[SrtCue],
    output: &std::path::Path,
    appearance: &TextAppearance,
) -> Result<(), String> {
    use std::io::Write;
    let mut f =
        std::fs::File::create(output).map_err(|e| format!("Failed to create output: {e}"))?;
    let style = appearance.style_attributes();

    writeln!(f, r#"<?xml version="1.0" encoding="UTF-8"?>"#).map_err(|e| e.to_string())?;
    match &style {
        Some(_) => writeln!(
            f,
            r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:tts="http://www.w3.org/ns/ttml#styling" xmlns:ttp="http://www.w3.org/ns/ttml#parameter"{}>"#,
            appearance.cell_resolution()
        ),
        None => writeln!(
            f,
            r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttp="http://www.w3.org/ns/ttml#parameter">"#
        ),
    }
    .map_err(|e| e.to_string())?;
    if let Some(attributes) = &style {
        writeln!(f, "  <head>").map_err(|e| e.to_string())?;
        write_styling(&mut f, attributes)?;
        writeln!(f, "  </head>").map_err(|e| e.to_string())?;
    }
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
        writeln!(
            f,
            r#"      <p begin="{start}" end="{end}"{}>{escaped}</p>"#,
            paragraph_style(style.as_ref())
        )
        .map_err(|e| e.to_string())?;
    }

    writeln!(f, "    </div>").map_err(|e| e.to_string())?;
    writeln!(f, "  </body>").map_err(|e| e.to_string())?;
    writeln!(f, "</tt>").map_err(|e| e.to_string())?;
    Ok(())
}

/// Write styled cues as IMSC-flavoured TTML, keeping per-run styling and one
/// `<region>` per distinct placement.
fn write_ttml_styled(
    cues: &[StyledCue],
    output: &std::path::Path,
    appearance: &TextAppearance,
) -> Result<(), String> {
    use std::io::Write;

    // dedup regions by their attribute string, in first-seen order.
    let mut regions: Vec<String> = Vec::new();
    let cue_region: Vec<usize> = cues
        .iter()
        .map(|c| {
            let attrs = region_attrs(c);
            match regions.iter().position(|a| a == &attrs) {
                Some(i) => i,
                None => {
                    regions.push(attrs);
                    regions.len() - 1
                }
            }
        })
        .collect();

    let mut f =
        std::fs::File::create(output).map_err(|e| format!("Failed to create output: {e}"))?;
    writeln!(f, r#"<?xml version="1.0" encoding="UTF-8"?>"#).map_err(|e| e.to_string())?;
    let style = appearance.style_attributes();
    writeln!(
        f,
        r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:tts="http://www.w3.org/ns/ttml#styling" xmlns:ttp="http://www.w3.org/ns/ttml#parameter"{}>"#,
        appearance.cell_resolution()
    )
    .map_err(|e| e.to_string())?;
    writeln!(f, "  <head>").map_err(|e| e.to_string())?;
    if let Some(attributes) = &style {
        write_styling(&mut f, attributes)?;
    }
    writeln!(f, "    <layout>").map_err(|e| e.to_string())?;
    for (i, attrs) in regions.iter().enumerate() {
        writeln!(f, r#"      <region xml:id="r{i}" {attrs}/>"#).map_err(|e| e.to_string())?;
    }
    writeln!(f, "    </layout>").map_err(|e| e.to_string())?;
    writeln!(f, "  </head>").map_err(|e| e.to_string())?;
    writeln!(f, "  <body>").map_err(|e| e.to_string())?;
    writeln!(f, "    <div>").map_err(|e| e.to_string())?;

    for (cue, region) in cues.iter().zip(&cue_region) {
        let start = format_ttml_time(cue.start_ms);
        let end = format_ttml_time(cue.end_ms);
        let body = render_runs(&cue.runs);
        writeln!(
            f,
            r#"      <p begin="{start}" end="{end}" region="r{region}"{}>{body}</p>"#,
            paragraph_style(style.as_ref())
        )
        .map_err(|e| e.to_string())?;
    }

    writeln!(f, "    </div>").map_err(|e| e.to_string())?;
    writeln!(f, "  </body>").map_err(|e| e.to_string())?;
    writeln!(f, "</tt>").map_err(|e| e.to_string())?;
    Ok(())
}

fn region_attrs(cue: &StyledCue) -> String {
    let text_align = match cue.align {
        Some(HAlign::Left) => "start",
        Some(HAlign::Right) => "end",
        _ => "center",
    };
    match cue.vposition {
        // explicit vertical position: a band anchored there
        Some(vp) => format!(
            r#"tts:origin="10% {vp:.1}%" tts:extent="80% 15%" tts:textAlign="{text_align}" tts:displayAlign="before""#
        ),
        None => {
            let display_align = match cue.valign {
                Some(VAlign::Top) => "before",
                Some(VAlign::Middle) => "center",
                _ => "after",
            };
            format!(
                r#"tts:origin="10% 10%" tts:extent="80% 80%" tts:textAlign="{text_align}" tts:displayAlign="{display_align}""#
            )
        }
    }
}

fn render_runs(runs: &[StyledRun]) -> String {
    let mut out = String::new();
    for run in runs {
        let text = escape_xml(&run.text).replace('\n', "<br/>");
        let mut styles: Vec<String> = Vec::new();
        if run.italic {
            styles.push(r#"tts:fontStyle="italic""#.to_string());
        }
        if run.bold {
            styles.push(r#"tts:fontWeight="bold""#.to_string());
        }
        if run.underline {
            styles.push(r#"tts:textDecoration="underline""#.to_string());
        }
        if let Some(c) = run.color {
            styles.push(format!(
                r##"tts:color="#{:02X}{:02X}{:02X}{:02X}""##,
                c.r, c.g, c.b, c.a
            ));
        }
        if styles.is_empty() {
            out.push_str(&text);
        } else {
            out.push_str(&format!("<span {}>{}</span>", styles.join(" "), text));
        }
    }
    out
}

pub(crate) fn format_ttml_time(ms: u64) -> String {
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
        convert_subtitles(
            &input,
            &output,
            SubtitleFormat::ImscTtml,
            &TextAppearance::default(),
        )
        .unwrap();
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
        convert_subtitles(
            &input,
            &output,
            SubtitleFormat::ImscTtml,
            &TextAppearance::default(),
        )
        .unwrap();
        let ttml = std::fs::read_to_string(output).unwrap();
        assert!(ttml.contains("HELLO"), "ttml: {ttml}");
        assert!(ttml.contains(r#"begin="00:00:01.001""#), "ttml: {ttml}");
    }

    #[test]
    fn preserves_authored_ttml() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.ttml");
        let output = dir.path().join("out.ttml");
        let authored = r#"<tt><head><styling><style xml:id="top"/></styling></head><body><div><p region="top">Hello</p></div></body></tt>"#;
        std::fs::write(&input, authored).unwrap();

        convert_subtitles(
            &input,
            &output,
            SubtitleFormat::ImscTtml,
            &TextAppearance::default(),
        )
        .unwrap();

        assert_eq!(std::fs::read_to_string(output).unwrap(), authored);
    }

    #[test]
    fn ass_to_ttml_keeps_styling() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.ass");
        let output = dir.path().join("out.ttml");
        std::fs::write(
            &input,
            "[Script Info]\nTitle: t\n\n\
[V4+ Styles]\n\
Format: Name, Fontname, Fontsize, PrimaryColour, Bold, Italic, Underline, Alignment\n\
Style: Default,Arial,40,&H00FFFFFF,0,-1,0,2\n\
Style: Top,Arial,40,&H00FFFFFF,0,0,0,8\n\n\
[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:03.50,Default,,0,0,0,,plain {\\b1}bold{\\b0}\n\
Dialogue: 0,0:00:04.00,0:00:05.00,Top,,0,0,0,,{\\an9}corner\n",
        )
        .unwrap();
        convert_subtitles(
            &input,
            &output,
            SubtitleFormat::ImscTtml,
            &TextAppearance::default(),
        )
        .unwrap();
        let ttml = std::fs::read_to_string(output).unwrap();
        // base Default style is italic (-1)
        assert!(ttml.contains(r#"tts:fontStyle="italic""#), "ttml: {ttml}");
        // inline {\b1} run is bold
        assert!(ttml.contains(r#"tts:fontWeight="bold""#), "ttml: {ttml}");
        // \an9 is top-right: a region with end/before
        assert!(ttml.contains(r#"tts:textAlign="end""#), "ttml: {ttml}");
        assert!(
            ttml.contains(r#"tts:displayAlign="before""#),
            "ttml: {ttml}"
        );
    }

    #[test]
    fn ass_to_plain_target_flattens() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.ass");
        let output = dir.path().join("out.ttml");
        std::fs::write(
            &input,
            "[Script Info]\nTitle: t\n\n\
[V4+ Styles]\n\
Format: Name, Fontname, Fontsize, PrimaryColour, Bold, Italic, Underline, Alignment\n\
Style: Default,Arial,40,&H00FFFFFF,0,-1,0,2\n\n\
[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:03.50,Default,,0,0,0,,hello\n",
        )
        .unwrap();
        convert_subtitles(
            &input,
            &output,
            SubtitleFormat::Srt,
            &TextAppearance::default(),
        )
        .unwrap();
        let ttml = std::fs::read_to_string(output).unwrap();
        assert!(
            !ttml.contains("tts:"),
            "plain target must drop styling: {ttml}"
        );
        assert!(ttml.contains(">hello</p>"), "ttml: {ttml}");
    }

    #[test]
    fn fcpxml_to_ttml_keeps_color() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.fcpxml");
        let output = dir.path().join("out.ttml");
        std::fs::write(
            &input,
            r#"<?xml version="1.0"?>
<fcpxml version="1.10">
 <library><event><project><sequence><spine>
   <caption offset="3600/2500s" duration="2500/2500s" role="captions">
     <text><text-style ref="ts1">Hello world</text-style></text>
     <text-style-def id="ts1"><text-style italic="1" bold="0"/></text-style-def>
   </caption>
   <title offset="10s" duration="5s">
     <text><text-style ref="ts2">A title</text-style></text>
     <text-style-def id="ts2"><text-style fontColor="1 0 0 1"/></text-style-def>
   </title>
 </spine></sequence></project></event></library>
</fcpxml>"#,
        )
        .unwrap();
        convert_subtitles(
            &input,
            &output,
            SubtitleFormat::ImscTtml,
            &TextAppearance::default(),
        )
        .unwrap();
        let ttml = std::fs::read_to_string(output).unwrap();
        assert!(ttml.contains(r#"tts:fontStyle="italic""#), "ttml: {ttml}");
        assert!(ttml.contains(r##"tts:color="#FF0000FF""##), "ttml: {ttml}");
        assert!(
            ttml.contains(">A title</span>") || ttml.contains(">A title<"),
            "ttml: {ttml}"
        );
    }

    #[test]
    fn mks_converts_or_skips_without_ffmpeg() {
        let dir = tempfile::tempdir().unwrap();
        // real mks fixture needs ffmpeg to author; skip gracefully when absent.
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
        {
            eprintln!("ffmpeg not present, skipping mks test");
            return;
        }
        // build a tiny mkv with an embedded srt subtitle track
        let srt = dir.path().join("s.srt");
        std::fs::write(&srt, "1\n00:00:01,000 --> 00:00:02,000\nhi there\n").unwrap();
        let mks = dir.path().join("s.mks");
        let built = std::process::Command::new("ffmpeg")
            .args(["-y", "-f", "lavfi", "-i", "color=c=black:s=64x64:d=3", "-i"])
            .arg(&srt)
            .args(["-c:s", "srt", "-shortest"])
            .arg(&mks)
            .output();
        let ok = matches!(&built, Ok(o) if o.status.success()) && mks.exists();
        if !ok {
            eprintln!("ffmpeg could not build mks fixture, skipping");
            return;
        }
        let output = dir.path().join("out.ttml");
        convert_subtitles(
            &mks,
            &output,
            SubtitleFormat::ImscTtml,
            &TextAppearance::default(),
        )
        .unwrap();
        let ttml = std::fs::read_to_string(output).unwrap();
        assert!(ttml.contains("hi there"), "ttml: {ttml}");
    }
}
