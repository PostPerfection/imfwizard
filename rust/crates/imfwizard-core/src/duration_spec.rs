//! Duration specs as the CLI and GUI spell them: "48f" for frames, "2s" for
//! seconds. dcpwizard uses the same two forms.

const SYNTAX: &str = "expected frames like 48f or seconds like 2s";

/// Parse a duration spec into a frame count at `fps_num`/`fps_den`.
///
/// The unit suffix is required: a bare number would otherwise be read as frames
/// by one caller and seconds by the next.
pub fn parse_duration_frames(spec: &str, fps_num: u32, fps_den: u32) -> Result<u64, String> {
    let spec = spec.trim();
    let Some(unit) = spec.chars().next_back() else {
        return Err(format!("empty duration, {SYNTAX}"));
    };
    let value = &spec[..spec.len() - unit.len_utf8()];

    match unit.to_ascii_lowercase() {
        'f' => value
            .parse::<u64>()
            .map_err(|_| format!("bad frame count '{spec}', {SYNTAX}")),
        's' => {
            let seconds = value
                .parse::<f64>()
                .map_err(|_| format!("bad duration '{spec}', {SYNTAX}"))?;
            if !seconds.is_finite() || seconds < 0.0 {
                return Err(format!("bad duration '{spec}', {SYNTAX}"));
            }
            let fps = fps_num.max(1) as f64 / fps_den.max(1) as f64;
            Ok((seconds * fps).round() as u64)
        }
        _ => Err(format!("bad duration '{spec}', {SYNTAX}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_and_seconds_both_land_on_a_frame_count() {
        assert_eq!(parse_duration_frames("48f", 24, 1).unwrap(), 48);
        assert_eq!(parse_duration_frames("2s", 24, 1).unwrap(), 48);
        assert_eq!(parse_duration_frames("2s", 25, 1).unwrap(), 50);
        assert_eq!(parse_duration_frames("0f", 24, 1).unwrap(), 0);
    }

    #[test]
    fn fractional_seconds_round_to_the_nearest_frame() {
        assert_eq!(parse_duration_frames("0.5s", 24, 1).unwrap(), 12);
        assert_eq!(parse_duration_frames("1s", 24000, 1001).unwrap(), 24);
    }

    #[test]
    fn a_missing_or_unknown_unit_is_refused() {
        for spec in ["48", "", "2x", "2 s", "f", "-1s", "1.5f"] {
            let error = parse_duration_frames(spec, 24, 1).unwrap_err();
            assert!(error.contains("48f"), "{spec} gave {error}");
        }
    }
}
