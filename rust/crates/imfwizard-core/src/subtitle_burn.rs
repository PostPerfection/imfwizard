//! `create --burn-subtitle`: cues drawn into the picture during the encode.
//!
//! Burnt-in text is part of the image, so nothing here registers a timed-text
//! track. The compositing itself is postkit's `subtitle_raster`.

use postkit::subtitle_formats::{StyledCue, StyledRun};
use std::path::Path;
use std::sync::Arc;

use crate::subtitle_convert::SubtitleFormat;

/// The burn sources there is a cue reader for, as the error messages spell them.
const BURN_SOURCE_FORMATS: &str = "SRT, ASS/SSA, SCC, FCPXML or MKS/MKV";

/// Load a burn source into `StyledCue`s.
///
/// The set is narrower than `subtitle-convert` takes: TTML/IMSC is the format
/// imfwizard packages rather than reads, and nothing here parses one back to
/// cues.
pub fn load_styled_cues(path: &Path) -> Result<Vec<StyledCue>, String> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let format = SubtitleFormat::from_extension(&extension).ok_or_else(|| {
        format!("unsupported subtitle format '.{extension}': burn from {BURN_SOURCE_FORMATS}")
    })?;

    let read = || std::fs::read_to_string(path).map_err(|e| format!("cannot read {path:?}: {e}"));
    let cues = match format {
        SubtitleFormat::Srt => plain_cues(postkit::subtitle_retime::parse_srt(&read()?)),
        SubtitleFormat::Scc => plain_cues(crate::scc::parse_scc(&read()?)?),
        SubtitleFormat::Ass => {
            let parsed = postkit::subtitle_formats::ass::parse_ass(&read()?)
                .map_err(|e| format!("ASS parse: {e}"))?;
            for tag in &parsed.warnings {
                tracing::warn!("ASS override tag not modelled, dropped: {tag}");
            }
            parsed.cues
        }
        SubtitleFormat::Fcpxml => postkit::subtitle_formats::fcpxml::parse_fcpxml(&read()?)
            .map_err(|e| format!("FCPXML parse: {e}"))?,
        SubtitleFormat::Mks => postkit::subtitle_formats::mks::parse_mks(path, None)
            .map_err(|e| format!("MKS parse: {e}"))?,
        SubtitleFormat::Ttml | SubtitleFormat::ImscTtml => {
            return Err(format!(
                "{} is TTML/IMSC, which has no cue reader here: burn from the {BURN_SOURCE_FORMATS} \
                 source it was made from",
                path.display()
            ));
        }
        SubtitleFormat::Vtt | SubtitleFormat::Stl => {
            return Err(format!(
                "{} has no cue reader here: burn from {BURN_SOURCE_FORMATS}",
                path.display()
            ));
        }
    };
    if cues.is_empty() {
        return Err(format!("no subtitle cues in {}", path.display()));
    }
    Ok(cues)
}

fn plain_cues(cues: Vec<postkit::subtitle_retime::SrtCue>) -> Vec<StyledCue> {
    cues.into_iter()
        .filter(|cue| !cue.text.is_empty())
        .map(|cue| StyledCue::text(cue.start_ms, cue.end_ms, vec![StyledRun::plain(cue.text)]))
        .collect()
}

/// Lay the caller's appearance settings over the burn defaults, naming the flag
/// group so a range error points at the appearance rather than at the cue file.
///
/// Both front ends call this before an encode starts, so a bad size or scale is
/// refused alongside the other burn refusals.
pub fn resolve_burn_style(
    appearance: &postkit::subtitle_raster::BurnStyleOverrides,
) -> Result<postkit::subtitle_raster::BurnStyle, String> {
    appearance
        .apply(postkit::subtitle_raster::BurnStyle::default())
        .map_err(|e| format!("burn-in appearance: {e}"))
}

/// Prepare `create --burn-subtitle`: parse the cue file and build the burn the
/// encoder threads composite onto every decoded frame.
///
/// `font` is the face to shape text with; without one the system faces fontdb
/// finds are used, and a machine with no font at all is an error rather than a
/// silently subtitle-free encode. `appearance` carries whatever the caller named
/// about how the text looks, and leaves the rest at the burn defaults.
pub fn prepare_subtitle_burn(
    input: &Path,
    font: Option<&Path>,
    appearance: &postkit::subtitle_raster::BurnStyleOverrides,
    fps: postkit::encode::FrameRate,
) -> Result<Arc<postkit::subtitle_raster::SubtitleBurn>, String> {
    if let Some(path) = font
        && !path.is_file()
    {
        return Err(format!("burn-in font not found: {}", path.display()));
    }
    let style = resolve_burn_style(appearance)?;
    let cues = load_styled_cues(input)?;
    postkit::subtitle_raster::SubtitleBurn::new(cues, font, style, fps.as_f64())
        .map(Arc::new)
        .map_err(|e| format!("cannot burn {}: {e}", input.display()))
}
