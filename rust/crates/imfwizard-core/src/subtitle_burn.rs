//! `create --burn-subtitle`: cues drawn into the picture during the encode.
//!
//! Burnt-in text is part of the image, so nothing here registers a timed-text
//! track. The compositing itself is postkit's `subtitle_raster`.

use postkit::subtitle_formats::{StyledCue, StyledRun};
use std::path::{Path, PathBuf};
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

/// Prepare `create --burn-subtitle`: parse the cue file and build the burn the
/// encoder threads composite onto every decoded frame.
///
/// `font` is the face to shape text with; without one the system faces fontdb
/// finds are used, and a machine with no font at all is an error rather than a
/// silently subtitle-free encode.
pub fn prepare_subtitle_burn(
    input: &Path,
    font: Option<&Path>,
    fps: u32,
) -> Result<Arc<postkit::subtitle_raster::SubtitleBurn>, String> {
    if let Some(path) = font
        && !path.is_file()
    {
        return Err(format!("burn-in font not found: {}", path.display()));
    }
    let cues = load_styled_cues(input)?;
    postkit::subtitle_raster::SubtitleBurn::new(
        cues,
        font,
        postkit::subtitle_raster::BurnStyle::default(),
        fps.max(1) as f64,
    )
    .map(Arc::new)
    .map_err(|e| format!("cannot burn {}: {e}", input.display()))
}

/// What the encode would hand a burn, as the pre-encode checks see it.
pub struct BurnTarget<'a> {
    /// Every timed-text file the composition packages.
    pub timed_text: &'a [PathBuf],
    /// Frames reach the encoder already X'Y'Z', so display-RGB text would land
    /// in the wrong space. Covers `--source-colourspace xyz` and `--hdr`, which
    /// declares the essence PQ.
    pub frames_already_xyz: bool,
    /// The picture is a J2K directory, so nothing decodes.
    pub input_is_codestreams: bool,
    /// The picture is one image held for a duration.
    pub input_is_held_still: bool,
}

/// Refuse a `--burn-subtitle` the encode cannot honour, before anything is
/// encoded.
pub fn check_burn_supported(burn_path: &Path, target: &BurnTarget) -> Result<(), String> {
    if !burn_path.is_file() {
        return Err(format!(
            "--burn-subtitle file not found: {}",
            burn_path.display()
        ));
    }
    if target.timed_text.iter().any(|t| same_file(burn_path, t)) {
        return Err(format!(
            "{} is given to both --burn-subtitle and --subtitle: a burnt-in subtitle must not \
             also be a timed-text track, so pick one",
            burn_path.display()
        ));
    }
    if target.input_is_codestreams {
        return Err(
            "--burn-subtitle needs frames to draw on, and a J2K directory is already compressed"
                .into(),
        );
    }
    if target.input_is_held_still {
        return Err(
            "--burn-subtitle cannot go on a --still-length hold: the image is encoded once and \
             the codestream repeated, so every frame would carry the cues of the first one"
                .into(),
        );
    }
    if target.frames_already_xyz {
        return Err(
            "--burn-subtitle draws in display RGB, but this source reaches the encoder as \
             X'Y'Z' already (--source-colourspace xyz, or --hdr declaring PQ essence): burn \
             from the display-RGB master instead"
                .into(),
        );
    }
    Ok(())
}

/// Whether two paths name the same file, falling back to the paths themselves
/// when either cannot be canonicalised.
fn same_file(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}
