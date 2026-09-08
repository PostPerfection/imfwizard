//! HDR/WCG picture metadata (SMPTE ST 2067-21) for the IMF picture essence.
//!
//! A `--hdr` preset selects the transfer characteristic and colour primaries ULs;
//! an optional mastering-display block (ST 2086) adds display primaries, white
//! point and luminance. The same values drive both the MXF RGBA descriptor
//! (via asdcplib `open_write_hdr`) and the CPL EssenceDescriptor, so the CPL only
//! claims what the essence carries.
//!
//! MaxCLL/MaxFALL are not essence-descriptor metadata: ST 2067-21 defines them as
//! CPL ExtensionProperties, so they ride on `HdrWcg` but only reach the CPL.

use serde::{Deserialize, Serialize};

// TransferCharacteristic_HLG_OETF, the COLOR.8 transfer ST 2067-21 added in 2020. asdcplib has no constant for it.
pub const TRANSFER_CHARACTERISTIC_HLG: [u8; 16] = [
    0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01, 0x0d, 0x04, 0x01, 0x01, 0x01, 0x01, 0x0b, 0x00, 0x00,
];

// every `--hdr` preset, one row each: the flag value, its transfer UL and its colour primaries UL
const PRESETS: [(&str, [u8; 16], [u8; 16]); 3] = [
    (
        "pq-bt2020",
        asdcplib::jp2k::TRANSFER_CHARACTERISTIC_ST2084,
        asdcplib::jp2k::COLOR_PRIMARIES_BT2020,
    ),
    (
        "pq-p3d65",
        asdcplib::jp2k::TRANSFER_CHARACTERISTIC_ST2084,
        asdcplib::jp2k::COLOR_PRIMARIES_P3D65,
    ),
    (
        "hlg-bt2020",
        TRANSFER_CHARACTERISTIC_HLG,
        asdcplib::jp2k::COLOR_PRIMARIES_BT2020,
    ),
];

pub fn preset_list() -> String {
    let names: Vec<&str> = PRESETS.iter().map(|(name, _, _)| *name).collect();
    names.join(", ")
}

pub fn presets_with_transfer(transfer: &[u8; 16]) -> String {
    let names: Vec<&str> = PRESETS
        .iter()
        .filter(|(_, preset_transfer, _)| preset_transfer == transfer)
        .map(|(name, _, _)| *name)
        .collect();
    names.join(" or ")
}

/// ST 2086 mastering display, raw units. Primaries are in R,G,B order; x/y are
/// 0.00002 increments (u16), luminance 0.0001 cd/m^2 increments (u32). These
/// match asdcplib's `HdrMetadata` and the x265 master-display string.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MasteringDisplay {
    pub primaries: [[u16; 2]; 3],
    pub white_point: [u16; 2],
    pub max_luminance: u32,
    pub min_luminance: u32,
}

/// HDR/WCG metadata for one picture: transfer + colour primaries ULs, an optional
/// mastering display, and the optional content light levels.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HdrWcg {
    pub transfer: [u8; 16],
    pub color_primaries: [u8; 16],
    pub mastering: Option<MasteringDisplay>,
    /// Maximum content light level, cd/m^2. Goes to the CPL ExtensionProperties,
    /// never to the essence descriptor.
    pub max_cll: Option<u16>,
    /// Maximum frame-average light level, cd/m^2. Same placement as `max_cll`.
    pub max_fall: Option<u16>,
}

impl HdrWcg {
    /// Build from the `--hdr <preset>` selector and optional `--mastering-display`
    /// (x265 master-display string). Unknown presets and malformed strings error.
    pub fn from_flags(preset: &str, mastering: Option<&str>) -> Result<Self, String> {
        let lowercased = preset.to_lowercase();
        let Some(&(_, transfer, color_primaries)) =
            PRESETS.iter().find(|(name, _, _)| *name == lowercased)
        else {
            return Err(format!(
                "unknown HDR preset '{preset}' (expected {})",
                preset_list()
            ));
        };
        let mastering = mastering.map(parse_mastering_display).transpose()?;
        Ok(Self {
            transfer,
            color_primaries,
            mastering,
            ..Default::default()
        })
    }

    /// Attach the `--max-cll` / `--max-fall` content light levels. Separate from
    /// `from_flags` because they need no parsing and never touch the essence.
    pub fn with_content_light_levels(
        mut self,
        max_cll: Option<u16>,
        max_fall: Option<u16>,
    ) -> Result<Self, String> {
        let given: Vec<&str> = [("--max-cll", max_cll), ("--max-fall", max_fall)]
            .iter()
            .filter(|(_, nits)| nits.is_some())
            .map(|(flag, _)| *flag)
            .collect();
        if self.is_hlg() && !given.is_empty() {
            return Err(format!(
                "{} only applies to a PQ preset: ST 2067-21 clause 7.5 defines MaxCLL and \
                 MaxFALL for COLOR.6 and COLOR.7, and {} is COLOR.8",
                given.join(" and "),
                self.preset_name()
            ));
        }
        self.max_cll = max_cll;
        self.max_fall = max_fall;
        Ok(self)
    }

    pub fn is_hlg(&self) -> bool {
        self.transfer == TRANSFER_CHARACTERISTIC_HLG
    }

    pub fn preset_name(&self) -> &'static str {
        PRESETS
            .iter()
            .find(|(_, transfer, primaries)| {
                *transfer == self.transfer && *primaries == self.color_primaries
            })
            .map(|(name, _, _)| *name)
            .unwrap_or("the HDR preset")
    }

    /// The asdcplib descriptor written into the picture MXF.
    pub fn to_asdcp(&self) -> asdcplib::jp2k::HdrMetadata {
        let mut m = asdcplib::jp2k::HdrMetadata {
            transfer_characteristic: Some(self.transfer),
            color_primaries: Some(self.color_primaries),
            ..Default::default()
        };
        if let Some(md) = &self.mastering {
            m.mastering_display_primaries = Some(md.primaries);
            m.mastering_display_white_point = Some(md.white_point);
            m.mastering_display_max_luminance = Some(md.max_luminance);
            m.mastering_display_min_luminance = Some(md.min_luminance);
        }
        m
    }
}

/// Parse an x265 master-display string, e.g.
/// `G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(40000000,50)`.
/// Component order in the string is free; every part is required. Primaries and
/// white point are 0.00002 units, luminance 0.0001 cd/m^2 units (raw ST 2086).
pub fn parse_mastering_display(s: &str) -> Result<MasteringDisplay, String> {
    let mut r = None;
    let mut g = None;
    let mut bl = None;
    let mut wp = None;
    let mut l = None;

    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // skip separators between tokens
        if !bytes[i].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let tag_start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        let tag = &s[tag_start..i];
        if i >= bytes.len() || bytes[i] != b'(' {
            return Err(format!(
                "expected '(' after '{tag}' in master-display string"
            ));
        }
        let open = i + 1;
        let close = s[open..]
            .find(')')
            .map(|p| open + p)
            .ok_or_else(|| format!("unterminated '(' after '{tag}'"))?;
        let inner = &s[open..close];
        let (a, bv) = parse_pair(inner, tag)?;
        match tag {
            "R" => r = Some([a as u16, bv as u16]),
            "G" => g = Some([a as u16, bv as u16]),
            "B" => bl = Some([a as u16, bv as u16]),
            "WP" => wp = Some([a as u16, bv as u16]),
            "L" => l = Some((a, bv)),
            other => return Err(format!("unknown master-display component '{other}'")),
        }
        i = close + 1;
    }

    let missing = |name: &str| format!("master-display string is missing {name}");
    let r = r.ok_or_else(|| missing("R primary"))?;
    let g = g.ok_or_else(|| missing("G primary"))?;
    let bl = bl.ok_or_else(|| missing("B primary"))?;
    let wp = wp.ok_or_else(|| missing("WP white point"))?;
    let (max, min) = l.ok_or_else(|| missing("L luminance"))?;
    Ok(MasteringDisplay {
        primaries: [r, g, bl],
        white_point: wp,
        max_luminance: max,
        min_luminance: min,
    })
}

/// Parse `"a,b"` into two u32 values.
fn parse_pair(inner: &str, tag: &str) -> Result<(u32, u32), String> {
    let (a, b) = inner
        .split_once(',')
        .ok_or_else(|| format!("expected 'x,y' in '{tag}(...)'"))?;
    let a = a
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("bad number '{a}' in '{tag}(...)'"))?;
    let b = b
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("bad number '{b}' in '{tag}(...)'"))?;
    Ok((a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_pq_bt2020_sets_st2084_and_bt2020() {
        let h = HdrWcg::from_flags("pq-bt2020", None).unwrap();
        assert_eq!(h.transfer, asdcplib::jp2k::TRANSFER_CHARACTERISTIC_ST2084);
        assert_eq!(h.color_primaries, asdcplib::jp2k::COLOR_PRIMARIES_BT2020);
        assert!(h.mastering.is_none());
    }

    #[test]
    fn preset_pq_p3d65_sets_p3d65_primaries() {
        let h = HdrWcg::from_flags("pq-p3d65", None).unwrap();
        assert_eq!(h.color_primaries, asdcplib::jp2k::COLOR_PRIMARIES_P3D65);
    }

    #[test]
    fn preset_hlg_bt2020_sets_the_hlg_transfer_and_bt2020() {
        let h = HdrWcg::from_flags("hlg-bt2020", None).unwrap();
        assert_eq!(h.transfer, TRANSFER_CHARACTERISTIC_HLG);
        assert_eq!(h.color_primaries, asdcplib::jp2k::COLOR_PRIMARIES_BT2020);
        assert!(h.is_hlg());
        assert_eq!(h.preset_name(), "hlg-bt2020");
    }

    // the HLG UL Photon's Colorimetry reads as the COLOR.8 transfer
    #[test]
    fn hlg_transfer_urn_matches_photon() {
        assert_eq!(
            postkit::regxml::urn_ul(&TRANSFER_CHARACTERISTIC_HLG),
            "urn:smpte:ul:060e2b34.0401010d.04010101.010b0000"
        );
    }

    #[test]
    fn content_light_levels_are_refused_on_hlg() {
        let hlg = HdrWcg::from_flags("hlg-bt2020", None).unwrap();
        let error = hlg
            .clone()
            .with_content_light_levels(Some(993), None)
            .unwrap_err();
        assert!(error.contains("--max-cll"), "{error}");
        assert!(!error.contains("--max-fall"), "{error}");
        assert!(error.contains("COLOR.8"), "{error}");

        let error = hlg
            .clone()
            .with_content_light_levels(Some(993), Some(362))
            .unwrap_err();
        assert!(error.contains("--max-cll and --max-fall"), "{error}");

        // an HLG preset on its own carries neither, so it passes
        assert!(hlg.with_content_light_levels(None, None).is_ok());
    }

    #[test]
    fn unknown_preset_names_every_preset() {
        let error = HdrWcg::from_flags("hlg-p3d65", None).unwrap_err();
        assert!(error.contains("'hlg-p3d65'"), "{error}");
        assert!(error.contains("pq-bt2020, pq-p3d65, hlg-bt2020"), "{error}");
    }

    #[test]
    fn ul_urn_format_matches_photon() {
        // ST 2084 transfer UL as Photon serialises it
        assert_eq!(
            postkit::regxml::urn_ul(&asdcplib::jp2k::TRANSFER_CHARACTERISTIC_ST2084),
            "urn:smpte:ul:060e2b34.0401010d.04010101.010a0000"
        );
    }

    #[test]
    fn master_display_parses_regardless_of_order() {
        let md = parse_mastering_display(
            "G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(40000000,50)",
        )
        .unwrap();
        // stored R,G,B
        assert_eq!(md.primaries[0], [34000, 16000]);
        assert_eq!(md.primaries[1], [13250, 34500]);
        assert_eq!(md.primaries[2], [7500, 3000]);
        assert_eq!(md.white_point, [15635, 16450]);
        assert_eq!(md.max_luminance, 40000000);
        assert_eq!(md.min_luminance, 50);
    }

    #[test]
    fn master_display_rejects_missing_component() {
        // no L luminance
        let err =
            parse_mastering_display("R(34000,16000)G(13250,34500)B(7500,3000)WP(15635,16450)")
                .unwrap_err();
        assert!(err.contains("L luminance"), "got: {err}");
    }

    #[test]
    fn to_asdcp_carries_all_mastering_values() {
        let h = HdrWcg::from_flags(
            "pq-bt2020",
            Some("R(34000,16000)G(13250,34500)B(7500,3000)WP(15635,16450)L(40000000,50)"),
        )
        .unwrap();
        let a = h.to_asdcp();
        assert_eq!(a.transfer_characteristic.unwrap(), h.transfer);
        assert_eq!(a.mastering_display_primaries.unwrap()[0], [34000, 16000]);
        assert_eq!(a.mastering_display_max_luminance, Some(40000000));
    }

    /// Content light levels are ST 2067-21 CPL ExtensionProperties, so they must not
    /// leak into the MXF descriptor or the CPL EssenceDescriptor body.
    #[test]
    fn content_light_levels_stay_out_of_the_essence_descriptor() {
        let h = HdrWcg::from_flags("pq-bt2020", None)
            .unwrap()
            .with_content_light_levels(Some(993), Some(362))
            .unwrap();
        assert_eq!(h.max_cll, Some(993));
        assert_eq!(h.max_fall, Some(362));
        assert_eq!(
            h.to_asdcp(),
            HdrWcg::from_flags("pq-bt2020", None).unwrap().to_asdcp()
        );
    }
}
