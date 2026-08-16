//! Advisory findings gathered before the encode.
//!
//! A hint is not a refusal: everything here builds and packages. It says the
//! result is likely to be wrong for the audience, so the front ends print it and
//! let the build go on.

use std::path::Path;

use crate::preflight::CreatePlan;
use crate::subtitle_convert::format_ttml_time;

/// One advisory finding, ready to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub text: String,
}

/// Sound peaking above this clips on some playback chains.
const LOUD_TRUE_PEAK_DBTP: f64 = -3.0;
/// A first subtitle earlier than this is easy to miss.
const FIRST_CUE_SECONDS: f64 = 4.0;
/// A cue shorter than this is hard to read.
const SHORTEST_CUE_FRAMES: f64 = 15.0;
/// Two cues closer than this read as one flicker.
const SMALLEST_CUE_GAP_FRAMES: f64 = 2.0;
const MOST_CUE_LINES: usize = 3;
/// Line lengths, in characters: the length to aim for, and the one past which
/// the text will not fit at all.
const ADVISED_LINE_CHARACTERS: usize = 52;
const MOST_LINE_CHARACTERS: usize = 79;

const MILLISECONDS_PER_SECOND: f64 = 1000.0;

/// What the hints are decided from. Kept apart from the probing so the rules can
/// be driven directly.
#[derive(Debug, Clone, Default)]
pub struct HintFacts {
    /// The measured level of each audio file the loudness pass could read.
    pub audio: Vec<AudioLevel>,
    /// Whether the composition carries sound at all, measured or not.
    pub has_audio: bool,
    pub audio_language: Option<String>,
    pub subtitles: Vec<SubtitleCues>,
    pub fps: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioLevel {
    pub file: String,
    pub true_peak_dbtp: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtitleCues {
    pub file: String,
    pub cues: Vec<HintCue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HintCue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub lines: Vec<String>,
}

/// Every hint the job raises, probing the source for the numbers it needs.
pub fn gather_hints(plan: &CreatePlan) -> Vec<Hint> {
    hints_from(&probe_hint_facts(plan))
}

fn hints_from(facts: &HintFacts) -> Vec<Hint> {
    let mut hints = Vec::new();
    if let Some(loud) = facts
        .audio
        .iter()
        .find(|level| level.true_peak_dbtp > LOUD_TRUE_PEAK_DBTP)
    {
        hints.push(Hint {
            text: format!(
                "The audio level is very high ({:.1} dBTP in {}). Reduce the gain.",
                loud.true_peak_dbtp, loud.file
            ),
        });
    }
    if facts.has_audio && named_language(facts.audio_language.as_deref()).is_none() {
        hints.push(Hint {
            text: "The sound has no language set. Set one unless it has no spoken parts."
                .to_string(),
        });
    }
    hints.extend(subtitle_hints(facts));
    hints
}

fn named_language(language: Option<&str>) -> Option<&str> {
    language.map(str::trim).filter(|value| !value.is_empty())
}

/// A cue with what the rules need around it.
struct CueInContext<'a> {
    cue: &'a HintCue,
    previous_end_ms: Option<u64>,
    is_first: bool,
    fps: f64,
}

/// How a rule words itself, given the file it found the fault in and the time of
/// the first cue that showed it.
type SayHint = fn(&str, &str) -> String;

/// One advisory rule over a cue: what counts as an offence, and what to say
/// about the first cue that offends.
struct CueRule {
    offends: fn(&CueInContext) -> bool,
    say: SayHint,
}

const CUE_RULES: [CueRule; 4] = [
    CueRule {
        offends: |context| {
            context.is_first && context.cue.start_ms < seconds_to_milliseconds(FIRST_CUE_SECONDS)
        },
        say: |file, at| {
            format!(
                "The first subtitle in {file} starts at {at}. Put it at least {FIRST_CUE_SECONDS:.0} seconds in, or it is easy to miss."
            )
        },
    },
    CueRule {
        offends: |context| {
            context.cue.end_ms.saturating_sub(context.cue.start_ms)
                < frames_to_milliseconds(SHORTEST_CUE_FRAMES, context.fps)
        },
        say: |file, at| {
            format!(
                "A subtitle in {file} at {at} lasts less than {SHORTEST_CUE_FRAMES:.0} frames. Make every subtitle at least that long."
            )
        },
    },
    CueRule {
        offends: |context| match context.previous_end_ms {
            Some(previous_end_ms) => {
                context.cue.start_ms
                    < previous_end_ms + frames_to_milliseconds(SMALLEST_CUE_GAP_FRAMES, context.fps)
            }
            None => false,
        },
        say: |file, at| {
            format!(
                "A subtitle in {file} at {at} starts less than {SMALLEST_CUE_GAP_FRAMES:.0} frames after the one before it ends. Leave at least that gap."
            )
        },
    },
    CueRule {
        offends: |context| context.cue.lines.len() > MOST_CUE_LINES,
        say: |file, at| {
            format!(
                "A subtitle in {file} at {at} has more than {MOST_CUE_LINES} lines. Use no more than {MOST_CUE_LINES}."
            )
        },
    },
];

fn subtitle_hints(facts: &HintFacts) -> Vec<Hint> {
    let mut hints: Vec<Hint> = CUE_RULES
        .iter()
        .filter_map(|rule| first_offence(facts, rule))
        .collect();
    hints.extend(line_length_hint(facts));
    hints
}

/// The first cue in reading order that breaks a rule, said once for the whole
/// job rather than once per cue.
fn first_offence(facts: &HintFacts, rule: &CueRule) -> Option<Hint> {
    for subtitle in &facts.subtitles {
        let mut previous_end_ms = None;
        for (index, cue) in subtitle.cues.iter().enumerate() {
            let context = CueInContext {
                cue,
                previous_end_ms,
                is_first: index == 0,
                fps: facts.fps,
            };
            if (rule.offends)(&context) {
                return Some(Hint {
                    text: (rule.say)(&subtitle.file, &format_ttml_time(cue.start_ms)),
                });
            }
            previous_end_ms = Some(cue.end_ms);
        }
    }
    None
}

/// A line past the hard limit is the same fault as one past the advised length,
/// said more strongly, so only the stronger hint is raised.
fn line_length_hint(facts: &HintFacts) -> Option<Hint> {
    let limits: [(usize, SayHint); 2] = [
        (MOST_LINE_CHARACTERS, |file, at| {
            format!(
                "A subtitle line in {file} at {at} is longer than {MOST_LINE_CHARACTERS} characters. Cut it to {MOST_LINE_CHARACTERS} at most."
            )
        }),
        (ADVISED_LINE_CHARACTERS, |file, at| {
            format!(
                "A subtitle line in {file} at {at} is longer than {ADVISED_LINE_CHARACTERS} characters. Keep lines to {ADVISED_LINE_CHARACTERS} where you can."
            )
        }),
    ];
    for (characters, say) in limits {
        for subtitle in &facts.subtitles {
            let offender = subtitle.cues.iter().find(|cue| {
                cue.lines
                    .iter()
                    .any(|line| line.chars().count() > characters)
            });
            if let Some(cue) = offender {
                return Some(Hint {
                    text: say(&subtitle.file, &format_ttml_time(cue.start_ms)),
                });
            }
        }
    }
    None
}

const fn seconds_to_milliseconds(seconds: f64) -> u64 {
    (seconds * MILLISECONDS_PER_SECOND) as u64
}

fn frames_to_milliseconds(frames: f64, fps: f64) -> u64 {
    (frames / fps.max(1.0) * MILLISECONDS_PER_SECOND).round() as u64
}

fn probe_hint_facts(plan: &CreatePlan) -> HintFacts {
    let fps = plan.fps();
    let audio = plan
        .audio_files
        .iter()
        .filter_map(|path| {
            // a WAV the loudness pass cannot read is the encode's problem to report
            let measured = postkit::loudness::measure_loudness(path);
            measured.success.then(|| AudioLevel {
                file: short_name(path),
                true_peak_dbtp: measured.true_peak_dbtp,
            })
        })
        .collect();

    let mut subtitles = Vec::new();
    for path in &plan.timed_text_files {
        if let Ok(cues) = crate::source_edits::read_timed_text_cues(path, fps) {
            subtitles.push(SubtitleCues {
                file: short_name(path),
                cues: cues
                    .into_iter()
                    .map(|cue| HintCue {
                        start_ms: cue.start_ms,
                        end_ms: cue.end_ms,
                        lines: cue.lines,
                    })
                    .collect(),
            });
        }
    }
    if let Some(burn) = &plan.burn_subtitle
        && let Ok(cues) = crate::subtitle_burn::load_styled_cues(burn)
    {
        subtitles.push(SubtitleCues {
            file: short_name(burn),
            cues: cues
                .into_iter()
                .map(|cue| HintCue {
                    start_ms: cue.start_ms,
                    end_ms: cue.end_ms,
                    lines: cue
                        .plain_text()
                        .lines()
                        .map(|line| line.trim().to_string())
                        .filter(|line| !line.is_empty())
                        .collect(),
                })
                .collect(),
        });
    }

    HintFacts {
        audio,
        has_audio: !plan.audio_files.is_empty(),
        audio_language: plan.audio_language.clone(),
        subtitles,
        fps,
    }
}

fn short_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FPS: f64 = 24.0;

    fn cue(start_ms: u64, end_ms: u64, lines: &[&str]) -> HintCue {
        HintCue {
            start_ms,
            end_ms,
            lines: lines.iter().map(|line| line.to_string()).collect(),
        }
    }

    fn with_cues(cues: Vec<HintCue>) -> HintFacts {
        HintFacts {
            subtitles: vec![SubtitleCues {
                file: "subs.srt".to_string(),
                cues,
            }],
            fps: FPS,
            ..Default::default()
        }
    }

    fn texts(facts: &HintFacts) -> Vec<String> {
        hints_from(facts)
            .into_iter()
            .map(|hint| hint.text)
            .collect()
    }

    fn mentions(facts: &HintFacts, needle: &str) -> bool {
        texts(facts).iter().any(|text| text.contains(needle))
    }

    #[test]
    fn a_loud_track_is_named_with_its_peak_and_a_quiet_one_is_not() {
        let loud = HintFacts {
            audio: vec![AudioLevel {
                file: "sound.wav".to_string(),
                true_peak_dbtp: -0.4,
            }],
            has_audio: true,
            audio_language: Some("en".to_string()),
            ..Default::default()
        };
        assert!(
            mentions(&loud, "-0.4 dBTP in sound.wav"),
            "{:?}",
            texts(&loud)
        );

        let quiet = HintFacts {
            audio: vec![AudioLevel {
                file: "sound.wav".to_string(),
                true_peak_dbtp: LOUD_TRUE_PEAK_DBTP,
            }],
            ..loud.clone()
        };
        assert!(!mentions(&quiet, "audio level"), "{:?}", texts(&quiet));
    }

    #[test]
    fn sound_without_a_language_is_hinted_and_sound_with_one_is_not() {
        let unset = HintFacts {
            has_audio: true,
            ..Default::default()
        };
        assert!(mentions(&unset, "no language set"));

        let blank = HintFacts {
            audio_language: Some("  ".to_string()),
            ..unset.clone()
        };
        assert!(mentions(&blank, "no language set"));

        let set = HintFacts {
            audio_language: Some("de-DE".to_string()),
            ..unset.clone()
        };
        assert!(!mentions(&set, "no language set"));

        let silent = HintFacts::default();
        assert!(!mentions(&silent, "no language set"));
    }

    #[test]
    fn a_first_cue_before_four_seconds_is_hinted_and_one_at_four_is_not() {
        let early = with_cues(vec![cue(3_999, 10_000, &["hello"])]);
        assert!(
            mentions(&early, "starts at 00:00:03.999"),
            "{:?}",
            texts(&early)
        );

        let late = with_cues(vec![cue(4_000, 10_000, &["hello"])]);
        assert!(!mentions(&late, "at least 4 seconds"), "{:?}", texts(&late));
    }

    /// 15 frames at 24 fps is 625 ms.
    #[test]
    fn a_cue_shorter_than_fifteen_frames_is_hinted_and_one_exactly_that_long_is_not() {
        let short = with_cues(vec![cue(10_000, 10_624, &["hello"])]);
        assert!(
            mentions(&short, "less than 15 frames"),
            "{:?}",
            texts(&short)
        );

        let long_enough = with_cues(vec![cue(10_000, 10_625, &["hello"])]);
        assert!(
            !mentions(&long_enough, "less than 15 frames"),
            "{:?}",
            texts(&long_enough)
        );
    }

    /// 2 frames at 24 fps is 83 ms.
    #[test]
    fn cues_closer_than_two_frames_are_hinted_and_an_overlap_counts() {
        let tight = with_cues(vec![
            cue(10_000, 12_000, &["first"]),
            cue(12_082, 14_000, &["second"]),
        ]);
        assert!(
            mentions(&tight, "less than 2 frames after"),
            "{:?}",
            texts(&tight)
        );

        let overlapping = with_cues(vec![
            cue(10_000, 12_000, &["first"]),
            cue(11_000, 14_000, &["second"]),
        ]);
        assert!(mentions(&overlapping, "less than 2 frames after"));

        let spaced = with_cues(vec![
            cue(10_000, 12_000, &["first"]),
            cue(12_083, 14_000, &["second"]),
        ]);
        assert!(
            !mentions(&spaced, "less than 2 frames after"),
            "{:?}",
            texts(&spaced)
        );
    }

    #[test]
    fn more_than_three_lines_is_hinted_and_three_is_not() {
        let four = with_cues(vec![cue(10_000, 12_000, &["a", "b", "c", "d"])]);
        assert!(mentions(&four, "more than 3 lines"), "{:?}", texts(&four));

        let three = with_cues(vec![cue(10_000, 12_000, &["a", "b", "c"])]);
        assert!(!mentions(&three, "more than 3 lines"));
    }

    #[test]
    fn a_long_line_is_hinted_and_the_hard_limit_replaces_the_advised_one() {
        let advised = with_cues(vec![cue(10_000, 12_000, &["x".repeat(53).as_str()])]);
        assert!(
            mentions(&advised, "longer than 52 characters"),
            "{:?}",
            texts(&advised)
        );
        assert!(!mentions(&advised, "longer than 79 characters"));

        let at_the_limit = with_cues(vec![cue(10_000, 12_000, &["x".repeat(52).as_str()])]);
        assert!(!mentions(&at_the_limit, "characters"));

        let hard = with_cues(vec![cue(10_000, 12_000, &["x".repeat(80).as_str()])]);
        assert!(
            mentions(&hard, "longer than 79 characters"),
            "{:?}",
            texts(&hard)
        );
        assert!(
            !mentions(&hard, "longer than 52 characters"),
            "the 79 hint replaces the 52 one: {:?}",
            texts(&hard)
        );
    }

    /// Characters, not bytes: a line of accented letters is as long as it looks.
    #[test]
    fn line_length_counts_characters_not_bytes() {
        let accented = with_cues(vec![cue(10_000, 12_000, &["é".repeat(52).as_str()])]);
        assert!(!mentions(&accented, "characters"), "{:?}", texts(&accented));
    }

    /// Each rule speaks once for the whole job, however many cues break it.
    #[test]
    fn a_rule_is_said_once_however_many_cues_break_it() {
        let facts = with_cues(vec![
            cue(10_000, 10_100, &["first"]),
            cue(20_000, 20_100, &["second"]),
            cue(30_000, 30_100, &["third"]),
        ]);
        let said = texts(&facts)
            .iter()
            .filter(|text| text.contains("less than 15 frames"))
            .count();
        assert_eq!(said, 1);
    }

    #[test]
    fn a_clean_job_raises_nothing() {
        let facts = HintFacts {
            audio: vec![AudioLevel {
                file: "sound.wav".to_string(),
                true_peak_dbtp: -6.0,
            }],
            has_audio: true,
            audio_language: Some("en".to_string()),
            subtitles: vec![SubtitleCues {
                file: "subs.srt".to_string(),
                cues: vec![
                    cue(5_000, 7_000, &["a line", "another"]),
                    cue(8_000, 10_000, &["one more"]),
                ],
            }],
            fps: FPS,
        };
        assert_eq!(hints_from(&facts), vec![]);
    }
}
