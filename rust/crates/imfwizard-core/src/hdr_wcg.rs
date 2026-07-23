//! HDR/WCG picture metadata (SMPTE ST 2067-21) for the IMF picture essence.
//!
//! A `--hdr` preset selects the transfer characteristic and colour primaries ULs;
//! an optional mastering-display block (ST 2086) adds display primaries, white
//! point and luminance. The same values drive both the MXF RGBA descriptor
//! (via asdcplib `open_write_hdr`) and the CPL EssenceDescriptor, so the CPL only
//! claims what the essence carries.
//!
//! MaxCLL/MaxFALL are not written: the vendored asdcplib has no descriptor
//! property for them.

use serde::{Deserialize, Serialize};

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

/// HDR/WCG metadata for one picture: transfer + colour primaries ULs plus an
/// optional mastering display.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HdrWcg {
    pub transfer: [u8; 16],
    pub color_primaries: [u8; 16],
    pub mastering: Option<MasteringDisplay>,
}

impl HdrWcg {
    /// Build from the `--hdr <preset>` selector and optional `--mastering-display`
    /// (x265 master-display string). Unknown presets and malformed strings error.
    pub fn from_flags(preset: &str, mastering: Option<&str>) -> Result<Self, String> {
        use asdcplib::jp2k::{
            COLOR_PRIMARIES_BT2020, COLOR_PRIMARIES_P3D65, TRANSFER_CHARACTERISTIC_ST2084,
        };
        let (transfer, color_primaries) = match preset.to_lowercase().as_str() {
            "pq-bt2020" => (TRANSFER_CHARACTERISTIC_ST2084, COLOR_PRIMARIES_BT2020),
            "pq-p3d65" => (TRANSFER_CHARACTERISTIC_ST2084, COLOR_PRIMARIES_P3D65),
            other => {
                return Err(format!(
                    "unknown HDR preset '{other}' (expected pq-bt2020 or pq-p3d65)"
                ));
            }
        };
        let mastering = mastering.map(parse_mastering_display).transpose()?;
        Ok(Self {
            transfer,
            color_primaries,
            mastering,
        })
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

    /// ST 2067-21 RGBADescriptor body for the CPL EssenceDescriptorList, matching
    /// the shape asdcplib writes into the MXF. Six-space base indent, already
    /// namespaced, ready to drop inside `<EssenceDescriptor>`.
    pub fn cpl_descriptor_body(&self) -> String {
        let mut b = String::new();
        b.push_str(concat!(
            "      <r0:RGBADescriptor ",
            "xmlns:r0=\"http://www.smpte-ra.org/reg/395/2014/13/1/aaf\" ",
            "xmlns:r1=\"http://www.smpte-ra.org/reg/335/2012\" ",
            "xmlns:r2=\"http://www.smpte-ra.org/reg/2003/2012\">\n"
        ));
        b.push_str(&format!(
            "        <r1:TransferCharacteristic>{}</r1:TransferCharacteristic>\n",
            ul_to_urn(&self.transfer)
        ));
        b.push_str(&format!(
            "        <r1:ColorPrimaries>{}</r1:ColorPrimaries>\n",
            ul_to_urn(&self.color_primaries)
        ));
        if let Some(md) = &self.mastering {
            let [wx, wy] = md.white_point;
            b.push_str(&format!(
                "        <r1:MasteringDisplayWhitePointChromaticity><r2:X>{wx}</r2:X><r2:Y>{wy}</r2:Y></r1:MasteringDisplayWhitePointChromaticity>\n"
            ));
            b.push_str("        <r1:MasteringDisplayPrimaries>\n");
            for [x, y] in md.primaries {
                b.push_str(&format!(
                    "          <r2:ColorPrimary><r2:X>{x}</r2:X><r2:Y>{y}</r2:Y></r2:ColorPrimary>\n"
                ));
            }
            b.push_str("        </r1:MasteringDisplayPrimaries>\n");
            b.push_str(&format!(
                "        <r1:MasteringDisplayMaximumLuminance>{}</r1:MasteringDisplayMaximumLuminance>\n",
                md.max_luminance
            ));
            b.push_str(&format!(
                "        <r1:MasteringDisplayMinimumLuminance>{}</r1:MasteringDisplayMinimumLuminance>\n",
                md.min_luminance
            ));
        }
        b.push_str("      </r0:RGBADescriptor>");
        b
    }
}

/// Format a 16-byte SMPTE UL as its `urn:smpte:ul:` form (four dot-separated
/// groups of 4 bytes), matching Photon's EssenceDescriptor serialisation.
fn ul_to_urn(ul: &[u8; 16]) -> String {
    let g = |o: usize| {
        format!(
            "{:02x}{:02x}{:02x}{:02x}",
            ul[o],
            ul[o + 1],
            ul[o + 2],
            ul[o + 3]
        )
    };
    format!("urn:smpte:ul:{}.{}.{}.{}", g(0), g(4), g(8), g(12))
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
    fn unknown_preset_errors() {
        assert!(HdrWcg::from_flags("hlg-bt2020", None).is_err());
    }

    #[test]
    fn ul_urn_format_matches_photon() {
        // ST 2084 transfer UL as Photon serialises it
        assert_eq!(
            ul_to_urn(&asdcplib::jp2k::TRANSFER_CHARACTERISTIC_ST2084),
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

    #[test]
    fn cpl_body_has_uls_and_mastering() {
        let h = HdrWcg::from_flags(
            "pq-p3d65",
            Some("R(34000,16000)G(13250,34500)B(7500,3000)WP(15635,16450)L(40000000,50)"),
        )
        .unwrap();
        let body = h.cpl_descriptor_body();
        assert!(body.contains("<r0:RGBADescriptor"));
        assert!(body.contains(
            "<r1:TransferCharacteristic>urn:smpte:ul:060e2b34.0401010d.04010101.010a0000"
        ));
        assert!(body.contains("<r1:MasteringDisplayMaximumLuminance>40000000<"));
        assert!(body.contains("<r2:ColorPrimary><r2:X>34000</r2:X><r2:Y>16000</r2:Y>"));
    }
}
