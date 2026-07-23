use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Accessibility role of an audio track, carried in the CPL as an MCA essence
/// descriptor (ST 2067-2/-3). None is normal main audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioRole {
    /// AD: visually impaired narration (MCA chVIN)
    AudioDescription,
    /// HI: hearing impaired mix (MCA chHI)
    HearingImpaired,
}

impl AudioRole {
    /// Parse a CLI role selector; None for an unknown value.
    pub fn from_flag(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "ad" | "vi" => Some(Self::AudioDescription),
            "hi" => Some(Self::HearingImpaired),
            _ => None,
        }
    }
}

/// One audio track and its RFC 5646 language tag (e.g. "de-DE").
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AudioTrack {
    pub path: PathBuf,
    pub language: Option<String>,
    /// Accessibility role; None is normal main audio.
    pub role: Option<AudioRole>,
}

/// One composition in an IMP. Each becomes a separate CPL sharing one PKL/ASSETMAP.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Composition {
    pub title: String,
    pub content_kind: String,
    /// J2K codestream directory
    pub j2k_dir: Option<PathBuf>,
    /// WAV audio tracks
    pub audio_files: Vec<AudioTrack>,
    /// Timed text files
    pub timed_text_files: Vec<PathBuf>,
    /// HDR/WCG picture metadata (ST 2067-21); None is SDR. Written into the
    /// picture MXF and mirrored in the CPL EssenceDescriptor.
    pub hdr: Option<crate::hdr_wcg::HdrWcg>,
}

/// IMP creation options.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImpOptions {
    pub output_dir: PathBuf,
    /// One or more compositions, each written as its own CPL.
    pub compositions: Vec<Composition>,
    /// Frame rate
    pub fps_num: u32,
    pub fps_den: u32,
    /// Edit rate
    pub edit_rate: String,
    /// Duration in frames
    pub duration: u64,
}

/// A CPL written into the IMP, referenced by the shared PKL and ASSETMAP.
#[derive(Debug, Clone)]
pub struct CplEntry {
    pub uuid: String,
    pub path: PathBuf,
}

/// IMP creation result.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImpResult {
    pub success: bool,
    pub error: String,
    pub output_dir: PathBuf,
    pub cpl_paths: Vec<PathBuf>,
    pub pkl_path: PathBuf,
    pub assetmap_path: PathBuf,
    pub track_files: Vec<crate::MxfTrackFile>,
}

/// Validate an RFC 5646 language tag well enough to reject obvious garbage.
/// Not a full BCP 47 parser: checks subtags are alphanumeric, hyphen-separated,
/// with a 2-8 letter primary subtag.
pub fn validate_language(tag: &str) -> Result<(), String> {
    let subtags: Vec<&str> = tag.split('-').collect();
    let bad = tag.is_empty()
        || tag.starts_with('-')
        || tag.ends_with('-')
        || subtags
            .iter()
            .any(|s| s.is_empty() || !s.chars().all(|c| c.is_ascii_alphanumeric()));
    let primary_ok = subtags
        .first()
        .map(|s| (2..=8).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphabetic()))
        .unwrap_or(false);
    if bad || !primary_ok {
        return Err(format!("invalid RFC 5646 language tag: {tag}"));
    }
    Ok(())
}

fn wrap_one(
    opts: &ImpOptions,
    output_dir: &std::path::Path,
    prefix: &str,
    input: &std::path::Path,
    essence: crate::EssenceType,
    hdr: Option<asdcplib::jp2k::HdrMetadata>,
) -> Result<crate::MxfTrackFile, String> {
    let uuid = uuid::Uuid::new_v4().to_string();
    let mxf_path = output_dir.join(format!("{prefix}_{uuid}.mxf"));
    let wrap_opts = crate::mxf_wrap::MxfWrapOptions {
        input_dir: input.to_path_buf(),
        output_file: mxf_path,
        essence_type: essence,
        edit_rate_num: opts.fps_num,
        edit_rate_den: opts.fps_den,
        duration: opts.duration,
        hdr,
    };
    let r = crate::mxf_wrap::wrap_mxf(&wrap_opts);
    if !r.success {
        return Err(r.error);
    }
    Ok(r.track_file)
}

/// Create an IMP (Interoperable Master Package).
pub fn create_imp(opts: &ImpOptions) -> ImpResult {
    if opts.compositions.is_empty() {
        return ImpResult {
            error: "A J2K input directory is required".into(),
            ..Default::default()
        };
    }

    // validate language tags up front so we fail before wrapping anything
    for comp in &opts.compositions {
        for a in &comp.audio_files {
            if let Some(lang) = &a.language
                && let Err(e) = validate_language(lang)
            {
                return ImpResult {
                    error: e,
                    ..Default::default()
                };
            }
        }
    }

    for comp in &opts.compositions {
        let Some(j2k_dir) = comp.j2k_dir.as_ref() else {
            return ImpResult {
                error: "A J2K input directory is required".into(),
                ..Default::default()
            };
        };
        if !j2k_dir.is_dir() {
            return ImpResult {
                error: format!("J2K input directory not found: {}", j2k_dir.display()),
                ..Default::default()
            };
        }
    }

    if let Err(e) = std::fs::create_dir_all(&opts.output_dir) {
        return ImpResult {
            error: format!("Failed to create output directory: {e}"),
            ..Default::default()
        };
    }

    let mut all_tracks: Vec<crate::MxfTrackFile> = Vec::new();
    let mut cpls: Vec<CplEntry> = Vec::new();

    for comp in &opts.compositions {
        let mut comp_tracks: Vec<crate::MxfTrackFile> = Vec::new();

        // picture (required, validated above)
        let j2k_dir = comp.j2k_dir.as_ref().expect("checked above");
        match wrap_one(
            opts,
            &opts.output_dir,
            "VIDEO",
            j2k_dir,
            crate::EssenceType::J2k,
            comp.hdr.as_ref().map(|h| h.to_asdcp()),
        ) {
            Ok(tf) => comp_tracks.push(tf),
            Err(e) => {
                return ImpResult {
                    error: format!("MXF wrapping failed: {e}"),
                    ..Default::default()
                };
            }
        }

        for a in &comp.audio_files {
            if !a.path.exists() {
                continue;
            }
            match wrap_one(
                opts,
                &opts.output_dir,
                "AUDIO",
                &a.path,
                crate::EssenceType::Wav,
                None,
            ) {
                Ok(tf) => comp_tracks.push(tf),
                Err(e) => {
                    return ImpResult {
                        error: format!("Audio wrap failed: {e}"),
                        ..Default::default()
                    };
                }
            }
        }

        for tt in &comp.timed_text_files {
            if !tt.exists() {
                continue;
            }
            match wrap_one(
                opts,
                &opts.output_dir,
                "SUBTITLE",
                tt,
                crate::EssenceType::TimedText,
                None,
            ) {
                Ok(tf) => comp_tracks.push(tf),
                Err(e) => {
                    return ImpResult {
                        error: format!("Subtitle wrap failed: {e}"),
                        ..Default::default()
                    };
                }
            }
        }

        let cpl_uuid = uuid::Uuid::new_v4().to_string();
        let cpl_path = opts.output_dir.join(format!("CPL_{cpl_uuid}.xml"));
        if let Err(e) = crate::cpl::write_cpl(&cpl_path, &cpl_uuid, opts, comp, &comp_tracks) {
            return ImpResult {
                error: format!("Failed to write CPL: {e}"),
                ..Default::default()
            };
        }
        cpls.push(CplEntry {
            uuid: cpl_uuid,
            path: cpl_path,
        });
        all_tracks.extend(comp_tracks);
    }

    // one PKL and one ASSETMAP over every CPL and track file
    let pkl_uuid = uuid::Uuid::new_v4().to_string();
    let pkl_path = opts.output_dir.join(format!("PKL_{pkl_uuid}.xml"));
    if let Err(e) = crate::pkl::write_pkl(&pkl_path, &pkl_uuid, &cpls, &all_tracks) {
        return ImpResult {
            error: format!("Failed to write PKL: {e}"),
            ..Default::default()
        };
    }

    let am_path = opts.output_dir.join("ASSETMAP.xml");
    if let Err(e) = crate::assetmap::write_assetmap(&am_path, &pkl_uuid, &cpls, &all_tracks) {
        return ImpResult {
            error: format!("Failed to write ASSETMAP: {e}"),
            ..Default::default()
        };
    }

    ImpResult {
        success: true,
        error: String::new(),
        output_dir: opts.output_dir.clone(),
        cpl_paths: cpls.into_iter().map(|c| c.path).collect(),
        pkl_path,
        assetmap_path: am_path,
        track_files: all_tracks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_imp_requires_picture_input() {
        let dir = tempfile::tempdir().unwrap();
        let opts = ImpOptions {
            output_dir: dir.path().to_path_buf(),
            compositions: vec![Composition {
                title: "Test Feature".into(),
                content_kind: "feature".into(),
                ..Default::default()
            }],
            fps_num: 24,
            fps_den: 1,
            ..Default::default()
        };
        let result = create_imp(&opts);
        assert!(!result.success);
        assert!(result.error.contains("J2K input directory is required"));
    }

    #[test]
    fn test_create_imp_xml_escape() {
        let dir = tempfile::tempdir().unwrap();
        let cpl_path = dir.path().join("CPL_test.xml");
        let opts = ImpOptions {
            output_dir: dir.path().to_path_buf(),
            fps_num: 25,
            fps_den: 1,
            ..Default::default()
        };
        let comp = Composition {
            title: "Test & <Special> \"Film\"".into(),
            ..Default::default()
        };
        crate::cpl::write_cpl(&cpl_path, "test", &opts, &comp, &[]).unwrap();
        let cpl_xml = std::fs::read_to_string(cpl_path).unwrap();
        assert!(cpl_xml.contains("Test &amp; &lt;Special&gt; &quot;Film&quot;"));
    }

    /// Minimal JPEG 2000 codestream the AS-02 writer accepts: SOC, a 2048x1080
    /// 3-component SIZ, then SOD/EOC.
    fn synthetic_j2k() -> Vec<u8> {
        let mut d = vec![0xFF, 0x4F]; // SOC
        d.extend_from_slice(&[0xFF, 0x51]); // SIZ
        let mut siz = Vec::new();
        siz.extend_from_slice(&3u16.to_be_bytes()); // Rsiz
        siz.extend_from_slice(&2048u32.to_be_bytes()); // Xsiz
        siz.extend_from_slice(&1080u32.to_be_bytes()); // Ysiz
        siz.extend_from_slice(&0u32.to_be_bytes()); // XOsiz
        siz.extend_from_slice(&0u32.to_be_bytes()); // YOsiz
        siz.extend_from_slice(&2048u32.to_be_bytes()); // XTsiz
        siz.extend_from_slice(&1080u32.to_be_bytes()); // YTsiz
        siz.extend_from_slice(&0u32.to_be_bytes()); // XTOsiz
        siz.extend_from_slice(&0u32.to_be_bytes()); // YTOsiz
        siz.extend_from_slice(&3u16.to_be_bytes()); // Csiz
        for _ in 0..3 {
            siz.push(11); // 12-bit unsigned
            siz.push(1);
            siz.push(1);
        }
        let lsiz = (siz.len() + 2) as u16;
        d.extend_from_slice(&lsiz.to_be_bytes());
        d.extend_from_slice(&siz);
        d.extend_from_slice(&[0xFF, 0x93]); // SOD
        d.extend_from_slice(&[0u8; 64]);
        d.extend_from_slice(&[0xFF, 0xD9]); // EOC
        d
    }

    /// A --hdr create writes the transfer/colour ULs and ST 2086 mastering values
    /// into the picture MXF, and they round-trip out via `hdr_metadata()`.
    #[test]
    fn hdr_create_roundtrips_through_the_picture_mxf() {
        let dir = tempfile::tempdir().unwrap();
        let (result, _out) = build_hdr_imp(dir.path());
        assert!(result.success, "create failed: {}", result.error);

        let video = result
            .track_files
            .iter()
            .find(|t| {
                t.path
                    .file_name()
                    .and_then(|f| f.to_str())
                    .is_some_and(|n| n.starts_with("VIDEO_"))
            })
            .expect("a VIDEO track file");

        let mut reader = asdcplib::as02::jp2k::MxfReader::new();
        reader
            .open_read(&video.path.to_string_lossy())
            .expect("open picture MXF");
        let md = reader.hdr_metadata().expect("read hdr metadata");
        assert_eq!(
            md.transfer_characteristic,
            Some(asdcplib::jp2k::TRANSFER_CHARACTERISTIC_ST2084)
        );
        assert_eq!(
            md.color_primaries,
            Some(asdcplib::jp2k::COLOR_PRIMARIES_BT2020)
        );
        assert_eq!(
            md.mastering_display_primaries,
            Some([[34000, 16000], [13250, 34500], [7500, 3000]])
        );
        assert_eq!(md.mastering_display_white_point, Some([15635, 16450]));
        assert_eq!(md.mastering_display_max_luminance, Some(40000000));
        assert_eq!(md.mastering_display_min_luminance, Some(50));

        // the CPL must carry a matching RGBA descriptor linked by SourceEncoding
        let cpl = std::fs::read_to_string(&result.cpl_paths[0]).unwrap();
        assert!(cpl.contains("<r0:RGBADescriptor"));
        assert!(cpl.contains("<SourceEncoding>"));
        assert!(cpl.contains("<r1:MasteringDisplayMaximumLuminance>40000000<"));
    }

    /// Build a one-frame HDR IMP into `out` and return the result.
    fn build_hdr_imp(dir: &std::path::Path) -> (ImpResult, std::path::PathBuf) {
        let j2k_dir = dir.join("j2k");
        std::fs::create_dir_all(&j2k_dir).unwrap();
        std::fs::write(j2k_dir.join("0001.j2c"), synthetic_j2k()).unwrap();
        let out = dir.join("imp");
        let hdr = crate::hdr_wcg::HdrWcg::from_flags(
            "pq-bt2020",
            Some("R(34000,16000)G(13250,34500)B(7500,3000)WP(15635,16450)L(40000000,50)"),
        )
        .unwrap();
        let opts = ImpOptions {
            output_dir: out.clone(),
            compositions: vec![Composition {
                title: "HDR".into(),
                content_kind: "feature".into(),
                j2k_dir: Some(j2k_dir),
                hdr: Some(hdr),
                ..Default::default()
            }],
            fps_num: 24,
            fps_den: 1,
            ..Default::default()
        };
        (create_imp(&opts), out)
    }

    /// Netflix Photon must analyse the HDR IMP without a launch failure. Gated on
    /// PHOTON_JAR (+ java); skips when absent. Synthetic essence is not fully
    /// conformant, so this checks the package parses, not that it is error-free.
    #[test]
    fn hdr_imp_analyzes_under_photon() {
        if std::env::var("PHOTON_JAR").is_err() {
            eprintln!("skipping: set PHOTON_JAR to run the Photon check");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let (result, out) = build_hdr_imp(dir.path());
        assert!(result.success, "create failed: {}", result.error);
        match crate::photon::run_photon(&out, None) {
            Ok(_) => {}
            Err(e) => panic!("Photon failed to analyse the HDR IMP: {e}"),
        }
    }

    #[test]
    fn test_validate_language() {
        assert!(validate_language("de-DE").is_ok());
        assert!(validate_language("en").is_ok());
        assert!(validate_language("zh-Hans-CN").is_ok());
        assert!(validate_language("").is_err());
        assert!(validate_language("de_DE").is_err());
        assert!(validate_language("-de").is_err());
        assert!(validate_language("123").is_err());
    }
}
