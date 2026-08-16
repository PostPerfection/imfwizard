//! The colour space the picture source carries, mapped onto the encode path.
//!
//! postkit decides the encoder transform from `encode::SourceColour`. Rec.709
//! takes the built-in Rec.709 to DCI X'Y'Z' transform and X'Y'Z' leaves the
//! frames alone. postkit also carries `DisplayRgbIn`, a per-space transform that
//! would cover the five wide-gamut and log spaces, but nothing here builds one
//! yet, so those spaces are refused rather than encoded through the Rec.709
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
        ColourSpace::P3
        | ColourSpace::Rec2020
        | ColourSpace::Aces
        | ColourSpace::AcesCg
        | ColourSpace::LogC => Err(format!(
            "no {space:?} to X'Y'Z' transform is available here: only Rec.709 is wired up, \
             and converting {space:?} through the Rec.709 matrix would be wrong colour. \
             Convert the source to Rec.709 or X'Y'Z' first"
        )),
    }
}

/// Whether the encoder applies its own transform to frames in this space, which
/// decides whether an HDR label would describe essence the encoder had already
/// rewritten.
pub fn applies_encoder_transform(space: ColourSpace) -> bool {
    to_source_colour(space).is_ok_and(|colour| colour.applies_xyz_transform())
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
        // nothing here builds one, so this arm only fires if postkit's per-space
        // transform ever reaches imfwizard, and it is unsupported when it does
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
        assert!(applies_encoder_transform(ColourSpace::Rec709));
    }

    #[test]
    fn xyz_skips_every_colour_transform() {
        assert_eq!(
            to_source_colour(ColourSpace::Xyz).unwrap(),
            SourceColour::AlreadyPq
        );
        assert!(!applies_encoder_transform(ColourSpace::Xyz));
    }

    #[test]
    fn the_spaces_with_no_transform_are_refused_by_name() {
        for space in [
            ColourSpace::P3,
            ColourSpace::Rec2020,
            ColourSpace::Aces,
            ColourSpace::AcesCg,
            ColourSpace::LogC,
        ] {
            let error = to_source_colour(space).unwrap_err();
            assert!(error.contains(&format!("{space:?}")), "{error}");
        }
    }
}
