use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use postkit::encode::{FrameRange, FrameRate, InputType, SourceColour, detect_input_type};
use postkit::picture_processing::PictureProcessing;
use postkit::pipeline::{EncodeRunOptions, PipelineProgress};
use postkit::timeline::{Clip, ClipSound, SingleClipBuild, SoundTrack, Timeline};

use crate::MxfTrackFile;
use crate::cpl::{CompositionSegment, SequenceResource};
use crate::imp::{AudioRole, AudioTrack, Composition, ImpOptions, SoundfieldLabels};
use crate::intermediates::ENCODE_SCRATCH_PREFIX;
use crate::source_edits::{CONVERTED_SUBTITLE_PREFIX, FITTED_AUDIO_PREFIX, WIDENED_AUDIO_PREFIX};
use crate::source_picture::SourcePictureOptions;
use crate::subtitle_convert::SubtitleFormat;

const TIMELINE_SOUND_FILE: &str = "timeline_sound.wav";
const TIMELINE_SUBTITLE_FILE: &str = "timeline_subtitles.srt";

/// How every clip's picture is compressed, as the create flags set it.
#[derive(Debug, Clone, Default)]
pub struct PictureCompression {
    /// `--bitrate`, else the delivery preset's ceiling. None is the default ratio.
    pub bitrate_mbps: Option<f64>,
    pub quality_psnr: Option<f64>,
    pub encode_threads: u32,
    pub detect_picture_findings: bool,
}

/// An IMP built from a timeline, with what the create flags say about every clip.
pub struct TimelineImp {
    pub timeline: Timeline,
    pub output_dir: PathBuf,
    pub title: String,
    pub content_kind: String,
    pub picture_options: SourcePictureOptions,
    /// The colour of every clip that does not carry its own.
    pub source_colour: SourceColour,
    pub compression: PictureCompression,
    pub hdr: Option<crate::hdr_wcg::HdrWcg>,
    pub audio_language: Option<String>,
    pub audio_role: Option<AudioRole>,
    pub timed_text_language: Option<String>,
    pub soundfield: SoundfieldLabels,
    pub cancel: Arc<AtomicBool>,
    pub pause: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
pub struct TimelineImpResult {
    pub cpl_path: PathBuf,
    pub pkl_path: PathBuf,
    pub assetmap_path: PathBuf,
    /// One a clip, in clip order.
    pub picture_track_files: Vec<MxfTrackFile>,
    pub sound_track_file: Option<MxfTrackFile>,
    pub frames: u64,
}

// each field is the flag's name when it was given
#[derive(Debug, Default, Clone, Copy)]
pub struct MultiClipFlags {
    pub burn_subtitle: Option<&'static str>,
    pub audio_delay: Option<&'static str>,
    pub audio_map: Option<&'static str>,
    pub atmos: Option<&'static str>,
}

const SOUND_IS_JOINED_FROM_THE_CLIPS: &str = "the sound is joined from the clips";

pub fn refuse_with_multi_clip(flags: &MultiClipFlags) -> Vec<String> {
    [
        (flags.burn_subtitle, None),
        (flags.audio_delay, Some(SOUND_IS_JOINED_FROM_THE_CLIPS)),
        (flags.audio_map, Some(SOUND_IS_JOINED_FROM_THE_CLIPS)),
        (flags.atmos, None),
    ]
    .into_iter()
    .filter_map(|(name, reason)| Some((name?, reason)))
    .map(|(name, reason)| match reason {
        Some(reason) => {
            format!("{name} is not supported on a timeline of more than one clip: {reason}")
        }
        None => format!("{name} is not supported on a timeline of more than one clip"),
    })
    .collect()
}

/// Whether a one-clip timeline builds as `--video` with the clip's trims. That
/// build writes one segment and moves sound and subtitles with the head trim.
pub fn takes_the_single_path(timeline: &Timeline, build: &SingleClipBuild) -> bool {
    let timed_against_the_timeline = matches!(timeline.sound, SoundTrack::Override(_))
        || (build.subtitle.is_some() && !build.subtitle_from_clip);
    build.reel_split_frames.is_empty()
        && (build.trim_start_frames == 0 || !timed_against_the_timeline)
}

fn clip_name(index: usize, clip: &Clip) -> String {
    format!("clip {} ({})", index + 1, clip.picture.display())
}

struct ClipPicture {
    processing: PictureProcessing,
    source_colour: SourceColour,
}

struct TimelinePlan {
    raster: (u32, u32),
    clips: Vec<ClipPicture>,
}

/// Every refusal the build can make, before anything is encoded.
pub fn check_timeline(build: &TimelineImp) -> Result<(), String> {
    plan(build).map(|_| ())
}

fn plan(build: &TimelineImp) -> Result<TimelinePlan, String> {
    let timeline = &build.timeline;
    timeline.check()?;
    postkit::timeline::refuse_two_subtitle_sources(timeline)?;
    if let SoundTrack::Override(sound) = &timeline.sound {
        crate::source_edits::check_sound_depth(sound)?;
    }
    if let Some(subtitle) = &timeline.subtitle {
        crate::subtitle_convert::readable_source_format(subtitle)?;
    }
    let first_clip = &timeline.clips[0];
    let raster = match build.picture_options.raster {
        Some(raster) => raster,
        None => postkit::encode::source_raster(&first_clip.picture)
            .map_err(|e| format!("{}: {e}", clip_name(0, first_clip)))?,
    };
    let clips = timeline
        .clips
        .iter()
        .enumerate()
        .map(|(index, clip)| {
            plan_clip(build, clip, raster).map_err(|e| format!("{}: {e}", clip_name(index, clip)))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(TimelinePlan { raster, clips })
}

fn plan_clip(build: &TimelineImp, clip: &Clip, raster: (u32, u32)) -> Result<ClipPicture, String> {
    let input_type = detect_input_type(&clip.picture);
    if matches!(input_type, InputType::J2kSequence | InputType::PictureMxf) {
        return Err(
            "it is already JPEG 2000, and a timeline built clip by clip takes video or image \
             sequence clips"
                .to_string(),
        );
    }
    if let Some(subtitle) = &clip.subtitle
        && crate::subtitle_convert::readable_source_format(subtitle)? != SubtitleFormat::Srt
    {
        return Err(format!(
            "its subtitle {} is not SRT, the one format clip subtitles are joined in",
            subtitle.display()
        ));
    }
    let source_colour = if clip.source_colour == SourceColour::default() {
        build.source_colour.clone()
    } else {
        clip.source_colour.clone()
    };
    let fitted_options = SourcePictureOptions {
        raster: Some(raster),
        ..build.picture_options.clone()
    };
    crate::preflight::check_before_encode(&crate::preflight::CreatePlan {
        picture: Some(clip.picture.clone()),
        audio_files: match &clip.sound {
            ClipSound::File(path) => vec![path.clone()],
            ClipSound::Embedded | ClipSound::None => Vec::new(),
        },
        audio_language: build.audio_language.clone(),
        timed_text_files: clip.subtitle.iter().cloned().collect(),
        timed_text_language: build.timed_text_language.clone(),
        fps_num: build.timeline.fps.numerator,
        fps_den: build.timeline.fps.denominator,
        picture_options: fitted_options.clone(),
        source_colour: source_colour.clone(),
        hdr: build.hdr.clone(),
        ..Default::default()
    })?;
    let (width, height) = postkit::encode::source_raster(&clip.picture)?;
    if clip.picture_processing.is_identity() {
        let resolved = crate::source_picture::resolve_picture(
            &fitted_options,
            &clip.picture,
            width,
            height,
            input_type == InputType::ImageSequence,
        )?;
        return Ok(ClipPicture {
            processing: resolved.processing,
            source_colour,
        });
    }
    let own = clip.picture_processing.plan(width, height)?;
    if (own.output_width, own.output_height) != raster {
        return Err(format!(
            "its own picture processing lands on {}x{} and the timeline's raster is {}x{}",
            own.output_width, own.output_height, raster.0, raster.1
        ));
    }
    Ok(ClipPicture {
        processing: clip.picture_processing.clone(),
        source_colour,
    })
}

/// A picture track file a clip, one sound and one subtitle track file, and a
/// segment a reel. Scratch is left under the output directory.
pub fn build_timeline_imp(
    build: &TimelineImp,
    on_progress: impl Fn(&PipelineProgress),
) -> Result<TimelineImpResult, String> {
    let plan = plan(build)?;
    let timeline = &build.timeline;
    let output_dir = &build.output_dir;
    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("cannot create {}: {e}", output_dir.display()))?;
    let frames = timeline.total_frames();
    let options = ImpOptions {
        output_dir: output_dir.clone(),
        fps_num: timeline.fps.numerator,
        fps_den: timeline.fps.denominator,
        edit_rate: format!("{} {}", timeline.fps.numerator, timeline.fps.denominator),
        duration: frames,
        soundfield: build.soundfield.clone(),
        cancel: build.cancel.clone(),
        ..Default::default()
    };
    let mut composition = Composition {
        title: build.title.clone(),
        content_kind: build.content_kind.clone(),
        hdr: build.hdr.clone(),
        ..Default::default()
    };

    let encode = picture_encode_options(&build.compression, plan.raster, timeline.fps)?;
    let mut picture_track_files = Vec::new();
    for (index, ((clip, clip_picture), first_frame)) in timeline
        .clips
        .iter()
        .zip(&plan.clips)
        .zip(timeline.clip_starts())
        .enumerate()
    {
        if build.cancel.load(Ordering::Relaxed) {
            return Err(crate::imp::CANCELLED_ERROR.to_string());
        }
        let name = clip_name(index, clip);
        tracing::info!(
            "Encoding {name} frames {}..{} at timeline frame {first_frame}",
            clip.in_frame,
            clip.out_frame
        );
        let clip_options = EncodeRunOptions {
            source_colour: clip_picture.source_colour.clone(),
            picture: clip_picture.processing.clone(),
            frame_range: Some(FrameRange {
                first_frame: clip.in_frame,
                frame_count: clip.out_frame - clip.in_frame,
            }),
            ..encode.clone()
        };
        let on_clip_progress = |progress: &PipelineProgress| {
            on_progress(&PipelineProgress {
                frame: first_frame + progress.frame,
                total_frames: frames,
                ..progress.clone()
            })
        };
        let track = encode_clip_picture(build, clip, index, &clip_options, on_clip_progress)
            .map_err(|e| format!("{name}: {e}"))?;
        picture_track_files.push(track);
    }

    if build.cancel.load(Ordering::Relaxed) {
        return Err(crate::imp::CANCELLED_ERROR.to_string());
    }
    let sound_track_file = wrap_timeline_sound(build, &options, &mut composition, frames)?;
    let timed_text_track_file = wrap_timeline_subtitle(build, &options, frames)?;

    let clip_lengths: Vec<u64> = timeline
        .clips
        .iter()
        .map(|clip| clip.out_frame - clip.in_frame)
        .collect();
    let segments: Vec<CompositionSegment> = segment_spans(&clip_lengths, &timeline.reel_starts)
        .into_iter()
        .map(|span| {
            let whole_span = |track_file: &MxfTrackFile| {
                vec![SequenceResource {
                    track_file: track_file.clone(),
                    entry_point: span.start,
                    source_duration: span.length,
                }]
            };
            CompositionSegment {
                picture: span
                    .clip_pieces
                    .iter()
                    .map(|piece| SequenceResource {
                        track_file: picture_track_files[piece.clip].clone(),
                        entry_point: piece.entry_point,
                        source_duration: piece.length,
                    })
                    .collect(),
                sound: sound_track_file.iter().flat_map(whole_span).collect(),
                timed_text: timed_text_track_file.iter().flat_map(whole_span).collect(),
            }
        })
        .collect();

    let cpl_uuid = uuid::Uuid::new_v4().to_string();
    let cpl_path = output_dir.join(format!("CPL_{cpl_uuid}.xml"));
    crate::cpl::write_sequenced_cpl(&cpl_path, &cpl_uuid, &options, &composition, &segments)
        .map_err(|e| format!("Failed to write CPL: {e}"))?;
    let cpls = vec![crate::imp::CplEntry {
        uuid: cpl_uuid,
        path: cpl_path.clone(),
    }];
    let track_files: Vec<MxfTrackFile> = picture_track_files
        .iter()
        .chain(&sound_track_file)
        .chain(&timed_text_track_file)
        .cloned()
        .collect();
    let pkl_uuid = uuid::Uuid::new_v4().to_string();
    let pkl_path = output_dir.join(format!("PKL_{pkl_uuid}.xml"));
    crate::pkl::write_pkl(&pkl_path, &pkl_uuid, &cpls, &track_files)
        .map_err(|e| format!("Failed to write PKL: {e}"))?;
    let assetmap_path = output_dir.join("ASSETMAP.xml");
    crate::assetmap::write_assetmap(&assetmap_path, &pkl_uuid, &cpls, &track_files)
        .map_err(|e| format!("Failed to write ASSETMAP: {e}"))?;

    Ok(TimelineImpResult {
        cpl_path,
        pkl_path,
        assetmap_path,
        picture_track_files,
        sound_track_file,
        frames,
    })
}

fn picture_encode_options(
    compression: &PictureCompression,
    (width, height): (u32, u32),
    fps: FrameRate,
) -> Result<EncodeRunOptions, String> {
    let rate = fps.as_f64();
    let rsiz = crate::encode::imf_rsiz_for_encode(
        width,
        height,
        rate,
        crate::encode::bitrate_mbps_for_job(compression.bitrate_mbps, None, width, height, rate),
    )?;
    Ok(EncodeRunOptions {
        compression_ratio: crate::encode::DEFAULT_COMPRESSION_RATIO,
        target_codestream_bytes: crate::encode::target_codestream_bytes_for_job(
            compression.bitrate_mbps,
            None,
            rate,
        ),
        quality_psnr: compression.quality_psnr,
        // under a PSNR target the bitrate is a ceiling per frame
        codestream_byte_cap: compression
            .quality_psnr
            .and(compression.bitrate_mbps)
            .map(|mbps| crate::encode::codestream_byte_cap_for_bitrate(rate, mbps)),
        fps,
        rsiz,
        encode_threads: compression.encode_threads,
        detect_picture_findings: compression.detect_picture_findings,
        ..Default::default()
    })
}

fn encode_clip_picture(
    build: &TimelineImp,
    clip: &Clip,
    index: usize,
    options: &EncodeRunOptions,
    on_progress: impl Fn(&PipelineProgress),
) -> Result<MxfTrackFile, String> {
    let encode_dir = build
        .output_dir
        .join(format!("{ENCODE_SCRATCH_PREFIX}{index}"));
    let (encode, track) = crate::overlapped_picture::encode_and_wrap_picture(
        &clip.picture,
        &encode_dir,
        options,
        crate::overlapped_picture::PictureWrapTarget {
            imp_dir: build.output_dir.clone(),
            fps_num: options.fps.numerator,
            fps_den: options.fps.denominator,
            colour: crate::mxf_wrap::picture_colour(build.hdr.as_ref()),
        },
        &build.cancel,
        &build.pause,
        on_progress,
        |message| tracing::info!("{message}"),
    )?;
    for finding in encode.picture_findings.describe(options.fps.as_f64()) {
        tracing::warn!("{finding}");
    }
    let kept = clip.out_frame - clip.in_frame;
    if track.duration != kept {
        return Err(format!(
            "the encode wrapped {} frames where the clip keeps {kept}",
            track.duration
        ));
    }
    Ok(track)
}

fn wrap_timeline_sound(
    build: &TimelineImp,
    options: &ImpOptions,
    composition: &mut Composition,
    frames: u64,
) -> Result<Option<MxfTrackFile>, String> {
    let output_dir = &build.output_dir;
    let rendered = output_dir.join(format!("{ENCODE_SCRATCH_PREFIX}{TIMELINE_SOUND_FILE}"));
    let Some(rendered) = postkit::timeline::render_sound(&build.timeline, &rendered)? else {
        return Ok(None);
    };
    let widened = crate::imp::widened_sound(
        &rendered,
        &output_dir.join(format!("{WIDENED_AUDIO_PREFIX}0.wav")),
    )?;
    let fitted = output_dir.join(format!("{FITTED_AUDIO_PREFIX}0.wav"));
    let sound = match crate::source_edits::fit_audio_to_picture(
        &widened,
        &fitted,
        frames,
        options.fps_num,
        options.fps_den,
    )? {
        None => widened,
        Some(fit) => {
            crate::imp::report_audio_fit(&rendered, &fit, frames);
            fitted
        }
    };
    let track = AudioTrack {
        path: sound.clone(),
        language: build.audio_language.clone(),
        role: build.audio_role,
    };
    let mca = crate::imp::soundfield_config(&track, composition, &build.soundfield)?;
    composition.audio_files = vec![track];
    crate::imp::wrap_one(
        options,
        output_dir,
        &sound,
        crate::EssenceType::Wav,
        None,
        Some(mca),
        frames,
    )
    .map(Some)
    .map_err(|e| format!("Audio wrap failed: {e}"))
}

fn wrap_timeline_subtitle(
    build: &TimelineImp,
    options: &ImpOptions,
    frames: u64,
) -> Result<Option<MxfTrackFile>, String> {
    let output_dir = &build.output_dir;
    let joined = output_dir.join(format!("{ENCODE_SCRATCH_PREFIX}{TIMELINE_SUBTITLE_FILE}"));
    let subtitle = match postkit::timeline::join_subtitles(&build.timeline, &joined)? {
        Some(joined) => joined,
        None => match &build.timeline.subtitle {
            Some(subtitle) => subtitle.clone(),
            None => return Ok(None),
        },
    };
    let timed_text = crate::subtitle_convert::timed_text_as_ttml(
        &subtitle,
        &output_dir.join(format!("{CONVERTED_SUBTITLE_PREFIX}0.ttml")),
        build.timed_text_language.as_deref(),
    )?;
    crate::imp::wrap_one(
        options,
        output_dir,
        &timed_text,
        crate::EssenceType::TimedText,
        None,
        None,
        frames,
    )
    .map(Some)
    .map_err(|e| format!("Subtitle wrap failed: {e}"))
}

#[derive(Debug, PartialEq)]
struct SegmentSpan {
    start: u64,
    length: u64,
    clip_pieces: Vec<ClipPiece>,
}

// the part of one clip's track file a segment plays
#[derive(Debug, PartialEq)]
struct ClipPiece {
    clip: usize,
    entry_point: u64,
    length: u64,
}

fn segment_spans(clip_lengths: &[u64], reel_starts: &[u64]) -> Vec<SegmentSpan> {
    let total: u64 = clip_lengths.iter().sum();
    let mut boundaries = vec![0];
    boundaries.extend_from_slice(reel_starts);
    boundaries.push(total);
    boundaries
        .windows(2)
        .map(|pair| {
            let (start, end) = (pair[0], pair[1]);
            let mut clip_start = 0;
            let mut clip_pieces = Vec::new();
            for (clip, &length) in clip_lengths.iter().enumerate() {
                let clip_end = clip_start + length;
                let first = start.max(clip_start);
                let last = end.min(clip_end);
                if first < last {
                    clip_pieces.push(ClipPiece {
                        clip,
                        entry_point: first - clip_start,
                        length: last - first,
                    });
                }
                clip_start = clip_end;
            }
            SegmentSpan {
                start,
                length: end - start,
                clip_pieces,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(clip: usize, entry_point: u64, length: u64) -> ClipPiece {
        ClipPiece {
            clip,
            entry_point,
            length,
        }
    }

    #[test]
    fn no_reel_start_is_one_segment_of_whole_clips() {
        assert_eq!(
            segment_spans(&[6, 6], &[]),
            vec![SegmentSpan {
                start: 0,
                length: 12,
                clip_pieces: vec![piece(0, 0, 6), piece(1, 0, 6)],
            }]
        );
    }

    #[test]
    fn a_reel_start_inside_a_clip_splits_its_track_file() {
        assert_eq!(
            segment_spans(&[6, 6], &[4]),
            vec![
                SegmentSpan {
                    start: 0,
                    length: 4,
                    clip_pieces: vec![piece(0, 0, 4)],
                },
                SegmentSpan {
                    start: 4,
                    length: 8,
                    clip_pieces: vec![piece(0, 4, 2), piece(1, 0, 6)],
                },
            ]
        );
    }

    #[test]
    fn a_reel_start_on_a_clip_boundary_splits_nothing() {
        let spans = segment_spans(&[6, 3, 5], &[6, 9]);
        let pieces: Vec<&Vec<ClipPiece>> = spans.iter().map(|span| &span.clip_pieces).collect();
        assert_eq!(
            pieces,
            vec![
                &vec![piece(0, 0, 6)],
                &vec![piece(1, 0, 3)],
                &vec![piece(2, 0, 5)]
            ]
        );
    }

    #[test]
    fn sound_timed_against_the_timeline_keeps_a_trimmed_clip_off_the_single_path() {
        let build = SingleClipBuild {
            picture: PathBuf::from("/media/a.mov"),
            trim_start_frames: 2,
            trim_end_frames: 0,
            audio: Some(PathBuf::from("/mix/full.wav")),
            silent: false,
            subtitle: None,
            subtitle_from_clip: false,
            reel_split_frames: Vec::new(),
        };
        let mut timeline = Timeline {
            version: postkit::timeline::TIMELINE_FORMAT_VERSION,
            fps: FrameRate::whole(24),
            clips: Vec::new(),
            sound: SoundTrack::PerClip,
            subtitle: None,
            reel_starts: Vec::new(),
        };
        assert!(takes_the_single_path(&timeline, &build));
        timeline.sound = SoundTrack::Override(PathBuf::from("/mix/full.wav"));
        assert!(!takes_the_single_path(&timeline, &build));
        let untrimmed = SingleClipBuild {
            trim_start_frames: 0,
            ..build.clone()
        };
        assert!(takes_the_single_path(&timeline, &untrimmed));
        let with_reels = SingleClipBuild {
            reel_split_frames: vec![1],
            ..untrimmed
        };
        assert!(!takes_the_single_path(&timeline, &with_reels));
    }

    #[test]
    fn the_multi_clip_refusals_name_each_flag_given() {
        let refusals = refuse_with_multi_clip(&MultiClipFlags {
            burn_subtitle: Some("--burn-subtitle"),
            audio_map: Some("--audio-map"),
            ..Default::default()
        });
        assert_eq!(refusals.len(), 2, "{refusals:?}");
        assert!(refusals[0].starts_with("--burn-subtitle"));
        assert!(refusals[1].contains(SOUND_IS_JOINED_FROM_THE_CLIPS));
    }
}
