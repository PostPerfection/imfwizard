//! Head/tail trim and audio delay, applied to a composition's source between
//! the J2K encode and packaging.
//!
//! The delay lands before the trim: it says how the sound lines up with the
//! picture, and the trim then cuts a range out of the aligned programme. Running
//! them the other way round would cut a range that had not been aligned yet.

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where the trimmed picture frames are written under the job's work directory.
const TRIMMED_PICTURE_DIR: &str = "j2k_trimmed";

/// Trim and audio delay for one composition's source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceEdits {
    /// Milliseconds to shift the sound against the picture; positive is later.
    /// The running time never changes, so the shift is padded and truncated.
    pub audio_delay_ms: i64,
    /// Picture frames removed from the head of the source.
    pub trim_start_frames: u64,
    /// Picture frames removed from the tail of the source.
    pub trim_end_frames: u64,
}

impl SourceEdits {
    pub fn trims(&self) -> bool {
        self.trim_start_frames > 0 || self.trim_end_frames > 0
    }
}

/// The source lengths the edit refusals measure against, gathered before the
/// encode by [`probe_source_facts`] and from the files themselves at edit time.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceFacts {
    pub picture: Option<PictureFacts>,
    pub audio: Vec<AudioFacts>,
    /// The edit rate the trim's frame counts are in.
    pub fps: f64,
}

/// How many frames the picture holds, and where they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PictureFacts {
    pub path: PathBuf,
    pub frames: u64,
}

/// How long one sound file runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFacts {
    pub path: PathBuf,
    pub sample_frames: u64,
    pub sample_rate: u32,
}

const NO_SOUND_TO_DELAY: &str =
    "an audio delay needs a sound track to shift, but the composition has none";
const NO_PICTURE_TO_TRIM: &str =
    "a trim needs picture frames to measure against, but the composition has none";

/// Refuse every edit the source cannot absorb.
///
/// The edit functions run the same pieces on the files they open, so a job that
/// cannot finish is refused before the encode as well as at the edit itself.
pub fn check_edits(edits: &SourceEdits, facts: &SourceFacts) -> Result<(), String> {
    if edits.audio_delay_ms != 0 && facts.audio.is_empty() {
        return Err(NO_SOUND_TO_DELAY.to_string());
    }
    if edits.trims() {
        let Some(picture) = &facts.picture else {
            return Err(NO_PICTURE_TO_TRIM.to_string());
        };
        check_picture_trim(edits, picture)?;
    }
    for audio in &facts.audio {
        check_audio_delay(edits.audio_delay_ms, audio)?;
        check_audio_trim(edits, audio, facts.fps)?;
    }
    Ok(())
}

fn check_picture_trim(edits: &SourceEdits, picture: &PictureFacts) -> Result<(), String> {
    let head = edits.trim_start_frames;
    let tail = edits.trim_end_frames;
    if head + tail < picture.frames {
        return Ok(());
    }
    Err(format!(
        "trimming {head} frames off the head and {tail} off the tail leaves nothing of the {} picture frames in {}",
        picture.frames,
        picture.path.display()
    ))
}

fn check_audio_delay(delay_ms: i64, audio: &AudioFacts) -> Result<(), String> {
    if delay_ms == 0 || delay_shift_frames(delay_ms, audio.sample_rate) < audio.sample_frames {
        return Ok(());
    }
    let programme_ms = audio.sample_frames as f64 * 1000.0 / audio.sample_rate.max(1) as f64;
    Err(format!(
        "an audio delay of {delay_ms} ms is at least as long as the {programme_ms:.0} ms of sound in {}",
        audio.path.display()
    ))
}

fn check_audio_trim(edits: &SourceEdits, audio: &AudioFacts, fps: f64) -> Result<(), String> {
    if !edits.trims() {
        return Ok(());
    }
    let (head, tail) = trim_sample_frames(edits, audio.sample_rate, fps);
    if head + tail < audio.sample_frames {
        return Ok(());
    }
    Err(format!(
        "trimming {head} + {tail} sample frames leaves nothing of the {} in {}",
        audio.sample_frames,
        audio.path.display()
    ))
}

/// How many sample frames a delay shifts the sound by.
fn delay_shift_frames(delay_ms: i64, sample_rate: u32) -> u64 {
    (delay_ms.unsigned_abs() as f64 * sample_rate.max(1) as f64 / 1000.0).round() as u64
}

/// The head and tail trims as sample frames, measured through the edit rate.
fn trim_sample_frames(edits: &SourceEdits, sample_rate: u32, fps: f64) -> (u64, u64) {
    let to_samples =
        |picture_frames: u64| (picture_frames as f64 / fps * sample_rate as f64).round() as u64;
    (
        to_samples(edits.trim_start_frames),
        to_samples(edits.trim_end_frames),
    )
}

/// Extensions the encoder enumerates as picture frames in a directory.
const PICTURE_FRAME_EXTENSIONS: [&str; 7] = ["j2c", "j2k", "tif", "tiff", "dpx", "exr", "bmp"];

/// What ffprobe is asked for to length a video: the stream's frame count, then
/// the container duration for the sources that do not carry one.
const FFPROBE_LENGTH_ARGUMENTS: [&str; 8] = [
    "-v",
    "error",
    "-select_streams",
    "v:0",
    "-show_entries",
    "stream=nb_frames:format=duration",
    "-of",
    "default=nw=1:nk=1",
];

/// Measure the source the edits will act on, before anything is encoded.
///
/// A still is as long as the hold asks for, since the source image is one frame
/// whatever the trim says.
pub fn probe_source_facts(
    picture: Option<&Path>,
    still_frames: Option<u64>,
    audio: &[PathBuf],
    fps_num: u32,
    fps_den: u32,
) -> Result<SourceFacts, String> {
    let fps = fps_num.max(1) as f64 / fps_den.max(1) as f64;
    let picture = match picture {
        Some(path) => Some(PictureFacts {
            path: path.to_path_buf(),
            frames: match still_frames {
                Some(frames) => frames,
                None => source_frame_count(path, fps)?,
            },
        }),
        None => None,
    };
    let audio = audio
        .iter()
        .map(|path| audio_facts(path))
        .collect::<Result<Vec<_>, String>>()?;
    Ok(SourceFacts {
        picture,
        audio,
        fps,
    })
}

fn audio_facts(path: &Path) -> Result<AudioFacts, String> {
    let reader =
        hound::WavReader::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(AudioFacts {
        path: path.to_path_buf(),
        sample_frames: reader.duration() as u64,
        sample_rate: reader.spec().sample_rate,
    })
}

fn source_frame_count(picture: &Path, fps: f64) -> Result<u64, String> {
    if picture.is_dir() {
        return directory_frame_count(picture);
    }
    video_frame_count(picture, fps)
}

fn directory_frame_count(dir: &Path) -> Result<u64, String> {
    let frames = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| {
                    PICTURE_FRAME_EXTENSIONS.contains(&extension.to_lowercase().as_str())
                })
                .unwrap_or(false)
        })
        .count() as u64;
    if frames == 0 {
        return Err(format!("no picture frames in {}", dir.display()));
    }
    Ok(frames)
}

fn video_frame_count(picture: &Path, fps: f64) -> Result<u64, String> {
    let unreadable = || {
        format!(
            "cannot read how long {} runs, so a trim cannot be measured against it",
            picture.display()
        )
    };
    let output = std::process::Command::new("ffprobe")
        .args(FFPROBE_LENGTH_ARGUMENTS)
        .arg(picture)
        .output()
        .map_err(|e| format!("cannot run ffprobe on {}: {e}", picture.display()))?;
    if !output.status.success() {
        return Err(unreadable());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines().map(str::trim);
    let counted = lines
        .next()
        .and_then(|line| line.parse::<u64>().ok())
        .filter(|frames| *frames > 0);
    if let Some(frames) = counted {
        return Ok(frames);
    }
    let seconds: f64 = lines
        .next()
        .and_then(|line| line.parse().ok())
        .ok_or_else(unreadable)?;
    Ok((seconds * fps).round() as u64)
}

/// One composition's picture, sound and timed text, before or after the edits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompositionSource {
    pub j2k_dir: Option<PathBuf>,
    pub audio_files: Vec<PathBuf>,
    pub timed_text_files: Vec<PathBuf>,
}

/// Apply the edits, writing intermediates under `work_dir`, and return the
/// paths to package.
pub fn apply_source_edits(
    edits: &SourceEdits,
    source: &CompositionSource,
    work_dir: &Path,
    fps_num: u32,
    fps_den: u32,
) -> Result<CompositionSource, String> {
    if *edits == SourceEdits::default() {
        return Ok(source.clone());
    }
    if edits.audio_delay_ms != 0 && source.audio_files.is_empty() {
        return Err(NO_SOUND_TO_DELAY.to_string());
    }
    let fps = fps_num.max(1) as f64 / fps_den.max(1) as f64;
    let mut edited = source.clone();

    if edits.trims() {
        let Some(picture_dir) = &source.j2k_dir else {
            return Err(NO_PICTURE_TO_TRIM.to_string());
        };
        let frames = picture_frames(picture_dir)?;
        check_picture_trim(
            edits,
            &PictureFacts {
                path: picture_dir.clone(),
                frames: frames.len() as u64,
            },
        )?;
        let head = edits.trim_start_frames as usize;
        let tail = edits.trim_end_frames as usize;
        edited.j2k_dir = Some(write_frame_dir(
            &frames[head..frames.len() - tail],
            &work_dir.join(TRIMMED_PICTURE_DIR),
        )?);

        let kept_start = head as f64 / fps;
        let kept_end = (frames.len() - tail) as f64 / fps;
        edited.timed_text_files = source
            .timed_text_files
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let output = work_dir.join(format!("trimmed_subtitle_{index}.xml"));
                trim_timed_text(path, &output, kept_start, kept_end, fps)?;
                Ok(output)
            })
            .collect::<Result<Vec<_>, String>>()?;
    }

    edited.audio_files = source
        .audio_files
        .iter()
        .enumerate()
        .map(|(index, wav)| edit_one_wav(wav, index, edits, work_dir, fps))
        .collect::<Result<Vec<_>, String>>()?;

    Ok(edited)
}

fn edit_one_wav(
    wav: &Path,
    index: usize,
    edits: &SourceEdits,
    work_dir: &Path,
    fps: f64,
) -> Result<PathBuf, String> {
    let mut current = wav.to_path_buf();
    if edits.audio_delay_ms != 0 {
        let delayed = work_dir.join(format!("delayed_audio_{index}.wav"));
        apply_audio_delay(&current, &delayed, edits.audio_delay_ms)?;
        current = delayed;
    }
    if edits.trims() {
        let trimmed = work_dir.join(format!("trimmed_audio_{index}.wav"));
        trim_wav(&current, &trimmed, edits, fps)?;
        current = trimmed;
    }
    Ok(current)
}

/// Shift a WAV against the picture without changing its running time: a positive
/// delay prepends silence and drops the same amount off the tail, a negative one
/// drops the head and pads the tail.
pub fn apply_audio_delay(input: &Path, output: &Path, delay_ms: i64) -> Result<(), String> {
    use postkit::wav_io::Samples;
    let (spec, samples) = postkit::wav_io::read_interleaved_exact(input)
        .map_err(|e| format!("cannot read {}: {e}", input.display()))?;
    let channels = spec.channels.max(1) as usize;
    let sample_rate = spec.sample_rate.max(1);
    let sample_frames = samples.len() / channels;

    check_audio_delay(
        delay_ms,
        &AudioFacts {
            path: input.to_path_buf(),
            sample_frames: sample_frames as u64,
            sample_rate,
        },
    )?;
    let shift = delay_shift_frames(delay_ms, sample_rate) as usize;

    // the shifted samples are copied untouched, so the output stays bit-exact
    // for every PCM format, 32-bit int included
    fn shifted_window<T: Copy + Default>(
        samples: &[T],
        shift: usize,
        channels: usize,
        sample_frames: usize,
        later: bool,
    ) -> Vec<T> {
        let kept = (sample_frames - shift) * channels;
        let mut out = vec![T::default(); sample_frames * channels];
        if later {
            out[shift * channels..].copy_from_slice(&samples[..kept]);
        } else {
            out[..kept].copy_from_slice(&samples[shift * channels..sample_frames * channels]);
        }
        out
    }
    let later = delay_ms >= 0;
    let shifted = match &samples {
        Samples::Int(v) => Samples::Int(shifted_window(v, shift, channels, sample_frames, later)),
        Samples::Float(v) => {
            Samples::Float(shifted_window(v, shift, channels, sample_frames, later))
        }
    };

    postkit::wav_io::write_interleaved_exact(output, spec, &shifted)
        .map_err(|e| format!("cannot write {}: {e}", output.display()))
}

/// Drop the head and tail of a WAV matching a picture trim of the same length.
fn trim_wav(input: &Path, output: &Path, edits: &SourceEdits, fps: f64) -> Result<(), String> {
    use postkit::wav_io::Samples;
    let (spec, samples) = postkit::wav_io::read_interleaved_exact(input)
        .map_err(|e| format!("cannot read {}: {e}", input.display()))?;
    let channels = spec.channels.max(1) as usize;
    let sample_rate = spec.sample_rate.max(1);
    let sample_frames = samples.len() / channels;

    check_audio_trim(
        edits,
        &AudioFacts {
            path: input.to_path_buf(),
            sample_frames: sample_frames as u64,
            sample_rate,
        },
        fps,
    )?;
    let (head, tail) = trim_sample_frames(edits, sample_rate, fps);
    let (head, tail) = (head as usize, tail as usize);

    let window = head * channels..(sample_frames - tail) * channels;
    let kept = match &samples {
        Samples::Int(v) => Samples::Int(v[window].to_vec()),
        Samples::Float(v) => Samples::Float(v[window].to_vec()),
    };
    postkit::wav_io::write_interleaved_exact(output, spec, &kept)
        .map_err(|e| format!("cannot write {}: {e}", output.display()))
}

/// Move timed text with the picture: shift every cue back by the head trim, drop
/// the cues that fall wholly outside the kept range, and clamp the ones that
/// straddle its edges.
fn trim_timed_text(
    input: &Path,
    output: &Path,
    kept_start: f64,
    kept_end: f64,
    fps: f64,
) -> Result<(), String> {
    let retimed = retimed_timed_text(input, kept_start, kept_end, fps)?;
    std::fs::write(output, retimed).map_err(|e| format!("cannot write {}: {e}", output.display()))
}

/// Refuse a timed-text file a trim could not move, without writing anything.
///
/// The whole file is kept, so every cue is read and every refusal the trim
/// itself would make fires here instead.
pub fn check_timed_text_trimmable(input: &Path, fps: f64) -> Result<(), String> {
    retimed_timed_text(input, 0.0, f64::INFINITY, fps).map(|_| ())
}

fn retimed_timed_text(
    input: &Path,
    kept_start: f64,
    kept_end: f64,
    fps: f64,
) -> Result<Vec<u8>, String> {
    let content = std::fs::read_to_string(input)
        .map_err(|e| format!("cannot read {}: {e}", input.display()))?;

    let mut reader = Reader::from_str(&content);
    let mut writer = Writer::new(Vec::new());

    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("cannot parse {}: {e}", input.display()))?;
        match event {
            Event::Eof => break,
            Event::Start(ref element) if carries_timing(element) => {
                match retimed_cue(element, kept_start, kept_end, fps)? {
                    Some(retimed) => write_event(&mut writer, Event::Start(retimed))?,
                    None => {
                        reader
                            .read_to_end(element.name())
                            .map_err(|e| format!("cannot parse {}: {e}", input.display()))?;
                    }
                }
            }
            Event::Empty(ref element) if carries_timing(element) => {
                if let Some(retimed) = retimed_cue(element, kept_start, kept_end, fps)? {
                    write_event(&mut writer, Event::Empty(retimed))?;
                }
            }
            other => write_event(&mut writer, other)?,
        }
    }

    Ok(writer.into_inner())
}

/// One `<p>` cue of a timed-text file, as the hints read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedTextCue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub lines: Vec<String>,
}

/// Read the `<p>` cues of a timed-text file, through the same time parser the
/// trim uses so both read the same shapes.
///
/// A cue whose times cannot be read is skipped rather than refused: this feeds
/// the advisory hints, and the trim is what refuses such a file.
pub fn read_timed_text_cues(input: &Path, fps: f64) -> Result<Vec<TimedTextCue>, String> {
    let content = std::fs::read_to_string(input)
        .map_err(|e| format!("cannot read {}: {e}", input.display()))?;
    let mut reader = Reader::from_str(&content);
    let mut cues = Vec::new();
    let mut open: Option<TimedTextCue> = None;

    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("cannot parse {}: {e}", input.display()))?;
        match event {
            Event::Eof => break,
            Event::Start(ref element) if is_named(element, b"p") => {
                open = cue_milliseconds(element, fps).map(|(start_ms, end_ms)| TimedTextCue {
                    start_ms,
                    end_ms,
                    lines: vec![String::new()],
                });
            }
            Event::End(ref element) if element.name().local_name().as_ref() == b"p" => {
                if let Some(cue) = open.take() {
                    cues.push(finished_cue(cue));
                }
            }
            Event::Start(ref element) | Event::Empty(ref element) if is_named(element, b"br") => {
                if let Some(cue) = open.as_mut() {
                    cue.lines.push(String::new());
                }
            }
            Event::Text(text) => {
                if let Some(cue) = open.as_mut() {
                    let decoded = text
                        .unescape()
                        .map_err(|e| format!("cannot parse {}: {e}", input.display()))?;
                    if let Some(line) = cue.lines.last_mut() {
                        line.push_str(decoded.as_ref());
                    }
                }
            }
            _ => {}
        }
    }
    Ok(cues)
}

fn is_named(element: &BytesStart, name: &[u8]) -> bool {
    element.name().local_name().as_ref() == name
}

fn finished_cue(mut cue: TimedTextCue) -> TimedTextCue {
    cue.lines = cue
        .lines
        .iter()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();
    cue
}

fn cue_milliseconds(element: &BytesStart, fps: f64) -> Option<(u64, u64)> {
    let to_milliseconds = |value: String| {
        cue_seconds(&value, fps).map(|seconds| (seconds.max(0.0) * 1000.0).round() as u64)
    };
    let begin = to_milliseconds(attribute(element, "begin")?)?;
    let end = to_milliseconds(attribute(element, "end")?)?;
    Some((begin, end))
}

/// TTML attributes that put an element on the timeline.
const TIMING_ATTRIBUTES: [&str; 3] = ["begin", "end", "dur"];

fn carries_timing(element: &BytesStart) -> bool {
    TIMING_ATTRIBUTES
        .iter()
        .any(|name| attribute(element, name).is_some())
}

/// The cue with its times moved into the kept range, or None if it falls wholly
/// outside and should be dropped.
fn retimed_cue<'a>(
    element: &BytesStart<'a>,
    kept_start: f64,
    kept_end: f64,
    fps: f64,
) -> Result<Option<BytesStart<'a>>, String> {
    let name = String::from_utf8_lossy(element.name().as_ref()).into_owned();
    // TTML times a timed element's children relative to it, so shifting a <div>
    // and its cues by the same amount would move the cues twice. Resolving that
    // tree is more than a trim needs, so only flat <p> timing is moved.
    if element.name().local_name().as_ref() != b"p" {
        return Err(format!(
            "<{name}> carries timing, which times its children relative to it and a trim here cannot resolve; put the times on the <p> cues"
        ));
    }
    // dur in place of end would leave the cue running past the range it was
    // clamped into, so it is refused rather than moved wrongly.
    let (Some(begin), Some(end)) = (attribute(element, "begin"), attribute(element, "end")) else {
        return Err(
            "a <p> cue is missing begin or end (dur is not supported), so a trim cannot move it"
                .to_string(),
        );
    };
    let Some(begin_seconds) = cue_seconds(&begin, fps) else {
        return Err(unreadable_time(&begin));
    };
    let Some(end_seconds) = cue_seconds(&end, fps) else {
        return Err(unreadable_time(&end));
    };
    if end_seconds <= kept_start || begin_seconds >= kept_end {
        return Ok(None);
    }

    let shifted_begin = begin_seconds.max(kept_start) - kept_start;
    let shifted_end = end_seconds.min(kept_end) - kept_start;

    let mut retimed =
        BytesStart::new(String::from_utf8_lossy(element.name().as_ref()).into_owned());
    for original in element.attributes().flatten() {
        // only the two times are rebuilt; every other attribute keeps its exact
        // bytes, so an escaped value is not escaped a second time
        let retimed_value = match original.key.local_name().as_ref() {
            b"begin" => Some(format_like(&begin, shifted_begin, fps)),
            b"end" => Some(format_like(&end, shifted_end, fps)),
            _ => None,
        };
        match retimed_value {
            Some(value) => retimed.push_attribute((
                String::from_utf8_lossy(original.key.as_ref()).as_ref(),
                value.as_str(),
            )),
            None => retimed.push_attribute(original),
        }
    }
    Ok(Some(retimed))
}

fn unreadable_time(value: &str) -> String {
    format!(
        "cannot read the subtitle time '{value}': expected a clock time like 00:00:01.500 or an offset like 0.8s"
    )
}

/// Seconds for a TTML time expression: a clock time ("00:00:01.500" or
/// "00:00:01:12" with a frame field) or an offset time ("0.8s", "40f",
/// "1500ms"). None for anything else, including ticks, which need the
/// document's tick rate to mean anything.
fn cue_seconds(value: &str, fps: f64) -> Option<f64> {
    let value = value.trim();
    let Some(metric) = offset_metric(value) else {
        return clock_seconds(value, fps);
    };
    let number: f64 = value[..value.len() - metric.len()].parse().ok()?;
    if !number.is_finite() {
        return None;
    }
    match metric {
        "h" => Some(number * 3600.0),
        "m" => Some(number * 60.0),
        "s" => Some(number),
        "ms" => Some(number / 1000.0),
        "f" => Some(number / fps),
        _ => None,
    }
}

fn clock_seconds(value: &str, fps: f64) -> Option<f64> {
    let parts: Vec<&str> = value.split(':').collect();
    let hours: f64 = parts.first()?.parse().ok()?;
    let minutes: f64 = parts.get(1)?.parse().ok()?;
    let seconds: f64 = parts.get(2)?.parse().ok()?;
    let frames: f64 = match parts.len() {
        3 => 0.0,
        4 => parts[3].parse().ok()?,
        _ => return None,
    };
    Some(hours * 3600.0 + minutes * 60.0 + seconds + frames / fps)
}

/// The metric of a TTML offset time ("0.8s" is "s"), or None for a clock time.
/// "ms" is tried before "m" and "s" so it is not read as either.
fn offset_metric(value: &str) -> Option<&'static str> {
    if value.contains(':') {
        return None;
    }
    ["ms", "h", "m", "s", "f", "t"]
        .into_iter()
        .find(|metric| value.ends_with(metric))
}

/// Format seconds back in the shape the source used: an offset time keeps its
/// metric, a clock time keeps its frame field, anything else is "HH:MM:SS.mmm".
fn format_like(original: &str, seconds: f64, fps: f64) -> String {
    let seconds = seconds.max(0.0);
    let milliseconds = (seconds * 1000.0).round();
    if let Some(metric) = offset_metric(original.trim()) {
        return match metric {
            "h" => format!("{}h", milliseconds / 3_600_000.0),
            "m" => format!("{}m", milliseconds / 60_000.0),
            "ms" => format!("{milliseconds}ms"),
            "f" => format!("{}f", (seconds * fps).round()),
            _ => format!("{}s", milliseconds / 1000.0),
        };
    }
    if original.split(':').count() == 4 {
        return frame_clock_time(seconds, fps);
    }
    crate::subtitle_convert::format_ttml_time(milliseconds as u64)
}

/// "HH:MM:SS:FF", counted in whole frames so rounding cannot push the frame
/// field up to the frame rate itself and name a frame that does not exist.
fn frame_clock_time(seconds: f64, fps: f64) -> String {
    let frames_per_second = fps.round().max(1.0) as u64;
    let total_frames = (seconds * fps).round() as u64;
    let whole_seconds = total_frames / frames_per_second;
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        whole_seconds / 3600,
        (whole_seconds / 60) % 60,
        whole_seconds % 60,
        total_frames % frames_per_second
    )
}

fn attribute(element: &BytesStart, name: &str) -> Option<String> {
    element.attributes().flatten().find_map(|a| {
        (a.key.local_name().as_ref() == name.as_bytes())
            .then(|| String::from_utf8_lossy(&a.value).into_owned())
    })
}

fn write_event(writer: &mut Writer<Vec<u8>>, event: Event) -> Result<(), String> {
    writer
        .write_event(event)
        .map_err(|e| format!("cannot write timed text: {e}"))
}

/// The J2K codestreams in a picture directory, in playing order.
pub(crate) fn picture_frames(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut frames: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("j2c" | "j2k")
                )
        })
        .collect();
    frames.sort();
    if frames.is_empty() {
        return Err(format!("no J2K codestreams in {}", dir.display()));
    }
    Ok(frames)
}

/// Write an ordered frame list into `dest`, hard-linking where the filesystem
/// allows it so holding a still or trimming a long programme costs no disk. The
/// names have to sort into playing order, since the MXF wrapper takes the
/// directory listing.
pub(crate) fn write_frame_dir(frames: &[PathBuf], dest: &Path) -> Result<PathBuf, String> {
    fresh_dir(dest)?;
    for (index, frame) in frames.iter().enumerate() {
        link_or_copy(frame, &dest.join(format!("frame_{index:08}.j2c")))?;
    }
    Ok(dest.to_path_buf())
}

/// Recreate `path` empty. Building into an output folder a second time with a
/// shorter trim or still length would otherwise leave the first run's frames
/// beside the new ones, and the MXF wrapper takes the whole directory listing,
/// so the package would claim the old length.
pub(crate) fn fresh_dir(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        std::fs::remove_dir_all(path)
            .map_err(|e| format!("cannot clear {}: {e}", path.display()))?;
    }
    std::fs::create_dir_all(path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    Ok(path.to_path_buf())
}

/// Place `source` at `target`, preferring a hard link. Falls back to a copy on
/// the filesystems and cross-device cases that refuse links.
pub(crate) fn link_or_copy(source: &Path, target: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(target);
    if std::fs::hard_link(source, target).is_ok() {
        return Ok(());
    }
    std::fs::copy(source, target)
        .map(|_| ())
        .map_err(|e| format!("cannot place {}: {e}", target.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hound::{SampleFormat, WavSpec};

    fn wav_spec(channels: u16) -> WavSpec {
        WavSpec {
            channels,
            sample_rate: 48000,
            bits_per_sample: 24,
            sample_format: SampleFormat::Int,
        }
    }

    /// A ramp so a shift is visible in the sample values, not just the count.
    fn ramp_wav(dir: &Path, name: &str, sample_frames: usize, channels: u16) -> PathBuf {
        let path = dir.join(name);
        let samples: Vec<f32> = (0..sample_frames * channels as usize)
            .map(|i| (i % 1000) as f32 / 2000.0)
            .collect();
        postkit::wav_io::write_interleaved(&path, wav_spec(channels), &samples).unwrap();
        path
    }

    fn read_samples(path: &Path) -> Vec<f32> {
        postkit::wav_io::read_interleaved(path).unwrap().1
    }

    #[test]
    fn a_delay_is_bit_exact_for_32_bit_int_pcm() {
        use postkit::wav_io::Samples;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("i32.wav");
        // values with low bits set, which the old f32 round-trip destroyed
        let samples: Vec<i32> = (0..48_000i32)
            .map(|i| i.wrapping_mul(2_654_435_761u32 as i32))
            .collect();
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Int,
        };
        postkit::wav_io::write_interleaved_exact(&path, spec, &Samples::Int(samples.clone()))
            .unwrap();

        let out = dir.path().join("delayed.wav");
        apply_audio_delay(&path, &out, 100).unwrap();

        let (_, delayed) = postkit::wav_io::read_interleaved_exact(&out).unwrap();
        let Samples::Int(delayed) = delayed else {
            panic!("32-bit int in must come back out as int");
        };
        assert_eq!(delayed.len(), samples.len());
        assert!(delayed[..4800].iter().all(|&s| s == 0), "prepended silence");
        assert_eq!(
            &delayed[4800..],
            &samples[..samples.len() - 4800],
            "every shifted sample must be bit-identical"
        );
    }

    #[test]
    fn a_delay_keeps_the_sample_count_identical() {
        let dir = tempfile::tempdir().unwrap();
        let source = ramp_wav(dir.path(), "source.wav", 48_000, 2);
        let before = read_samples(&source).len();

        for delay_ms in [250_i64, -250, 1, -1] {
            let out = dir.path().join(format!("delayed{delay_ms}.wav"));
            apply_audio_delay(&source, &out, delay_ms).unwrap();
            assert_eq!(
                read_samples(&out).len(),
                before,
                "delay of {delay_ms} ms changed the sample count"
            );
        }
    }

    #[test]
    fn a_positive_delay_prepends_silence_and_a_negative_one_drops_the_head() {
        let dir = tempfile::tempdir().unwrap();
        let source = ramp_wav(dir.path(), "source.wav", 48_000, 2);
        let original = read_samples(&source);
        // 100 ms at 48 kHz is 4800 sample frames, 9600 interleaved stereo samples
        let shift = 9600;

        let later = dir.path().join("later.wav");
        apply_audio_delay(&source, &later, 100).unwrap();
        let later = read_samples(&later);
        assert!(later[..shift].iter().all(|s| *s == 0.0), "head not silent");
        assert_eq!(later[shift], original[0]);

        let earlier = dir.path().join("earlier.wav");
        apply_audio_delay(&source, &earlier, -100).unwrap();
        let earlier = read_samples(&earlier);
        assert_eq!(earlier[0], original[shift]);
        assert!(
            earlier[original.len() - shift..].iter().all(|s| *s == 0.0),
            "tail not silent"
        );
    }

    #[test]
    fn a_delay_at_least_as_long_as_the_programme_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        // 24000 sample frames at 48 kHz is 500 ms of sound
        let source = ramp_wav(dir.path(), "short.wav", 24_000, 2);
        let error = apply_audio_delay(&source, &dir.path().join("out.wav"), 500).unwrap_err();
        assert!(error.contains("500 ms"), "{error}");
    }

    #[test]
    fn a_trim_moves_timed_text_with_the_picture() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("subs.xml");
        std::fs::write(
            &input,
            r#"<tt xmlns="http://www.w3.org/ns/ttml"><body><div>
<p begin="00:00:00.500" end="00:00:01.500">dropped, ends before the trim</p>
<p begin="00:00:01.500" end="00:00:03.000">clamped at the head</p>
<p begin="00:00:04.000" end="00:00:05.000">shifted whole</p>
<p begin="00:00:08.000" end="00:00:12.000">clamped at the tail</p>
<p begin="00:00:11.000" end="00:00:13.000">dropped, starts after the trim</p>
</div></body></tt>"#,
        )
        .unwrap();

        // keep source seconds 2..10
        let output = dir.path().join("trimmed.xml");
        trim_timed_text(&input, &output, 2.0, 10.0, 24.0).unwrap();
        let trimmed = std::fs::read_to_string(&output).unwrap();

        assert!(!trimmed.contains("dropped, ends before"), "{trimmed}");
        assert!(!trimmed.contains("dropped, starts after"), "{trimmed}");
        // the head-straddling cue starts at the trim point and keeps its tail
        assert!(
            trimmed.contains(r#"begin="00:00:00.000" end="00:00:01.000""#),
            "{trimmed}"
        );
        // a cue wholly inside just shifts back by the two second head trim
        assert!(
            trimmed.contains(r#"begin="00:00:02.000" end="00:00:03.000""#),
            "{trimmed}"
        );
        // the tail-straddling cue is cut at the end of the kept range
        assert!(
            trimmed.contains(r#"begin="00:00:06.000" end="00:00:08.000""#),
            "{trimmed}"
        );
    }

    /// Only the two times are rebuilt, so an escaped attribute value has to come
    /// through unchanged rather than escaped a second time.
    #[test]
    fn a_trim_leaves_other_cue_attributes_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("subs.xml");
        std::fs::write(
            &input,
            r#"<tt><body><div><p xml:id="a&amp;b" region="r0" begin="00:00:03.000" end="00:00:04.000">Sam &amp; Ella</p></div></body></tt>"#,
        )
        .unwrap();

        let output = dir.path().join("trimmed.xml");
        trim_timed_text(&input, &output, 2.0, 10.0, 24.0).unwrap();
        let trimmed = std::fs::read_to_string(&output).unwrap();

        assert!(trimmed.contains(r#"xml:id="a&amp;b""#), "{trimmed}");
        assert!(trimmed.contains(r#"region="r0""#), "{trimmed}");
        assert!(trimmed.contains("Sam &amp; Ella"), "{trimmed}");
        assert!(!trimmed.contains("&amp;amp;"), "{trimmed}");
    }

    /// Offset times are legal TTML and IMSC. Reading them as zero would put every
    /// such cue before the kept range and drop the lot without a word.
    #[test]
    fn offset_time_cues_move_like_clock_time_cues() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("subs.xml");
        std::fs::write(
            &input,
            r#"<tt><body><div>
<p begin="00:00:00.700" end="00:00:01.300">clock form</p>
<p begin="0.8s" end="1.2s">offset seconds</p>
<p begin="900ms" end="1100ms">offset milliseconds</p>
<p begin="24f" end="36f">offset frames</p>
</div></body></tt>"#,
        )
        .unwrap();

        // keep source seconds 0.5..1.5
        let output = dir.path().join("trimmed.xml");
        trim_timed_text(&input, &output, 0.5, 1.5, 24.0).unwrap();
        let trimmed = std::fs::read_to_string(&output).unwrap();

        for text in [
            "clock form",
            "offset seconds",
            "offset milliseconds",
            "offset frames",
        ] {
            assert!(trimmed.contains(text), "{text} was dropped: {trimmed}");
        }
        // each keeps its own metric, shifted back by the half second head trim
        assert!(trimmed.contains(r#"begin="00:00:00.200""#), "{trimmed}");
        assert!(trimmed.contains(r#"begin="0.3s""#), "{trimmed}");
        assert!(trimmed.contains(r#"begin="400ms""#), "{trimmed}");
        assert!(trimmed.contains(r#"begin="12f""#), "{trimmed}");
    }

    /// A time this cannot read must stop the trim, not silently become zero.
    #[test]
    fn an_unreadable_cue_time_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("subs.xml");
        // ticks need the document's tickRate, which this does not resolve
        std::fs::write(
            &input,
            r#"<tt><body><div><p begin="4000000t" end="8000000t">ticks</p></div></body></tt>"#,
        )
        .unwrap();

        let error =
            trim_timed_text(&input, &dir.path().join("out.xml"), 0.5, 1.5, 24.0).unwrap_err();
        assert!(error.contains("4000000t"), "{error}");
    }

    /// TTML times a timed element's children relative to it, so shifting both
    /// would move the cues twice. That shape is refused, not passed through.
    #[test]
    fn timing_on_an_element_other_than_a_cue_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("subs.xml");
        std::fs::write(
            &input,
            r#"<tt><body><div begin="00:00:01.500" end="00:00:01.900"><p>timed on the div</p></div></body></tt>"#,
        )
        .unwrap();

        let error =
            trim_timed_text(&input, &dir.path().join("out.xml"), 0.5, 1.5, 24.0).unwrap_err();
        assert!(error.contains("<div>"), "{error}");
    }

    /// Rounding must never push the frame field up to the frame rate and name a
    /// frame that does not exist.
    #[test]
    fn a_frame_field_never_reaches_the_frame_rate() {
        for fps in [24.0_f64, 25.0, 30.0, 24000.0 / 1001.0, 30000.0 / 1001.0] {
            let frames_per_second = fps.round() as u64;
            for frame in 0..(frames_per_second * 4) {
                let formatted = frame_clock_time(frame as f64 / fps, fps);
                let field: u64 = formatted.rsplit(':').next().unwrap().parse().unwrap();
                assert!(field < frames_per_second, "{formatted} at {fps} fps");
            }
        }
    }

    /// The reported case: 26 frames less a 2 frame trim is one second exactly,
    /// which float error used to render as the non-existent frame 24.
    #[test]
    fn a_head_trim_landing_on_a_whole_second_rolls_the_frame_field_over() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("subs.xml");
        std::fs::write(
            &input,
            r#"<tt><body><div><p begin="00:00:01:02" end="00:00:01:10">frames</p></div></body></tt>"#,
        )
        .unwrap();

        let output = dir.path().join("trimmed.xml");
        trim_timed_text(&input, &output, 2.0 / 24.0, 10.0, 24.0).unwrap();
        let trimmed = std::fs::read_to_string(&output).unwrap();
        assert!(
            trimmed.contains(r#"begin="00:00:01:00" end="00:00:01:08""#),
            "{trimmed}"
        );
    }

    /// Building into the same folder again with a shorter length has to replace
    /// the frames, not leave the first run's beside them.
    #[test]
    fn a_shorter_rerun_replaces_the_frames_it_does_not_add_to_them() {
        let dir = tempfile::tempdir().unwrap();
        let source: Vec<PathBuf> = (0..40)
            .map(|index| {
                let path = dir.path().join(format!("src_{index:08}.j2c"));
                std::fs::write(&path, [index as u8]).unwrap();
                path
            })
            .collect();
        let dest = dir.path().join("picture");

        write_frame_dir(&source, &dest).unwrap();
        assert_eq!(picture_frames(&dest).unwrap().len(), 40);

        write_frame_dir(&source[..18], &dest).unwrap();
        assert_eq!(picture_frames(&dest).unwrap().len(), 18);
    }

    /// A cue timed with dur would keep running past the range it was clamped
    /// into, so it fails rather than being moved wrongly.
    #[test]
    fn a_cue_timed_with_dur_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("subs.xml");
        std::fs::write(
            &input,
            r#"<tt><body><div><p begin="00:00:03.000" dur="00:00:02.000">Hello</p></div></body></tt>"#,
        )
        .unwrap();

        let error =
            trim_timed_text(&input, &dir.path().join("out.xml"), 2.0, 10.0, 24.0).unwrap_err();
        assert!(error.contains("dur"), "{error}");
    }

    #[test]
    fn a_trim_cuts_picture_and_sound_by_the_same_duration() {
        let dir = tempfile::tempdir().unwrap();
        let picture = dir.path().join("j2k");
        std::fs::create_dir_all(&picture).unwrap();
        for index in 0..48 {
            std::fs::write(picture.join(format!("frame_{index:08}.j2c")), [index as u8]).unwrap();
        }
        // 48 frames at 24 fps is two seconds, so 96000 sample frames at 48 kHz
        let audio = ramp_wav(dir.path(), "source.wav", 96_000, 2);

        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let edits = SourceEdits {
            audio_delay_ms: 0,
            trim_start_frames: 12,
            trim_end_frames: 12,
        };
        let source = CompositionSource {
            j2k_dir: Some(picture),
            audio_files: vec![audio],
            timed_text_files: vec![],
        };
        let edited = apply_source_edits(&edits, &source, &work, 24, 1).unwrap();

        assert_eq!(picture_frames(&edited.j2k_dir.unwrap()).unwrap().len(), 24);
        // half a second off each end of two seconds leaves one second of sound
        assert_eq!(read_samples(&edited.audio_files[0]).len(), 48_000 * 2);
    }

    #[test]
    fn a_trim_that_leaves_no_picture_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let picture = dir.path().join("j2k");
        std::fs::create_dir_all(&picture).unwrap();
        for index in 0..10 {
            std::fs::write(picture.join(format!("frame_{index:08}.j2c")), [0u8]).unwrap();
        }
        let edits = SourceEdits {
            audio_delay_ms: 0,
            trim_start_frames: 5,
            trim_end_frames: 5,
        };
        let source = CompositionSource {
            j2k_dir: Some(picture),
            ..Default::default()
        };
        let error = apply_source_edits(&edits, &source, dir.path(), 24, 1).unwrap_err();
        assert!(error.contains("10 picture frames"), "{error}");
    }

    /// A delay with nothing to shift would otherwise be accepted and do nothing.
    #[test]
    fn a_delay_without_a_sound_track_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let edits = SourceEdits {
            audio_delay_ms: 250,
            ..Default::default()
        };
        let source = CompositionSource {
            j2k_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let error = apply_source_edits(&edits, &source, dir.path(), 24, 1).unwrap_err();
        assert!(error.contains("sound track"), "{error}");
    }

    fn facts(picture_frames: Option<u64>, sample_frames: Option<u64>) -> SourceFacts {
        SourceFacts {
            picture: picture_frames.map(|frames| PictureFacts {
                path: PathBuf::from("/pictures"),
                frames,
            }),
            audio: sample_frames
                .map(|sample_frames| AudioFacts {
                    path: PathBuf::from("/sound.wav"),
                    sample_frames,
                    sample_rate: 48_000,
                })
                .into_iter()
                .collect(),
            fps: 24.0,
        }
    }

    /// Every refusal the edits themselves make has to fire from the plan-time
    /// check too, so nothing spends an encode finding out.
    #[test]
    fn the_plan_time_check_refuses_what_the_edits_refuse() {
        // 48 frames at 24 fps is two seconds, so 96000 sample frames at 48 kHz
        let whole = facts(Some(48), Some(96_000));

        let delay_without_sound = SourceEdits {
            audio_delay_ms: 250,
            ..Default::default()
        };
        let error = check_edits(&delay_without_sound, &facts(Some(48), None)).unwrap_err();
        assert!(error.contains("sound track"), "{error}");

        let trim_without_picture = SourceEdits {
            trim_start_frames: 2,
            ..Default::default()
        };
        let error = check_edits(&trim_without_picture, &facts(None, Some(96_000))).unwrap_err();
        assert!(error.contains("picture frames to measure"), "{error}");

        let trim_past_the_picture = SourceEdits {
            trim_start_frames: 24,
            trim_end_frames: 24,
            ..Default::default()
        };
        let error = check_edits(&trim_past_the_picture, &whole).unwrap_err();
        assert!(error.contains("48 picture frames"), "{error}");

        let delay_past_the_programme = SourceEdits {
            audio_delay_ms: 2_000,
            ..Default::default()
        };
        let error = check_edits(&delay_past_the_programme, &whole).unwrap_err();
        assert!(error.contains("2000 ms"), "{error}");
        assert!(error.contains("2000 ms of sound"), "{error}");

        // the sound is shorter than the picture, so only the audio trim refuses
        let trim_past_the_sound = SourceEdits {
            trim_start_frames: 20,
            trim_end_frames: 20,
            ..Default::default()
        };
        let error = check_edits(&trim_past_the_sound, &facts(Some(48), Some(48_000))).unwrap_err();
        assert!(error.contains("sample frames leaves nothing"), "{error}");
    }

    #[test]
    fn a_trim_the_source_can_absorb_passes_the_plan_time_check() {
        let edits = SourceEdits {
            audio_delay_ms: 250,
            trim_start_frames: 12,
            trim_end_frames: 12,
        };
        assert_eq!(check_edits(&edits, &facts(Some(48), Some(96_000))), Ok(()));
    }

    #[test]
    fn the_plan_time_check_measures_a_directory_and_a_still() {
        let dir = tempfile::tempdir().unwrap();
        let picture = dir.path().join("j2k");
        std::fs::create_dir_all(&picture).unwrap();
        for index in 0..30 {
            std::fs::write(picture.join(format!("frame_{index:08}.j2c")), [0u8]).unwrap();
        }

        let counted = probe_source_facts(Some(&picture), None, &[], 24, 1).unwrap();
        assert_eq!(counted.picture.unwrap().frames, 30);

        let held = probe_source_facts(Some(&picture), Some(240), &[], 24, 1).unwrap();
        assert_eq!(held.picture.unwrap().frames, 240);
    }

    #[test]
    fn the_plan_time_check_measures_a_wav_header() {
        let dir = tempfile::tempdir().unwrap();
        let wav = ramp_wav(dir.path(), "sound.wav", 96_000, 2);
        let measured = probe_source_facts(None, None, &[wav], 24, 1).unwrap();
        assert_eq!(measured.audio[0].sample_frames, 96_000);
        assert_eq!(measured.audio[0].sample_rate, 48_000);
    }

    #[test]
    fn a_timed_text_file_is_checked_without_writing_anything() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.xml");
        std::fs::write(
            &good,
            r#"<tt><body><div><p begin="00:00:01.000" end="00:00:02.000">fine</p></div></body></tt>"#,
        )
        .unwrap();
        assert_eq!(check_timed_text_trimmable(&good, 24.0), Ok(()));

        let timed_div = dir.path().join("div.xml");
        std::fs::write(
            &timed_div,
            r#"<tt><body><div begin="00:00:01.000" end="00:00:02.000"><p>on the div</p></div></body></tt>"#,
        )
        .unwrap();
        let error = check_timed_text_trimmable(&timed_div, 24.0).unwrap_err();
        assert!(error.contains("<div>"), "{error}");

        let with_dur = dir.path().join("dur.xml");
        std::fs::write(
            &with_dur,
            r#"<tt><body><div><p begin="00:00:01.000" dur="00:00:02.000">held</p></div></body></tt>"#,
        )
        .unwrap();
        let error = check_timed_text_trimmable(&with_dur, 24.0).unwrap_err();
        assert!(error.contains("dur"), "{error}");

        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            3,
            "the check must write nothing beside the files it read"
        );
    }

    #[test]
    fn timed_text_cues_come_back_with_their_times_and_lines() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("subs.xml");
        std::fs::write(
            &input,
            r#"<tt><body><div>
<p begin="00:00:01.500" end="00:00:03.000">first line<br/>second line</p>
<p begin="0.8s" end="1.2s">offset &amp; escaped</p>
<p>no times at all</p>
</div></body></tt>"#,
        )
        .unwrap();

        let cues = read_timed_text_cues(&input, 24.0).unwrap();
        assert_eq!(cues.len(), 2, "{cues:?}");
        assert_eq!(cues[0].start_ms, 1_500);
        assert_eq!(cues[0].end_ms, 3_000);
        assert_eq!(cues[0].lines, ["first line", "second line"]);
        assert_eq!(cues[1].start_ms, 800);
        assert_eq!(cues[1].lines, ["offset & escaped"]);
    }

    #[test]
    fn no_edits_leaves_every_path_untouched() {
        let source = CompositionSource {
            j2k_dir: Some(PathBuf::from("/pictures")),
            audio_files: vec![PathBuf::from("/sound.wav")],
            timed_text_files: vec![PathBuf::from("/subs.xml")],
        };
        let edited =
            apply_source_edits(&SourceEdits::default(), &source, Path::new("/work"), 24, 1)
                .unwrap();
        assert_eq!(edited, source);
    }
}
