pub use postkit::mca::*;

use std::path::{Path, PathBuf};

// what an AS-02 sound wrap takes for each layout `mca --layout` names, with the
// channel count the track file has to carry. asdcplib's AS02_MCAConfigParser
// spells the stereo group ST, and takes one soundfield group per track file, so
// there is no 5.1+HI+VI spelling: an accessibility track is its own track file.
const LAYOUTS: [(&str, u32, &str); 6] = [
    ("mono", 1, "DNS(NSC001)"),
    ("10", 1, "DNS(NSC001)"),
    ("stereo", 2, "ST(L,R)"),
    ("20", 2, "ST(L,R)"),
    ("51", 6, "51(L,R,C,LFE,Ls,Rs)"),
    ("71", 8, "71(L,R,C,LFE,Lss,Rss,Lrs,Rrs)"),
];

// PCM is clip-wrapped, so the file records no wrap edit rate and the reader
// takes one only to size its frames. One second a frame keeps the copy buffer
// under a megabyte a channel.
const COPY_FRAMES_PER_SECOND: i32 = 1;

// the header size postkit's own wrap reserves
const HEADER_SIZE: u32 = 16384;

const MCA_TITLE_VERSION: &str = "Original Version";
const MCA_AUDIO_CONTENT_KIND: &str = "PRM";
const MCA_AUDIO_ELEMENT_KIND: &str = "FCMP";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub name: &'static str,
    pub channels: u32,
    pub labels: &'static str,
}

pub fn layout(name: &str) -> Option<Layout> {
    LAYOUTS
        .iter()
        .find(|(spelling, _, _)| spelling.eq_ignore_ascii_case(name))
        .map(|&(name, channels, labels)| Layout {
            name,
            channels,
            labels,
        })
}

pub fn layout_names() -> String {
    LAYOUTS
        .iter()
        .map(|(name, _, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

// what writing the labels changed
#[derive(Debug, Clone)]
pub struct WrittenLabels {
    pub layout: Layout,
    pub language: String,
    /// The package documents rewritten so they still describe the track file.
    /// Empty when the MXF sits outside a package.
    pub package_documents: Vec<PathBuf>,
}

/// Write the MCA labels of `layout` into the sound MXF's descriptor, rewrapping
/// its PCM into a new file and replacing the input. The track file keeps its
/// asset id, and any package around it is rewritten to match the new descriptor.
pub fn write_mca_labels(
    mxf: &Path,
    layout: Layout,
    language: &str,
) -> Result<WrittenLabels, String> {
    crate::imp::validate_language(language)?;

    let existing = read_labels(mxf)?;
    let channels = sound_channel_count(mxf)?;
    if channels != layout.channels {
        return Err(format!(
            "{} carries {channels} channels and --layout {} names {}",
            mxf.display(),
            layout.name,
            layout.channels
        ));
    }

    let config = postkit::mxf_wrap::McaConfig {
        labels: layout.labels.to_string(),
        spoken_language: Some(language.to_string()),
        soundfield_group: Some(soundfield_group(mxf, existing.as_ref())),
    };
    rewrap_with_labels(mxf, &config)?;

    Ok(WrittenLabels {
        layout,
        language: language.to_string(),
        package_documents: refresh_package(mxf)?,
    })
}

fn open_sound(mxf: &Path) -> Result<asdcplib::as02::pcm::MxfReader, String> {
    let mut reader = asdcplib::as02::pcm::MxfReader::new();
    reader
        .open_read(
            &mxf.to_string_lossy(),
            asdcplib::Rational::new(COPY_FRAMES_PER_SECOND, 1),
        )
        .map_err(|error| format!("cannot read the sound MXF {}: {error}", mxf.display()))?;
    Ok(reader)
}

fn sound_channel_count(mxf: &Path) -> Result<u32, String> {
    Ok(open_sound(mxf)?
        .audio_descriptor()
        .map_err(|error| format!("cannot read the descriptor of {}: {error}", mxf.display()))?
        .channel_count)
}

// the soundfield group label the track file already carries, None when it has none
fn read_labels(mxf: &Path) -> Result<Option<asdcplib::pcm::McaLabelSubDescriptor>, String> {
    let labels = open_sound(mxf)?
        .mca_label_subdescriptors()
        .map_err(|error| format!("cannot read the MCA labels of {}: {error}", mxf.display()))?;
    Ok(labels
        .into_iter()
        .find(|label| label.kind == asdcplib::pcm::McaLabelKind::SoundfieldGroup))
}

// ST 2067-2 wants all four non-empty, so what the track file already says about
// itself is kept and only a file with no labels yet takes the defaults
fn soundfield_group(
    mxf: &Path,
    existing: Option<&asdcplib::pcm::McaLabelSubDescriptor>,
) -> postkit::mxf_wrap::SoundfieldGroup {
    let carried = |read: fn(&asdcplib::pcm::McaLabelSubDescriptor) -> Option<&String>,
                   fallback: &str| {
        existing
            .and_then(read)
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| fallback.to_string())
    };
    let stem = mxf
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Untitled");
    postkit::mxf_wrap::SoundfieldGroup {
        title: carried(|label| label.title.as_ref(), &composition_title(mxf, stem)),
        title_version: carried(|label| label.title_version.as_ref(), MCA_TITLE_VERSION),
        audio_content_kind: carried(
            |label| label.audio_content_kind.as_ref(),
            MCA_AUDIO_CONTENT_KIND,
        ),
        audio_element_kind: carried(
            |label| label.audio_element_kind.as_ref(),
            MCA_AUDIO_ELEMENT_KIND,
        ),
    }
}

fn composition_title(mxf: &Path, fallback: &str) -> String {
    let Some(package) = mxf.parent() else {
        return fallback.to_string();
    };
    crate::timeline::list_cpls(package)
        .into_iter()
        .map(|cpl| cpl.title)
        .find(|title| !title.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

// Copy the clip into a new file under the same identity, with the labels on its
// descriptor. Written beside the input so the rename cannot cross a filesystem.
fn rewrap_with_labels(mxf: &Path, config: &postkit::mxf_wrap::McaConfig) -> Result<(), String> {
    let mut reader = open_sound(mxf)?;
    let descriptor = reader
        .audio_descriptor()
        .map_err(|error| format!("cannot read the descriptor of {}: {error}", mxf.display()))?;
    let info = reader
        .writer_info()
        .map_err(|error| format!("cannot read the identity of {}: {error}", mxf.display()))?;

    let sample_rate = descriptor.audio_sampling_rate.numerator as u32
        / descriptor.audio_sampling_rate.denominator.max(1) as u32;
    let sample_size = (descriptor.quantization_bits / 8) * descriptor.channel_count;
    let samples_per_frame = sample_rate.div_ceil(COPY_FRAMES_PER_SECOND as u32);
    let frames = descriptor
        .container_duration
        .div_ceil(samples_per_frame.max(1));
    if sample_size == 0 || frames == 0 {
        return Err(format!("{} carries no audio samples", mxf.display()));
    }

    let rewrapped = mxf.with_extension("mca-rewrap.mxf");
    let mut writer = asdcplib::as02::pcm::MxfWriter::new();
    let group = config
        .soundfield_group
        .as_ref()
        .ok_or("an AS-02 MCA wrap needs the soundfield group properties")?;
    writer
        .open_write_mca(
            &rewrapped.to_string_lossy(),
            &info,
            &descriptor,
            &config.labels,
            &asdcplib::as02::pcm::SoundfieldGroupProperties {
                language: config.spoken_language.as_deref().unwrap_or_default(),
                title: &group.title,
                title_version: &group.title_version,
                audio_content_kind: &group.audio_content_kind,
                audio_element_kind: &group.audio_element_kind,
            },
            HEADER_SIZE,
        )
        .map_err(|error| {
            let _ = std::fs::remove_file(&rewrapped);
            format!(
                "cannot write {} with the MCA labels {}: {error}",
                rewrapped.display(),
                config.labels
            )
        })?;

    let mut copy = || -> Result<(), String> {
        let mut frame = vec![0u8; (sample_size * samples_per_frame) as usize];
        for index in 0..frames {
            let read = reader
                .read_frame(index, &mut frame, None, None)
                .map_err(|error| format!("cannot read {} frame {index}: {error}", mxf.display()))?;
            writer
                .write_frame(&frame[..read], None, None)
                .map_err(|error| format!("cannot write {}: {error}", rewrapped.display()))?;
        }
        writer
            .finalize()
            .map_err(|error| format!("cannot finalize {}: {error}", rewrapped.display()))
    };
    if let Err(error) = copy() {
        let _ = std::fs::remove_file(&rewrapped);
        return Err(error);
    }
    let _ = reader.close();

    std::fs::rename(&rewrapped, mxf).map_err(|error| {
        let _ = std::fs::remove_file(&rewrapped);
        format!("cannot replace {}: {error}", mxf.display())
    })
}

/// Rewrite the CPL's essence descriptor for this track file and the PKL hashes
/// of both, so the package still describes what the rewrap wrote. Empty when the
/// MXF sits outside a package.
fn refresh_package(mxf: &Path) -> Result<Vec<PathBuf>, String> {
    let Some(package) = mxf.parent() else {
        return Ok(Vec::new());
    };
    let asset_id = track_file_id(mxf)?;
    let Some(pkl) = document_naming(package, "PKL_", &asset_id) else {
        return Ok(Vec::new());
    };

    // every file the rewrap changed, and the id the PKL lists it under
    let mut changed = vec![(asset_id.clone(), mxf.to_path_buf())];
    let mut rewritten = Vec::new();
    if let Some(cpl) = document_naming(package, "CPL_", &asset_id) {
        let xml = read(&cpl)?;
        let descriptor_id = source_encoding_of(&xml, &asset_id).ok_or_else(|| {
            format!(
                "{} names the track file but no essence descriptor for it",
                cpl.display()
            )
        })?;
        let body = sound_descriptor_regxml(mxf)?;
        let updated = replace_descriptor_body(&xml, &descriptor_id, &body).ok_or_else(|| {
            format!(
                "{} has no EssenceDescriptor {descriptor_id} to rewrite",
                cpl.display()
            )
        })?;
        write(&cpl, &updated)?;
        let id = cpl_id(&cpl)
            .ok_or_else(|| format!("{} is not named after its composition id", cpl.display()))?;
        changed.push((id, cpl.clone()));
        rewritten.push(cpl);
    }

    let mut pkl_xml = read(&pkl)?;
    for (asset, file) in &changed {
        pkl_xml = patch_pkl_asset(&pkl_xml, asset, file)?;
    }
    write(&pkl, &pkl_xml)?;
    rewritten.push(pkl);
    Ok(rewritten)
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))
}

fn write(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))
}

fn track_file_id(mxf: &Path) -> Result<String, String> {
    let info = open_sound(mxf)?
        .writer_info()
        .map_err(|error| format!("cannot read the identity of {}: {error}", mxf.display()))?;
    Ok(uuid::Uuid::from_bytes(info.asset_uuid)
        .hyphenated()
        .to_string())
}

// the package document whose name starts with `prefix` and whose text holds
// `needle`, so a PKL that lists another composition's assets is passed over
fn document_naming(package: &Path, prefix: &str, needle: &str) -> Option<PathBuf> {
    std::fs::read_dir(package)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(prefix))
        })
        .find(|path| std::fs::read_to_string(path).is_ok_and(|xml| xml.contains(needle)))
}

fn cpl_id(cpl: &Path) -> Option<String> {
    cpl.file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.strip_prefix("CPL_"))
        .map(str::to_string)
}

// the id of the essence descriptor the resource for `track_file_id` names
fn source_encoding_of(cpl: &str, track_file_id: &str) -> Option<String> {
    const RESOURCE: &str = "<Resource ";
    const RESOURCE_END: &str = "</Resource>";
    let mut rest = cpl;
    while let Some(start) = rest.find(RESOURCE) {
        let end = rest[start..].find(RESOURCE_END)? + start + RESOURCE_END.len();
        let resource = &rest[start..end];
        if resource.contains(track_file_id) {
            return between(resource, "<SourceEncoding>urn:uuid:", "</SourceEncoding>");
        }
        rest = &rest[end..];
    }
    None
}

// swap the body of the named EssenceDescriptor, keeping its own Id
fn replace_descriptor_body(cpl: &str, descriptor_id: &str, body: &str) -> Option<String> {
    const DESCRIPTOR: &str = "<EssenceDescriptor>";
    const DESCRIPTOR_END: &str = "</EssenceDescriptor>";
    let mut searched = 0;
    while let Some(start) = cpl[searched..].find(DESCRIPTOR).map(|at| at + searched) {
        let end = cpl[start..].find(DESCRIPTOR_END)? + start;
        let block = &cpl[start..end];
        if block.contains(descriptor_id) {
            let after_id = start + block.find("</Id>")? + "</Id>".len();
            return Some(format!("{}\n{body}      {}", &cpl[..after_id], &cpl[end..]));
        }
        searched = end + DESCRIPTOR_END.len();
    }
    None
}

// the sound MXF's own descriptor and MCA labels, as the CPL carries them. The
// items all come out of the file header, so the edit rate the reader is opened
// with does not reach the XML.
fn sound_descriptor_regxml(mxf: &Path) -> Result<String, String> {
    let mut reader = open_sound(mxf)?;
    let descriptor = reader
        .wave_audio_descriptor()
        .map_err(|error| format!("cannot read the descriptor of {}: {error}", mxf.display()))?;
    let labels = reader
        .mca_label_subdescriptors()
        .map_err(|error| format!("cannot read the MCA labels of {}: {error}", mxf.display()))?;
    Ok(postkit::regxml::sound_descriptor_regxml(
        &descriptor,
        &labels,
    ))
}

// the Hash and Size the PKL carries for one asset, taken off the file again
fn patch_pkl_asset(pkl: &str, asset_id: &str, file: &Path) -> Result<String, String> {
    const ASSET: &str = "<Asset>";
    const ASSET_END: &str = "</Asset>";
    let hash = postkit::hash::hash_file(file, postkit::hash::HashAlgorithm::Sha1)
        .map_err(|error| format!("cannot hash {}: {error}", file.display()))?
        .base64;
    let size = std::fs::metadata(file)
        .map_err(|error| format!("cannot stat {}: {error}", file.display()))?
        .len();

    let mut out = String::with_capacity(pkl.len());
    let mut rest = pkl;
    let mut patched = false;
    while let Some(start) = rest.find(ASSET) {
        let end = rest[start..]
            .find(ASSET_END)
            .map_or(rest.len(), |at| start + at + ASSET_END.len());
        out.push_str(&rest[..start]);
        let block = &rest[start..end];
        if block.contains(asset_id) {
            out.push_str(&replace_element(
                &replace_element(block, "Hash", &hash),
                "Size",
                &size.to_string(),
            ));
            patched = true;
        } else {
            out.push_str(block);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    if !patched {
        return Err(format!("the packing list lists no asset {asset_id}"));
    }
    Ok(out)
}

fn replace_element(xml: &str, element: &str, value: &str) -> String {
    let (open, close) = (format!("<{element}>"), format!("</{element}>"));
    match (xml.find(&open), xml.find(&close)) {
        (Some(start), Some(end)) if start < end => {
            format!("{}{open}{value}{}", &xml[..start], &xml[end..])
        }
        _ => xml.to_string(),
    }
}

fn between(text: &str, open: &str, close: &str) -> Option<String> {
    let start = text.find(open)? + open.len();
    let rest = &text[start..];
    Some(rest[..rest.find(close)?].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layout_names_the_labels_create_writes_for_that_channel_count() {
        for name in ["mono", "stereo", "51"] {
            let layout = layout(name).unwrap();
            assert_eq!(
                layout.labels,
                crate::imp::mca_labels(layout.channels as usize, None),
                "{name} does not label its channels the way create does"
            );
        }
    }

    #[test]
    fn an_unknown_layout_is_not_a_layout() {
        assert_eq!(layout("51+HI+VI"), None);
        assert_eq!(layout("quad"), None);
        assert_eq!(layout("STEREO").unwrap().channels, 2);
    }

    #[test]
    fn the_source_encoding_of_a_resource_is_read_by_its_track_file() {
        let cpl = "<Resource xsi:type=\"TrackFileResourceType\">\
             <SourceEncoding>urn:uuid:picture-descriptor</SourceEncoding>\
             <TrackFileId>urn:uuid:picture</TrackFileId></Resource>\
             <Resource xsi:type=\"TrackFileResourceType\">\
             <SourceEncoding>urn:uuid:sound-descriptor</SourceEncoding>\
             <TrackFileId>urn:uuid:sound</TrackFileId></Resource>";
        assert_eq!(
            source_encoding_of(cpl, "sound").as_deref(),
            Some("sound-descriptor")
        );
        assert_eq!(source_encoding_of(cpl, "subtitle"), None);
    }

    #[test]
    fn only_the_named_descriptor_body_is_swapped() {
        let cpl = "<EssenceDescriptorList>\
             <EssenceDescriptor><Id>urn:uuid:one</Id>\n      <old/>\n      </EssenceDescriptor>\
             <EssenceDescriptor><Id>urn:uuid:two</Id>\n      <keep/>\n      </EssenceDescriptor>\
             </EssenceDescriptorList>";
        let updated = replace_descriptor_body(cpl, "one", "<new/>\n").unwrap();
        assert!(updated.contains("<new/>"), "{updated}");
        assert!(!updated.contains("<old/>"), "{updated}");
        assert!(updated.contains("<keep/>"), "{updated}");
        assert_eq!(replace_descriptor_body(cpl, "three", "<new/>"), None);
    }
}
