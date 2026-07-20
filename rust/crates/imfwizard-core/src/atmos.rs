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

/// Extract the "axml" chunk from a BWF (RIFF) file.
///
/// BWF files are RIFF containers; we scan top-level chunks for the "axml" FourCC
/// which contains the ADM XML document.
pub fn extract_axml_chunk(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("Cannot open BWF: {e}"))?;

    // Read RIFF header
    let mut riff_id = [0u8; 4];
    file.read_exact(&mut riff_id)
        .map_err(|e| format!("Read error: {e}"))?;
    if &riff_id != b"RIFF" {
        return Err("Not a RIFF file".into());
    }

    let mut size_buf = [0u8; 4];
    file.read_exact(&mut size_buf)
        .map_err(|e| format!("Read error: {e}"))?;

    let mut wave_id = [0u8; 4];
    file.read_exact(&mut wave_id)
        .map_err(|e| format!("Read error: {e}"))?;
    if &wave_id != b"WAVE" {
        return Err("Not a WAVE file".into());
    }

    // Scan chunks
    loop {
        let mut chunk_id = [0u8; 4];
        if file.read_exact(&mut chunk_id).is_err() {
            break;
        }

        let mut chunk_size_buf = [0u8; 4];
        if file.read_exact(&mut chunk_size_buf).is_err() {
            break;
        }
        let chunk_size = u32::from_le_bytes(chunk_size_buf) as u64;

        if &chunk_id == b"axml" {
            let mut xml_buf = vec![0u8; chunk_size as usize];
            file.read_exact(&mut xml_buf)
                .map_err(|e| format!("Failed to read axml chunk: {e}"))?;
            let xml = String::from_utf8_lossy(&xml_buf).to_string();
            return Ok(xml);
        }

        // Skip this chunk (pad to even boundary)
        let skip = if chunk_size % 2 == 1 {
            chunk_size + 1
        } else {
            chunk_size
        };
        file.seek(SeekFrom::Current(skip as i64))
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

/// Import a Dolby Atmos ADM BWF file into an IMF-compatible MXF.
pub fn import_atmos(input: &Path, output_dir: &Path) -> AtmosImportResult {
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
        tracing::warn!("Failed to write ADM sidecar: {e}");
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

    let wrap = crate::mxf_wrap::wrap_mxf(&crate::mxf_wrap::MxfWrapOptions {
        input_dir: combined.clone(),
        output_file: mxf_output.clone(),
        essence_type: crate::EssenceType::Wav,
        edit_rate_num: 24,
        edit_rate_den: 1,
        duration: 0,
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
