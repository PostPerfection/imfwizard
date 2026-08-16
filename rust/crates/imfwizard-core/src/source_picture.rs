//! What happens to the source picture before it is compressed, and the raster
//! the encode lands on.
//!
//! postkit does the arithmetic: this module holds the policy around it, which
//! flag combinations are legal and what the picture is fitted into, so the CLI
//! and the GUI cannot answer either differently. The resolved plan's output size
//! is the raster the App 2E check has to run on, since that is what the encoder
//! writes and the wrapper sees.

use std::path::{Path, PathBuf};

use postkit::encode::{DecodeSource, InputType, detect_input_type, find_source_frames};
use postkit::picture_processing::{
    Crop, Fit, PicturePlan, PictureProcessing, Rotation, detect_black_borders,
};

/// Frames `--auto-crop` measures before it unions their content rectangles.
const AUTO_CROP_SAMPLE_COUNT: u32 = 8;

/// Fraction of full scale a pixel stays under to count as border.
pub const DEFAULT_AUTO_CROP_THRESHOLD: f32 = 0.1;

/// Edit rate the auto-crop concat list holds each still at. Only the total
/// duration it implies matters, since detection seeks through the list.
const AUTO_CROP_LIST_FPS: u32 = 24;

/// Every raster App 2E allows, as `--raster` spells them.
const RASTER_SPELLINGS: &str = "1920x1080, 2048x1080, 3840x2160 or 4096x2160";

/// Separator between the two numbers of a `--raster` value.
const RASTER_SEPARATOR: char = 'x';

/// Everything the picture flags ask for, before a source size turns it into a
/// plan. Crops are in source pixels, in the source's own orientation.
#[derive(Debug, Clone, PartialEq)]
pub struct SourcePictureOptions {
    pub crop: Crop,
    pub auto_crop: bool,
    pub auto_crop_threshold: f32,
    pub fill_crop: bool,
    pub deinterlace: bool,
    pub denoise: bool,
    pub rotation: Rotation,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    /// Raster the picture is fitted into. None keeps the source's own.
    pub raster: Option<(u32, u32)>,
}

impl Default for SourcePictureOptions {
    fn default() -> Self {
        Self {
            crop: Crop::default(),
            auto_crop: false,
            auto_crop_threshold: DEFAULT_AUTO_CROP_THRESHOLD,
            fill_crop: false,
            deinterlace: false,
            denoise: false,
            rotation: Rotation::None,
            flip_horizontal: false,
            flip_vertical: false,
            raster: None,
        }
    }
}

impl SourcePictureOptions {
    /// Whether nothing was asked for, so the source encodes as it decodes.
    pub fn is_default(&self) -> bool {
        *self == SourcePictureOptions::default()
    }
}

/// The processing, the sizes it resolves to, and the raster the encoder writes.
#[derive(Debug, Clone)]
pub struct ResolvedPicture {
    pub processing: PictureProcessing,
    pub plan: PicturePlan,
    pub encode_width: u32,
    pub encode_height: u32,
}

/// Turn the flags into a plan against a source of the given size.
///
/// The picture is fitted into `options.raster` when one is named and into the
/// source raster otherwise, so the encode always lands either on an App 2E
/// raster or on the source untouched.
pub fn resolve_picture(
    options: &SourcePictureOptions,
    source: &Path,
    source_width: u32,
    source_height: u32,
    is_image_sequence: bool,
) -> Result<ResolvedPicture, String> {
    let deciders: Vec<&str> = [
        (
            !options.crop.is_none(),
            "--crop-left/--crop-right/--crop-top/--crop-bottom",
        ),
        (options.auto_crop, "--auto-crop"),
        (options.fill_crop, "--fill-crop"),
    ]
    .into_iter()
    .filter(|(given, _)| *given)
    .map(|(_, name)| name)
    .collect();
    if deciders.len() > 1 {
        return Err(format!(
            "{} each decide the crop, so give only one of them",
            deciders.join(" and ")
        ));
    }

    let (target_width, target_height) = match options.raster {
        Some((width, height)) => {
            require_app2e_raster(width, height)?;
            (width, height)
        }
        None => (source_width, source_height),
    };

    let crop = if options.auto_crop {
        detect_crop(
            source,
            options.auto_crop_threshold,
            is_image_sequence,
            source_width,
            source_height,
        )?
    } else if options.fill_crop {
        // the crop runs before the turn, so a quarter turn means filling the
        // target's aspect the other way up
        let (aspect_width, aspect_height) = match options.rotation {
            Rotation::Clockwise90 | Rotation::CounterClockwise90 => (target_height, target_width),
            Rotation::None | Rotation::Half => (target_width, target_height),
        };
        Crop::to_aspect(source_width, source_height, aspect_width, aspect_height)
    } else {
        options.crop
    };

    let mut processing = PictureProcessing {
        deinterlace: options.deinterlace,
        denoise: options.denoise,
        crop,
        rotation: options.rotation,
        flip_horizontal: options.flip_horizontal,
        flip_vertical: options.flip_vertical,
        fit: None,
    };
    let changes_the_raster = (target_width, target_height) != (source_width, source_height);
    if !processing.is_identity() || changes_the_raster {
        processing.fit = Some(Fit {
            box_width: target_width,
            box_height: target_height,
            raster_width: target_width,
            raster_height: target_height,
        });
    }

    let plan = processing.plan(source_width, source_height)?;
    Ok(ResolvedPicture {
        processing,
        encode_width: plan.output_width,
        encode_height: plan.output_height,
        plan,
    })
}

/// Refuse picture processing on a source that never decodes: `create` hands a
/// J2K directory straight to the wrapper, so a crop would be dropped in silence.
pub fn reject_on_precompressed_picture(
    picture: &Path,
    options: &SourcePictureOptions,
) -> Result<(), String> {
    if options.is_default() || detect_input_type(picture) != InputType::J2kSequence {
        return Ok(());
    }
    Err(format!(
        "{} is already J2K, so there are no frames to crop, rotate or fit",
        picture.display()
    ))
}

/// The size the plan is measured against: the container's own raster, or the
/// first frame of an image sequence.
pub fn source_raster(picture: &Path) -> Result<(u32, u32), String> {
    let measured = match detect_input_type(picture) {
        InputType::ImageSequence if picture.is_dir() => first_frame(picture)?,
        _ => picture.to_path_buf(),
    };
    let info = crate::probe::probe_video(&measured).ok_or_else(|| {
        format!(
            "cannot read the picture size of {}, so the picture processing cannot be planned",
            measured.display()
        )
    })?;
    Ok((info.width, info.height))
}

/// Read a `--rotate` value: whole clockwise quarter turns.
pub fn parse_rotation(value: &str) -> Result<Rotation, String> {
    match value.trim() {
        "0" => Ok(Rotation::None),
        "90" => Ok(Rotation::Clockwise90),
        "180" => Ok(Rotation::Half),
        "270" => Ok(Rotation::CounterClockwise90),
        _ => Err(format!(
            "unknown rotation '{value}' (expected 90, 180 or 270, clockwise)"
        )),
    }
}

/// Read a `--flip` value into (horizontal, vertical).
pub fn parse_flip(value: &str) -> Result<(bool, bool), String> {
    match value.trim().to_lowercase().as_str() {
        "horizontal" => Ok((true, false)),
        "vertical" => Ok((false, true)),
        "both" => Ok((true, true)),
        _ => Err(format!(
            "unknown flip '{value}' (expected horizontal, vertical or both)"
        )),
    }
}

/// Read a `--raster` value, which has to be one of the App 2E rasters.
pub fn parse_raster(value: &str) -> Result<(u32, u32), String> {
    let unreadable = || {
        format!("cannot read raster '{value}': spell it WIDTHxHEIGHT, one of {RASTER_SPELLINGS}")
    };
    let (width, height) = value
        .trim()
        .split_once(RASTER_SEPARATOR)
        .ok_or_else(unreadable)?;
    let width: u32 = width.trim().parse().map_err(|_| unreadable())?;
    let height: u32 = height.trim().parse().map_err(|_| unreadable())?;
    require_app2e_raster(width, height)?;
    Ok((width, height))
}

fn require_app2e_raster(width: u32, height: u32) -> Result<(), String> {
    if crate::mxf_wrap::APP2E_RASTERS.contains(&(width, height)) {
        return Ok(());
    }
    Err(format!(
        "raster {width}x{height} is not one App 2E allows: pick {RASTER_SPELLINGS}"
    ))
}

/// Measure the black borders of a source, taking an image sequence through a
/// concat list so cropdetect sees the same stream the encode will.
fn detect_crop(
    source: &Path,
    threshold: f32,
    is_image_sequence: bool,
    source_width: u32,
    source_height: u32,
) -> Result<Crop, String> {
    if !(0.0..=1.0).contains(&threshold) {
        return Err(format!(
            "black threshold {threshold} is outside 0..1, where 0.1 is the usual value"
        ));
    }
    let detected = if is_image_sequence {
        let directory = if source.is_dir() {
            source.to_path_buf()
        } else {
            source.parent().unwrap_or(source).to_path_buf()
        };
        let frames = find_source_frames(&directory)
            .map_err(|error| format!("cannot list {}: {error}", directory.display()))?;
        if frames.is_empty() {
            return Err(format!("no images in {}", directory.display()));
        }
        let list = std::env::temp_dir().join(format!(
            "imfwizard-auto-crop-{}.ffconcat",
            std::process::id()
        ));
        postkit::encode::write_image_concat_list(&frames, AUTO_CROP_LIST_FPS, &list)?;
        let detected = detect_black_borders(
            &list,
            DecodeSource::ImageList,
            threshold,
            AUTO_CROP_SAMPLE_COUNT,
        );
        let _ = std::fs::remove_file(&list);
        detected?
    } else {
        detect_black_borders(
            source,
            DecodeSource::Video,
            threshold,
            AUTO_CROP_SAMPLE_COUNT,
        )?
    };
    if detected.left + detected.right >= source_width
        || detected.top + detected.bottom >= source_height
    {
        return Err(format!(
            "black border detection found no picture in {}: it is black at a threshold of {threshold}",
            source.display()
        ));
    }
    Ok(detected)
}

fn first_frame(directory: &Path) -> Result<PathBuf, String> {
    let frames = find_source_frames(directory)
        .map_err(|error| format!("cannot list {}: {error}", directory.display()))?;
    frames
        .into_iter()
        .next()
        .ok_or_else(|| format!("no images in {}", directory.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> SourcePictureOptions {
        SourcePictureOptions::default()
    }

    fn resolve(options: &SourcePictureOptions) -> Result<ResolvedPicture, String> {
        resolve_picture(options, Path::new("clip.mov"), 1920, 1080, false)
    }

    #[test]
    fn the_default_leaves_the_source_alone() {
        assert!(options().is_default());
        let resolved = resolve(&options()).unwrap();
        assert!(resolved.processing.is_identity());
        assert!(resolved.plan.is_identity());
        assert_eq!(
            (resolved.encode_width, resolved.encode_height),
            (1920, 1080)
        );
    }

    #[test]
    fn the_three_crops_refuse_each_other_by_name() {
        let manual_and_fill = SourcePictureOptions {
            crop: Crop {
                left: 10,
                ..Crop::default()
            },
            fill_crop: true,
            ..options()
        };
        let error = resolve(&manual_and_fill).unwrap_err();
        assert!(error.contains("--crop-left"), "{error}");
        assert!(error.contains("--fill-crop"), "{error}");

        let auto_and_fill = SourcePictureOptions {
            auto_crop: true,
            fill_crop: true,
            ..options()
        };
        let error = resolve(&auto_and_fill).unwrap_err();
        assert!(error.contains("--auto-crop"), "{error}");
        assert!(error.contains("--fill-crop"), "{error}");

        // each on its own is fine, and only the fill one needs no ffmpeg
        assert!(
            resolve(&SourcePictureOptions {
                fill_crop: true,
                raster: Some((2048, 1080)),
                ..options()
            })
            .is_ok()
        );
    }

    #[test]
    fn a_raster_has_to_be_one_app_2e_allows() {
        let error = resolve(&SourcePictureOptions {
            raster: Some((1998, 1080)),
            ..options()
        })
        .unwrap_err();
        assert!(error.contains("1998x1080"), "{error}");
        assert!(error.contains("2048x1080"), "{error}");

        for raster in [(1920, 1080), (2048, 1080), (3840, 2160), (4096, 2160)] {
            assert_eq!(
                parse_raster(&format!("{}x{}", raster.0, raster.1)),
                Ok(raster)
            );
        }
        assert!(parse_raster("1920").is_err());
        assert!(parse_raster("1920x").is_err());
        assert!(parse_raster("1998x1080").is_err());
    }

    /// The fit is what keeps the encode on a legal raster, so it has to appear
    /// as soon as anything changes the picture or the raster differs.
    #[test]
    fn anything_but_the_identity_fits_into_the_target_raster() {
        let deinterlaced = resolve(&SourcePictureOptions {
            deinterlace: true,
            ..options()
        })
        .unwrap();
        assert_eq!(
            deinterlaced.processing.fit,
            Some(Fit {
                box_width: 1920,
                box_height: 1080,
                raster_width: 1920,
                raster_height: 1080,
            })
        );
        assert_eq!(
            (deinterlaced.encode_width, deinterlaced.encode_height),
            (1920, 1080)
        );

        // a quarter turn stays on the source raster, pillarboxed
        let turned = resolve(&SourcePictureOptions {
            rotation: Rotation::Clockwise90,
            ..options()
        })
        .unwrap();
        assert_eq!((turned.encode_width, turned.encode_height), (1920, 1080));
        assert!(turned.plan.pad_left > 0, "{}", turned.plan.describe());

        // a named raster fits into that one instead
        let scaled = resolve(&SourcePictureOptions {
            raster: Some((4096, 2160)),
            ..options()
        })
        .unwrap();
        assert_eq!((scaled.encode_width, scaled.encode_height), (4096, 2160));
        assert_eq!(
            (scaled.plan.scaled_width, scaled.plan.scaled_height),
            (3840, 2160)
        );
    }

    #[test]
    fn a_fill_crop_reaches_the_target_aspect_full_frame() {
        let filled = resolve(&SourcePictureOptions {
            fill_crop: true,
            raster: Some((2048, 1080)),
            ..options()
        })
        .unwrap();
        assert_eq!((filled.encode_width, filled.encode_height), (2048, 1080));
        // the picture reaches both edges, bar the row even rounding can leave
        assert_eq!(filled.plan.scaled_width, filled.encode_width);
        assert!(filled.plan.pad_top <= 1, "{}", filled.plan.describe());

        // the crop runs before the turn, so a turned fill crops the other way
        let turned = resolve(&SourcePictureOptions {
            fill_crop: true,
            rotation: Rotation::Clockwise90,
            raster: Some((2048, 1080)),
            ..options()
        })
        .unwrap();
        assert!(turned.plan.crop.left > 0, "{}", turned.plan.describe());
        assert_eq!(turned.plan.crop.top, 0, "{}", turned.plan.describe());
        assert_eq!(turned.plan.pad_left, 0, "{}", turned.plan.describe());
    }

    #[test]
    fn a_crop_that_eats_the_picture_fails_loud() {
        let error = resolve(&SourcePictureOptions {
            crop: Crop {
                left: 960,
                right: 960,
                top: 0,
                bottom: 0,
            },
            ..options()
        })
        .unwrap_err();
        assert!(error.contains("leaves nothing"), "{error}");
    }

    #[test]
    fn the_rotation_and_flip_spellings_are_the_ones_the_help_names() {
        assert_eq!(parse_rotation("90"), Ok(Rotation::Clockwise90));
        assert_eq!(parse_rotation("180"), Ok(Rotation::Half));
        assert_eq!(parse_rotation("270"), Ok(Rotation::CounterClockwise90));
        assert!(parse_rotation("45").unwrap_err().contains("45"));

        assert_eq!(parse_flip("horizontal"), Ok((true, false)));
        assert_eq!(parse_flip("Vertical"), Ok((false, true)));
        assert_eq!(parse_flip("both"), Ok((true, true)));
        assert!(parse_flip("sideways").unwrap_err().contains("sideways"));
    }

    #[test]
    fn precompressed_picture_refuses_processing_and_passes_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let frames = dir.path().join("j2k");
        std::fs::create_dir_all(&frames).unwrap();
        std::fs::write(frames.join("frame_00000000.j2c"), b"codestream").unwrap();

        assert!(reject_on_precompressed_picture(&frames, &options()).is_ok());
        let error = reject_on_precompressed_picture(
            &frames,
            &SourcePictureOptions {
                deinterlace: true,
                ..options()
            },
        )
        .unwrap_err();
        assert!(error.contains("already J2K"), "{error}");
    }
}
