//! Dolby Atmos ADM BWF import.
//!
//! Extracts ADM (Audio Definition Model) XML from BWF RIFF "axml" chunks, parses bed channels
//! and audio objects, wraps the PCM essence to an MXF via asdcplib, and writes an ADM sidecar XML.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;

use quick_xml::events::Event;
use quick_xml::name::QName;
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};

/// A bed channel (DirectSpeakers type).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BedChannel {
    pub label: String,
    pub speaker_label: String,
    pub track_index: u32,
}

/// A dynamic audio object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioObject {
    pub id: String,
    pub name: String,
    pub track_index: u32,
    pub azimuth: f32,
    pub elevation: f32,
    pub distance: f32,
}

/// Parsed ADM metadata from a BWF file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmMetadata {
    pub programme_name: String,
    pub beds: Vec<BedChannel>,
    pub objects: Vec<AudioObject>,
    pub total_channels: u32,
}

/// Result of an Atmos import operation.
#[derive(Debug)]
pub struct AtmosImportResult {
    pub success: bool,
    pub bed_count: usize,
    pub object_count: usize,
    pub total_channels: u32,
    pub mxf_output: PathBuf,
    pub adm_sidecar: PathBuf,
    pub error: String,
}

/// The form types an ADM master arrives in: plain RIFF, and the two 64-bit forms
/// BS.2088 defines, whose sizes live in a `ds64` chunk.
const WAVE_FORM_TYPES: [&[u8; 4]; 3] = [b"RIFF", b"RF64", b"BW64"];

/// What a 32-bit RIFF size field holds when the real size is in the `ds64` chunk.
const SIZE_IS_IN_DS64: u32 = 0xFFFF_FFFF;

/// Form type, size and `WAVE` together.
const RIFF_HEADER_BYTES: u64 = 12;

/// A chunk's FourCC and its 32-bit size.
const CHUNK_HEADER_BYTES: u64 = 8;

/// riffSize, dataSize and sampleCount, then the table length.
const DS64_FIXED_BYTES: usize = 28;

/// A FourCC and a 64-bit size.
const DS64_TABLE_ENTRY_BYTES: usize = 12;

/// The 64-bit sizes a `ds64` chunk carries (ITU-R BS.2088): the data chunk's, plus
/// one table entry for every other chunk whose 32-bit size field is 0xFFFFFFFF.
struct Ds64Sizes {
    data_size: u64,
    table: Vec<([u8; 4], u64)>,
}

impl Ds64Sizes {
    fn parse(payload: &[u8]) -> Result<Self, String> {
        if payload.len() < DS64_FIXED_BYTES {
            return Err(format!(
                "ds64 chunk is {} bytes, too short to hold the 64-bit sizes",
                payload.len()
            ));
        }
        let data_size = u64::from_le_bytes(payload[8..16].try_into().unwrap());
        let entries = u32::from_le_bytes(payload[24..28].try_into().unwrap()) as usize;
        let wanted = DS64_FIXED_BYTES + entries * DS64_TABLE_ENTRY_BYTES;
        if payload.len() < wanted {
            return Err(format!(
                "ds64 chunk declares {entries} sizes but is only {} bytes",
                payload.len()
            ));
        }
        let table = payload[DS64_FIXED_BYTES..wanted]
            .as_chunks::<DS64_TABLE_ENTRY_BYTES>()
            .0
            .iter()
            .map(|entry| {
                (
                    entry[..4].try_into().unwrap(),
                    u64::from_le_bytes(entry[4..].try_into().unwrap()),
                )
            })
            .collect();
        Ok(Self { data_size, table })
    }

    fn size_of(&self, chunk_id: &[u8; 4]) -> Option<u64> {
        if chunk_id == b"data" {
            return Some(self.data_size);
        }
        self.table
            .iter()
            .find_map(|(id, size)| (id == chunk_id).then_some(*size))
    }
}

fn chunk_name(chunk_id: &[u8; 4]) -> String {
    String::from_utf8_lossy(chunk_id).into_owned()
}

/// Extract the "axml" chunk from a BWF file.
///
/// ADM masters are RIFF containers, usually the 64-bit BW64 or RF64 form, so a chunk
/// size of 0xFFFFFFFF is read from the `ds64` chunk instead.
pub fn extract_axml_chunk(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|e| format!("Cannot open BWF {}: {e}", path.display()))?;
    let file_length = file
        .metadata()
        .map_err(|e| format!("Cannot read BWF size: {e}"))?
        .len();

    let mut header = [0u8; RIFF_HEADER_BYTES as usize];
    file.read_exact(&mut header)
        .map_err(|e| format!("Read error: {e}"))?;
    let form_type: [u8; 4] = header[..4].try_into().unwrap();
    if !WAVE_FORM_TYPES.contains(&&form_type) {
        return Err(format!(
            "Not a RIFF, RF64 or BW64 file: form type is {}",
            chunk_name(&form_type)
        ));
    }
    if &header[8..12] != b"WAVE" {
        return Err("Not a WAVE file".into());
    }

    let mut ds64: Option<Ds64Sizes> = None;
    let mut position = RIFF_HEADER_BYTES;
    while position + CHUNK_HEADER_BYTES <= file_length {
        let mut chunk_header = [0u8; CHUNK_HEADER_BYTES as usize];
        file.read_exact(&mut chunk_header)
            .map_err(|e| format!("Read error: {e}"))?;
        position += CHUNK_HEADER_BYTES;
        let chunk_id: [u8; 4] = chunk_header[..4].try_into().unwrap();
        let declared_size = u32::from_le_bytes(chunk_header[4..].try_into().unwrap());

        let chunk_size = match declared_size {
            SIZE_IS_IN_DS64 => {
                let sizes = ds64.as_ref().ok_or_else(|| {
                    format!(
                        "{} chunk takes its size from a ds64 chunk this file does not have",
                        chunk_name(&chunk_id)
                    )
                })?;
                sizes.size_of(&chunk_id).ok_or_else(|| {
                    format!(
                        "ds64 chunk carries no size for the {} chunk",
                        chunk_name(&chunk_id)
                    )
                })?
            }
            size => size as u64,
        };
        if position
            .checked_add(chunk_size)
            .is_none_or(|end| end > file_length)
        {
            return Err(format!(
                "{} chunk claims {chunk_size} bytes at offset {position}, past the end of the {file_length} byte file",
                chunk_name(&chunk_id)
            ));
        }

        if &chunk_id == b"axml" {
            let mut xml_buf = vec![0u8; chunk_size as usize];
            file.read_exact(&mut xml_buf)
                .map_err(|e| format!("Failed to read axml chunk: {e}"))?;
            return Ok(String::from_utf8_lossy(&xml_buf).to_string());
        }
        if &chunk_id == b"ds64" {
            let mut payload = vec![0u8; chunk_size as usize];
            file.read_exact(&mut payload)
                .map_err(|e| format!("Failed to read ds64 chunk: {e}"))?;
            ds64 = Some(Ds64Sizes::parse(&payload)?);
        }

        position += chunk_size + chunk_size % 2;
        file.seek(SeekFrom::Start(position))
            .map_err(|e| format!("Seek error: {e}"))?;
    }

    Err("No axml chunk found in BWF file".into())
}

fn local_name(qname: QName) -> String {
    String::from_utf8_lossy(qname.local_name().as_ref()).into_owned()
}

/// Read a named attribute off a start/empty element.
fn attr_value(e: &quick_xml::events::BytesStart, key: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        (local_name(a.key) == key).then(|| String::from_utf8_lossy(&a.value).into_owned())
    })
}

/// Fields accumulated while inside one `<audioChannelFormat>` element.
#[derive(Default)]
struct ChannelFormatAcc {
    id: String,
    name: String,
    type_label: String,
    speaker_label: String,
    azimuth: Option<f32>,
    elevation: Option<f32>,
    distance: Option<f32>,
}

/// Parse ADM XML into structured metadata.
pub fn parse_adm_xml(xml: &str) -> AdmMetadata {
    let mut adm = AdmMetadata {
        programme_name: String::new(),
        beds: Vec::new(),
        objects: Vec::new(),
        total_channels: 0,
    };
    if xml.is_empty() {
        return adm;
    }

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut track_idx: u32 = 0;
    let mut acc: Option<ChannelFormatAcc> = None;
    let mut cur = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if adm.programme_name.is_empty()
                    && let Some(v) = attr_value(&e, "audioProgrammeName")
                {
                    adm.programme_name = v;
                }
                let name = local_name(e.name());
                if name == "audioChannelFormat" {
                    acc = Some(ChannelFormatAcc {
                        id: attr_value(&e, "audioChannelFormatID").unwrap_or_default(),
                        name: attr_value(&e, "audioChannelFormatName").unwrap_or_default(),
                        type_label: attr_value(&e, "typeLabel").unwrap_or_default(),
                        ..Default::default()
                    });
                }
                cur = name;
            }
            Ok(Event::Text(t)) => {
                if let Some(a) = acc.as_mut() {
                    let text = t.unescape().unwrap_or_default().trim().to_string();
                    if text.is_empty() {
                        continue;
                    }
                    match cur.as_str() {
                        "speakerLabel" if a.speaker_label.is_empty() => a.speaker_label = text,
                        "azimuth" if a.azimuth.is_none() => a.azimuth = text.parse().ok(),
                        "elevation" if a.elevation.is_none() => a.elevation = text.parse().ok(),
                        "distance" if a.distance.is_none() => a.distance = text.parse().ok(),
                        _ => {}
                    }
                }
            }
            Ok(Event::End(e)) => {
                cur.clear();
                if local_name(e.name()) == "audioChannelFormat"
                    && let Some(a) = acc.take()
                {
                    match a.type_label.as_str() {
                        "0001" => adm.beds.push(BedChannel {
                            label: a.name,
                            speaker_label: a.speaker_label,
                            track_index: track_idx,
                        }),
                        "0003" => adm.objects.push(AudioObject {
                            id: a.id,
                            name: a.name,
                            track_index: track_idx,
                            azimuth: a.azimuth.unwrap_or(0.0),
                            elevation: a.elevation.unwrap_or(0.0),
                            distance: a.distance.unwrap_or(1.0),
                        }),
                        _ => {}
                    }
                    track_idx += 1;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    adm.total_channels = track_idx;
    adm
}

/// Import a Dolby Atmos ADM BWF file into an IMF-compatible MXF at the given edit rate.
pub fn import_atmos(
    input: &Path,
    output_dir: &Path,
    edit_rate_num: u32,
    edit_rate_den: u32,
) -> AtmosImportResult {
    let mxf_output = output_dir.join("atmos.mxf");
    let adm_sidecar = output_dir.join("adm_metadata.xml");

    // Extract ADM XML
    let xml = match extract_axml_chunk(input) {
        Ok(xml) => xml,
        Err(e) => {
            return AtmosImportResult {
                success: false,
                bed_count: 0,
                object_count: 0,
                total_channels: 0,
                mxf_output,
                adm_sidecar,
                error: format!("Failed to extract ADM XML: {e}"),
            };
        }
    };

    // Parse ADM
    let adm = parse_adm_xml(&xml);
    tracing::info!(
        "ADM: {} beds, {} objects, {} total channels",
        adm.beds.len(),
        adm.objects.len(),
        adm.total_channels
    );

    // Create output directory
    if let Err(e) = std::fs::create_dir_all(output_dir) {
        return AtmosImportResult {
            success: false,
            bed_count: adm.beds.len(),
            object_count: adm.objects.len(),
            total_channels: adm.total_channels,
            mxf_output,
            adm_sidecar,
            error: format!("Failed to create output dir: {e}"),
        };
    }

    // Write ADM sidecar XML
    if let Err(e) = std::fs::write(&adm_sidecar, &xml) {
        return AtmosImportResult {
            success: false,
            bed_count: adm.beds.len(),
            object_count: adm.objects.len(),
            total_channels: adm.total_channels,
            mxf_output,
            adm_sidecar,
            error: format!("Failed to write ADM sidecar: {e}"),
        };
    }

    // ffmpeg decodes the BWF PCM essence to a single multichannel WAV, then asdcplib (via
    // postkit's AS-02 writer) wraps it into an MXF. asdcplib has no AS-02 IAB/Atmos writer, so
    // the immersive channels are carried as PCM rather than re-encoded to a Dolby IAB bitstream.
    let combined = output_dir.join("atmos_pcm.wav");
    let extract = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            &input.to_string_lossy(),
            "-map",
            "0:a",
            "-c:a",
            "pcm_s24le",
            &combined.to_string_lossy(),
        ])
        .output();
    match extract {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            return AtmosImportResult {
                success: false,
                bed_count: adm.beds.len(),
                object_count: adm.objects.len(),
                total_channels: adm.total_channels,
                mxf_output,
                adm_sidecar,
                error: format!(
                    "ffmpeg WAV extract failed: {}",
                    String::from_utf8_lossy(&o.stderr)
                        .chars()
                        .take(200)
                        .collect::<String>()
                ),
            };
        }
        Err(e) => {
            return AtmosImportResult {
                success: false,
                bed_count: adm.beds.len(),
                object_count: adm.objects.len(),
                total_channels: adm.total_channels,
                mxf_output,
                adm_sidecar,
                error: format!("ffmpeg not found: {e}"),
            };
        }
    }

    // the PCM wrap counts its own frames from the WAV's data length and the edit rate,
    // so duration is not read on this path
    let wrap = crate::mxf_wrap::wrap_mxf(&crate::mxf_wrap::MxfWrapOptions {
        input_dir: combined.clone(),
        output_file: mxf_output.clone(),
        essence_type: crate::EssenceType::Wav,
        edit_rate_num,
        edit_rate_den,
        duration: 0,
        hdr: None,
        mca: None,
        asset_uuid: None,
    });
    let _ = std::fs::remove_file(&combined);

    AtmosImportResult {
        success: wrap.success,
        bed_count: adm.beds.len(),
        object_count: adm.objects.len(),
        total_channels: adm.total_channels,
        mxf_output,
        adm_sidecar,
        error: wrap.error,
    }
}

/// The RIFF forms an ADM master is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BwfForm {
    /// 32-bit sizes throughout.
    Riff,
    /// BS.2088, every size taken from the ds64 chunk.
    Bw64,
}

/// The channels [`synthetic_adm_xml`] describes: a 7.1.2 bed and two objects.
pub const SYNTHETIC_ADM_CHANNELS: u16 = 12;

/// The sample rate and depth [`synthetic_adm_bwf`] writes, what an Atmos master carries.
pub const SYNTHETIC_ADM_SAMPLE_RATE: u32 = 48_000;
pub const SYNTHETIC_ADM_BITS: u16 = 24;

/// The constant sample the fixture writes on one channel, distinct per channel so the
/// wrapped essence can be checked against it.
pub fn synthetic_adm_sample(channel_index: u16) -> i32 {
    (channel_index as i32 + 1) * 4096
}

/// An ADM document for a 7.1.2 bed and two objects. Public so the import tests can write
/// a master and compare the chunk that comes back.
pub fn synthetic_adm_xml() -> String {
    const BED_SPEAKERS: [&str; 10] = [
        "RC_L", "RC_R", "RC_C", "RC_LFE", "RC_Lss", "RC_Rss", "RC_Lrs", "RC_Rrs", "RC_Ltf",
        "RC_Rtf",
    ];
    let mut xml = String::from(
        "<?xml version=\"1.0\"?>\n<audioFormatExtended>\n  \
         <audioProgramme audioProgrammeName=\"Main Mix\"/>\n",
    );
    for (index, speaker) in BED_SPEAKERS.iter().enumerate() {
        let id = index + 1;
        xml.push_str(&format!(
            "  <audioChannelFormat audioChannelFormatID=\"AC_0001{id:04x}\" \
             audioChannelFormatName=\"{speaker}\" typeLabel=\"0001\">\n    \
             <audioBlockFormat><speakerLabel>{speaker}</speakerLabel></audioBlockFormat>\n  \
             </audioChannelFormat>\n"
        ));
    }
    for object in 1..=2u32 {
        let azimuth = 30 * (object as i32 * 2 - 3);
        xml.push_str(&format!(
            "  <audioChannelFormat audioChannelFormatID=\"AC_0003{object:04x}\" \
             audioChannelFormatName=\"Object{object}\" typeLabel=\"0003\">\n    \
             <audioBlockFormat><azimuth>{azimuth}</azimuth><elevation>10</elevation>\
             <distance>0.5</distance></audioBlockFormat>\n  </audioChannelFormat>\n"
        ));
    }
    xml.push_str("</audioFormatExtended>\n");
    xml
}

fn push_chunk(out: &mut Vec<u8>, chunk_id: &[u8; 4], payload: &[u8], declared_size: u32) {
    out.extend_from_slice(chunk_id);
    out.extend_from_slice(&declared_size.to_le_bytes());
    out.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        out.push(0);
    }
}

/// A BWF carrying [`synthetic_adm_xml`] and `sample_frames` of PCM, in either RIFF form.
/// Public so the CLI import test can write a master too.
pub fn synthetic_adm_bwf(form: BwfForm, sample_frames: u32) -> Vec<u8> {
    let xml = synthetic_adm_xml();
    let bytes_per_sample = (SYNTHETIC_ADM_BITS / 8) as usize;
    let block_align = SYNTHETIC_ADM_CHANNELS as usize * bytes_per_sample;

    let mut data = Vec::with_capacity(block_align * sample_frames as usize);
    for _ in 0..sample_frames {
        for channel in 0..SYNTHETIC_ADM_CHANNELS {
            data.extend_from_slice(
                &synthetic_adm_sample(channel).to_le_bytes()[..bytes_per_sample],
            );
        }
    }

    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_le_bytes()); // WAVE_FORMAT_PCM
    fmt.extend_from_slice(&SYNTHETIC_ADM_CHANNELS.to_le_bytes());
    fmt.extend_from_slice(&SYNTHETIC_ADM_SAMPLE_RATE.to_le_bytes());
    fmt.extend_from_slice(&(SYNTHETIC_ADM_SAMPLE_RATE * block_align as u32).to_le_bytes());
    fmt.extend_from_slice(&(block_align as u16).to_le_bytes());
    fmt.extend_from_slice(&SYNTHETIC_ADM_BITS.to_le_bytes());

    let mut out = Vec::new();
    let form_type: &[u8; 4] = match form {
        BwfForm::Riff => b"RIFF",
        BwfForm::Bw64 => b"BW64",
    };
    out.extend_from_slice(form_type);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(b"WAVE");

    let mut riff_size_offset = 4;
    match form {
        BwfForm::Riff => {
            push_chunk(&mut out, b"fmt ", &fmt, fmt.len() as u32);
            push_chunk(&mut out, b"data", &data, data.len() as u32);
            push_chunk(&mut out, b"axml", xml.as_bytes(), xml.len() as u32);
        }
        BwfForm::Bw64 => {
            out[4..8].copy_from_slice(&SIZE_IS_IN_DS64.to_le_bytes());
            let mut ds64 = Vec::new();
            ds64.extend_from_slice(&0u64.to_le_bytes()); // riffSize, filled in below
            ds64.extend_from_slice(&(data.len() as u64).to_le_bytes());
            ds64.extend_from_slice(&(sample_frames as u64).to_le_bytes());
            ds64.extend_from_slice(&1u32.to_le_bytes()); // one table entry
            ds64.extend_from_slice(b"axml");
            ds64.extend_from_slice(&(xml.len() as u64).to_le_bytes());
            riff_size_offset = out.len() + CHUNK_HEADER_BYTES as usize;
            push_chunk(&mut out, b"ds64", &ds64, ds64.len() as u32);
            push_chunk(&mut out, b"fmt ", &fmt, fmt.len() as u32);
            push_chunk(&mut out, b"data", &data, SIZE_IS_IN_DS64);
            push_chunk(&mut out, b"axml", xml.as_bytes(), SIZE_IS_IN_DS64);
        }
    }

    let riff_size = out.len() as u64 - CHUNK_HEADER_BYTES;
    match form {
        BwfForm::Riff => {
            out[riff_size_offset..riff_size_offset + 4]
                .copy_from_slice(&(riff_size as u32).to_le_bytes());
        }
        BwfForm::Bw64 => {
            out[riff_size_offset..riff_size_offset + 8].copy_from_slice(&riff_size.to_le_bytes());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_adm_extracts_beds_and_objects() {
        let xml = r#"<?xml version="1.0"?>
<audioFormatExtended>
  <audioProgramme audioProgrammeName="Main Mix"/>
  <audioChannelFormat audioChannelFormatID="AC_1" audioChannelFormatName="RoomCentricLeft" typeLabel="0001">
    <audioBlockFormat><speakerLabel>RC_L</speakerLabel></audioBlockFormat>
  </audioChannelFormat>
  <audioChannelFormat audioChannelFormatID="AC_9" audioChannelFormatName="Object1" typeLabel="0003">
    <audioBlockFormat><azimuth>30</azimuth><elevation>10</elevation><distance>0.5</distance></audioBlockFormat>
  </audioChannelFormat>
</audioFormatExtended>"#;
        let adm = parse_adm_xml(xml);
        assert_eq!(adm.programme_name, "Main Mix");
        assert_eq!(adm.total_channels, 2);
        assert_eq!(adm.beds.len(), 1);
        assert_eq!(adm.beds[0].speaker_label, "RC_L");
        assert_eq!(adm.objects.len(), 1);
        assert_eq!(adm.objects[0].azimuth, 30.0);
        assert_eq!(adm.objects[0].distance, 0.5);
    }

    #[test]
    fn parse_adm_empty_is_safe() {
        let adm = parse_adm_xml("");
        assert_eq!(adm.total_channels, 0);
    }
}
