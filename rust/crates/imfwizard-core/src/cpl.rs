use std::path::Path;

use postkit::packaging::{ImfCpl, ImfEssenceDescriptor, ImfResource, ImfTrackKind, escape_xml};

use crate::MxfTrackFile;
use crate::imp::{AudioRole, Composition, ImpOptions};

/// Write an IMF CPL (ST 2067-3) using the shared postkit writer.
///
/// Track files are classified by their filename prefix (VIDEO_/AUDIO_/SUBTITLE_),
/// the same convention the MXF wrapper writes. Audio languages from `comp` are
/// written into a composition-level LocaleList (ST 2067-3 Locale/LanguageList).
/// Audio tracks with an accessibility role (AD/HI) get an MCA EssenceDescriptor
/// linked to the resource via SourceEncoding (ST 2067-2/-3).
pub fn write_cpl(
    path: &Path,
    cpl_uuid: &str,
    opts: &ImpOptions,
    comp: &Composition,
    track_files: &[MxfTrackFile],
) -> std::io::Result<()> {
    let mut resources = Vec::new();
    let mut descriptors = Vec::new();
    // audio track files are wrapped in composition order, so the Nth AUDIO_ file
    // maps to the Nth audio track; used to attach accessibility MCA descriptors.
    let mut audio_idx = 0;
    for tf in track_files {
        let fname = tf.path.file_name().and_then(|f| f.to_str()).unwrap_or("");
        let kind = if fname.starts_with("VIDEO_") {
            ImfTrackKind::Image
        } else if fname.starts_with("AUDIO_") {
            ImfTrackKind::Audio
        } else if fname.starts_with("SUBTITLE_") {
            ImfTrackKind::Subtitle
        } else {
            continue;
        };
        let mut source_encoding = None;
        if kind == ImfTrackKind::Audio {
            if let Some(track) = comp.audio_files.get(audio_idx)
                && let Some(role) = track.role
            {
                let se = uuid::Uuid::new_v4().to_string();
                descriptors.push(ImfEssenceDescriptor {
                    id: se.clone(),
                    body: audio_descriptor_body(role, track.language.as_deref()),
                });
                source_encoding = Some(se);
            }
            audio_idx += 1;
        }
        resources.push(ImfResource {
            track_file_uuid: tf.uuid.clone(),
            duration: tf.duration,
            kind,
            source_encoding,
        });
    }

    // distinct audio languages, in first-seen order
    let mut langs: Vec<String> = Vec::new();
    for a in &comp.audio_files {
        if let Some(l) = &a.language
            && !langs.contains(l)
        {
            langs.push(l.clone());
        }
    }

    let cpl = ImfCpl {
        uuid: cpl_uuid.to_string(),
        title: comp.title.clone(),
        content_kind: comp.content_kind.clone(),
        issuer: "IMF Wizard".to_string(),
        creator: "IMF Wizard".to_string(),
        issue_date: crate::issue_date(),
        fps_num: opts.fps_num,
        fps_den: opts.fps_den,
        resources,
        languages: langs,
        essence_descriptors: descriptors,
    };

    std::fs::write(path, cpl.to_xml())
}

/// MCA essence descriptor body for an accessibility audio track, matching the
/// XSD-validated shape in postkit's packaging tests: a WAVEPCMDescriptor with a
/// SoundfieldGroup plus one AudioChannelLabel carrying the accessibility MCA
/// symbol (chVIN/chHI) and RFC 5646 spoken language. postkit carries it verbatim.
fn audio_descriptor_body(role: AudioRole, lang: Option<&str>) -> String {
    let (symbol, name) = match role {
        AudioRole::AudioDescription => ("chVIN", "Visually Impaired"),
        AudioRole::HearingImpaired => ("chHI", "Hearing Impaired"),
    };
    // optional spoken language, emitted at the given indent when set
    let spoken = |indent: &str| match lang {
        Some(l) => format!(
            "\n{indent}<r1:RFC5646SpokenLanguage>{}</r1:RFC5646SpokenLanguage>",
            escape_xml(l)
        ),
        None => String::new(),
    };
    format!(
        r#"      <r0:WAVEPCMDescriptor xmlns:r0="http://www.smpte-ra.org/reg/395/2014/13/1/aaf" xmlns:r1="http://www.smpte-ra.org/reg/335/2012">
        <r1:ChannelCount>1</r1:ChannelCount>
        <r1:SubDescriptors>
          <r0:SoundfieldGroupLabelSubDescriptor>
            <r1:MCATagSymbol>sg51</r1:MCATagSymbol>{sg_lang}
          </r0:SoundfieldGroupLabelSubDescriptor>
          <r0:AudioChannelLabelSubDescriptor>
            <r1:MCAChannelID>1</r1:MCAChannelID>
            <r1:MCATagSymbol>{symbol}</r1:MCATagSymbol>
            <r1:MCATagName>{name}</r1:MCATagName>{ch_lang}
          </r0:AudioChannelLabelSubDescriptor>
        </r1:SubDescriptors>
      </r0:WAVEPCMDescriptor>"#,
        sg_lang = spoken("            "),
        ch_lang = spoken("            "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imp::AudioTrack;

    #[test]
    fn write_cpl_identifies_app_2e() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CPL_test.xml");
        let opts = ImpOptions::default();
        let comp = Composition {
            title: "Test".into(),
            ..Default::default()
        };

        write_cpl(&path, "cpl", &opts, &comp, &[]).unwrap();
        let xml = std::fs::read_to_string(path).unwrap();
        assert!(xml.contains("<IssueDate>"));
        assert!(xml.contains("<cc:ApplicationIdentification>http://www.smpte-ra.org/schemas/2067-21/2016</cc:ApplicationIdentification>"));
    }

    #[test]
    fn write_cpl_writes_language_locale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CPL_test.xml");
        let opts = ImpOptions::default();
        let comp = Composition {
            title: "Test".into(),
            audio_files: vec![
                AudioTrack {
                    path: "de.wav".into(),
                    language: Some("de-DE".into()),
                    role: None,
                },
                AudioTrack {
                    path: "en.wav".into(),
                    language: Some("en-US".into()),
                    role: None,
                },
            ],
            ..Default::default()
        };

        write_cpl(&path, "cpl", &opts, &comp, &[]).unwrap();
        let xml = std::fs::read_to_string(path).unwrap();
        assert!(xml.contains("<LocaleList>"));
        assert!(xml.contains("<Language>de-DE</Language>"));
        assert!(xml.contains("<Language>en-US</Language>"));
        // LocaleList must precede ExtensionProperties per ST 2067-3 order
        assert!(xml.find("<LocaleList>").unwrap() < xml.find("<ExtensionProperties>").unwrap());
    }

    // the wrapped audio track file; its accessibility role is carried by the
    // composition's AudioTrack, matched by position, not by the file itself.
    fn accessibility_track() -> MxfTrackFile {
        MxfTrackFile {
            path: "AUDIO_ad.mxf".into(),
            uuid: "aaaaaaaa-1111-2222-3333-444444444444".into(),
            duration: 240,
            ..Default::default()
        }
    }

    #[test]
    fn accessibility_role_emits_mca_descriptor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CPL_ad.xml");
        let opts = ImpOptions {
            fps_num: 24,
            fps_den: 1,
            ..Default::default()
        };
        let comp = Composition {
            title: "AD".into(),
            audio_files: vec![AudioTrack {
                path: "ad.wav".into(),
                language: Some("en-US".into()),
                role: Some(AudioRole::AudioDescription),
            }],
            ..Default::default()
        };
        let tracks = [accessibility_track()];
        write_cpl(&path, "cpl", &opts, &comp, &tracks).unwrap();
        let xml = std::fs::read_to_string(path).unwrap();
        assert!(xml.contains("<EssenceDescriptorList>"));
        assert!(xml.contains("<r1:MCATagSymbol>chVIN</r1:MCATagSymbol>"));
        assert!(xml.contains("<r1:MCATagName>Visually Impaired</r1:MCATagName>"));
        assert!(xml.contains("<r1:RFC5646SpokenLanguage>en-US</r1:RFC5646SpokenLanguage>"));
        // the audio resource must link to the descriptor via SourceEncoding
        assert!(xml.contains("<SourceEncoding>"));
    }

    /// Locate the ST 2067-3 XSDs and validate `cpl_xml` with xmllint. Returns None
    /// when the gate is unmet (no IMFWIZARD_IMF_XSD_DIR, no xmllint); panics if the
    /// dir is set but the XSDs are missing (a misconfiguration).
    fn validate_st2067_3(cpl_xml: &str) -> Option<bool> {
        let xsd_dir = std::env::var("IMFWIZARD_IMF_XSD_DIR").ok()?;
        if std::process::Command::new("xmllint")
            .arg("--version")
            .output()
            .is_err()
        {
            return None;
        }
        fn walk(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
            for e in std::fs::read_dir(dir).ok()?.flatten() {
                let p = e.path();
                if p.is_dir() {
                    if let Some(f) = walk(&p, name) {
                        return Some(f);
                    }
                } else if p.file_name().and_then(|f| f.to_str()) == Some(name) {
                    return Some(p);
                }
            }
            None
        }
        let root = std::path::Path::new(&xsd_dir);
        let (Some(cpl_xsd), Some(dsig_xsd)) = (
            walk(root, "imf-cpl-20160411.xsd"),
            walk(root, "xmldsig-core-schema.xsd"),
        ) else {
            panic!(
                "could not locate imf-cpl-20160411.xsd and xmldsig-core-schema.xsd under {xsd_dir}"
            );
        };

        let dir = tempfile::tempdir().unwrap();
        let cpl_path = dir.path().join("CPL.xml");
        std::fs::write(&cpl_path, cpl_xml).unwrap();
        let driver = dir.path().join("driver.xsd");
        std::fs::write(
            &driver,
            format!(
                r#"<?xml version="1.0"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:import namespace="http://www.smpte-ra.org/schemas/2067-3/2016" schemaLocation="{cpl}"/>
  <xs:import namespace="http://www.w3.org/2000/09/xmldsig#" schemaLocation="{dsig}"/>
</xs:schema>"#,
                cpl = cpl_xsd.display(),
                dsig = dsig_xsd.display(),
            ),
        )
        .unwrap();
        let out = std::process::Command::new("xmllint")
            .arg("--noout")
            .arg("--schema")
            .arg(&driver)
            .arg(&cpl_path)
            .output()
            .unwrap();
        if !out.status.success() {
            eprintln!("xmllint failed: {}", String::from_utf8_lossy(&out.stderr));
        }
        Some(out.status.success())
    }

    /// Validate a language CPL against the official ST 2067-3:2016 XSD. Gated on
    /// IMFWIZARD_IMF_XSD_DIR (a dir holding imf-cpl-20160411.xsd and
    /// xmldsig-core-schema.xsd anywhere below it) plus xmllint; skips when absent.
    #[test]
    fn language_cpl_passes_st2067_3_xsd() {
        let dir = tempfile::tempdir().unwrap();
        let cpl_path = dir.path().join("CPL_lang.xml");
        let opts = ImpOptions {
            fps_num: 24,
            fps_den: 1,
            ..Default::default()
        };
        let comp = Composition {
            title: "Lang Test".into(),
            content_kind: "feature".into(),
            audio_files: vec![AudioTrack {
                path: "de.wav".into(),
                language: Some("de-DE".into()),
                role: None,
            }],
            ..Default::default()
        };
        write_cpl(
            &cpl_path,
            "11111111-2222-3333-4444-555555555555",
            &opts,
            &comp,
            &[],
        )
        .unwrap();
        let cpl_xml = std::fs::read_to_string(&cpl_path).unwrap();
        match validate_st2067_3(&cpl_xml) {
            Some(ok) => assert!(ok, "language CPL must pass ST 2067-3 XSD"),
            None => eprintln!("skipping: set IMFWIZARD_IMF_XSD_DIR and install xmllint"),
        }
    }

    /// Validate an accessibility CPL (audio-description MCA descriptor + LocaleList)
    /// against the ST 2067-3:2016 XSD. Same gating as the language test.
    #[test]
    fn accessibility_cpl_passes_st2067_3_xsd() {
        let dir = tempfile::tempdir().unwrap();
        let cpl_path = dir.path().join("CPL_ad.xml");
        let opts = ImpOptions {
            fps_num: 24,
            fps_den: 1,
            ..Default::default()
        };
        let comp = Composition {
            title: "AD Test".into(),
            content_kind: "feature".into(),
            audio_files: vec![AudioTrack {
                path: "ad.wav".into(),
                language: Some("en-US".into()),
                role: Some(AudioRole::AudioDescription),
            }],
            ..Default::default()
        };
        let tracks = [accessibility_track()];
        write_cpl(
            &cpl_path,
            "22222222-3333-4444-5555-666666666666",
            &opts,
            &comp,
            &tracks,
        )
        .unwrap();
        let cpl_xml = std::fs::read_to_string(&cpl_path).unwrap();
        match validate_st2067_3(&cpl_xml) {
            Some(ok) => assert!(ok, "accessibility CPL must pass ST 2067-3 XSD"),
            None => eprintln!("skipping: set IMFWIZARD_IMF_XSD_DIR and install xmllint"),
        }
    }
}
