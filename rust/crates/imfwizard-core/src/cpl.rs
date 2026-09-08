use std::path::Path;

use postkit::packaging::{App2eEdition, ImfCpl, ImfEssenceDescriptor, ImfResource, ImfTrackKind};

use crate::MxfTrackFile;
use crate::imp::{Composition, ImpOptions};

/// Write an IMF CPL (ST 2067-3) using the shared postkit writer.
///
/// Track files are classified by their filename prefix (VIDEO_/AUDIO_/SUBTITLE_),
/// the same convention the MXF wrapper writes. Audio languages from `comp` are
/// written into a composition-level LocaleList (ST 2067-3 Locale/LanguageList).
/// Every picture and sound resource names an EssenceDescriptorList entry read
/// back off its own track file, linked by SourceEncoding, which ST 2067-3 makes
/// mandatory. HDR content light levels become ST 2067-21 ExtensionProperties.
pub fn write_cpl(
    path: &Path,
    cpl_uuid: &str,
    opts: &ImpOptions,
    comp: &Composition,
    track_files: &[MxfTrackFile],
) -> std::io::Result<()> {
    let mut resources = Vec::new();
    let mut descriptors = Vec::new();
    let edit_rate = asdcplib::Rational::new(opts.fps_num as i32, opts.fps_den as i32);
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
        let source_encoding = track_file_descriptor(&tf.path, kind, edit_rate)?.map(|descriptor| {
            let id = descriptor.id.clone();
            descriptors.push(descriptor);
            id
        });
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
        max_cll: comp.hdr.as_ref().and_then(|h| h.max_cll),
        max_fall: comp.hdr.as_ref().and_then(|h| h.max_fall),
        // the picture is full range RGB 4:4:4, which the 2016 edition's image
        // characteristics allow for PQ but not for Rec.709, and COLOR.8 is 2020 only
        app2e_edition: App2eEdition::Edition2020,
    };

    std::fs::write(path, cpl.to_xml())
}

/// One EssenceDescriptorList entry, read back out of the track file it describes
/// under a fresh id, since a validator compares the entry with the MXF and every
/// item has to be the MXF's own. `None` for a kind that names no descriptor.
pub(crate) fn track_file_descriptor(
    track_file: &Path,
    kind: ImfTrackKind,
    edit_rate: asdcplib::Rational,
) -> std::io::Result<Option<ImfEssenceDescriptor>> {
    let body = match kind {
        ImfTrackKind::Image => picture_descriptor_body(track_file)?,
        ImfTrackKind::Audio => sound_descriptor_body(track_file, edit_rate)?,
        ImfTrackKind::Subtitle => return Ok(None),
    };
    Ok(Some(ImfEssenceDescriptor {
        id: uuid::Uuid::new_v4().to_string(),
        body,
    }))
}

fn read<T>(result: asdcplib::Result<T>, what: &str, track_file: &Path) -> std::io::Result<T> {
    result.map_err(|e| {
        std::io::Error::other(format!(
            "cannot read the {what} of {}: {e}",
            track_file.display()
        ))
    })
}

/// The picture MXF's own RGBA descriptor and JPEG 2000 sub-descriptor, as the
/// RegXML the CPL's EssenceDescriptorList carries.
fn picture_descriptor_body(picture: &Path) -> std::io::Result<String> {
    let mut reader = asdcplib::as02::jp2k::MxfReader::new();
    read(
        reader.open_read(&picture.to_string_lossy()),
        "picture MXF",
        picture,
    )?;
    let descriptor = read(reader.rgba_essence_descriptor(), "RGBA descriptor", picture)?;
    let jpeg2000 = read(
        reader.jpeg2000_sub_descriptor(),
        "JPEG 2000 sub-descriptor",
        picture,
    )?;
    let _ = reader.close();
    Ok(postkit::regxml::picture_descriptor_regxml(
        &descriptor,
        &jpeg2000,
    ))
}

/// The sound MXF's own WAVE PCM descriptor and MCA label sub-descriptors, as the
/// RegXML the CPL's EssenceDescriptorList carries.
fn sound_descriptor_body(sound: &Path, edit_rate: asdcplib::Rational) -> std::io::Result<String> {
    let mut reader = asdcplib::as02::pcm::MxfReader::new();
    read(
        reader.open_read(&sound.to_string_lossy(), edit_rate),
        "sound MXF",
        sound,
    )?;
    let descriptor = read(reader.wave_audio_descriptor(), "WAVE PCM descriptor", sound)?;
    let labels = read(
        reader.mca_label_subdescriptors(),
        "MCA label sub-descriptors",
        sound,
    )?;
    let _ = reader.close();
    Ok(postkit::regxml::sound_descriptor_regxml(
        &descriptor,
        &labels,
    ))
}

/// The SMPTE and xmldsig XSDs Photon vendors, which hold imf-cpl-20160411.xsd
/// and xmldsig-core-schema.xsd. IMFWIZARD_IMF_XSD_DIR overrides it.
#[cfg(test)]
const VENDORED_IMF_XSD_DIR: &str = "../../../extern/dcpdoctor/extern/photon/src/main/resources";

#[cfg(test)]
fn imf_xsd_dir() -> std::path::PathBuf {
    match std::env::var("IMFWIZARD_IMF_XSD_DIR") {
        Ok(dir) => std::path::PathBuf::from(dir),
        Err(_) => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(VENDORED_IMF_XSD_DIR),
    }
}

/// xmllint's complaint about `cpl_xml` against the ST 2067-3 XSDs, empty
/// when it validates.
#[cfg(test)]
pub(crate) fn st2067_3_complaint(cpl_xml: &str) -> String {
    let xsd_dir = imf_xsd_dir();
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
    let root = xsd_dir.as_path();
    let (Some(cpl_xsd), Some(dsig_xsd)) = (
        walk(root, "imf-cpl-20160411.xsd"),
        walk(root, "xmldsig-core-schema.xsd"),
    ) else {
        panic!(
            "could not locate imf-cpl-20160411.xsd and xmldsig-core-schema.xsd under {}",
            root.display()
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
            cpl = postkit::file_uri::file_uri(&cpl_xsd),
            dsig = postkit::file_uri::file_uri(&dsig_xsd),
        ),
    )
    .unwrap();
    let out = std::process::Command::new("xmllint")
        .arg("--noout")
        .arg("--schema")
        .arg(&driver)
        .arg(&cpl_path)
        .output()
        .expect("run xmllint");
    if out.status.success() {
        return String::new();
    }
    String::from_utf8_lossy(&out.stderr).into_owned()
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::imp::{AudioRole, AudioTrack};

    const APP2E_2020_NAMESPACE: &str = "http://www.smpte-ra.org/ns/2067-21/2020";

    fn app2e_2020_property(cpl_xml: &str, local_name: &str) -> Option<String> {
        use quick_xml::events::Event;
        use quick_xml::name::ResolveResult;

        let mut reader = quick_xml::NsReader::from_str(cpl_xml);
        let mut wanted = false;
        loop {
            let (namespace, event) = reader.read_resolved_event().expect("parse the CPL");
            match event {
                Event::Start(element) => {
                    wanted = matches!(namespace, ResolveResult::Bound(bound)
                        if bound.as_ref() == APP2E_2020_NAMESPACE.as_bytes())
                        && element.local_name().as_ref() == local_name.as_bytes();
                }
                Event::Text(text) if wanted => {
                    return Some(text.unescape().expect("unescape the text").into_owned());
                }
                Event::Eof => return None,
                _ => {}
            }
        }
    }

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
        assert!(xml.contains("<cc:ApplicationIdentification>http://www.smpte-ra.org/ns/2067-21/2020</cc:ApplicationIdentification>"));
    }

    // Photon picks its constraints validator off this string, and only the 2020
    // edition's table holds COLOR.8 and full range Rec.709 RGB
    #[test]
    fn every_composition_claims_the_2020_edition() {
        let dir = tempfile::tempdir().unwrap();
        let identification = |preset: &str| {
            let path = dir.path().join(format!("CPL_{preset}.xml"));
            let comp = Composition {
                title: "Test".into(),
                hdr: Some(crate::hdr_wcg::HdrWcg::from_flags(preset, None).unwrap()),
                ..Default::default()
            };
            write_cpl(&path, "cpl", &ImpOptions::default(), &comp, &[]).unwrap();
            let xml = std::fs::read_to_string(&path).unwrap();
            let start = xml.find("<cc:ApplicationIdentification>").unwrap();
            let end = xml.find("</cc:ApplicationIdentification>").unwrap();
            xml[start + "<cc:ApplicationIdentification>".len()..end].to_string()
        };
        assert_eq!(
            identification("hlg-bt2020"),
            "http://www.smpte-ra.org/ns/2067-21/2020"
        );
        assert_eq!(
            identification("pq-bt2020"),
            "http://www.smpte-ra.org/ns/2067-21/2020"
        );
        assert_eq!(
            identification("pq-p3d65"),
            "http://www.smpte-ra.org/ns/2067-21/2020"
        );
    }

    /// ST 2067-21 carries MaxCLL/MaxFALL in the CPL ExtensionProperties, not on the
    /// essence descriptor. Photon only schema-validates them, it never compares them
    /// against the picture essence.
    #[test]
    fn write_cpl_writes_content_light_levels_from_hdr() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CPL_test.xml");
        let hdr = crate::hdr_wcg::HdrWcg::from_flags("pq-bt2020", None)
            .unwrap()
            .with_content_light_levels(Some(993), Some(362))
            .unwrap();
        let comp = Composition {
            title: "Test".into(),
            hdr: Some(hdr),
            ..Default::default()
        };

        write_cpl(&path, "cpl", &ImpOptions::default(), &comp, &[]).unwrap();
        let xml = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            app2e_2020_property(&xml, "MaxCLL").as_deref(),
            Some("993"),
            "MaxCLL must be bound to {APP2E_2020_NAMESPACE}"
        );
        assert_eq!(
            app2e_2020_property(&xml, "MaxFALL").as_deref(),
            Some("362"),
            "MaxFALL must be bound to {APP2E_2020_NAMESPACE}"
        );
        // they follow ApplicationIdentification inside ExtensionProperties
        assert!(
            xml.find("<cc:ApplicationIdentification>").unwrap()
                < xml.find("<app2e:MaxCLL").unwrap()
        );
        assert!(xml.find("<app2e:MaxFALL").unwrap() < xml.find("</ExtensionProperties>").unwrap());
        // HDR without light levels leaves the CPL free of them
        let plain = Composition {
            title: "Test".into(),
            hdr: Some(crate::hdr_wcg::HdrWcg::from_flags("pq-bt2020", None).unwrap()),
            ..Default::default()
        };
        write_cpl(&path, "cpl", &ImpOptions::default(), &plain, &[]).unwrap();
        let xml = std::fs::read_to_string(&path).unwrap();
        assert!(!xml.contains("MaxCLL"));
        assert!(!xml.contains("MaxFALL"));
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

    const TWENTY_FOUR_FPS: asdcplib::Rational = asdcplib::Rational {
        numerator: 24,
        denominator: 1,
    };

    fn sound_track(dir: &Path, name: &str, channels: u16, labels: String) -> MxfTrackFile {
        crate::mxf_wrap::wrapped_sound(
            dir,
            name,
            channels,
            Some(postkit::mxf_wrap::McaConfig {
                labels,
                spoken_language: Some("en-US".into()),
                soundfield_group: Some(postkit::mxf_wrap::SoundfieldGroup {
                    title: "Sound Test".into(),
                    title_version: "Original Version".into(),
                    audio_content_kind: "PRM".into(),
                    audio_element_kind: "FCMP".into(),
                }),
            }),
        )
    }

    /// The WaveAudioDescriptor InstanceID the sound MXF itself carries, as the
    /// CPL spells it. Photon compares the two, so they have to be one value.
    fn sound_instance_id(sound: &Path) -> String {
        let mut reader = asdcplib::as02::pcm::MxfReader::new();
        reader
            .open_read(&sound.to_string_lossy(), TWENTY_FOUR_FPS)
            .expect("the sound MXF opens");
        let descriptor = reader
            .wave_audio_descriptor()
            .expect("a WAVE PCM descriptor");
        reader.close().unwrap();
        format!(
            "urn:uuid:{}",
            uuid::Uuid::from_bytes(descriptor.instance_id)
        )
    }

    /// Plain sound, no accessibility role: ST 2067-3 still makes SourceEncoding
    /// mandatory, and the descriptor it names is the track file's own.
    #[test]
    fn a_sound_resource_names_the_track_files_own_descriptor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CPL_sound.xml");
        let opts = ImpOptions {
            fps_num: 24,
            fps_den: 1,
            ..Default::default()
        };
        let comp = Composition {
            title: "Sound Test".into(),
            audio_files: vec![AudioTrack {
                path: "stereo.wav".into(),
                language: Some("en-US".into()),
                role: None,
            }],
            ..Default::default()
        };
        let track = sound_track(dir.path(), "stereo", 2, crate::imp::mca_labels(2, None));
        write_cpl(&path, "cpl", &opts, &comp, std::slice::from_ref(&track)).unwrap();
        let xml = std::fs::read_to_string(path).unwrap();

        assert!(xml.contains("<SourceEncoding>"), "{xml}");
        assert!(xml.contains("<r0:WAVEPCMDescriptor"), "{xml}");
        let instance_id = sound_instance_id(&track.path);
        assert!(
            xml.contains(&format!("<r1:InstanceID>{instance_id}</r1:InstanceID>")),
            "the CPL must carry the MXF's own InstanceID {instance_id}:\n{xml}"
        );
        assert!(
            xml.contains("<r1:MCATagSymbol>chL</r1:MCATagSymbol>"),
            "{xml}"
        );
        assert!(
            xml.contains("<r1:MCATagSymbol>chR</r1:MCATagSymbol>"),
            "{xml}"
        );
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
        let track = sound_track(
            dir.path(),
            "ad",
            1,
            crate::imp::mca_labels(1, Some(AudioRole::AudioDescription)),
        );
        write_cpl(&path, "cpl", &opts, &comp, std::slice::from_ref(&track)).unwrap();
        let xml = std::fs::read_to_string(path).unwrap();
        assert!(xml.contains("<EssenceDescriptorList>"));
        assert!(
            xml.contains("<r1:MCATagSymbol>chVIN</r1:MCATagSymbol>"),
            "{xml}"
        );
        assert!(
            xml.contains("<r1:MCATagName>Visually Impaired-Narrative</r1:MCATagName>"),
            "{xml}"
        );
        assert!(xml.contains("<r1:RFC5646SpokenLanguage>en-US</r1:RFC5646SpokenLanguage>"));
        // the audio resource must link to the descriptor via SourceEncoding
        assert!(xml.contains("<SourceEncoding>"));
        // and the descriptor has to be the one the MXF carries, not a fresh one
        let instance_id = sound_instance_id(&track.path);
        assert!(
            xml.contains(&format!("<r1:InstanceID>{instance_id}</r1:InstanceID>")),
            "the CPL must carry the MXF's own InstanceID {instance_id}:\n{xml}"
        );
    }

    /// An HDR/WCG CPL (image RGBADescriptor with transfer/colour ULs + ST 2086
    /// mastering display) must pass the ST 2067-3:2016 XSD.
    #[test]
    fn hdr_cpl_passes_st2067_3_xsd() {
        let dir = tempfile::tempdir().unwrap();
        let cpl_path = dir.path().join("CPL_hdr.xml");
        let opts = ImpOptions {
            fps_num: 24,
            fps_den: 1,
            ..Default::default()
        };
        let hdr = crate::hdr_wcg::HdrWcg::from_flags(
            "pq-bt2020",
            Some("R(34000,16000)G(13250,34500)B(7500,3000)WP(15635,16450)L(40000000,50)"),
        )
        .unwrap()
        .with_content_light_levels(Some(993), Some(362))
        .unwrap();
        let comp = Composition {
            title: "HDR Test".into(),
            content_kind: "feature".into(),
            hdr: Some(hdr),
            ..Default::default()
        };
        let video =
            crate::mxf_wrap::wrapped_picture(dir.path(), Some(&comp.hdr.clone().unwrap()), 1);
        write_cpl(
            &cpl_path,
            "33333333-4444-5555-6666-777777777777",
            &opts,
            &comp,
            &[video],
        )
        .unwrap();
        let cpl_xml = std::fs::read_to_string(&cpl_path).unwrap();
        assert!(cpl_xml.contains("<r0:RGBADescriptor"));
        assert!(cpl_xml.contains("<r1:TransferCharacteristic>"));
        // no 2020 App 2E schema in the tree, so xmllint skips these under xs:any lax
        assert_eq!(
            app2e_2020_property(&cpl_xml, "MaxCLL").as_deref(),
            Some("993"),
            "MaxCLL must be bound to {APP2E_2020_NAMESPACE}"
        );
        assert_eq!(
            app2e_2020_property(&cpl_xml, "MaxFALL").as_deref(),
            Some("362"),
            "MaxFALL must be bound to {APP2E_2020_NAMESPACE}"
        );
        let complaint = st2067_3_complaint(&cpl_xml);
        assert!(
            complaint.is_empty(),
            "HDR CPL must pass ST 2067-3 XSD:\n{complaint}"
        );
    }

    /// Validate a language CPL against the official ST 2067-3:2016 XSD.
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
        let complaint = st2067_3_complaint(&cpl_xml);
        assert!(
            complaint.is_empty(),
            "language CPL must pass ST 2067-3 XSD:\n{complaint}"
        );
    }

    /// Validate an accessibility CPL (audio-description MCA descriptor + LocaleList)
    /// against the ST 2067-3:2016 XSD.
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
        let tracks = [sound_track(
            dir.path(),
            "ad",
            1,
            crate::imp::mca_labels(1, Some(AudioRole::AudioDescription)),
        )];
        write_cpl(
            &cpl_path,
            "22222222-3333-4444-5555-666666666666",
            &opts,
            &comp,
            &tracks,
        )
        .unwrap();
        let cpl_xml = std::fs::read_to_string(&cpl_path).unwrap();
        let complaint = st2067_3_complaint(&cpl_xml);
        assert!(
            complaint.is_empty(),
            "accessibility CPL must pass ST 2067-3 XSD:\n{complaint}"
        );
    }

    /// Validate an IMP with plain sound, no accessibility role, against the ST
    /// 2067-3:2016 XSD. TrackFileResourceType makes SourceEncoding mandatory, so
    /// the sound resource fails the schema without its descriptor.
    #[test]
    fn a_sound_cpl_passes_st2067_3_xsd() {
        let dir = tempfile::tempdir().unwrap();
        let cpl_path = dir.path().join("CPL_sound.xml");
        let opts = ImpOptions {
            fps_num: 24,
            fps_den: 1,
            ..Default::default()
        };
        let comp = Composition {
            title: "Sound Test".into(),
            content_kind: "feature".into(),
            audio_files: vec![AudioTrack {
                path: "stereo.wav".into(),
                language: Some("en-US".into()),
                role: None,
            }],
            ..Default::default()
        };
        let tracks = [
            crate::mxf_wrap::wrapped_picture(dir.path(), None, 1),
            sound_track(dir.path(), "stereo", 2, crate::imp::mca_labels(2, None)),
        ];
        write_cpl(
            &cpl_path,
            "44444444-5555-6666-7777-888888888888",
            &opts,
            &comp,
            &tracks,
        )
        .unwrap();
        let cpl_xml = std::fs::read_to_string(&cpl_path).unwrap();
        let complaint = st2067_3_complaint(&cpl_xml);
        assert!(
            complaint.is_empty(),
            "sound CPL must pass ST 2067-3 XSD:\n{complaint}"
        );
    }
}
