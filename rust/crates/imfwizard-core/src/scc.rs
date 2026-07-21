//! Scenarist Closed Caption (.scc) CEA-608 decoding, pop-on captions only.
//!
//! SCC timecodes are interpreted as 29.97fps (30000/1001), applying drop-frame
//! frame-count correction when the frames separator is ';'. Roll-up, paint-on,
//! and text-mode captions fail loud: only the common pop-on case is handled.

use postkit::subtitle_retime::SrtCue;

/// Parse an SCC file into cues. Errors on unsupported (non pop-on) caption modes.
pub fn parse_scc(content: &str) -> Result<Vec<SrtCue>, String> {
    let mut dec = Decoder::default();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("Scenarist_SCC") {
            continue;
        }
        let Some((tc, data)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let tc_ms = parse_timecode(tc.trim())?;
        for word in data.split_whitespace() {
            let raw = u16::from_str_radix(word, 16)
                .map_err(|_| format!("invalid SCC hex word: {word}"))?;
            dec.feed(raw, tc_ms)?;
        }
        dec.last_tc = tc_ms;
    }
    dec.finish();
    Ok(dec.cues)
}

/// SCC HH:MM:SS:FF (or ;FF for drop-frame) to milliseconds at 29.97fps.
fn parse_timecode(tc: &str) -> Result<u64, String> {
    let drop_frame = tc.contains(';') || tc.contains(',');
    let parts: Vec<&str> = tc.split([':', ';', ',']).collect();
    if parts.len() != 4 {
        return Err(format!("invalid SCC timecode: {tc}"));
    }
    let p = |s: &str| {
        s.parse::<u64>()
            .map_err(|_| format!("invalid SCC timecode: {tc}"))
    };
    let (hh, mm, ss, ff) = (p(parts[0])?, p(parts[1])?, p(parts[2])?, p(parts[3])?);
    let mut frames = (hh * 3600 + mm * 60 + ss) * 30 + ff;
    if drop_frame {
        let total_min = hh * 60 + mm;
        frames -= 2 * (total_min - total_min / 10);
    }
    Ok(frames * 1001 / 30)
}

#[derive(Default)]
struct Decoder {
    popon: bool,
    back: Vec<String>,
    displayed: Option<(u64, String)>,
    cues: Vec<SrtCue>,
    prev_ctrl: Option<u16>,
    last_tc: u64,
}

impl Decoder {
    fn feed(&mut self, raw: u16, tc_ms: u64) -> Result<(), String> {
        // strip odd-parity bits
        let b0 = ((raw >> 8) & 0x7f) as u8;
        let b1 = (raw & 0x7f) as u8;
        let stripped = ((b0 as u16) << 8) | b1 as u16;

        if (0x10..=0x1f).contains(&b0) {
            // control codes are doubled for reliability; skip the repeat
            if self.prev_ctrl == Some(stripped) {
                self.prev_ctrl = None;
                return Ok(());
            }
            self.prev_ctrl = Some(stripped);
            return self.control(b0, b1, tc_ms);
        }

        self.prev_ctrl = None;
        self.push_char(b0);
        self.push_char(b1);
        Ok(())
    }

    fn control(&mut self, b0: u8, b1: u8, tc_ms: u64) -> Result<(), String> {
        // only channel 1 (0x10-0x17); ignore channel 2 (0x18-0x1f)
        if !(0x10..=0x17).contains(&b0) {
            return Ok(());
        }
        // miscellaneous command set
        if b0 == 0x14 && (0x20..=0x2f).contains(&b1) {
            match b1 {
                0x20 => self.popon = true,           // RCL, resume caption loading
                0x2e => self.back.clear(),           // ENM, erase non-displayed memory
                0x2f => self.end_of_caption(tc_ms),  // EOC, flip buffers
                0x2c => self.erase_displayed(tc_ms), // EDM, erase displayed memory
                0x21 => {
                    if let Some(l) = self.back.last_mut() {
                        l.pop();
                    }
                }
                0x24 | 0x28 => {} // DER, FON: ignore
                0x25..=0x27 => {
                    return Err("SCC roll-up captions are not supported (only pop-on)".into());
                }
                0x29 => {
                    return Err("SCC paint-on captions are not supported (only pop-on)".into());
                }
                0x2a | 0x2b => {
                    return Err("SCC text-mode captions are not supported (only pop-on)".into());
                }
                0x2d => {
                    return Err("SCC roll-up captions are not supported (only pop-on)".into());
                }
                _ => {}
            }
            return Ok(());
        }
        // PAC: row/indent address. Treat as a line break in a multi-row caption.
        if (0x40..=0x7f).contains(&b1) {
            if self.back.last().is_some_and(|l| !l.is_empty()) {
                self.back.push(String::new());
            }
            return Ok(());
        }
        // special character set (0x1130-0x113f)
        if b0 == 0x11 && (0x30..=0x3f).contains(&b1) {
            if let Some(c) = special_char(b1) {
                self.push(c);
            }
            return Ok(());
        }
        // mid-row style (0x1120-0x112f), extended chars (0x12/0x13), tab offset
        // (0x17): not rendered here, skip.
        Ok(())
    }

    fn end_of_caption(&mut self, tc_ms: u64) {
        if let Some((start, text)) = self.displayed.take() {
            self.emit(start, tc_ms, text);
        }
        let text = self.back_text();
        self.displayed = Some((tc_ms, text));
        self.back.clear();
    }

    fn erase_displayed(&mut self, tc_ms: u64) {
        if let Some((start, text)) = self.displayed.take() {
            self.emit(start, tc_ms, text);
        }
    }

    fn emit(&mut self, start: u64, end: u64, text: String) {
        if text.trim().is_empty() || end <= start {
            return;
        }
        self.cues.push(SrtCue {
            index: self.cues.len() as u32 + 1,
            start_ms: start,
            end_ms: end,
            text,
        });
    }

    fn back_text(&self) -> String {
        self.back
            .iter()
            .map(|l| l.trim_end())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn push_char(&mut self, byte: u8) {
        if let Some(c) = basic_char(byte) {
            self.push(c);
        }
    }

    fn push(&mut self, c: char) {
        if !self.popon {
            return;
        }
        if self.back.is_empty() {
            self.back.push(String::new());
        }
        self.back.last_mut().unwrap().push(c);
    }

    fn finish(&mut self) {
        if let Some((start, text)) = self.displayed.take() {
            let end = (self.last_tc).max(start + 2000);
            self.emit(start, end, text);
        }
    }
}

/// CEA-608 basic character set (0x20-0x7f), with the standard non-ASCII slots.
fn basic_char(byte: u8) -> Option<char> {
    if !(0x20..=0x7f).contains(&byte) {
        return None;
    }
    Some(match byte {
        0x2a => 'á',
        0x5c => 'é',
        0x5e => 'í',
        0x5f => 'ó',
        0x60 => 'ú',
        0x7b => 'ç',
        0x7c => '÷',
        0x7d => 'Ñ',
        0x7e => 'ñ',
        0x7f => '█',
        b => b as char,
    })
}

/// CEA-608 special character set (0x1130-0x113f).
fn special_char(b1: u8) -> Option<char> {
    Some(match b1 {
        0x30 => '®',
        0x31 => '°',
        0x32 => '½',
        0x33 => '¿',
        0x34 => '™',
        0x35 => '¢',
        0x36 => '£',
        0x37 => '♪',
        0x38 => 'à',
        0x39 => ' ',
        0x3a => 'è',
        0x3b => 'â',
        0x3c => 'ê',
        0x3d => 'î',
        0x3e => 'ô',
        0x3f => 'û',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_timecode_nondrop_2997() {
        // frame 30 at 29.97fps
        assert_eq!(parse_timecode("00:00:01:00").unwrap(), 1001);
        assert_eq!(parse_timecode("00:00:03:00").unwrap(), 3003);
    }

    #[test]
    fn popon_caption_decodes() {
        // RCL, ENM, PAC(row15), "HELLO", EOC at 1s; EDM at 3s
        let scc = "Scenarist_SCC V1.0\n\n\
00:00:01:00\t9420 9420 942e 942e 9470 9470 4845 4c4c 4f80 942f 942f\n\n\
00:00:03:00\t942c 942c\n";
        let cues = parse_scc(scc).unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "HELLO");
        assert_eq!(cues[0].start_ms, 1001);
        assert_eq!(cues[0].end_ms, 3003);
    }

    #[test]
    fn two_row_caption_joins_with_newline() {
        // "HI" on row 14, PAC to row 15, "YOU", displayed at 1s
        let scc = "Scenarist_SCC V1.0\n\n\
00:00:01:00\t9420 9420 9440 9440 4849 9470 9470 594f 5580 942f 942f\n\n\
00:00:04:00\t942c 942c\n";
        let cues = parse_scc(scc).unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "HI\nYOU");
    }

    #[test]
    fn rollup_fails_loud() {
        // RU2 (0x9425)
        let scc = "Scenarist_SCC V1.0\n\n00:00:01:00\t9425 9425\n";
        let err = parse_scc(scc).unwrap_err();
        assert!(err.contains("roll-up"), "got: {err}");
    }
}
