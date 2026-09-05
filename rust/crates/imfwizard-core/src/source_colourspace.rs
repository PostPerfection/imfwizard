//! The colour space the picture source carries, mapped onto the encode path.
//!
//! An App 2E picture ships RGB samples in the space its essence descriptor
//! names, and the descriptor imfwizard writes names Rec.709 (or the `--hdr`
//! preset's PQ primaries). Rec.709 is `SourceColour::KeepRgb` and is compressed
//! untouched. P3-D65, Rec.2020 and LogC are `SourceColour::KeepRgbFrom`, which
//! converts every frame to Rec.709 RGB before compression.
//!
//! X'Y'Z' is refused because a DCI codestream is a DCP picture, which the AS-02
//! wrap will not carry at all. ACES and ACEScg are refused because reaching
//! Rec.709 from them needs a rendering transform, and P3 with the DCI white is
//! refused because nothing here adapts the white point.

use postkit::colour::ColourSpace;
use postkit::encode::SourceColour;

/// Spell a colour space as the CLI and GUI take it. The set is postkit's
/// `ColourSpace`; dcpwizard spells the values the same way.
pub fn parse(value: &str) -> Result<ColourSpace, String> {
    postkit::colour::parse_colour_space(value)
        .ok_or_else(|| format!("unknown source colour space '{value}' (expected {VALUES})"))
}

/// Every value `parse` accepts, in the order the help text lists them.
pub const VALUES: &str =
    "rec709, p3d65, rec2020 or logc, with p3, xyz, aces and acescg refused by name";

/// The colour an App 2E picture essence descriptor declares, and therefore the
/// only space a source may already be in.
pub const APP2E_SOURCE_SPACE: ColourSpace = ColourSpace::Rec709;

/// How the encoder should treat frames carrying this colour.
pub fn to_source_colour(space: ColourSpace) -> Result<SourceColour, String> {
    match space {
        // the samples ship in the space the descriptor names, so the encoder
        // compresses the decoded RGB as it stands
        ColourSpace::Rec709 => Ok(SourceColour::KeepRgb),
        ColourSpace::Xyz => Err(
            "xyz is DCI X'Y'Z', a DCP picture: an App 2E track file carries RGB, and the \
             AS-02 wrap refuses a cinema codestream. Convert the source to Rec.709 RGB \
             first, or package it as a DCP with dcpwizard"
                .to_string(),
        ),
        // converted to the Rec.709 RGB the essence descriptor declares
        ColourSpace::P3D65 | ColourSpace::Rec2020 | ColourSpace::LogC => {
            Ok(SourceColour::KeepRgbFrom(space))
        }
        ColourSpace::P3 => Err(
            "p3 is P3 with the DCI white point, and converting it to Rec.709 without \
             adapting the white leaves a green cast: name p3d65 for a D65 master, or \
             convert the source first"
                .to_string(),
        ),
        ColourSpace::Aces | ColourSpace::AcesCg => Err(format!(
            "{space:?} is scene-referred: reaching the Rec.709 RGB an App 2E picture \
             declares needs a rendering transform, which is not a matrix. Convert the \
             source first with `imfwizard aces --idt <IDT> --odt <ODT>`"
        )),
    }
}

pub fn reject_converting_source_under_hdr(colour: &SourceColour) -> Result<(), String> {
    let flag = match colour {
        SourceColour::KeepRgbFrom(space) => format!("--source-colourspace {space:?}"),
        SourceColour::KeepRgbAfterLut(lut) => format!("--source-lut {}", lut.display()),
        _ => return Ok(()),
    };
    Err(format!(
        "{flag} converts the source to Rec.709 SDR, which an --hdr picture is not: \
         convert the source to the preset's colour first"
    ))
}

/// Whether the frames reaching the compressor are already X'Y'Z', which is where
/// a display-RGB subtitle burn would land in the wrong colour space.
///
/// `DisplayRgbIn` converts after the burn is composited, so its frames are
/// display RGB when the text is drawn.
pub fn frames_reach_the_compressor_as_xyz(colour: &SourceColour) -> bool {
    match colour {
        SourceColour::KeepRgb
        | SourceColour::DisplayRgb
        | SourceColour::DisplayRgbIn(_)
        | SourceColour::KeepRgbFrom(_)
        | SourceColour::KeepRgbAfterLut(_) => false,
        SourceColour::AlreadyPq | SourceColour::DciLut(_) => true,
    }
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
        SourceColour::KeepRgb | SourceColour::DisplayRgb => false,
        SourceColour::DisplayRgbIn(_)
        | SourceColour::DciLut(_)
        | SourceColour::AlreadyPq
        | SourceColour::KeepRgbFrom(_)
        | SourceColour::KeepRgbAfterLut(_) => true,
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
            ("p3d65", ColourSpace::P3D65),
            ("p3-d65", ColourSpace::P3D65),
            ("displayp3", ColourSpace::P3D65),
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

    /// An App 2E codestream holds the RGB the descriptor declares, so the
    /// encoder's own X'Y'Z' transform must never run.
    #[test]
    fn rec709_reaches_the_encoder_as_untransformed_rgb() {
        let colour = to_source_colour(APP2E_SOURCE_SPACE).unwrap();
        assert_eq!(colour, SourceColour::KeepRgb);
        assert!(!colour.applies_xyz_transform());
        assert!(colour.frame_transform().unwrap().is_none());
    }

    /// A DCI codestream is a DCP picture the AS-02 wrap refuses outright, so the
    /// refusal has to say that rather than offer a conversion that does not exist.
    #[test]
    fn xyz_is_refused_as_a_dcp_picture() {
        let error = to_source_colour(ColourSpace::Xyz).unwrap_err();
        assert!(error.contains("X'Y'Z'"), "{error}");
        assert!(error.contains("App 2E"), "{error}");
    }

    /// Every space postkit can matrix into Rec.709 reaches the encoder as a
    /// conversion, with the encoder's own X'Y'Z' transform off.
    #[test]
    fn a_wide_gamut_space_is_converted_to_rec709_rgb() {
        for space in [ColourSpace::P3D65, ColourSpace::Rec2020, ColourSpace::LogC] {
            let colour = to_source_colour(space).unwrap();
            assert_eq!(colour, SourceColour::KeepRgbFrom(space));
            assert!(!colour.applies_xyz_transform());
            assert!(colour.frame_transform().unwrap().is_some());
        }
    }

    /// P3-DCI's white is not D65, and nothing here adapts it, so the refusal has
    /// to send a D65 master to the spelling that works.
    #[test]
    fn p3_with_the_dci_white_is_refused_naming_p3d65() {
        let error = to_source_colour(ColourSpace::P3).unwrap_err();
        assert!(error.contains("p3d65"), "{error}");
        assert!(error.contains("white"), "{error}");
    }

    #[test]
    fn the_scene_referred_spaces_name_the_route_that_converts_them() {
        for space in [ColourSpace::Aces, ColourSpace::AcesCg] {
            let error = to_source_colour(space).unwrap_err();
            assert!(error.contains(&format!("{space:?}")), "{error}");
            assert!(error.contains("rendering transform"), "{error}");
            assert!(error.contains("imfwizard aces"), "{error}");
        }
    }

    /// Both converting routes land on Rec.709 SDR, which contradicts the PQ an
    /// `--hdr` picture's descriptor declares.
    #[test]
    fn a_converting_source_is_refused_under_hdr() {
        let error =
            reject_converting_source_under_hdr(&SourceColour::KeepRgbFrom(ColourSpace::P3D65))
                .unwrap_err();
        assert!(error.contains("--source-colourspace P3D65"), "{error}");
        assert!(error.contains("--hdr"), "{error}");

        let lut = std::path::PathBuf::from("/luts/to_rec709.cube");
        let error = reject_converting_source_under_hdr(&SourceColour::KeepRgbAfterLut(lut.clone()))
            .unwrap_err();
        assert!(error.contains(&lut.display().to_string()), "{error}");

        assert!(reject_converting_source_under_hdr(&SourceColour::KeepRgb).is_ok());
        assert!(reject_converting_source_under_hdr(&SourceColour::AlreadyPq).is_ok());
    }

    /// A J2K directory never decodes, so a colour that asks the encoder for
    /// nothing has to pass it and one that asks for something has to stop it.
    #[test]
    fn a_precompressed_picture_takes_only_the_colour_the_encoder_ignores() {
        let directory = tempfile::tempdir().unwrap();
        let frames = directory.path().join("j2k");
        std::fs::create_dir_all(&frames).unwrap();
        std::fs::write(
            frames.join("frame_00000000.j2c"),
            crate::mxf_wrap::synthetic_j2k_codestream(1920, 1080, 12),
        )
        .unwrap();

        assert!(reject_on_precompressed_picture(&frames, &SourceColour::KeepRgb).is_ok());
        let error = reject_on_precompressed_picture(&frames, &SourceColour::AlreadyPq).unwrap_err();
        assert!(error.contains("already J2K"), "{error}");
    }
}
