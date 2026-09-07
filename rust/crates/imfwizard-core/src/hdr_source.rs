use std::path::Path;

use asdcplib::jp2k::TRANSFER_CHARACTERISTIC_ST2084;
use postkit::encode::{InputType, detect_input_type};

use crate::hdr_wcg::{HdrWcg, TRANSFER_CHARACTERISTIC_HLG, presets_with_transfer};

const PQ_TRANSFER_TAG: &str = "smpte2084";
const HLG_TRANSFER_TAG: &str = "arib-std-b67";
const DOLBY_VISION_SIDE_DATA: &str = "DOVI";
// both front ends read these refusals, so each names the flag and the control that carry the preset
const PRESET_CONTROLS: &str = "--hdr, the HDR panel control in the GUI";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceHdr {
    Untagged,
    Sdr,
    Pq,
    Hlg,
    DolbyVision,
}

impl SourceHdr {
    fn describe(self) -> String {
        match self {
            SourceHdr::Untagged => "no transfer characteristic".into(),
            SourceHdr::Sdr => "an SDR transfer characteristic".into(),
            SourceHdr::Pq => format!("the PQ transfer characteristic ({PQ_TRANSFER_TAG})"),
            SourceHdr::Hlg => format!("the HLG transfer characteristic ({HLG_TRANSFER_TAG})"),
            SourceHdr::DolbyVision => "a Dolby Vision RPU".into(),
        }
    }

    fn presets(self) -> String {
        match self {
            SourceHdr::Hlg => presets_with_transfer(&TRANSFER_CHARACTERISTIC_HLG),
            _ => presets_with_transfer(&TRANSFER_CHARACTERISTIC_ST2084),
        }
    }

    fn is_hdr(self) -> bool {
        matches!(
            self,
            SourceHdr::Pq | SourceHdr::Hlg | SourceHdr::DolbyVision
        )
    }
}

// a j2k or image sequence has no stream metadata to read
pub fn probe(picture: &Path) -> Result<SourceHdr, String> {
    if detect_input_type(picture) != InputType::Video {
        return Ok(SourceHdr::Untagged);
    }
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_streams",
            "-of",
            "json",
        ])
        .arg(picture)
        .output()
        .map_err(|e| format!("Failed to run ffprobe on {}: {e}", picture.display()))?;
    if !out.status.success() {
        return Err(format!(
            "ffprobe failed to read {}: {}",
            picture.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(classify(&String::from_utf8_lossy(&out.stdout)))
}

fn classify(ffprobe_json: &str) -> SourceHdr {
    let probed: serde_json::Value = serde_json::from_str(ffprobe_json).unwrap_or_default();
    let stream = &probed["streams"][0];
    if stream["side_data_list"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|side_data| {
            side_data["side_data_type"]
                .as_str()
                .is_some_and(|kind| kind.contains(DOLBY_VISION_SIDE_DATA))
        })
    {
        return SourceHdr::DolbyVision;
    }
    match stream["color_transfer"].as_str() {
        Some(PQ_TRANSFER_TAG) => SourceHdr::Pq,
        Some(HLG_TRANSFER_TAG) => SourceHdr::Hlg,
        Some("unknown") | None => SourceHdr::Untagged,
        Some(_) => SourceHdr::Sdr,
    }
}

pub fn check_source_matches_preset(source: SourceHdr, hdr: Option<&HdrWcg>) -> Result<(), String> {
    let Some(hdr) = hdr else {
        if source.is_hdr() {
            return Err(format!(
                "the source carries {}, which --hdr {} declares: without it the picture is \
                 written as Rec.709 SDR ({PRESET_CONTROLS})",
                source.describe(),
                source.presets()
            ));
        }
        return Ok(());
    };
    let agrees = match source {
        SourceHdr::Pq | SourceHdr::DolbyVision => !hdr.is_hlg(),
        SourceHdr::Hlg => hdr.is_hlg(),
        SourceHdr::Untagged | SourceHdr::Sdr => true,
    };
    if agrees {
        return Ok(());
    }
    Err(format!(
        "the source carries {} and --hdr {} declares {}: package it as --hdr {} instead \
         ({PRESET_CONTROLS})",
        source.describe(),
        hdr.preset_name(),
        if hdr.is_hlg() { "HLG" } else { "PQ" },
        source.presets()
    ))
}

pub fn fill_light_levels_from_rpu(picture: &Path, hdr: HdrWcg) -> Result<HdrWcg, String> {
    let Some(summary) = postkit::dolby_vision::read_dolby_vision(picture)? else {
        return Ok(hdr);
    };
    light_levels_from_summary(&summary, hdr)
}

fn light_levels_from_summary(
    summary: &postkit::dolby_vision::DolbyVisionSummary,
    hdr: HdrWcg,
) -> Result<HdrWcg, String> {
    postkit::dolby_vision::refuse_undecodable_dolby_vision(summary)?;
    // the flags are the operator's own measurement, so they win over the RPU
    if hdr.max_cll.is_some() || hdr.max_fall.is_some() {
        return Ok(hdr);
    }
    let nits =
        |level: Option<f32>| level.map(|nits| nits.round().clamp(0.0, f32::from(u16::MAX)) as u16);
    hdr.with_content_light_levels(
        nits(summary.max_content_light_level_nits),
        nits(summary.max_frame_average_light_level_nits),
    )
}

pub fn resolve(picture: Option<&Path>, hdr: Option<HdrWcg>) -> Result<Option<HdrWcg>, String> {
    let Some(picture) = picture else {
        return Ok(hdr);
    };
    let source = probe(picture)?;
    check_source_matches_preset(source, hdr.as_ref())?;
    match hdr {
        Some(hdr) if source == SourceHdr::DolbyVision => {
            fill_light_levels_from_rpu(picture, hdr).map(Some)
        }
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream_json(fields: &str) -> String {
        format!("{{\"streams\":[{{{fields}}}]}}")
    }

    #[test]
    fn transfer_tags_classify_the_source() {
        assert_eq!(
            classify(&stream_json("\"color_transfer\":\"smpte2084\"")),
            SourceHdr::Pq
        );
        assert_eq!(
            classify(&stream_json("\"color_transfer\":\"arib-std-b67\"")),
            SourceHdr::Hlg
        );
        assert_eq!(
            classify(&stream_json("\"color_transfer\":\"bt709\"")),
            SourceHdr::Sdr
        );
        // ffprobe leaves the key out for an untagged stream
        assert_eq!(
            classify(&stream_json("\"width\":1920")),
            SourceHdr::Untagged
        );
        assert_eq!(
            classify(&stream_json(
                "\"side_data_list\":[{\"side_data_type\":\"DOVI configuration record\",\
                 \"dv_profile\":8}],\"color_transfer\":\"smpte2084\""
            )),
            SourceHdr::DolbyVision
        );
    }

    #[test]
    fn an_hdr_source_without_a_preset_names_the_preset_to_pass() {
        let error = check_source_matches_preset(SourceHdr::Hlg, None).unwrap_err();
        assert!(error.contains("hlg-bt2020"), "{error}");
        assert!(error.contains("Rec.709 SDR"), "{error}");

        let error = check_source_matches_preset(SourceHdr::Pq, None).unwrap_err();
        assert!(error.contains("pq-bt2020 or pq-p3d65"), "{error}");

        let error = check_source_matches_preset(SourceHdr::DolbyVision, None).unwrap_err();
        assert!(error.contains("Dolby Vision RPU"), "{error}");
        // a GUI user has no flags, so the refusal names their control too
        assert!(
            error.contains("the HDR panel control in the GUI"),
            "{error}"
        );
    }

    // an untagged source says nothing about its colour, so the preset is the only claim
    #[test]
    fn an_untagged_or_sdr_source_is_left_to_the_preset() {
        for source in [SourceHdr::Untagged, SourceHdr::Sdr] {
            assert_eq!(check_source_matches_preset(source, None), Ok(()));
            for preset in ["pq-bt2020", "hlg-bt2020"] {
                let hdr = HdrWcg::from_flags(preset, None).unwrap();
                assert_eq!(check_source_matches_preset(source, Some(&hdr)), Ok(()));
            }
        }
    }

    fn dolby_vision_summary(
        profile: u8,
        max_content_light_level_nits: Option<f32>,
        max_frame_average_light_level_nits: Option<f32>,
    ) -> postkit::dolby_vision::DolbyVisionSummary {
        postkit::dolby_vision::DolbyVisionSummary {
            profile,
            frames: 48,
            shots: 2,
            max_content_light_level_nits,
            max_frame_average_light_level_nits,
            peak_luminance_nits: 1000.0,
            mastering_display_max_nits: Some(1000.0),
            mastering_display_min_nits: Some(0.005),
        }
    }

    #[test]
    fn a_dolby_vision_rpu_fills_the_light_levels_the_flags_left_out() {
        let hdr = HdrWcg::from_flags("pq-bt2020", None).unwrap();
        let filled = light_levels_from_summary(
            &dolby_vision_summary(8, Some(993.0), Some(362.4)),
            hdr.clone(),
        )
        .unwrap();
        assert_eq!(filled.max_cll, Some(993));
        assert_eq!(filled.max_fall, Some(362));

        // an RPU with no level 6 block leaves both absent
        let bare =
            light_levels_from_summary(&dolby_vision_summary(8, None, None), hdr.clone()).unwrap();
        assert_eq!(bare.max_cll, None);
        assert_eq!(bare.max_fall, None);
    }

    #[test]
    fn the_flags_win_over_the_rpu() {
        let flagged = HdrWcg::from_flags("pq-bt2020", None)
            .unwrap()
            .with_content_light_levels(Some(500), None)
            .unwrap();
        let kept =
            light_levels_from_summary(&dolby_vision_summary(8, Some(993.0), Some(362.0)), flagged)
                .unwrap();
        assert_eq!(kept.max_cll, Some(500));
        assert_eq!(kept.max_fall, None);
    }

    #[test]
    fn dolby_vision_profile_5_is_refused_by_name() {
        let hdr = HdrWcg::from_flags("pq-bt2020", None).unwrap();
        let error = light_levels_from_summary(&dolby_vision_summary(5, Some(993.0), None), hdr)
            .unwrap_err();
        assert!(error.contains("profile 5"), "{error}");
        assert!(error.contains("profile 8.1"), "{error}");
    }

    #[test]
    fn a_transfer_the_preset_contradicts_is_refused_naming_both() {
        let hlg = HdrWcg::from_flags("hlg-bt2020", None).unwrap();
        let error = check_source_matches_preset(SourceHdr::Pq, Some(&hlg)).unwrap_err();
        assert!(error.contains("smpte2084"), "{error}");
        assert!(error.contains("hlg-bt2020"), "{error}");
        assert!(error.contains("pq-bt2020 or pq-p3d65"), "{error}");

        for preset in ["pq-bt2020", "pq-p3d65"] {
            let pq = HdrWcg::from_flags(preset, None).unwrap();
            let error = check_source_matches_preset(SourceHdr::Hlg, Some(&pq)).unwrap_err();
            assert!(error.contains("arib-std-b67"), "{error}");
            assert!(error.contains(preset), "{error}");
            assert!(error.contains("hlg-bt2020"), "{error}");
        }
    }

    // dolby vision 8.1 is pq, so an hlg preset over it is the same mismatch
    #[test]
    fn dolby_vision_agrees_with_a_pq_preset_only() {
        let pq = HdrWcg::from_flags("pq-bt2020", None).unwrap();
        assert_eq!(
            check_source_matches_preset(SourceHdr::DolbyVision, Some(&pq)),
            Ok(())
        );
        let hlg = HdrWcg::from_flags("hlg-bt2020", None).unwrap();
        let error = check_source_matches_preset(SourceHdr::DolbyVision, Some(&hlg)).unwrap_err();
        assert!(error.contains("Dolby Vision"), "{error}");
    }
}
