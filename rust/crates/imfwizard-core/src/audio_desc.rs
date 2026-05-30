use std::path::Path;

/// Mix an audio description track with main audio using sidechain compression (ducking).
///
/// When narration is present, the main audio level is reduced by `duck_level_db`.
/// Uses ffmpeg's `sidechaincompress` filter.
pub fn mix_audio_description(
    main_audio: &Path,
    narration: &Path,
    output: &Path,
    duck_level_db: f64,
    threshold_db: f64,
    attack_ms: f64,
    release_ms: f64,
) -> Result<(), String> {
    // Convert duck level to ratio for the sidechain compressor
    // ratio of ~20:1 with appropriate threshold achieves the ducking effect
    let ratio = 20.0;

    let filter = format!(
        "[0:a][1:a]sidechaincompress=threshold={}:ratio={}:attack={}:release={}:level_sc=1[ducked];\
         [ducked][1:a]amix=inputs=2:duration=first:dropout_transition=0:weights=1 {}[out]",
        threshold_db,
        ratio,
        attack_ms,
        release_ms,
        // Narration volume relative to ducked main
        format_args!("{:.1}", 10.0_f64.powf(duck_level_db.abs() / 20.0).recip())
    );

    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-i"])
        .arg(main_audio)
        .args(["-i"])
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
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .status()
        .map_err(|e| format!("Failed to run ffmpeg: {e}"))?;

    if !status.success() {
        return Err("ffmpeg audio description mix failed".to_string());
    }

    Ok(())
}
