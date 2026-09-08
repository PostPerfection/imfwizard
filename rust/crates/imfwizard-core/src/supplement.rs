use std::path::{Path, PathBuf};

use postkit::packaging::{App2eEdition, ImfCpl, ImfEssenceDescriptor, ImfResource, ImfTrackKind};
use quick_xml::events::Event;
use quick_xml::name::QName;
use quick_xml::reader::Reader;

use crate::EssenceType;
use crate::MxfTrackFile;

/// Options for creating a supplemental IMP.
///
/// A supplemental IMP (ST 2067-2/-3 OV+supplemental model) packages ONLY the new
/// or changed track files plus a new CPL. The CPL references the new track files
/// (present here) and the unchanged OV track files by their UUIDs (present in the
/// OV IMP, not duplicated). ASSETMAP/PKL list only the physically-present assets.
pub struct SupplementOptions {
    pub ov_dir: PathBuf,
    pub title: String,
    pub output_dir: PathBuf,
    /// Replace an OV track, each spec `<path>@<track>` where track is
    /// `video`, `audio[:N]`, or `subtitle[:N]` (N defaults to 0).
    pub replace: Vec<String>,
    /// Add a new track, each spec `<path>@<track>` where track is `audio` or `subtitle`.
    pub add: Vec<String>,
}

/// Result of supplemental IMP creation.
pub struct SupplementResult {
    pub success: bool,
    pub error: String,
    pub output_dir: PathBuf,
}

impl SupplementResult {
    fn fail(output_dir: &Path, error: impl Into<String>) -> Self {
        SupplementResult {
            success: false,
            error: error.into(),
            output_dir: output_dir.to_path_buf(),
        }
    }
}

/// A parsed `<path>@<track>` spec.
struct TrackSpec {
    path: PathBuf,
    kind: ImfTrackKind,
    index: usize,
}

fn parse_spec(spec: &str) -> Result<TrackSpec, String> {
    let (path, track) = spec
        .rsplit_once('@')
        .ok_or_else(|| format!("bad track spec `{spec}`, expected <path>@<track>"))?;
    if path.is_empty() {
        return Err(format!("bad track spec `{spec}`, empty path"));
    }
    let (name, index) = match track.split_once(':') {
        Some((n, i)) => {
            let i = i
                .parse::<usize>()
                .map_err(|_| format!("bad track index in `{spec}`"))?;
            (n, i)
        }
        None => (track, 0),
    };
    let kind = match name {
        "video" | "image" => ImfTrackKind::Image,
        "audio" => ImfTrackKind::Audio,
        "subtitle" | "subtitles" => ImfTrackKind::Subtitle,
        other => return Err(format!("unknown track `{other}` in `{spec}`")),
    };
    Ok(TrackSpec {
        path: PathBuf::from(path),
        kind,
        index,
    })
}

fn essence_for(kind: ImfTrackKind) -> EssenceType {
    match kind {
        ImfTrackKind::Image => EssenceType::J2k,
        ImfTrackKind::Audio => EssenceType::Wav,
        ImfTrackKind::Subtitle => EssenceType::TimedText,
    }
}

fn prefix_for(kind: ImfTrackKind) -> &'static str {
    match kind {
        ImfTrackKind::Image => "VIDEO",
        ImfTrackKind::Audio => "AUDIO",
        ImfTrackKind::Subtitle => "SUBTITLE",
    }
}

/// One resource read from a CPL.
pub(crate) struct CplResource {
    pub(crate) kind: ImfTrackKind,
    pub(crate) uuid: String,
    pub(crate) duration: u64,
    /// The EssenceDescriptorList entry this resource names, when it names one.
    pub(crate) source_encoding: Option<String>,
}

pub(crate) struct CplResources {
    pub(crate) fps_num: u32,
    pub(crate) fps_den: u32,
    pub(crate) content_kind: String,
    pub(crate) resources: Vec<CplResource>,
    /// The ST 2067-21 edition the OV claims, which a supplement over it claims too.
    pub(crate) app2e_edition: App2eEdition,
    /// The OV's EssenceDescriptorList, carried through verbatim.
    pub(crate) essence_descriptors: Vec<ImfEssenceDescriptor>,
}

fn local_name(qname: QName) -> String {
    String::from_utf8_lossy(qname.local_name().as_ref()).into_owned()
}

fn strip_urn(s: &str) -> String {
    s.replace("urn:uuid:", "")
}

/// Parse an IMF CPL into its composition edit rate and per-kind resources.
pub(crate) fn parse_cpl_resources(cpl_path: &Path) -> Result<CplResources, String> {
    let content = std::fs::read_to_string(cpl_path)
        .map_err(|e| format!("cannot read CPL {}: {e}", cpl_path.display()))?;

    let mut reader = Reader::from_str(&content);
    reader.config_mut().trim_text(true);

    let mut cur = String::new();
    let mut seq_kind: Option<ImfTrackKind> = None;
    let mut in_resource = false;
    let mut edit_rate: Option<(u32, u32)> = None;
    let mut content_kind = String::new();
    let mut resources: Vec<CplResource> = Vec::new();
    let mut cur_uuid = String::new();
    let mut cur_dur = 0u64;
    let mut cur_source_encoding: Option<String> = None;
    let mut app2e_edition = App2eEdition::Edition2016;
    let mut essence_descriptors: Vec<ImfEssenceDescriptor> = Vec::new();
    let mut descriptor_body_start: Option<usize> = None;
    let mut descriptor_id = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name());
                cur = name.clone();
                match name.as_str() {
                    "MainImageSequence" => seq_kind = Some(ImfTrackKind::Image),
                    "MainAudioSequence" => seq_kind = Some(ImfTrackKind::Audio),
                    "SubtitlesSequence" => seq_kind = Some(ImfTrackKind::Subtitle),
                    "Resource" => {
                        in_resource = true;
                        cur_uuid.clear();
                        cur_dur = 0;
                        cur_source_encoding = None;
                    }
                    // the body is the source text between this tag and its end tag
                    "EssenceDescriptor" => {
                        descriptor_body_start = Some(reader.buffer_position() as usize);
                        descriptor_id.clear();
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default().trim().to_string();
                if text.is_empty() {
                    continue;
                }
                match cur.as_str() {
                    // composition edit rate is the first EditRate, before any sequence
                    "EditRate" if edit_rate.is_none() && seq_kind.is_none() => {
                        let mut it = text.split_whitespace();
                        if let (Some(n), Some(d)) = (it.next(), it.next())
                            && let (Ok(n), Ok(d)) = (n.parse::<u32>(), d.parse::<u32>())
                            && d > 0
                        {
                            edit_rate = Some((n, d));
                        }
                    }
                    "ContentKind" if content_kind.is_empty() && seq_kind.is_none() => {
                        content_kind = text;
                    }
                    "ApplicationIdentification" => {
                        if text == App2eEdition::Edition2020.namespace() {
                            app2e_edition = App2eEdition::Edition2020;
                        }
                    }
                    "Id" if descriptor_body_start.is_some() && descriptor_id.is_empty() => {
                        descriptor_id = strip_urn(&text);
                    }
                    "TrackFileId" if in_resource => cur_uuid = strip_urn(&text),
                    "SourceEncoding" if in_resource => cur_source_encoding = Some(strip_urn(&text)),
                    "SourceDuration" if in_resource => cur_dur = text.parse().unwrap_or(cur_dur),
                    "IntrinsicDuration" if in_resource && cur_dur == 0 => {
                        cur_dur = text.parse().unwrap_or(0)
                    }
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                cur.clear();
                match local_name(e.name()).as_str() {
                    "Resource" => {
                        in_resource = false;
                        if let Some(kind) = seq_kind
                            && !cur_uuid.is_empty()
                        {
                            resources.push(CplResource {
                                kind,
                                uuid: std::mem::take(&mut cur_uuid),
                                duration: cur_dur,
                                source_encoding: cur_source_encoding.take(),
                            });
                        }
                    }
                    "MainImageSequence" | "MainAudioSequence" | "SubtitlesSequence" => {
                        seq_kind = None
                    }
                    name @ "EssenceDescriptor" => {
                        if let Some(start) = descriptor_body_start.take()
                            && !descriptor_id.is_empty()
                        {
                            let end = reader.buffer_position() as usize - (name.len() + 3);
                            essence_descriptors.push(ImfEssenceDescriptor {
                                id: std::mem::take(&mut descriptor_id),
                                body: descriptor_body(&content[start..end]),
                            });
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("CPL parse error in {}: {e}", cpl_path.display())),
            _ => {}
        }
    }

    let (fps_num, fps_den) =
        edit_rate.ok_or_else(|| format!("{} has no composition EditRate", cpl_path.display()))?;
    if resources.is_empty() {
        return Err(format!(
            "{} has no track-file resources",
            cpl_path.display()
        ));
    }
    Ok(CplResources {
        fps_num,
        fps_den,
        content_kind,
        resources,
        app2e_edition,
        essence_descriptors,
    })
}

/// The descriptor itself, which is everything after the entry's own Id.
fn descriptor_body(entry: &str) -> String {
    match entry.find("</Id>") {
        Some(id_end) => entry[id_end + "</Id>".len()..].trim_end().to_string(),
        None => entry.trim_end().to_string(),
    }
}

/// The OV descriptors a resource still names. A replaced track file has new
/// essence, so the descriptor the OV wrote for it is dropped with the link.
fn descriptors_still_referenced(
    descriptors: &[ImfEssenceDescriptor],
    resources: &[ImfResource],
) -> Vec<ImfEssenceDescriptor> {
    descriptors
        .iter()
        .filter(|descriptor| {
            resources
                .iter()
                .any(|resource| resource.source_encoding.as_deref() == Some(&descriptor.id))
        })
        .cloned()
        .collect()
}

/// Wrap one new/changed asset into an MXF track file inside the supplemental IMP.
fn wrap_asset(
    spec: &TrackSpec,
    output_dir: &Path,
    fps_num: u32,
    fps_den: u32,
) -> Result<MxfTrackFile, String> {
    if !spec.path.exists() {
        return Err(format!("input not found: {}", spec.path.display()));
    }
    let asset_uuid = uuid::Uuid::new_v4();
    let mxf_path = output_dir.join(format!("{}_{asset_uuid}.mxf", prefix_for(spec.kind)));
    let wrap = crate::mxf_wrap::wrap_mxf(&crate::mxf_wrap::MxfWrapOptions {
        input_dir: spec.path.clone(),
        output_file: mxf_path,
        essence_type: essence_for(spec.kind),
        edit_rate_num: fps_num,
        edit_rate_den: fps_den,
        duration: 0,
        hdr: None,
        mca: None,
        asset_uuid: Some(*asset_uuid.as_bytes()),
    });
    if !wrap.success {
        return Err(format!(
            "wrap failed for {}: {}",
            spec.path.display(),
            wrap.error
        ));
    }
    Ok(wrap.track_file)
}

/// Create a supplemental IMP referencing an Original Version (OV).
pub fn create_supplement(opts: &SupplementOptions) -> SupplementResult {
    let out = &opts.output_dir;

    if !opts.ov_dir.is_dir() {
        return SupplementResult::fail(
            out,
            format!("OV is not a directory: {}", opts.ov_dir.display()),
        );
    }
    if opts.replace.is_empty() && opts.add.is_empty() {
        return SupplementResult::fail(
            out,
            "nothing to change; a supplemental needs at least one --replace or --add",
        );
    }

    // 1. locate and parse the OV CPL
    let cpls = crate::timeline::list_cpls(&opts.ov_dir);
    let Some(cpl) = cpls.first() else {
        return SupplementResult::fail(out, "no CPL found in OV IMP");
    };
    let ov = match parse_cpl_resources(&opts.ov_dir.join(&cpl.file_path)) {
        Ok(v) => v,
        Err(e) => return SupplementResult::fail(out, e),
    };

    // 2. parse specs
    let mut replace_specs = Vec::new();
    for s in &opts.replace {
        match parse_spec(s) {
            Ok(v) => replace_specs.push(v),
            Err(e) => return SupplementResult::fail(out, e),
        }
    }
    let mut add_specs = Vec::new();
    for s in &opts.add {
        match parse_spec(s) {
            Ok(v) => add_specs.push(v),
            Err(e) => return SupplementResult::fail(out, e),
        }
    }
    // a supplemental cannot add a second image track in the App2E single-image model
    if let Some(s) = add_specs.iter().find(|s| s.kind == ImfTrackKind::Image) {
        return SupplementResult::fail(
            out,
            format!(
                "cannot --add an image track (only audio/subtitle); got {}",
                s.path.display()
            ),
        );
    }

    // 3. validate every replace target resolves to a real OV resource before wrapping
    for s in &replace_specs {
        let count = ov.resources.iter().filter(|r| r.kind == s.kind).count();
        if s.index >= count {
            return SupplementResult::fail(
                out,
                format!(
                    "OV has no {:?} track at index {} (it has {count})",
                    s.kind, s.index
                ),
            );
        }
    }

    if let Err(e) = std::fs::create_dir_all(out) {
        return SupplementResult::fail(out, format!("cannot create output dir: {e}"));
    }

    // 4. wrap the new/changed assets (only these are physically present here)
    let mut present: Vec<MxfTrackFile> = Vec::new();
    // each new track file's own descriptor, which its resource names instead of
    // the OV entry that described the essence it replaces
    let mut wrapped_descriptors: Vec<ImfEssenceDescriptor> = Vec::new();
    let edit_rate = asdcplib::Rational::new(ov.fps_num as i32, ov.fps_den as i32);
    let mut resources: Vec<ImfResource> = ov
        .resources
        .iter()
        .map(|r| ImfResource {
            track_file_uuid: r.uuid.clone(),
            duration: r.duration,
            kind: r.kind,
            source_encoding: r.source_encoding.clone(),
        })
        .collect();

    for s in &replace_specs {
        let tf = match wrap_asset(s, out, ov.fps_num, ov.fps_den) {
            Ok(tf) => tf,
            Err(e) => return SupplementResult::fail(out, e),
        };
        // swap the Nth resource of this kind to the new track file
        let target = resources
            .iter_mut()
            .filter(|r| r.kind == s.kind)
            .nth(s.index)
            .expect("replace target validated above");
        target.track_file_uuid = tf.uuid.clone();
        target.duration = tf.duration;
        let descriptor = match crate::cpl::track_file_descriptor(&tf.path, s.kind, edit_rate) {
            Ok(descriptor) => descriptor,
            Err(e) => return SupplementResult::fail(out, e.to_string()),
        };
        target.source_encoding = descriptor.as_ref().map(|d| d.id.clone());
        wrapped_descriptors.extend(descriptor);
        present.push(tf);
    }
    for s in &add_specs {
        let tf = match wrap_asset(s, out, ov.fps_num, ov.fps_den) {
            Ok(tf) => tf,
            Err(e) => return SupplementResult::fail(out, e),
        };
        let descriptor = match crate::cpl::track_file_descriptor(&tf.path, s.kind, edit_rate) {
            Ok(descriptor) => descriptor,
            Err(e) => return SupplementResult::fail(out, e.to_string()),
        };
        resources.push(ImfResource {
            track_file_uuid: tf.uuid.clone(),
            duration: tf.duration,
            kind: s.kind,
            source_encoding: descriptor.as_ref().map(|d| d.id.clone()),
        });
        wrapped_descriptors.extend(descriptor);
        present.push(tf);
    }

    if present.is_empty() {
        return SupplementResult::fail(out, "no track files were packaged; nothing changed");
    }

    // 5. build the supplemental CPL: references both new and unchanged OV UUIDs
    let cpl_uuid = uuid::Uuid::new_v4().to_string();
    let mut essence_descriptors = descriptors_still_referenced(&ov.essence_descriptors, &resources);
    essence_descriptors.extend(wrapped_descriptors);
    let cpl = ImfCpl {
        uuid: cpl_uuid.clone(),
        title: opts.title.clone(),
        content_kind: ov.content_kind.clone(),
        issuer: "IMF Wizard".to_string(),
        creator: "IMF Wizard".to_string(),
        issue_date: crate::issue_date(),
        fps_num: ov.fps_num,
        fps_den: ov.fps_den,
        resources,
        languages: Vec::new(),
        essence_descriptors,
        max_cll: None,
        max_fall: None,
        app2e_edition: ov.app2e_edition,
    };
    let cpl_path = out.join(format!("CPL_{cpl_uuid}.xml"));
    if let Err(e) = std::fs::write(&cpl_path, cpl.to_xml()) {
        return SupplementResult::fail(out, format!("cannot write CPL: {e}"));
    }

    // 6. PKL + ASSETMAP over only the present assets (new CPL + new track files)
    let pkl_uuid = uuid::Uuid::new_v4().to_string();
    let pkl_path = out.join(format!("PKL_{pkl_uuid}.xml"));
    let cpls = [crate::imp::CplEntry {
        uuid: cpl_uuid.clone(),
        path: cpl_path.clone(),
    }];
    if let Err(e) = crate::pkl::write_pkl(&pkl_path, &pkl_uuid, &cpls, &present) {
        return SupplementResult::fail(out, format!("cannot write PKL: {e}"));
    }
    let am_path = out.join("ASSETMAP.xml");
    if let Err(e) = crate::assetmap::write_assetmap(&am_path, &pkl_uuid, &cpls, &present) {
        return SupplementResult::fail(out, format!("cannot write ASSETMAP: {e}"));
    }

    SupplementResult {
        success: true,
        error: String::new(),
        output_dir: out.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ov_cpl_xml() -> &'static str {
        r#"<?xml version="1.0"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016">
  <Id>urn:uuid:ov-cpl</Id>
  <ContentTitle>OV</ContentTitle>
  <ContentKind>feature</ContentKind>
  <EditRate>24 1</EditRate>
  <SegmentList><Segment>
    <Id>urn:uuid:seg</Id>
    <SequenceList>
      <cc:MainImageSequence>
        <EditRate>24 1</EditRate>
        <ResourceList><Resource>
          <TrackFileId>urn:uuid:ov-video</TrackFileId>
          <IntrinsicDuration>240</IntrinsicDuration>
          <SourceDuration>240</SourceDuration>
        </Resource></ResourceList>
      </cc:MainImageSequence>
      <cc:MainAudioSequence>
        <EditRate>24 1</EditRate>
        <ResourceList><Resource>
          <TrackFileId>urn:uuid:ov-audio</TrackFileId>
          <IntrinsicDuration>240</IntrinsicDuration>
          <SourceDuration>240</SourceDuration>
        </Resource></ResourceList>
      </cc:MainAudioSequence>
    </SequenceList>
  </Segment></SegmentList>
</CompositionPlaylist>"#
    }

    fn write_ov(dir: &Path) {
        std::fs::write(dir.join("CPL_ov.xml"), ov_cpl_xml()).unwrap();
        std::fs::write(
            dir.join("ASSETMAP.xml"),
            r#"<?xml version="1.0"?><AssetMap><AssetList>
              <Asset><Id>urn:uuid:ov-cpl</Id><ChunkList><Chunk><Path>CPL_ov.xml</Path></Chunk></ChunkList></Asset>
            </AssetList></AssetMap>"#,
        )
        .unwrap();
    }

    #[test]
    fn parse_ov_reads_edit_rate_and_resources() {
        let dir = tempfile::tempdir().unwrap();
        write_ov(dir.path());
        let ov = parse_cpl_resources(&dir.path().join("CPL_ov.xml")).unwrap();
        assert_eq!((ov.fps_num, ov.fps_den), (24, 1));
        assert_eq!(ov.content_kind, "feature");
        assert_eq!(ov.resources.len(), 2);
        assert_eq!(ov.resources[0].kind, ImfTrackKind::Image);
        assert_eq!(ov.resources[0].uuid, "ov-video");
        assert_eq!(ov.resources[1].uuid, "ov-audio");
    }

    /// The OV as imfwizard itself writes it: an HLG picture, so the CPL claims the
    /// 2020 edition and carries an RGBADescriptor the image resource names.
    fn write_hlg_ov(dir: &Path) {
        let hdr = crate::hdr_wcg::HdrWcg::from_flags("hlg-bt2020", None).unwrap();
        let comp = crate::imp::Composition {
            title: "OV".into(),
            content_kind: "feature".into(),
            hdr: Some(hdr.clone()),
            ..Default::default()
        };
        let track_files = [
            crate::mxf_wrap::wrapped_picture(dir, Some(&hdr)),
            MxfTrackFile {
                uuid: "ov-audio".into(),
                ..crate::mxf_wrap::wrapped_sound(dir, "ov-audio", 2, None)
            },
        ];
        let opts = crate::imp::ImpOptions {
            fps_num: 24,
            fps_den: 1,
            ..Default::default()
        };
        crate::cpl::write_cpl(
            &dir.join("CPL_ov.xml"),
            "ov-cpl",
            &opts,
            &comp,
            &track_files,
        )
        .unwrap();
        std::fs::write(
            dir.join("ASSETMAP.xml"),
            r#"<?xml version="1.0"?><AssetMap><AssetList>
              <Asset><Id>urn:uuid:ov-cpl</Id><ChunkList><Chunk><Path>CPL_ov.xml</Path></Chunk></ChunkList></Asset>
            </AssetList></AssetMap>"#,
        )
        .unwrap();
    }

    #[test]
    fn a_supplement_keeps_the_ov_edition_and_picture_descriptor() {
        let dir = tempfile::tempdir().unwrap();
        write_hlg_ov(dir.path());
        let out = tempfile::tempdir().unwrap();
        let wav = out.path().join("dub.wav");
        std::fs::write(&wav, make_wav(2, 48000, 16, 48000)).unwrap();

        let result = create_supplement(&SupplementOptions {
            ov_dir: dir.path().to_path_buf(),
            title: "French dub".into(),
            output_dir: out.path().to_path_buf(),
            replace: vec![format!("{}@audio", wav.display())],
            add: vec![],
        });
        assert!(result.success, "supplement failed: {}", result.error);

        let cpl = read_one(out.path(), "CPL_");
        assert!(
            cpl.contains(&format!(
                "<cc:ApplicationIdentification>{}</cc:ApplicationIdentification>",
                App2eEdition::Edition2020.namespace()
            )),
            "the supplement dropped the OV's 2020 identification: {cpl}"
        );
        assert!(
            cpl.contains(
                "<r1:TransferCharacteristic>urn:smpte:ul:060e2b34.0401010d.04010101.010b0000"
            ),
            "the supplement dropped the OV's HLG descriptor: {cpl}"
        );

        // the picture resource still names the descriptor it kept
        let descriptor_id = {
            let start = cpl.find("<EssenceDescriptor>").expect("a descriptor");
            let id_start = cpl[start..].find("<Id>urn:uuid:").expect("a descriptor id") + start;
            let id_end = cpl[id_start..].find("</Id>").expect("a closed id") + id_start;
            cpl[id_start + "<Id>urn:uuid:".len()..id_end].to_string()
        };
        assert!(
            cpl.contains(&format!("<SourceEncoding>urn:uuid:{descriptor_id}<")),
            "no resource names the descriptor {descriptor_id}: {cpl}"
        );
    }

    /// A supplemental CPL's replaced and added resources name a descriptor read
    /// back off the track file the supplement wrapped, so the CPL passes the ST
    /// 2067-3 XSD, which makes SourceEncoding mandatory on a TrackFileResource.
    #[test]
    fn a_supplement_names_the_descriptors_of_what_it_wrapped() {
        let dir = tempfile::tempdir().unwrap();
        write_hlg_ov(dir.path());
        let out = tempfile::tempdir().unwrap();
        let dub = out.path().join("dub.wav");
        std::fs::write(&dub, make_wav(2, 48000, 16, 48000)).unwrap();
        let added = out.path().join("added.wav");
        std::fs::write(&added, make_wav(2, 48000, 16, 48000)).unwrap();

        let result = create_supplement(&SupplementOptions {
            ov_dir: dir.path().to_path_buf(),
            title: "French dub".into(),
            output_dir: out.path().to_path_buf(),
            replace: vec![format!("{}@audio", dub.display())],
            add: vec![format!("{}@audio", added.display())],
        });
        assert!(result.success, "supplement failed: {}", result.error);

        let cpl = read_one(out.path(), "CPL_");
        // the OV picture's descriptor plus one for each sound file wrapped here
        assert_eq!(
            cpl.matches("<EssenceDescriptor>").count(),
            3,
            "every resource but the OV's own sound needs a descriptor: {cpl}"
        );
        assert_eq!(
            cpl.matches("<SourceEncoding>").count(),
            3,
            "every resource but the OV's own sound must name one: {cpl}"
        );
        assert_eq!(
            cpl.matches("<r0:WAVEPCMDescriptor").count(),
            2,
            "both wrapped sound files must carry their own descriptor: {cpl}"
        );

        let complaint = crate::cpl::st2067_3_complaint(&cpl);
        assert!(
            complaint.is_empty(),
            "supplemental CPL must pass ST 2067-3 XSD:\n{complaint}"
        );
    }

    #[test]
    fn spec_parses_track_and_index() {
        let s = parse_spec("/x/y.wav@audio:2").unwrap();
        assert_eq!(s.kind, ImfTrackKind::Audio);
        assert_eq!(s.index, 2);
        assert_eq!(parse_spec("/v@video").unwrap().index, 0);
        assert!(parse_spec("no-at-sign").is_err());
        assert!(parse_spec("/x@bogus").is_err());
    }

    #[test]
    fn fails_loud_when_nothing_changes() {
        let dir = tempfile::tempdir().unwrap();
        write_ov(dir.path());
        let out = tempfile::tempdir().unwrap();
        let r = create_supplement(&SupplementOptions {
            ov_dir: dir.path().to_path_buf(),
            title: "Supp".into(),
            output_dir: out.path().to_path_buf(),
            replace: vec![],
            add: vec![],
        });
        assert!(!r.success);
        assert!(r.error.contains("nothing to change"));
    }

    #[test]
    fn fails_loud_on_missing_ov_track() {
        let dir = tempfile::tempdir().unwrap();
        write_ov(dir.path());
        let out = tempfile::tempdir().unwrap();
        let wav = out.path().join("dub.wav");
        std::fs::write(&wav, b"x").unwrap();
        let r = create_supplement(&SupplementOptions {
            ov_dir: dir.path().to_path_buf(),
            title: "Supp".into(),
            output_dir: out.path().to_path_buf(),
            replace: vec![format!("{}@audio:3", wav.display())],
            add: vec![],
        });
        assert!(!r.success);
        assert!(
            r.error.contains("no Audio track at index 3"),
            "got: {}",
            r.error
        );
    }

    /// A real supplemental: replace the OV audio with a new WAV. The CPL must
    /// reference the OV's unchanged video UUID and the new audio UUID, while the
    /// ASSETMAP lists only the present (new) track file. Guards against the old
    /// bug where supplement built a standalone IMP duplicating every OV asset.
    #[test]
    fn supplemental_references_ov_and_lists_only_present() {
        let dir = tempfile::tempdir().unwrap();
        write_ov(dir.path());
        let out = tempfile::tempdir().unwrap();
        let wav = out.path().join("dub.wav");
        std::fs::write(&wav, make_wav(2, 48000, 16, 48000)).unwrap();

        let r = create_supplement(&SupplementOptions {
            ov_dir: dir.path().to_path_buf(),
            title: "French dub".into(),
            output_dir: out.path().to_path_buf(),
            replace: vec![format!("{}@audio", wav.display())],
            add: vec![],
        });
        assert!(r.success, "supplement failed: {}", r.error);

        let cpl_xml = read_one(out.path(), "CPL_");
        let am_xml = std::fs::read_to_string(out.path().join("ASSETMAP.xml")).unwrap();

        // CPL still references the unchanged OV video by its UUID (present in OV, not here)
        assert!(
            cpl_xml.contains("urn:uuid:ov-video"),
            "CPL must reference OV video"
        );
        // the OV audio was replaced, so the old audio UUID must be gone
        assert!(
            !cpl_xml.contains("urn:uuid:ov-audio"),
            "replaced audio must not remain"
        );

        // exactly one MXF present
        let mxfs: Vec<_> = std::fs::read_dir(out.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "mxf"))
            .collect();
        assert_eq!(mxfs.len(), 1, "only the new track file should be present");

        // the new track-file UUID (asdcplib-assigned) is the ASSETMAP asset whose path is the mxf;
        // the CPL's replaced audio resource must reference exactly that UUID
        let new_uuid = assetmap_mxf_id(&am_xml);
        assert!(
            cpl_xml.contains(&new_uuid),
            "CPL must reference the new audio UUID"
        );
        assert!(
            !am_xml.contains("ov-video"),
            "ASSETMAP must not list absent OV assets"
        );
        assert!(
            !am_xml.contains("ov-audio"),
            "ASSETMAP must not list absent OV assets"
        );
    }

    /// Pull the UUID of the ASSETMAP asset whose chunk path is an .mxf file.
    fn assetmap_mxf_id(am_xml: &str) -> String {
        for block in am_xml.split("<Asset>").skip(1) {
            if block.contains(".mxf") {
                let id = block
                    .split("<Id>")
                    .nth(1)
                    .unwrap()
                    .split("</Id>")
                    .next()
                    .unwrap();
                return strip_urn(id.trim());
            }
        }
        panic!("no mxf asset in ASSETMAP");
    }

    fn read_one(dir: &Path, prefix: &str) -> String {
        let entry = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().starts_with(prefix))
            .expect("file with prefix");
        std::fs::read_to_string(entry.path()).unwrap()
    }

    fn make_wav(channels: u16, sample_rate: u32, bits: u16, sample_frames: u32) -> Vec<u8> {
        let block_align = (bits / 8) as u32 * channels as u32;
        let data_len = block_align * sample_frames;
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data_len).to_le_bytes());
        w.extend_from_slice(b"WAVE");
        w.extend_from_slice(b"fmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&channels.to_le_bytes());
        w.extend_from_slice(&sample_rate.to_le_bytes());
        w.extend_from_slice(&(sample_rate * block_align).to_le_bytes());
        w.extend_from_slice(&(block_align as u16).to_le_bytes());
        w.extend_from_slice(&bits.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&data_len.to_le_bytes());
        w.resize(w.len() + data_len as usize, 0);
        w
    }
}
