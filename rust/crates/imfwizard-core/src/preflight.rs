//! Every refusal a `create` job can make before the encode starts.
//!
//! The rule: a refusal that fires once the encode has run must also fire from
//! here, so nothing spends a whole encode to find out it cannot be packaged.
//! Each check still lives in the module that owns it, and this runs them in one
//! order over one description of the job.

use std::path::PathBuf;

use postkit::encode::{InputType, SourceColour, detect_input_type};
use postkit::subtitle_raster::BurnStyleOverrides;

use crate::source_edits::SourceEdits;
use crate::source_picture::SourcePictureOptions;

/// What a `create` job settles before the encode, as both front ends describe it.
#[derive(Debug, Clone, Default)]
pub struct CreatePlan {
    /// Video file, image directory, J2K directory or still image. None is an
    /// audio-only composition.
    pub picture: Option<PathBuf>,
    pub audio_files: Vec<PathBuf>,
    pub audio_language: Option<String>,
    pub timed_text_files: Vec<PathBuf>,
    pub fps_num: u32,
    pub fps_den: u32,
    pub edits: SourceEdits,
    pub audio_map: Option<String>,
    pub burn_subtitle: Option<PathBuf>,
    pub burn_subtitle_font: Option<PathBuf>,
    pub burn_style: BurnStyleOverrides,
    pub picture_options: SourcePictureOptions,
    pub source_colour: SourceColour,
    /// Frames to hold a still for; None when the picture is not a still.
    pub still_frames: Option<u64>,
}

impl CreatePlan {
    pub fn fps(&self) -> f64 {
        self.fps_num.max(1) as f64 / self.fps_den.max(1) as f64
    }

    fn picture_is_codestreams(&self) -> bool {
        self.picture
            .as_deref()
            .map(|picture| detect_input_type(picture) == InputType::J2kSequence)
            .unwrap_or(false)
    }
}

/// What a picture has to be for the encode to read it. A held still is not in
/// this set: it is decoded on its own, so the check skips one.
pub fn unclassified_picture_refusal(picture: &std::path::Path) -> String {
    format!(
        "{} is not a picture the encoder can read: it takes a video container \
         (mp4, mov, mkv, avi, mxf, ts, m2ts, webm), a directory of images \
         (tif, tiff, dpx, exr, bmp), a directory of J2K codestreams (j2c, j2k), \
         or a single still image held for a length",
        picture.display()
    )
}

/// Run every plan-time refusal, cheapest and most specific first so a job with
/// two faults names the one a reader can act on.
pub fn check_before_encode(plan: &CreatePlan) -> Result<(), String> {
    plan.picture_options.check()?;
    if let Some(picture) = &plan.picture {
        if plan.still_frames.is_none() && detect_input_type(picture) == InputType::Unknown {
            return Err(unclassified_picture_refusal(picture));
        }
        crate::source_colourspace::reject_on_precompressed_picture(picture, &plan.source_colour)?;
        crate::source_picture::reject_on_precompressed_picture(picture, &plan.picture_options)?;
    }
    check_burn(plan)?;
    check_app2e_picture(plan)?;
    check_audio_map(plan)?;
    check_source_edits(plan)
}

/// A burn with no picture is refused by the front end that can be in that state,
/// naming the control that carried it, so there is nothing to draw on here.
fn check_burn(plan: &CreatePlan) -> Result<(), String> {
    let (Some(burn), Some(picture)) = (&plan.burn_subtitle, &plan.picture) else {
        return Ok(());
    };
    crate::subtitle_burn::check_burn_supported(
        burn,
        &crate::subtitle_burn::BurnTarget {
            timed_text: &plan.timed_text_files,
            frames_already_xyz: !plan.source_colour.applies_xyz_transform(),
            input_is_codestreams: detect_input_type(picture) == InputType::J2kSequence,
        },
    )?;
    crate::subtitle_burn::prepare_subtitle_burn(
        burn,
        plan.burn_subtitle_font.as_deref(),
        &plan.burn_style,
        plan.fps_num,
    )
    .map(|_| ())
}

fn check_app2e_picture(plan: &CreatePlan) -> Result<(), String> {
    let Some(picture) = &plan.picture else {
        return Ok(());
    };
    if plan.picture_is_codestreams() {
        return crate::mxf_wrap::precheck_j2k(picture);
    }
    let (source_width, source_height) = crate::source_picture::source_raster(picture)?;
    let (width, height) =
        crate::source_picture::encode_raster(&plan.picture_options, source_width, source_height);
    crate::mxf_wrap::validate_app2e_raster(width, height)
}

fn check_audio_map(plan: &CreatePlan) -> Result<(), String> {
    let Some(spec) = &plan.audio_map else {
        return Ok(());
    };
    for wav in &plan.audio_files {
        crate::audio_map::parse_audio_map(spec, crate::audio_map::input_channels(wav)?)?;
    }
    Ok(())
}

fn check_source_edits(plan: &CreatePlan) -> Result<(), String> {
    if plan.edits == SourceEdits::default() {
        return Ok(());
    }
    let facts = crate::source_edits::probe_source_facts(
        plan.picture.as_deref(),
        plan.still_frames,
        &plan.audio_files,
        plan.fps_num,
        plan.fps_den,
    )?;
    crate::source_edits::check_edits(&plan.edits, &facts)?;
    if !plan.edits.trims() {
        return Ok(());
    }
    for path in &plan.timed_text_files {
        crate::source_edits::check_timed_text_trimmable(path, facts.fps)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A codestream at a raster App 2E does not allow has to be refused before
    /// the encode, not by the wrapper after it.
    #[test]
    fn an_illegal_j2k_directory_is_refused_with_nothing_encoded() {
        let dir = tempfile::tempdir().unwrap();
        let codestreams = dir.path().join("j2k");
        std::fs::create_dir_all(&codestreams).unwrap();
        std::fs::write(
            codestreams.join("frame_00000000.j2c"),
            crate::mxf_wrap::synthetic_j2k_codestream(2048, 872, 12),
        )
        .unwrap();
        let output = dir.path().join("out");

        let plan = CreatePlan {
            picture: Some(codestreams),
            fps_num: 24,
            fps_den: 1,
            ..Default::default()
        };
        let error = check_before_encode(&plan).unwrap_err();

        assert!(error.contains("2048x872"), "{error}");
        assert!(!output.exists(), "the check must write nothing");
    }

    /// A picture the encoder cannot classify has to be named as such, not left
    /// to a size probe that fails for a different reason.
    #[test]
    fn a_picture_that_is_none_of_the_shapes_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let loose = dir.path().join("loose");
        std::fs::create_dir_all(&loose).unwrap();
        std::fs::write(loose.join("notes.txt"), "not a frame").unwrap();

        let plan = CreatePlan {
            picture: Some(loose),
            fps_num: 24,
            fps_den: 1,
            ..Default::default()
        };
        let error = check_before_encode(&plan).unwrap_err();

        assert!(error.contains("tif, tiff, dpx, exr, bmp"), "{error}");
    }

    #[test]
    fn a_legal_j2k_directory_passes() {
        let dir = tempfile::tempdir().unwrap();
        let codestreams = dir.path().join("j2k");
        std::fs::create_dir_all(&codestreams).unwrap();
        std::fs::write(
            codestreams.join("frame_00000000.j2c"),
            crate::mxf_wrap::synthetic_j2k_codestream(1920, 1080, 12),
        )
        .unwrap();

        let plan = CreatePlan {
            picture: Some(codestreams),
            fps_num: 24,
            fps_den: 1,
            ..Default::default()
        };
        assert_eq!(check_before_encode(&plan), Ok(()));
    }
}
