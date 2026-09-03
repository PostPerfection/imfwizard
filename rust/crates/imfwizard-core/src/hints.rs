//! Advisory findings gathered before the encode.
//!
//! Every rule is postkit's: this reads the plan into the facts the rules are
//! decided from. A hint is not a refusal, so the front ends print these and let
//! the build go on.

use std::path::Path;

use postkit::hints::{
    AudioLevel, Hint, HintCue, SubtitleCues, audio_language_hint, audio_level_hint, subtitle_hints,
};

use crate::preflight::CreatePlan;

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

/// Every hint the job raises, probing the source for the numbers it needs.
pub fn gather_hints(plan: &CreatePlan) -> Vec<Hint> {
    hints_from(&probe_hint_facts(plan))
}

fn hints_from(facts: &HintFacts) -> Vec<Hint> {
    let mut hints = Vec::new();
    hints.extend(audio_level_hint(&facts.audio));
    hints.extend(audio_language_hint(
        facts.has_audio,
        facts.audio_language.as_deref(),
    ));
    hints.extend(subtitle_hints(&facts.subtitles, facts.fps));
    hints
}

fn probe_hint_facts(plan: &CreatePlan) -> HintFacts {
    let fps = plan.fps();
    let audio = plan
        .audio_files
        .iter()
        .filter_map(|path| {
            // a WAV the true peak pass cannot read is the encode's problem to report
            postkit::loudness::measure_true_peak_dbtp(path)
                .ok()
                .map(|true_peak_dbtp| AudioLevel {
                    file: short_name(path),
                    true_peak_dbtp,
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

    /// Every rule the composition is held to is reached, sound and subtitles
    /// both.
    #[test]
    fn a_job_raises_the_sound_and_the_subtitle_hints_together() {
        let facts = HintFacts {
            audio: vec![AudioLevel {
                file: "sound.wav".to_string(),
                true_peak_dbtp: -0.4,
            }],
            has_audio: true,
            audio_language: None,
            subtitles: vec![SubtitleCues {
                file: "subs.srt".to_string(),
                cues: vec![HintCue {
                    start_ms: 1_000,
                    end_ms: 3_000,
                    lines: vec!["hello".to_string()],
                }],
            }],
            fps: 24.0,
        };
        let texts: Vec<String> = hints_from(&facts)
            .into_iter()
            .map(|hint| hint.text)
            .collect();

        assert!(
            texts.iter().any(|text| text.contains("-0.4 dBTP")),
            "{texts:?}"
        );
        assert!(
            texts.iter().any(|text| text.contains("no language set")),
            "{texts:?}"
        );
        assert!(
            texts
                .iter()
                .any(|text| text.contains("starts at 00:00:01.000")),
            "{texts:?}"
        );
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
                cues: vec![HintCue {
                    start_ms: 5_000,
                    end_ms: 7_000,
                    lines: vec!["a line".to_string()],
                }],
            }],
            fps: 24.0,
        };
        assert_eq!(hints_from(&facts), vec![]);
    }
}
