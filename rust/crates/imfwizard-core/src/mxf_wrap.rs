use serde::{Deserialize, Serialize};

/// MXF wrapping options.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MxfWrapOptions {
    pub input_dir: std::path::PathBuf,
    pub output_file: std::path::PathBuf,
    pub essence_type: crate::EssenceType,
    pub edit_rate_num: u32,
    pub edit_rate_den: u32,
    pub duration: u64,
}

/// MXF wrapping result.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MxfWrapResult {
    pub success: bool,
    pub error: String,
    pub track_file: crate::MxfTrackFile,
}

fn make_writer_info() -> asdcplib::WriterInfo {
    let asset_uuid = uuid::Uuid::new_v4();
    let context_id = uuid::Uuid::new_v4();
    asdcplib::WriterInfo {
        asset_uuid: *asset_uuid.as_bytes(),
        context_id: *context_id.as_bytes(),
        label_set: asdcplib::LabelSet::Smpte,
        ..Default::default()
    }
}

fn compute_hash_and_size(path: &std::path::Path) -> (String, u64) {
    use sha1::Digest;
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return (String::new(), 0),
    };
    let hash = sha1::Sha1::digest(&data);
    (
        hash.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        data.len() as u64,
    )
}

fn collect_input_files(dir: &std::path::Path) -> Result<Vec<std::path::PathBuf>, String> {
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read input dir {}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("no files found in {}", dir.display()));
    }
    Ok(files)
}

/// Wrap essence into an MXF track file using asdcplib FFI.
pub fn wrap_mxf(opts: &MxfWrapOptions) -> MxfWrapResult {
    match opts.essence_type {
        crate::EssenceType::J2k => wrap_j2k(opts),
        crate::EssenceType::Wav => wrap_pcm(opts),
        crate::EssenceType::TimedText => wrap_timed_text(opts),
        crate::EssenceType::Atmos => wrap_atmos(opts),
    }
}

fn wrap_j2k(opts: &MxfWrapOptions) -> MxfWrapResult {
    let files = match collect_input_files(&opts.input_dir) {
        Ok(f) => f,
        Err(e) => {
            return MxfWrapResult {
                error: e,
                ..Default::default()
            };
        }
    };

    let mut frames: Vec<Vec<u8>> = Vec::new();
    for f in &files {
        match std::fs::read(f) {
            Ok(data) => frames.push(data),
            Err(e) => {
                return MxfWrapResult {
                    error: format!("failed to read {}: {e}", f.display()),
                    ..Default::default()
                };
            }
        }
    }

    let info = make_writer_info();
    let desc = asdcplib::jp2k::PictureDescriptor {
        edit_rate: asdcplib::Rational::new(opts.edit_rate_num as i32, opts.edit_rate_den as i32),
        sample_rate: asdcplib::Rational::new(opts.edit_rate_num as i32, opts.edit_rate_den as i32),
        stored_width: 2048,
        stored_height: 1080,
        aspect_ratio: asdcplib::Rational::new(2048, 1080),
        container_duration: frames.len() as u32,
        component_count: 3,
    };

    let mut writer = asdcplib::jp2k::MxfWriter::new();
    let output_str = opts.output_file.to_string_lossy().to_string();
    if let Err(e) = writer.open_write(&output_str, &info, &desc, 16384) {
        return MxfWrapResult {
            error: format!("JP2K open_write: {e}"),
            ..Default::default()
        };
    }

    for frame in &frames {
        if let Err(e) = writer.write_frame(frame, None, None) {
            return MxfWrapResult {
                error: format!("JP2K write_frame: {e}"),
                ..Default::default()
            };
        }
    }

    if let Err(e) = writer.finalize() {
        return MxfWrapResult {
            error: format!("JP2K finalize: {e}"),
            ..Default::default()
        };
    }

    let (hash, size) = compute_hash_and_size(&opts.output_file);
    let uuid_str = uuid::Uuid::from_bytes(info.asset_uuid)
        .hyphenated()
        .to_string();

    MxfWrapResult {
        success: true,
        error: String::new(),
        track_file: crate::MxfTrackFile {
            path: opts.output_file.clone(),
            uuid: uuid_str,
            hash,
            size,
            duration: frames.len() as u64,
        },
    }
}

fn wrap_pcm(opts: &MxfWrapOptions) -> MxfWrapResult {
    let files = match collect_input_files(&opts.input_dir) {
        Ok(f) => f,
        Err(e) => {
            return MxfWrapResult {
                error: e,
                ..Default::default()
            };
        }
    };

    let wav_data = match std::fs::read(&files[0]) {
        Ok(d) => d,
        Err(e) => {
            return MxfWrapResult {
                error: format!("failed to read WAV: {e}"),
                ..Default::default()
            };
        }
    };

    let info = make_writer_info();
    let channels = 6u32;
    let bits = 24u32;
    let sample_rate = 48000u32;
    let block_align = (bits / 8) * channels;
    let samples_per_frame = (sample_rate as f64
        / (opts.edit_rate_num as f64 / opts.edit_rate_den as f64))
        .ceil() as u32;
    let frame_size = samples_per_frame * block_align;

    let pcm_start = if wav_data.len() > 44 { 44 } else { 0 };
    let pcm_data = &wav_data[pcm_start..];
    let num_frames = (pcm_data.len() as u32).checked_div(frame_size).unwrap_or(0);

    let desc = asdcplib::pcm::AudioDescriptor {
        edit_rate: asdcplib::Rational::new(opts.edit_rate_num as i32, opts.edit_rate_den as i32),
        audio_sampling_rate: asdcplib::Rational::new(sample_rate as i32, 1),
        locked: true,
        channel_count: channels,
        quantization_bits: bits,
        block_align,
        avg_bps: sample_rate * block_align,
        linked_track_id: 0,
        container_duration: num_frames,
        channel_format: asdcplib::pcm::ChannelFormat::Cfg1,
    };

    let mut writer = asdcplib::pcm::MxfWriter::new();
    let output_str = opts.output_file.to_string_lossy().to_string();
    if let Err(e) = writer.open_write(&output_str, &info, &desc, 16384) {
        return MxfWrapResult {
            error: format!("PCM open_write: {e}"),
            ..Default::default()
        };
    }

    for i in 0..num_frames {
        let start = (i * frame_size) as usize;
        let end = start + frame_size as usize;
        if end > pcm_data.len() {
            break;
        }
        if let Err(e) = writer.write_frame(&pcm_data[start..end], None, None) {
            return MxfWrapResult {
                error: format!("PCM write_frame: {e}"),
                ..Default::default()
            };
        }
    }

    if let Err(e) = writer.finalize() {
        return MxfWrapResult {
            error: format!("PCM finalize: {e}"),
            ..Default::default()
        };
    }

    let (hash, size) = compute_hash_and_size(&opts.output_file);
    let uuid_str = uuid::Uuid::from_bytes(info.asset_uuid)
        .hyphenated()
        .to_string();

    MxfWrapResult {
        success: true,
        error: String::new(),
        track_file: crate::MxfTrackFile {
            path: opts.output_file.clone(),
            uuid: uuid_str,
            hash,
            size,
            duration: num_frames as u64,
        },
    }
}

fn wrap_timed_text(opts: &MxfWrapOptions) -> MxfWrapResult {
    let files = match collect_input_files(&opts.input_dir) {
        Ok(f) => f,
        Err(e) => {
            return MxfWrapResult {
                error: e,
                ..Default::default()
            };
        }
    };

    let xml_data = match std::fs::read_to_string(&files[0]) {
        Ok(d) => d,
        Err(e) => {
            return MxfWrapResult {
                error: format!("failed to read XML: {e}"),
                ..Default::default()
            };
        }
    };

    let info = make_writer_info();
    let desc = asdcplib::timed_text::TimedTextDescriptor {
        edit_rate: asdcplib::Rational::new(opts.edit_rate_num as i32, opts.edit_rate_den as i32),
        container_duration: (opts.edit_rate_num * 60) / opts.edit_rate_den.max(1),
        asset_id: info.asset_uuid,
    };

    let mut writer = asdcplib::timed_text::MxfWriter::new();
    let output_str = opts.output_file.to_string_lossy().to_string();
    if let Err(e) = writer.open_write(&output_str, &info, &desc, 16384) {
        return MxfWrapResult {
            error: format!("TimedText open_write: {e}"),
            ..Default::default()
        };
    }

    if let Err(e) = writer.write_timed_text_resource(&xml_data, None, None) {
        return MxfWrapResult {
            error: format!("TimedText write: {e}"),
            ..Default::default()
        };
    }

    // Ancillary resources (fonts, images)
    for f in files.iter().skip(1) {
        let resource_data = match std::fs::read(f) {
            Ok(d) => d,
            Err(e) => {
                return MxfWrapResult {
                    error: format!("failed to read resource {}: {e}", f.display()),
                    ..Default::default()
                };
            }
        };
        let resource_uuid = *uuid::Uuid::new_v4().as_bytes();
        let ext = f
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let mime = match ext.as_str() {
            "ttf" | "otf" => "application/x-font-opentype",
            "png" => "image/png",
            _ => "application/octet-stream",
        };
        if let Err(e) =
            writer.write_ancillary_resource(&resource_data, &resource_uuid, mime, None, None)
        {
            return MxfWrapResult {
                error: format!("TimedText ancillary: {e}"),
                ..Default::default()
            };
        }
    }

    if let Err(e) = writer.finalize() {
        return MxfWrapResult {
            error: format!("TimedText finalize: {e}"),
            ..Default::default()
        };
    }

    let (hash, size) = compute_hash_and_size(&opts.output_file);
    let uuid_str = uuid::Uuid::from_bytes(info.asset_uuid)
        .hyphenated()
        .to_string();

    MxfWrapResult {
        success: true,
        error: String::new(),
        track_file: crate::MxfTrackFile {
            path: opts.output_file.clone(),
            uuid: uuid_str,
            hash,
            size,
            duration: desc.container_duration as u64,
        },
    }
}

fn wrap_atmos(opts: &MxfWrapOptions) -> MxfWrapResult {
    let files = match collect_input_files(&opts.input_dir) {
        Ok(f) => f,
        Err(e) => {
            return MxfWrapResult {
                error: e,
                ..Default::default()
            };
        }
    };

    let mut frames: Vec<Vec<u8>> = Vec::new();
    for f in &files {
        match std::fs::read(f) {
            Ok(data) => frames.push(data),
            Err(e) => {
                return MxfWrapResult {
                    error: format!("failed to read {}: {e}", f.display()),
                    ..Default::default()
                };
            }
        }
    }

    let info = make_writer_info();
    let desc = asdcplib::atmos::AtmosDescriptor {
        edit_rate: asdcplib::Rational::new(opts.edit_rate_num as i32, opts.edit_rate_den as i32),
        container_duration: frames.len() as u32,
        asset_id: info.asset_uuid,
        data_essence_coding: [0; 16],
        first_frame: 0,
        max_channel_count: 128,
        max_object_count: 118,
        atmos_id: *uuid::Uuid::new_v4().as_bytes(),
        atmos_version: 1,
    };

    let mut writer = asdcplib::atmos::MxfWriter::new();
    let output_str = opts.output_file.to_string_lossy().to_string();
    if let Err(e) = writer.open_write(&output_str, &info, &desc, 16384) {
        return MxfWrapResult {
            error: format!("Atmos open_write: {e}"),
            ..Default::default()
        };
    }

    for frame in &frames {
        if let Err(e) = writer.write_frame(frame, None, None) {
            return MxfWrapResult {
                error: format!("Atmos write_frame: {e}"),
                ..Default::default()
            };
        }
    }

    if let Err(e) = writer.finalize() {
        return MxfWrapResult {
            error: format!("Atmos finalize: {e}"),
            ..Default::default()
        };
    }

    let (hash, size) = compute_hash_and_size(&opts.output_file);
    let uuid_str = uuid::Uuid::from_bytes(info.asset_uuid)
        .hyphenated()
        .to_string();

    MxfWrapResult {
        success: true,
        error: String::new(),
        track_file: crate::MxfTrackFile {
            path: opts.output_file.clone(),
            uuid: uuid_str,
            hash,
            size,
            duration: frames.len() as u64,
        },
    }
}
