//! Every refusal a `create` job can make before the encode starts.
//!
//! The rule: a refusal that fires once the encode has run must also fire from
//! here, so nothing spends a whole encode to find out it cannot be packaged.
//! Each check still lives in the module that owns it, and this runs them in one
//! order over one description of the job.

use std::path::{Path, PathBuf};

use postkit::encode::{FrameRate, InputType, SourceColour, detect_input_type};
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
    pub timed_text_language: Option<String>,
    pub fps_num: u32,
    pub fps_den: u32,
    pub edits: SourceEdits,
    pub audio_map: Option<String>,
    pub burn_subtitle: Option<PathBuf>,
    pub burn_subtitle_font: Option<PathBuf>,
    pub burn_style: BurnStyleOverrides,
    pub picture_options: SourcePictureOptions,
    pub source_colour: SourceColour,
    /// The `--hdr` preset's metadata, which makes the picture's descriptor
    /// declare PQ. None is an SDR Rec.709 picture.
    pub hdr: Option<crate::hdr_wcg::HdrWcg>,
    /// Frames to hold a still for; None when the picture is not a still.
    pub still_frames: Option<u64>,
    pub atmos_frame_directory: Option<PathBuf>,
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
         (tif, tiff, dpx, exr, bmp, png, jpg, jpeg), a directory of J2K \
         codestreams (j2c, j2k), \
         or a single still image held for a length",
        picture.display()
    )
}

pub const DEFAULT_EDIT_RATE: FrameRate = FrameRate {
    numerator: 24,
    denominator: 1,
};

// a video file plays at the rate it carries, every other picture takes the one asked for
pub fn resolve_edit_rate(
    picture: Option<&Path>,
    requested: Option<FrameRate>,
) -> Result<FrameRate, String> {
    let probed = picture
        .filter(|picture| detect_input_type(picture) == InputType::Video)
        .and_then(|picture| Some((picture, crate::probe::probe_video(picture)?)))
        // ffprobe answers 0/0 for a stream whose rate it cannot read
        .filter(|(_, info)| info.fps_num > 0 && info.fps_den > 0)
        .map(|(picture, info)| (picture, FrameRate::new(info.fps_num, info.fps_den)));
    match (probed, requested) {
        (Some((_, probed)), None) => Ok(probed),
        (Some((picture, probed)), Some(requested)) if !same_rate(probed, requested) => {
            let (probed, requested) = (spelled_rate(probed), spelled_rate(requested));
            Err(format!(
                "{} plays at {probed} fps, but the edit rate asked for is {requested}: \
                 package it at {probed} or conform the source to {requested} first",
                picture.display()
            ))
        }
        (_, Some(requested)) => Ok(requested),
        (None, None) => Ok(DEFAULT_EDIT_RATE),
    }
}

fn same_rate(left: FrameRate, right: FrameRate) -> bool {
    u64::from(left.numerator) * u64::from(right.denominator)
        == u64::from(right.numerator) * u64::from(left.denominator)
}

fn spelled_rate(rate: FrameRate) -> String {
    format!("{}/{}", rate.numerator, rate.denominator)
}

/// Run every plan-time refusal, cheapest and most specific first so a job with
/// two faults names the one a reader can act on.
pub fn check_before_encode(plan: &CreatePlan) -> Result<(), String> {
    for language in [&plan.audio_language, &plan.timed_text_language]
        .into_iter()
        .flatten()
    {
        crate::imp::validate_language(language)?;
    }
    plan.picture_options.check()?;
    if plan.hdr.is_some() {
        crate::source_colourspace::reject_converting_source_under_hdr(&plan.source_colour)?;
    }
    if plan.still_frames.is_some()
        && let Some(lut) = plan.source_colour.decode_lut()
    {
        return Err(format!(
            "--source-lut {} runs inside the decode, and a held still is encoded without \
             one: apply the LUT to the image first",
            lut.display()
        ));
    }
    if let Some(picture) = &plan.picture {
        let input_type = detect_input_type(picture);
        if plan.still_frames.is_none() && input_type == InputType::Unknown {
            return Err(unclassified_picture_refusal(picture));
        }
        if input_type == InputType::PictureMxf {
            return Err(format!(
                "{} is a JPEG 2000 picture MXF, which IMF Wizard cannot import yet",
                picture.display()
            ));
        }
        crate::source_colourspace::reject_on_precompressed_picture(picture, &plan.source_colour)?;
        crate::source_picture::reject_on_precompressed_picture(picture, &plan.picture_options)?;
    }
    check_timed_text(plan)?;
    check_burn(plan)?;
    check_app2e_picture(plan)?;
    resolve_edit_rate(
        plan.picture.as_deref(),
        Some(FrameRate::new(plan.fps_num, plan.fps_den)),
    )?;
    check_hdr_signalling(plan)?;
    check_sound_depth(plan)?;
    check_audio_map(plan)?;
    check_source_edits(plan)?;
    check_atmos(plan)
}

fn check_timed_text(plan: &CreatePlan) -> Result<(), String> {
    for path in &plan.timed_text_files {
        if !path.is_file() {
            return Err(format!("subtitle file not found: {}", path.display()));
        }
        crate::subtitle_convert::readable_source_format(path)?;
    }
    Ok(())
}

// a master too deep for an App 2E wrap is refused here rather than after the
// encode, where the wrap would have had to drop the bits that do not fit
fn check_sound_depth(plan: &CreatePlan) -> Result<(), String> {
    for wav in &plan.audio_files {
        crate::source_edits::check_sound_depth(wav)?;
    }
    Ok(())
}

/// The routes that hand the encoder X'Y'Z' frames, as the refusal names them.
const XYZ_ROUTES: &str = "--source-colourspace xyz, or --hdr declaring PQ essence";

/// A burn with no picture is refused by the front end that can be in that state,
/// naming the control that carried it, so there is nothing to draw on here.
fn check_burn(plan: &CreatePlan) -> Result<(), String> {
    let (Some(burn), Some(picture)) = (&plan.burn_subtitle, &plan.picture) else {
        return Ok(());
    };
    postkit::preflight::check_burn_supported(
        burn,
        &postkit::preflight::BurnTarget {
            timed_text: &plan.timed_text_files,
            frames_already_xyz: crate::source_colourspace::frames_reach_the_compressor_as_xyz(
                &plan.source_colour,
            )
            .then_some(XYZ_ROUTES),
            input_is_codestreams: detect_input_type(picture) == InputType::J2kSequence,
        },
    )?;
    crate::subtitle_burn::prepare_subtitle_burn(
        burn,
        plan.burn_subtitle_font.as_deref(),
        &plan.burn_style,
        crate::encode::FrameRate::new(plan.fps_num, plan.fps_den),
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
    let (source_width, source_height) = postkit::encode::source_raster(picture)?;
    let (width, height) =
        crate::source_picture::encode_raster(&plan.picture_options, source_width, source_height);
    crate::mxf_wrap::validate_app2e_raster(width, height)
}

// runs after the raster check, which is what names a picture ffprobe cannot read
fn check_hdr_signalling(plan: &CreatePlan) -> Result<(), String> {
    let Some(picture) = &plan.picture else {
        return Ok(());
    };
    crate::hdr_source::check_source_matches_preset(
        crate::hdr_source::probe(picture)?,
        plan.hdr.as_ref(),
    )
}

fn check_audio_map(plan: &CreatePlan) -> Result<(), String> {
    let Some(spec) = &plan.audio_map else {
        return Ok(());
    };
    for wav in &plan.audio_files {
        crate::audio_map::parse_audio_map(spec, postkit::wav_io::channel_count(wav)?)?;
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
        // what a conversion writes is always trimmable
        if crate::subtitle_convert::readable_source_format(path)?.is_ttml() {
            crate::source_edits::check_timed_text_trimmable(path, facts.fps)?;
        }
    }
    Ok(())
}

// after check_source_edits, which refuses a trim longer than the picture
fn check_atmos(plan: &CreatePlan) -> Result<(), String> {
    let Some(frame_directory) = &plan.atmos_frame_directory else {
        return Ok(());
    };
    let iab_frames = crate::atmos::iab_frame_files(frame_directory)?.len();
    let facts = crate::source_edits::probe_source_facts(
        plan.picture.as_deref(),
        plan.still_frames,
        &[],
        plan.fps_num,
        plan.fps_den,
    )?;
    let Some(picture) = facts.picture else {
        return Ok(());
    };
    let packaged_frames =
        picture.frames - plan.edits.trim_start_frames - plan.edits.trim_end_frames;
    crate::atmos::check_iab_frame_count(iab_frames, packaged_frames)
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
    fn a_jpeg_2000_picture_mxf_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let picture = crate::mxf_wrap::wrapped_picture(dir.path(), None, 1).path;

        let plan = CreatePlan {
            picture: Some(picture.clone()),
            fps_num: 24,
            fps_den: 1,
            ..Default::default()
        };
        let error = check_before_encode(&plan).unwrap_err();

        assert_eq!(
            error,
            format!(
                "{} is a JPEG 2000 picture MXF, which IMF Wizard cannot import yet",
                picture.display()
            )
        );
    }

    /// A LUT whose output is Rec.709 RGB is exactly what the descriptor declares,
    /// so it runs in the decode and the plan passes.
    #[test]
    fn a_source_lut_reaches_the_encode_as_a_rec709_conversion() {
        let dir = tempfile::tempdir().unwrap();
        let lut = dir.path().join("to_rec709.cube");
        std::fs::write(&lut, "LUT_3D_SIZE 2\n").unwrap();

        let plan = CreatePlan {
            fps_num: 24,
            fps_den: 1,
            source_colour: SourceColour::KeepRgbAfterLut(lut.clone()),
            ..Default::default()
        };
        assert_eq!(check_before_encode(&plan), Ok(()));
        assert_eq!(
            plan.source_colour.decode_lut(),
            Some(lut.as_path()),
            "the LUT has to reach ffmpeg's decode"
        );
    }

    /// An `--hdr` picture's descriptor declares PQ, and every converting source
    /// lands on Rec.709 SDR, so the pair is refused before the encode.
    #[test]
    fn an_hdr_picture_refuses_a_converting_source() {
        let hdr = crate::hdr_wcg::HdrWcg::from_flags("pq-bt2020", None).unwrap();
        let lut = std::path::PathBuf::from("/luts/to_rec709.cube");
        for source_colour in [
            SourceColour::KeepRgbFrom(postkit::colour::ColourSpace::P3D65),
            SourceColour::KeepRgbAfterLut(lut),
        ] {
            let plan = CreatePlan {
                fps_num: 24,
                fps_den: 1,
                source_colour,
                hdr: Some(hdr.clone()),
                ..Default::default()
            };
            let error = check_before_encode(&plan).unwrap_err();
            assert!(error.contains("--hdr"), "{error}");
            assert!(error.contains("Rec.709 SDR"), "{error}");
        }

        let sdr = CreatePlan {
            fps_num: 24,
            fps_den: 1,
            source_colour: SourceColour::KeepRgbFrom(postkit::colour::ColourSpace::P3D65),
            ..Default::default()
        };
        assert_eq!(check_before_encode(&sdr), Ok(()));
    }

    fn pal_clip(dir: &Path) -> PathBuf {
        let clip = dir.join("pal.mov");
        let made = std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("color=c=gray:s=1920x1080:r=25")
            .args(["-frames:v", "2", "-pix_fmt", "yuv420p"])
            .arg(&clip)
            .output()
            .expect("ffmpeg");
        assert!(
            made.status.success(),
            "{}",
            String::from_utf8_lossy(&made.stderr)
        );
        clip
    }

    #[test]
    fn a_video_is_declared_at_its_own_rate_unless_another_is_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let clip = pal_clip(dir.path());
        let pal = FrameRate::new(25, 1);

        assert_eq!(resolve_edit_rate(Some(&clip), None), Ok(pal));
        assert_eq!(
            resolve_edit_rate(Some(&clip), Some(FrameRate::new(50, 2))),
            Ok(FrameRate::new(50, 2))
        );
        assert_eq!(resolve_edit_rate(None, None), Ok(DEFAULT_EDIT_RATE));

        let declared_at_24 = CreatePlan {
            picture: Some(clip.clone()),
            fps_num: 24,
            fps_den: 1,
            ..Default::default()
        };
        let error = check_before_encode(&declared_at_24).unwrap_err();
        assert!(error.contains("plays at 25/1 fps"), "{error}");
        assert!(error.contains("asked for is 24/1"), "{error}");

        let declared_at_25 = CreatePlan {
            fps_num: 25,
            ..declared_at_24
        };
        assert_eq!(check_before_encode(&declared_at_25), Ok(()));
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
