//! Dolby Atmos ADM BWF import.
//!
//! Extracts ADM (Audio Definition Model) XML from BWF RIFF "axml" chunks,
//! parses bed channels and audio objects, splits to per-channel stems,
//! wraps to MXF, and writes an ADM sidecar XML file.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;

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

    // Extract programme name
    if let Some(start) = xml.find("audioProgrammeName=\"") {
        let rest = &xml[start + 20..];
        if let Some(end) = rest.find('"') {
            adm.programme_name = rest[..end].to_string();
        }
    }

    // Parse audioChannelFormat entries
    let mut track_idx: u32 = 0;
    let mut search_from = 0;

    while let Some(pos) = xml[search_from..].find("<audioChannelFormat") {
        let abs_pos = search_from + pos;
        let chunk_end = match xml[abs_pos..].find('>') {
            Some(e) => abs_pos + e,
            None => break,
        };
        let tag = &xml[abs_pos..=chunk_end];

        // Extract attributes
        let id = extract_attr(tag, "audioChannelFormatID");
        let name = extract_attr(tag, "audioChannelFormatName");
        let type_label = extract_attr(tag, "typeLabel");

        // Find scope of this channel format (up to next audioChannelFormat or end)
        let scope_end = xml[chunk_end..]
            .find("<audioChannelFormat")
            .map(|p| chunk_end + p)
            .unwrap_or(xml.len());
        let scope = &xml[chunk_end..scope_end];

        match type_label.as_str() {
            "0001" => {
                // DirectSpeakers — bed channel
                let speaker_label = find_in_scope(scope, "speakerLabel");
                adm.beds.push(BedChannel {
                    label: name,
                    speaker_label,
                    track_index: track_idx,
                });
                track_idx += 1;
            }
            "0003" => {
                // Objects — dynamic audio object
                let azimuth = find_element_value(scope, "azimuth")
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(0.0);
                let elevation = find_element_value(scope, "elevation")
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(0.0);
                let distance = find_element_value(scope, "distance")
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(1.0);

                adm.objects.push(AudioObject {
                    id,
                    name,
                    track_index: track_idx,
                    azimuth,
                    elevation,
                    distance,
                });
                track_idx += 1;
            }
            _ => {
                track_idx += 1;
            }
        }

        search_from = chunk_end + 1;
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

    // Extract individual channel stems and wrap to MXF via ffmpeg
    let mut channel_files = Vec::new();
    for i in 0..adm.total_channels {
        let stem = output_dir.join(format!("ch_{i:03}.wav"));
        let status = Command::new("ffmpeg")
            .args([
                "-y",
                "-i",
                &input.to_string_lossy(),
                "-map_channel",
                &format!("0.0.{i}"),
                stem.to_str().unwrap_or(""),
            ])
            .output();
        match status {
            Ok(o) if o.status.success() => channel_files.push(stem),
            Ok(o) => {
                tracing::warn!(
                    "ffmpeg channel extract failed for ch {i}: {}",
                    String::from_utf8_lossy(&o.stderr)
                );
            }
            Err(e) => {
                tracing::warn!("ffmpeg not available for channel extract: {e}");
            }
        }
    }

    // Wrap all channels into a single MXF
    let mut ffmpeg_args: Vec<String> = vec!["-y".into()];
    for f in &channel_files {
        ffmpeg_args.push("-i".into());
        ffmpeg_args.push(f.to_string_lossy().to_string());
    }
    // Map all inputs
    for i in 0..channel_files.len() {
        ffmpeg_args.push("-map".into());
        ffmpeg_args.push(format!("{i}:a"));
    }
    ffmpeg_args.extend(["-c:a".into(), "pcm_s24le".into(), "-f".into(), "mxf".into()]);
    ffmpeg_args.push(mxf_output.to_string_lossy().to_string());

    let wrap_result = Command::new("ffmpeg").args(&ffmpeg_args).output();

    let success = match wrap_result {
        Ok(o) if o.status.success() => true,
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            tracing::error!("MXF wrap failed: {err}");
            return AtmosImportResult {
                success: false,
                bed_count: adm.beds.len(),
                object_count: adm.objects.len(),
                total_channels: adm.total_channels,
                mxf_output,
                adm_sidecar,
                error: format!(
                    "MXF wrap failed: {}",
                    err.chars().take(200).collect::<String>()
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
    };

    // Cleanup intermediate channel WAVs
    for f in &channel_files {
        let _ = std::fs::remove_file(f);
    }

    AtmosImportResult {
        success,
        bed_count: adm.beds.len(),
        object_count: adm.objects.len(),
        total_channels: adm.total_channels,
        mxf_output,
        adm_sidecar,
        error: String::new(),
    }
}

// Helper: extract XML attribute value from a tag string.
fn extract_attr(tag: &str, attr: &str) -> String {
    let needle = format!("{attr}=\"");
    if let Some(start) = tag.find(&needle) {
        let rest = &tag[start + needle.len()..];
        if let Some(end) = rest.find('"') {
            return rest[..end].to_string();
        }
    }
    String::new()
}

// Helper: find an element value like <speakerLabel>RC_M+030</speakerLabel> in scope.
fn find_in_scope(scope: &str, element: &str) -> String {
    let open = format!("<{element}>");
    let close = format!("</{element}>");
    if let Some(start) = scope.find(&open) {
        let rest = &scope[start + open.len()..];
        if let Some(end) = rest.find(&close) {
            return rest[..end].trim().to_string();
        }
    }
    String::new()
}

// Helper: find element value, returning Option.
fn find_element_value(scope: &str, element: &str) -> Option<String> {
    let v = find_in_scope(scope, element);
    if v.is_empty() { None } else { Some(v) }
}
