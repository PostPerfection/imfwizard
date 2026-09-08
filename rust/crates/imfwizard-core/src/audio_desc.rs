use std::path::Path;

// ffmpeg's sidechaincompress caps ratio at 20 and threshold at 0.000976563
const COMPRESSOR_RATIO: f64 = 20.0;
const MIN_COMPRESSOR_THRESHOLD: f64 = 0.000_976_563;
// smooths the narration's mean square into an envelope before it is compared
const ENVELOPE_CUTOFF_HZ: f64 = 20.0;
// sidechaincompress drops however much of the main it holds unpaired when the main
// reaches its end, up to about 5000 samples, so the tail it eats is padding
const MAIN_PAD_SECONDS: f64 = 2.0;
const STDERR_TAIL_LINES: usize = 10;

/// Mix a narration track into the main audio at unity gain, reducing the main by
/// `duck_level_db` while the narration's level is above `threshold_db`.
pub fn mix_audio_description(
    main_audio: &Path,
    narration: &Path,
    output: &Path,
    duck_level_db: f64,
    threshold_db: f64,
    attack_ms: f64,
    release_ms: f64,
) -> Result<(), String> {
    let reduction_db = duck_level_db.abs();
    let compressor_slope = 1.0 - 1.0 / COMPRESSOR_RATIO;
    // the control signal sits at 0 dB, so this threshold buys exactly the reduction asked for
    let compressor_threshold = 10.0_f64.powf(-reduction_db / compressor_slope / 20.0);
    if compressor_threshold < MIN_COMPRESSOR_THRESHOLD {
        let deepest_db = -20.0 * MIN_COMPRESSOR_THRESHOLD.log10() * compressor_slope;
        return Err(format!(
            "duck level {duck_level_db} dB is deeper than the {deepest_db:.1} dB ffmpeg can duck"
        ));
    }
    // compared against a mean square, so the threshold is squared
    let narration_threshold = 10.0_f64.powf(threshold_db / 10.0);

    // the narration becomes a 0-or-1 control signal, which makes the reduction the
    // same whatever the narration's own level is. the untouched main goes into the mix
    // at weight 0, where amix takes the output's length off it
    let filter = format!(
        "[0:a]asplit=2[main_length][main_to_duck];\
         [main_to_duck]apad=pad_dur={MAIN_PAD_SECONDS}[main_padded];\
         [1:a]asplit=2[narration_mix][narration_level];\
         [narration_level]aeval=exprs=val(0)*val(0):c=same,\
         lowpass=f={ENVELOPE_CUTOFF_HZ},\
         aeval=exprs=gte(val(0)\\,{narration_threshold}):c=same,\
         apad[duck_control];\
         [main_padded][duck_control]sidechaincompress=threshold={compressor_threshold}:\
         ratio={COMPRESSOR_RATIO}:attack={attack_ms}:release={release_ms}:\
         knee=1:detection=rms:link=maximum[ducked];\
         [main_length][ducked][narration_mix]\
         amix=inputs=3:duration=first:dropout_transition=0:normalize=0:weights=0 1 1[out]"
    );

    let run = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats", "-nostdin", "-y", "-i"])
        .arg(main_audio)
        .arg("-i")
        .arg(narration)
        .args([
            "-filter_complex",
            &filter,
            "-map",
            "[out]",
            "-c:a",
            "pcm_s24le",
        ])
        .arg(output)
        .output()
        .map_err(|e| format!("Failed to run ffmpeg: {e}"))?;

    if !run.status.success() {
        let stderr = String::from_utf8_lossy(&run.stderr);
        let lines: Vec<&str> = stderr.lines().collect();
        let tail = lines[lines.len().saturating_sub(STDERR_TAIL_LINES)..].join("\n");
        return Err(format!("ffmpeg audio description mix failed:\n{tail}"));
    }

    Ok(())
}
