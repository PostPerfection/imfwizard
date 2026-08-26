//! The colour space the picture source carries, mapped onto the encode path.
//!
//! postkit decides the encoder transform from `encode::SourceColour`. Rec.709
//! takes the built-in Rec.709 to DCI X'Y'Z' transform, X'Y'Z' leaves the frames
//! alone, and P3, Rec.2020 and LogC go through `DisplayRgbIn`, postkit's
//! per-space curve and matrix applied to every frame with the compressor
//! transform off. ACES and ACEScg are scene-referred, so no matrix reaches
//! X'Y'Z' from them and they are refused rather than encoded through the Rec.709
//! matrix, which would be silently wrong colour.

use postkit::colour::ColourSpace;
use postkit::encode::SourceColour;

/// Spell a colour space as the CLI and GUI take it. The set is postkit's
/// `ColourSpace`; dcpwizard spells the values the same way.
pub fn parse(value: &str) -> Result<ColourSpace, String> {
    match value.to_lowercase().as_str() {
        "rec709" => Ok(ColourSpace::Rec709),
        "p3" => Ok(ColourSpace::P3),
        "xyz" => Ok(ColourSpace::Xyz),
        "rec2020" => Ok(ColourSpace::Rec2020),
        "aces" => Ok(ColourSpace::Aces),
        "acescg" => Ok(ColourSpace::AcesCg),
        "logc" => Ok(ColourSpace::LogC),
        _ => Err(format!(
            "unknown source colour space '{value}' (expected {VALUES})"
        )),
    }
}

/// Every value `parse` accepts, in the order the help text lists them.
pub const VALUES: &str = "rec709, p3, xyz, rec2020, aces, acescg or logc";

/// How the encoder should treat frames carrying this colour.
pub fn to_source_colour(space: ColourSpace) -> Result<SourceColour, String> {
    match space {
        // the default, and the only space postkit has a built-in transform for
        ColourSpace::Rec709 => Ok(SourceColour::DisplayRgb),
        // already X'Y'Z', so nothing may transform it again. postkit spells this
        // AlreadyPq for its HDR origin; skipping the transform is its only effect
        ColourSpace::Xyz => Ok(SourceColour::AlreadyPq),
        // a wider display gamut, or ARRI LogC3: postkit linearises each frame
        // with that space's own curve, matrixes it, and the compressor transform
        // stays off
        ColourSpace::P3 | ColourSpace::Rec2020 | ColourSpace::LogC => {
            Ok(SourceColour::DisplayRgbIn(space))
        }
        ColourSpace::Aces | ColourSpace::AcesCg => Err(format!(
            "{space:?} is scene-referred: no matrix reaches X'Y'Z' from it, so it needs \
             a rendering transform. Pass --source-lut a 3D LUT that lands on X'Y'Z', or \
             convert the source first with `imfwizard aces --idt <IDT> --odt <ODT>`"
        )),
    }
}

/// Refuse a 3D LUT that is not on disk, since ffmpeg only reads it once the
/// decode has started and reports it as a filter failure.
pub fn reject_missing_lut(colour: &SourceColour) -> Result<(), String> {
    let SourceColour::DciLut(lut) = colour else {
        return Ok(());
    };
    if lut.is_file() {
        return Ok(());
    }
    Err(format!(
        "--source-lut {} is not a file: it takes a 3D LUT (.cube) that converts the \
         source to X'Y'Z'",
        lut.display()
    ))
}

/// Refuse a source colour that asks the encoder for something when the picture
/// arrives already compressed, since `create` passes a J2K directory straight to
/// the wrapper and the request would otherwise be dropped without a word.
pub fn reject_on_precompressed_picture(
    picture: &std::path::Path,
    colour: &SourceColour,
) -> Result<(), String> {
    let precompressed =
        postkit::encode::detect_input_type(picture) == postkit::encode::InputType::J2kSequence;
    if !precompressed {
        return Ok(());
    }
    // exhaustive on purpose: a new postkit SourceColour has to be classified here
    // rather than silently counting as "asks the encoder for nothing"
    let asks_the_encoder_for_something = match colour {
        SourceColour::DisplayRgb => false,
        SourceColour::DisplayRgbIn(_) | SourceColour::DciLut(_) | SourceColour::AlreadyPq => true,
    };
    if asks_the_encoder_for_something {
        return Err(format!(
            "{} is already J2K, so there is no encode for a source colour space to change",
            picture.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_postkit_colour_space_has_a_spelling() {
        for (value, space) in [
            ("rec709", ColourSpace::Rec709),
            ("p3", ColourSpace::P3),
            ("xyz", ColourSpace::Xyz),
            ("rec2020", ColourSpace::Rec2020),
            ("aces", ColourSpace::Aces),
            ("acescg", ColourSpace::AcesCg),
            ("logc", ColourSpace::LogC),
        ] {
            assert_eq!(parse(value).unwrap(), space);
            assert_eq!(parse(&value.to_uppercase()).unwrap(), space);
        }
    }

    /// dcpomatic's rec601/rec1886/sgamut3 have no postkit transform, so they are
    /// not spellings this accepts.
    #[test]
    fn a_space_postkit_cannot_transform_is_not_a_spelling() {
        for value in ["rec601", "rec1886", "sgamut3", ""] {
            assert!(parse(value).is_err(), "{value} should not parse");
        }
    }

    /// The rec709 default has to reach the encoder as the same `SourceColour` the
    /// pipeline used before the flag existed, or today's output shifts.
    #[test]
    fn rec709_is_the_encode_configuration_the_pipeline_already_had() {
        assert_eq!(
            to_source_colour(ColourSpace::Rec709).unwrap(),
            SourceColour::DisplayRgb
        );
        assert_eq!(
            to_source_colour(ColourSpace::Rec709).unwrap(),
            postkit::pipeline::EncodeRunOptions::default().source_colour
        );
    }

    #[test]
    fn xyz_skips_every_colour_transform() {
        assert_eq!(
            to_source_colour(ColourSpace::Xyz).unwrap(),
            SourceColour::AlreadyPq
        );
    }

    #[test]
    fn a_wide_gamut_space_carries_its_own_transform_into_the_encode() {
        for space in [ColourSpace::P3, ColourSpace::Rec2020] {
            assert_eq!(
                to_source_colour(space).unwrap(),
                SourceColour::DisplayRgbIn(space)
            );
        }
    }

    /// postkit's `DcdmTransform` decodes LogC3 ahead of the matrix, so LogC takes
    /// the same per-frame route P3 and Rec.2020 take.
    #[test]
    fn logc_takes_the_per_frame_transform() {
        assert_eq!(
            to_source_colour(ColourSpace::LogC).unwrap(),
            SourceColour::DisplayRgbIn(ColourSpace::LogC)
        );
        assert!(postkit::colour::DcdmTransform::to_xyz(ColourSpace::LogC).is_ok());
    }

    #[test]
    fn the_scene_referred_spaces_are_refused_by_name_with_both_routes() {
        for space in [ColourSpace::Aces, ColourSpace::AcesCg] {
            let error = to_source_colour(space).unwrap_err();
            assert!(error.contains(&format!("{space:?}")), "{error}");
            assert!(error.contains("--source-lut"), "{error}");
            assert!(error.contains("imfwizard aces"), "{error}");
        }
    }

    #[test]
    fn a_lut_that_is_not_there_is_refused_by_path() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("nothing.cube");
        let error = reject_missing_lut(&SourceColour::DciLut(missing.clone())).unwrap_err();
        assert!(error.contains(&missing.display().to_string()), "{error}");

        let present = directory.path().join("identity.cube");
        std::fs::write(&present, "LUT_3D_SIZE 2\n").unwrap();
        assert!(reject_missing_lut(&SourceColour::DciLut(present)).is_ok());
        assert!(reject_missing_lut(&SourceColour::DisplayRgb).is_ok());
    }
}
