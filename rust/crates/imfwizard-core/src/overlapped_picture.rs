//! Writing a composition's picture MXF while its encode runs, instead of reading
//! the whole J2K directory back once the encode is over.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// What decides whether a composition's picture can be wrapped as it encodes.
/// Every field is known before the encode starts.
pub struct PictureJob {
    /// What postkit classified the picture source as.
    pub input_type: postkit::encode::InputType,
    /// The source is one image held for a run of frames.
    pub still_hold: bool,
}

/// Why this picture cannot be wrapped as it encodes, or None when it can.
///
/// postkit hands asdcplib one codestream at a time as the in-process encoder
/// finishes it, so the overlap holds only where every frame the encoder produces
/// is a frame the composition ships.
pub fn overlap_refusal(job: &PictureJob) -> Option<&'static str> {
    if job.still_hold {
        return Some(
            "a held still is encoded once and its codestream linked for the rest of the hold, so \
             nothing feeds the wrap frame by frame",
        );
    }
    if job.input_type != postkit::encode::InputType::Video {
        return Some(
            "only a video source is decoded frame by frame: a J2K sequence is never encoded, and \
             an image sequence can go straight to grk_compress",
        );
    }
    None
}

/// Where a composition's picture MXF goes when it is written as the encode runs,
/// and what it declares. The file name and the asset id are minted inside
/// [`encode_and_wrap_picture`] so they match what `create_imp` would have written.
pub struct PictureWrapTarget {
    /// The IMP directory the MXF is written straight into.
    pub imp_dir: std::path::PathBuf,
    pub fps_num: u32,
    pub fps_den: u32,
    /// The colour the picture signals, from [`crate::mxf_wrap::picture_colour`].
    /// Not optional: an AS-02 picture wrap that signals none is refused.
    pub colour: asdcplib::jp2k::HdrMetadata,
}

/// Encode a composition's picture and write its AS-02 picture MXF as the frames
/// finish, returning the track file [`crate::imp::create_imp`] would otherwise
/// have wrapped for itself. Pass it as `Composition::picture_mxf`; the J2K
/// codestreams stay behind in `encode_dir` as they always did.
#[allow(clippy::too_many_arguments)]
pub fn encode_and_wrap_picture(
    video: &Path,
    encode_dir: &Path,
    options: &postkit::pipeline::EncodeRunOptions,
    target: PictureWrapTarget,
    cancel: &Arc<AtomicBool>,
    pause: &Arc<AtomicBool>,
    on_progress: impl Fn(&postkit::pipeline::PipelineProgress),
    on_log: impl Fn(&str),
) -> Result<(postkit::pipeline::EncodeResult, crate::MxfTrackFile), String> {
    std::fs::create_dir_all(&target.imp_dir)
        .map_err(|e| format!("cannot create {}: {e}", target.imp_dir.display()))?;
    let asset_uuid = uuid::Uuid::new_v4();
    let (encode, track) = postkit::pipeline::run_encode_and_wrap_picture(
        video,
        encode_dir,
        options,
        postkit::mxf_wrap::IncrementalWrapOptions {
            output: crate::imp::track_file_path(
                &target.imp_dir,
                crate::imp::PICTURE_PREFIX,
                &asset_uuid,
            ),
            standard: postkit::mxf_wrap::MxfStandard::As02,
            fps_num: target.fps_num,
            fps_den: target.fps_den,
            encryption: None,
            hdr: Some(target.colour),
            asset_uuid: Some(*asset_uuid.as_bytes()),
        },
        cancel,
        pause,
        on_progress,
        on_log,
    )?;
    Ok((encode, crate::mxf_wrap::track_file_from_postkit(track)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video() -> PictureJob {
        PictureJob {
            input_type: postkit::encode::InputType::Video,
            still_hold: false,
        }
    }

    #[test]
    fn a_plain_video_qualifies() {
        assert_eq!(overlap_refusal(&video()), None);
    }

    #[test]
    fn everything_that_does_not_stream_every_packaged_frame_is_refused() {
        for job in [
            PictureJob {
                still_hold: true,
                ..video()
            },
            PictureJob {
                input_type: postkit::encode::InputType::J2kSequence,
                ..video()
            },
            PictureJob {
                input_type: postkit::encode::InputType::ImageSequence,
                ..video()
            },
        ] {
            assert!(overlap_refusal(&job).is_some());
        }
    }
}
