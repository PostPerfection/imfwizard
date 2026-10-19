use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use postkit::dolby_vision::DolbyVisionSummary;
use postkit::encode::{FrameRate, InputType, SourceColour, detect_input_type};
use postkit::picture_processing::{Fit, PictureProcessing, Placement};
use postkit::pipeline::{EncodeRunOptions, PipelineProgress, StretchBase, TimelineStretch};
use postkit::timeline::{Clip, ClipSound, SingleClipBuild, SoundTrack, Timeline};
use serde::{Deserialize, Serialize};

use crate::MxfTrackFile;
use crate::cpl::{CompositionSegment, SequenceResource};
use crate::hdr_source::SourceHdr;
use crate::hdr_wcg::HdrWcg;
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
    /// One a stretch of timeline frames whose clips stay the same, in timeline order.
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

// postkit's TimelineStretch, which does not serialise, with None for a black base
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PlannedStretch {
    frames: Range<u64>,
    base_clip: Option<usize>,
    overlays: Vec<usize>,
}

impl PlannedStretch {
    fn of(stretch: TimelineStretch) -> Self {
        PlannedStretch {
            base_clip: match stretch.base {
                StretchBase::Clip(index) => Some(index),
                StretchBase::Black { .. } => None,
            },
            frames: stretch.frames,
            overlays: stretch.overlays,
        }
    }

    fn timeline_stretch(&self, (width, height): (u32, u32)) -> TimelineStretch {
        TimelineStretch {
            frames: self.frames.clone(),
            base: match self.base_clip {
                Some(index) => StretchBase::Clip(index),
                None => StretchBase::Black { width, height },
            },
            overlays: self.overlays.clone(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TimelinePlan {
    raster: (u32, u32),
    // the timeline with every clip's planned source colour and picture processing in it
    planned_timeline: Timeline,
    stretches: Vec<PlannedStretch>,
    hdr: Option<HdrWcg>,
}

fn raster_fit((width, height): (u32, u32)) -> PictureProcessing {
    PictureProcessing {
        fit: Some(Fit {
            box_width: width,
            box_height: height,
            raster_width: width,
            raster_height: height,
            placement: Placement::default(),
        }),
        ..PictureProcessing::default()
    }
}

// a clip whose planned colour or processing is postkit's default takes these instead
fn with_clip_defaults(
    options: EncodeRunOptions,
    build: &TimelineImp,
    raster: (u32, u32),
) -> EncodeRunOptions {
    EncodeRunOptions {
        source_colour: build.source_colour.clone(),
        picture: raster_fit(raster),
        ..options
    }
}

/// Every refusal the build can make, before anything is encoded.
pub fn check_timeline(build: &TimelineImp) -> Result<TimelinePlan, String> {
    let timeline = &build.timeline;
    postkit::timeline::refuse_two_subtitle_sources(timeline)?;
    if let SoundTrack::Override(sound) = &timeline.sound {
        crate::source_edits::check_sound_depth(sound)?;
    }
    if let Some(subtitle) = &timeline.subtitle {
        crate::subtitle_convert::readable_source_format(subtitle)?;
    }
    let raster = match build.picture_options.raster {
        Some(raster) => raster,
        None => timeline_raster(timeline)?,
    };
    // the stretches check the timeline too
    let stretches = postkit::pipeline::timeline_stretches(
        timeline,
        &with_clip_defaults(Default::default(), build, raster),
    )?
    .into_iter()
    .map(PlannedStretch::of)
    .collect();
    let clips = timeline
        .clips
        .iter()
        .enumerate()
        .map(|(index, clip)| {
            let planned = plan_clip(build, clip, raster)
                .map_err(|e| format!("{}: {e}", clip_name(index, clip)))?;
            Ok(Clip {
                source_colour: planned.source_colour,
                picture_processing: planned.processing,
                ..clip.clone()
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let hdr = match build.hdr.clone() {
        Some(hdr) => Some(light_levels_from_clip_rpus(timeline, hdr)?),
        None => None,
    };
    Ok(TimelinePlan {
        raster,
        planned_timeline: Timeline {
            clips,
            ..timeline.clone()
        },
        stretches,
        hdr,
    })
}

fn timeline_raster(timeline: &Timeline) -> Result<(u32, u32), String> {
    let base_clips = timeline.base_clips();
    let Some(&(index, first_base_clip)) = base_clips.first() else {
        if timeline.clips.is_empty() {
            return Err("the timeline has no clips".to_string());
        }
        return Err(
            "layer 0 has no clip to take the raster from, so the raster has to be given"
                .to_string(),
        );
    };
    postkit::encode::source_raster(&first_base_clip.picture)
        .map_err(|e| format!("{}: {e}", clip_name(index, first_base_clip)))
}

// a clip without an RPU adds no light level
fn light_levels_from_clip_rpus(timeline: &Timeline, hdr: HdrWcg) -> Result<HdrWcg, String> {
    let summaries = timeline
        .clips
        .iter()
        .enumerate()
        .filter_map(|(index, clip)| {
            clip_rpu_summary(clip)
                .map_err(|e| format!("{}: {e}", clip_name(index, clip)))
                .transpose()
        })
        .collect::<Result<Vec<_>, String>>()?;
    crate::hdr_source::light_levels_from_summaries(&summaries, hdr)
}

fn clip_rpu_summary(clip: &Clip) -> Result<Option<DolbyVisionSummary>, String> {
    if crate::hdr_source::probe(&clip.picture)? != SourceHdr::DolbyVision {
        return Ok(None);
    }
    let Some(summary) = postkit::dolby_vision::read_dolby_vision(&clip.picture)? else {
        return Ok(None);
    };
    postkit::dolby_vision::refuse_undecodable_dolby_vision(&summary)?;
    Ok(Some(summary))
}

fn plan_clip(build: &TimelineImp, clip: &Clip, raster: (u32, u32)) -> Result<ClipPicture, String> {
    let input_type = detect_input_type(&clip.picture);
    if matches!(input_type, InputType::J2kSequence | InputType::PictureMxf) {
        return Err(
            "it is already JPEG 2000, and a timeline built clip by clip takes video, image \
             sequence or still image clips"
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
            input_type == InputType::ImageSequence && clip.picture.is_dir(),
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

/// A picture track file a stretch of timeline frames whose clips stay the same,
/// one sound and one subtitle track file, and a segment a reel. Scratch is left
/// under the output directory.
pub fn build_timeline_imp(
    build: &TimelineImp,
    plan: &TimelinePlan,
    on_progress: impl Fn(&PipelineProgress),
    on_log: impl Fn(&str),
) -> Result<TimelineImpResult, String> {
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
        hdr: plan.hdr.clone(),
        ..Default::default()
    };

    let encode = timeline_encode(build, plan)?;
    let mut picture_track_files = Vec::new();
    for (index, planned) in plan.stretches.iter().enumerate() {
        if build.cancel.load(Ordering::Relaxed) {
            return Err(crate::imp::CANCELLED_ERROR.to_string());
        }
        let stretch = planned.timeline_stretch(plan.raster);
        on_log(&format!(
            "Encoding {} at timeline frames {}..{}",
            stretch_name(encode.timeline, &stretch),
            stretch.frames.start,
            stretch.frames.end
        ));
        let on_stretch_progress = |progress: &PipelineProgress| {
            on_progress(&PipelineProgress {
                frame: stretch.frames.start + progress.frame,
                total_frames: frames,
                ..progress.clone()
            })
        };
        let track = encode_stretch_picture(
            build,
            encode.timeline,
            &stretch,
            index,
            &encode.options,
            on_stretch_progress,
            &on_log,
        )?;
        picture_track_files.push(track);
    }

    if build.cancel.load(Ordering::Relaxed) {
        return Err(crate::imp::CANCELLED_ERROR.to_string());
    }
    let sound_track_file = wrap_timeline_sound(build, &options, &mut composition, frames)?;
    let timed_text_track_file = wrap_timeline_subtitle(build, &options, frames)?;

    let stretch_lengths: Vec<u64> = plan
        .stretches
        .iter()
        .map(|stretch| stretch.frames.end - stretch.frames.start)
        .collect();
    let segments: Vec<CompositionSegment> = segment_spans(&stretch_lengths, &timeline.reel_starts)
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
                    .stretch_pieces
                    .iter()
                    .map(|piece| SequenceResource {
                        track_file: picture_track_files[piece.stretch].clone(),
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

pub struct TimelineEncode<'a> {
    pub timeline: &'a Timeline,
    pub options: EncodeRunOptions,
}

// the build encodes and the preview renders the planned timeline with these options
pub fn timeline_encode<'a>(
    build: &TimelineImp,
    plan: &'a TimelinePlan,
) -> Result<TimelineEncode<'a>, String> {
    let options = with_clip_defaults(
        picture_encode_options(&build.compression, plan.raster, build.timeline.fps)?,
        build,
        plan.raster,
    );
    Ok(TimelineEncode {
        timeline: &plan.planned_timeline,
        options,
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

fn stretch_name(timeline: &Timeline, stretch: &TimelineStretch) -> String {
    let base = match stretch.base {
        StretchBase::Clip(index) => clip_name(index, &timeline.clips[index]),
        StretchBase::Black { .. } => "black".to_string(),
    };
    if stretch.overlays.is_empty() {
        return base;
    }
    let overlays: Vec<String> = stretch
        .overlays
        .iter()
        .map(|&index| clip_name(index, &timeline.clips[index]))
        .collect();
    format!("{base} under {}", overlays.join(", "))
}

fn encode_stretch_picture(
    build: &TimelineImp,
    timeline: &Timeline,
    stretch: &TimelineStretch,
    index: usize,
    options: &EncodeRunOptions,
    on_progress: impl Fn(&PipelineProgress),
    on_log: impl Fn(&str),
) -> Result<MxfTrackFile, String> {
    let encode_dir = build
        .output_dir
        .join(format!("{ENCODE_SCRATCH_PREFIX}{index}"));
    std::fs::create_dir_all(&encode_dir)
        .map_err(|e| format!("cannot create {}: {e}", encode_dir.display()))?;
    let wrap = crate::overlapped_picture::picture_wrap_options(
        crate::overlapped_picture::PictureWrapTarget {
            imp_dir: build.output_dir.clone(),
            fps_num: options.fps.numerator,
            fps_den: options.fps.denominator,
            colour: crate::mxf_wrap::picture_colour(build.hdr.as_ref()),
        },
    )?;
    let (encode, track) = postkit::pipeline::run_encode_timeline_stretch_and_wrap_picture(
        timeline,
        stretch,
        &encode_dir,
        options,
        wrap,
        &build.cancel,
        &build.pause,
        on_progress,
        on_log,
    )?;
    let track = crate::mxf_wrap::track_file_from_postkit(track)?;
    for finding in encode.picture_findings.describe(options.fps.as_f64()) {
        tracing::warn!("{finding}");
    }
    let length = stretch.frames.end - stretch.frames.start;
    if track.duration != length {
        return Err(format!(
            "{}: the encode wrapped {} frames where timeline frames {}..{} are {length}",
            stretch_name(timeline, stretch),
            track.duration,
            stretch.frames.start,
            stretch.frames.end
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
    stretch_pieces: Vec<StretchPiece>,
}

// the part of one stretch's track file a segment plays
#[derive(Debug, PartialEq)]
struct StretchPiece {
    stretch: usize,
    entry_point: u64,
    length: u64,
}

// the stretches run end to end from timeline frame 0
fn segment_spans(stretch_lengths: &[u64], reel_starts: &[u64]) -> Vec<SegmentSpan> {
    let total: u64 = stretch_lengths.iter().sum();
    let mut boundaries = vec![0];
    boundaries.extend_from_slice(reel_starts);
    boundaries.push(total);
    boundaries
        .windows(2)
        .map(|pair| {
            let (start, end) = (pair[0], pair[1]);
            let mut stretch_start = 0;
            let mut stretch_pieces = Vec::new();
            for (stretch, &length) in stretch_lengths.iter().enumerate() {
                let stretch_end = stretch_start + length;
                let first = start.max(stretch_start);
                let last = end.min(stretch_end);
                if first < last {
                    stretch_pieces.push(StretchPiece {
                        stretch,
                        entry_point: first - stretch_start,
                        length: last - first,
                    });
                }
                stretch_start = stretch_end;
            }
            SegmentSpan {
                start,
                length: end - start,
                stretch_pieces,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hdr_source::tests::dolby_vision_summary;
    use std::cell::RefCell;
    use std::path::Path;

    const LOGGED_CLIP_FRAMES: u64 = 2;
    const LOGGED_CLIP_RATE: u32 = 24;
    const OTHER_CLIP_RATE: u32 = 25;
    const LOGGED_CLIP_RASTER: &str = "1920x1080";

    fn piece(stretch: usize, entry_point: u64, length: u64) -> StretchPiece {
        StretchPiece {
            stretch,
            entry_point,
            length,
        }
    }

    #[test]
    fn no_reel_start_is_one_segment_of_whole_stretches() {
        assert_eq!(
            segment_spans(&[6, 6], &[]),
            vec![SegmentSpan {
                start: 0,
                length: 12,
                stretch_pieces: vec![piece(0, 0, 6), piece(1, 0, 6)],
            }]
        );
    }

    #[test]
    fn a_reel_start_inside_a_stretch_splits_its_track_file() {
        assert_eq!(
            segment_spans(&[6, 6], &[4]),
            vec![
                SegmentSpan {
                    start: 0,
                    length: 4,
                    stretch_pieces: vec![piece(0, 0, 4)],
                },
                SegmentSpan {
                    start: 4,
                    length: 8,
                    stretch_pieces: vec![piece(0, 4, 2), piece(1, 0, 6)],
                },
            ]
        );
    }

    #[test]
    fn a_reel_start_on_a_stretch_boundary_splits_nothing() {
        let spans = segment_spans(&[6, 3, 5], &[6, 9]);
        let pieces: Vec<&Vec<StretchPiece>> =
            spans.iter().map(|span| &span.stretch_pieces).collect();
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
            still_hold_frames: None,
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

    #[test]
    fn a_two_clip_timeline_takes_the_brightest_light_levels_of_its_clips_rpus() {
        let hdr = HdrWcg::from_flags("pq-bt2020", None).unwrap();
        let brighter_peak = dolby_vision_summary(8, Some(993.0), Some(200.0));
        let brighter_average = dolby_vision_summary(8, Some(600.0), Some(362.4));
        let filled =
            crate::hdr_source::light_levels_from_summaries(&[brighter_peak, brighter_average], hdr)
                .unwrap();
        assert_eq!(filled.max_cll, Some(993));
        assert_eq!(filled.max_fall, Some(362));
    }

    fn write_clip_media(dir: &Path, name: &str, rate: u32) -> PathBuf {
        let media = dir.join(name);
        let made = std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg(format!("testsrc2=size={LOGGED_CLIP_RASTER}:rate={rate}"))
            .args(["-frames:v", &LOGGED_CLIP_FRAMES.to_string(), "-c:v", "ffv1"])
            .arg(&media)
            .output()
            .expect("ffmpeg");
        assert!(
            made.status.success(),
            "{}",
            String::from_utf8_lossy(&made.stderr)
        );
        media
    }

    fn logged_clip(picture: PathBuf, start: u64) -> Clip {
        Clip {
            picture,
            start,
            layer: 0,
            opacity: 1.0,
            in_frame: 0,
            out_frame: LOGGED_CLIP_FRAMES,
            sound: ClipSound::None,
            subtitle: None,
            source_colour: Default::default(),
            picture_processing: Default::default(),
        }
    }

    fn two_clip_build(dir: &Path) -> TimelineImp {
        let clips = [("first.mov", 0), ("second.mov", LOGGED_CLIP_FRAMES)]
            .map(|(name, start)| logged_clip(write_clip_media(dir, name, LOGGED_CLIP_RATE), start));
        TimelineImp {
            timeline: Timeline {
                version: postkit::timeline::TIMELINE_FORMAT_VERSION,
                fps: FrameRate::whole(LOGGED_CLIP_RATE),
                clips: clips.to_vec(),
                sound: SoundTrack::PerClip,
                subtitle: None,
                reel_starts: Vec::new(),
            },
            output_dir: dir.join("imp"),
            title: "Logged Clips".to_string(),
            content_kind: "feature".to_string(),
            picture_options: Default::default(),
            source_colour: crate::source_colourspace::to_source_colour(
                crate::source_colourspace::APP2E_SOURCE_SPACE,
            )
            .unwrap(),
            compression: Default::default(),
            hdr: None,
            audio_language: None,
            audio_role: None,
            timed_text_language: None,
            soundfield: Default::default(),
            cancel: Default::default(),
            pause: Default::default(),
        }
    }

    // black, clip 1, clip 1 under clip 3, clip 2 under clip 3, clip 2
    fn layered_build(dir: &Path) -> TimelineImp {
        let mut build = two_clip_build(dir);
        let clips = &mut build.timeline.clips;
        clips[0].start = 2;
        clips[1].start = 4;
        clips.push(Clip {
            layer: 1,
            opacity: 0.5,
            ..logged_clip(write_clip_media(dir, "over.mov", LOGGED_CLIP_RATE), 3)
        });
        build
    }

    fn planned(frames: Range<u64>, base_clip: Option<usize>, overlays: &[usize]) -> PlannedStretch {
        PlannedStretch {
            frames,
            base_clip,
            overlays: overlays.to_vec(),
        }
    }

    #[test]
    fn a_gap_and_an_overlay_cut_the_timeline_into_stretches() {
        let dir = tempfile::tempdir().unwrap();
        let build = layered_build(dir.path());
        let plan = check_timeline(&build).unwrap();
        assert_eq!(
            plan.stretches,
            vec![
                planned(0..2, None, &[]),
                planned(2..3, Some(0), &[]),
                planned(3..4, Some(0), &[2]),
                planned(4..5, Some(1), &[2]),
                planned(5..6, Some(1), &[]),
            ]
        );
        assert_eq!(
            plan.stretches[0].timeline_stretch(plan.raster).base,
            StretchBase::Black {
                width: 1920,
                height: 1080
            }
        );
        assert!(
            plan.planned_timeline
                .clips
                .iter()
                .all(|clip| clip.source_colour == build.source_colour),
            "every clip carries the colour it was planned with"
        );
    }

    #[test]
    fn the_preview_cuts_the_builds_stretches_from_the_encoded_timeline_and_options() {
        const FITTED_RASTER: (u32, u32) = (2048, 1080);
        let dir = tempfile::tempdir().unwrap();
        let mut build = layered_build(dir.path());
        build.picture_options.raster = Some(FITTED_RASTER);
        let plan = check_timeline(&build).unwrap();

        let encode = timeline_encode(&build, &plan).unwrap();

        assert_eq!(encode.timeline, &plan.planned_timeline);
        let raster_of = |picture: &PictureProcessing| {
            picture.fit.map(|fit| (fit.raster_width, fit.raster_height))
        };
        assert!(
            encode
                .timeline
                .clips
                .iter()
                .all(|clip| raster_of(&clip.picture_processing) == Some(FITTED_RASTER)),
            "every clip is fitted into the build's raster"
        );
        assert_eq!(raster_of(&encode.options.picture), Some(FITTED_RASTER));
        assert!(encode.options.source_colour == build.source_colour);
        let stretches =
            postkit::pipeline::timeline_stretches(encode.timeline, &encode.options).unwrap();
        assert_eq!(
            stretches[0].base,
            StretchBase::Black {
                width: FITTED_RASTER.0,
                height: FITTED_RASTER.1
            }
        );
        let stretches: Vec<PlannedStretch> =
            stretches.into_iter().map(PlannedStretch::of).collect();
        assert_eq!(stretches, plan.stretches);
    }

    #[test]
    fn a_reel_start_inside_a_composite_splits_its_stretch() {
        let spans = segment_spans(&[2, 1, 1, 1, 1], &[3]);
        let pieces: Vec<&Vec<StretchPiece>> =
            spans.iter().map(|span| &span.stretch_pieces).collect();
        assert_eq!(
            pieces,
            vec![
                &vec![piece(0, 0, 2), piece(1, 0, 1)],
                &vec![piece(2, 0, 1), piece(3, 0, 1), piece(4, 0, 1)],
            ]
        );
        let spans = segment_spans(&[2, 2], &[3]);
        assert_eq!(spans[1].stretch_pieces, vec![piece(1, 1, 1)]);
    }

    #[test]
    fn every_stretch_names_its_clips_in_the_log_and_wraps_a_track_file() {
        let dir = tempfile::tempdir().unwrap();
        let build = layered_build(dir.path());
        let plan = check_timeline(&build).unwrap();
        let lines = RefCell::new(Vec::new());
        let built = build_timeline_imp(
            &build,
            &plan,
            |_| {},
            |message| lines.borrow_mut().push(message.to_string()),
        )
        .unwrap();
        let lines = lines.into_inner();
        let clips = &build.timeline.clips;
        let names: Vec<String> = (0..clips.len())
            .map(|index| clip_name(index, &clips[index]))
            .collect();
        for encoding in [
            "Encoding black at timeline frames 0..2".to_string(),
            format!("Encoding {} at timeline frames 2..3", names[0]),
            format!(
                "Encoding {} under {} at timeline frames 3..4",
                names[0], names[2]
            ),
            format!(
                "Encoding {} under {} at timeline frames 4..5",
                names[1], names[2]
            ),
        ] {
            assert!(
                lines.iter().any(|line| line == &encoding),
                "no {encoding:?} in {lines:?}"
            );
        }
        let durations: Vec<u64> = built
            .picture_track_files
            .iter()
            .map(|track| track.duration)
            .collect();
        assert_eq!(durations, vec![2, 1, 1, 1, 1]);
    }

    #[test]
    fn the_build_encodes_from_its_plan_without_probing_the_clips_again() {
        let dir = tempfile::tempdir().unwrap();
        let build = two_clip_build(dir.path());
        let plan = check_timeline(&build).unwrap();
        write_clip_media(dir.path(), "second.mov", OTHER_CLIP_RATE);
        let refusal = check_timeline(&build)
            .err()
            .expect("a clip at another rate is refused");
        assert!(refusal.contains("clip 2"), "{refusal}");

        let built = build_timeline_imp(&build, &plan, |_| {}, |_| {}).unwrap();

        assert_eq!(built.frames, build.timeline.total_frames());
    }
}
