//! The colour space the picture source carries, mapped onto the encode path.
//!
//! An App 2E picture ships RGB samples in the space its essence descriptor
//! names, and the descriptor imfwizard writes names Rec.709 (or the `--hdr`
//! preset's PQ primaries). So the encode compresses the decoded RGB untouched:
//! Rec.709 is `SourceColour::KeepRgb`, and every other spelling is refused by
//! name rather than encoded into essence whose descriptor lies about it.
//!
//! X'Y'Z' is refused because a DCI codestream is a DCP picture, which the AS-02
//! wrap will not carry at all. P3, Rec.2020, LogC, ACES and ACEScg are refused
//! because the only transform postkit has for them lands on X'Y'Z': there is no
//! RGB-to-RGB gamut conversion that would reach Rec.709 from them.

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
        ColourSpace::P3 | ColourSpace::P3D65 | ColourSpace::Rec2020 | ColourSpace::LogC => {
            Err(format!(
                "{space:?} would have to be converted to the Rec.709 RGB an App 2E picture \
                 declares, and the only transform postkit has for it lands on X'Y'Z'. Convert \
                 the source to Rec.709 RGB first"
            ))
        }
        ColourSpace::Aces | ColourSpace::AcesCg => Err(format!(
            "{space:?} is scene-referred: it needs a rendering transform, and the one \
             postkit has lands on X'Y'Z' rather than the Rec.709 RGB an App 2E picture \
             declares. Convert the source first with `imfwizard aces --idt <IDT> --odt <ODT>`"
        )),
    }
}

/// Refuse `--source-lut`, which lands the decoded frames on DCI X'Y'Z'.
///
/// The LUT runs in the decode and nothing transforms the frames again, so the
/// codestreams would hold X'Y'Z' samples under a descriptor declaring Rec.709
/// RGB. Kept as a refusal rather than dropped so the flag says why.
pub fn reject_dci_lut(colour: &SourceColour) -> Result<(), String> {
    let SourceColour::DciLut(lut) = colour else {
        return Ok(());
    };
    Err(format!(
        "--source-lut {} converts the source to DCI X'Y'Z', which an App 2E picture \
         cannot carry: its essence descriptor declares Rec.709 RGB. Apply the LUT to the \
         source first, or package the result as a DCP with dcpwizard",
        lut.display()
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

    /// Every space whose only postkit transform lands on X'Y'Z' is refused by
    /// name, since nothing here converts it to the Rec.709 RGB the descriptor
    /// declares.
    #[test]
    fn a_space_with_no_route_to_rec709_rgb_is_refused_by_name() {
        for space in [
            ColourSpace::P3,
            ColourSpace::Rec2020,
            ColourSpace::LogC,
            ColourSpace::Aces,
            ColourSpace::AcesCg,
        ] {
            let error = to_source_colour(space).unwrap_err();
            assert!(error.contains(&format!("{space:?}")), "{error}");
            assert!(error.contains("Rec.709 RGB"), "{error}");
        }
    }

    #[test]
    fn the_scene_referred_spaces_name_the_route_that_converts_them() {
        for space in [ColourSpace::Aces, ColourSpace::AcesCg] {
            let error = to_source_colour(space).unwrap_err();
            assert!(error.contains("imfwizard aces"), "{error}");
        }
    }

    /// The LUT lands the frames on X'Y'Z' and nothing transforms them again, so
    /// the codestreams would contradict the descriptor.
    #[test]
    fn a_dci_lut_is_refused_by_path() {
        let directory = tempfile::tempdir().unwrap();
        let lut = directory.path().join("hdr_to_dci.cube");
        std::fs::write(&lut, "LUT_3D_SIZE 2\n").unwrap();
        let error = reject_dci_lut(&SourceColour::DciLut(lut.clone())).unwrap_err();
        assert!(error.contains(&lut.display().to_string()), "{error}");
        assert!(error.contains("X'Y'Z'"), "{error}");

        assert!(reject_dci_lut(&SourceColour::KeepRgb).is_ok());
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
