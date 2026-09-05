//! IMP to DCP conversion, by rewrapping DCI-compatible essence or transcoding
//! Rec.709 IMF picture into it.
//!
//! Picture already in a DCI 2K/4K cinema profile at 12 bits and within the DCI
//! bitrate is unwrapped from its AS-02 (ST 2067) MXF and rewrapped as AS-DCP
//! (ST 429) MXF via postkit. Picture in an IMF profile, which is Rec.709 RGB, is
//! decoded, converted to DCI X'Y'Z' and re-encoded under the cinema profile.
//! Sound is linear PCM either way and is always rewrapped. A DCP (ST 429-7 CPL,
//! 429-8 PKL, 429-9 ASSETMAP/VOLINDEX) is written around the result.
//!
//! Anything else is a hard error naming what is unsupported: a picture whose
//! primaries or transfer characteristic need a gamut conversion or a tone map,
//! non-J2K video, and an edit rate outside the DCI set.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use asdcplib::Rational;

// the DCI set plus the HFR addendum's rates
const DCP_EDIT_RATES: [u32; 9] = [24, 25, 30, 48, 50, 60, 96, 100, 120];

const PROGRESS_FRAME_INTERVAL: u64 = 100;

const FRAME_READ_BUFFER_BYTES: usize = 16 * 1024 * 1024;

/// Options for IMP to DCP conversion.
pub struct ToDcpOptions {
    pub imp_dir: PathBuf,
    pub output_dir: PathBuf,
    pub title: Option<String>,
    pub content_kind: String,
    // defaults to the DCI maximum, is refused above it, and the rewrap path ignores it
    pub bitrate_mbps: Option<f64>,
}

/// Result of IMP to DCP conversion.
pub struct ToDcpResult {
    pub success: bool,
    pub error: String,
    pub output_dir: PathBuf,
    pub picture_report: String,
}

/// A track file that will go into the DCP: its DCP-side asset id, filename, hash
/// and timing.
struct DcpAsset {
    uuid: String,
    filename: String,
    hash_b64: String,
    size: u64,
    duration: u64,
    fps_num: u32,
    fps_den: u32,
    kind: AssetKind,
}

enum AssetKind {
    Picture { width: u32, height: u32 },
    Sound,
}

/// Convert an IMP to a DCP. See the module docs for what is rewrapped vs errored.
pub fn imp_to_dcp(opts: &ToDcpOptions) -> ToDcpResult {
    match convert(opts) {
        Ok(picture_report) => {
            tracing::info!("{picture_report}");
            ToDcpResult {
                success: true,
                error: String::new(),
                output_dir: opts.output_dir.clone(),
                picture_report,
            }
        }
        Err(e) => ToDcpResult {
            success: false,
            error: e,
            output_dir: opts.output_dir.clone(),
            picture_report: String::new(),
        },
    }
}

fn convert(opts: &ToDcpOptions) -> Result<String, String> {
    if !opts.imp_dir.is_dir() {
        return Err(format!("IMP not found: {}", opts.imp_dir.display()));
    }
    std::fs::create_dir_all(&opts.output_dir)
        .map_err(|e| format!("cannot create output {}: {e}", opts.output_dir.display()))?;

    let mxf_files = collect_mxf(&opts.imp_dir)?;
    if mxf_files.is_empty() {
        return Err(format!("no MXF track files in {}", opts.imp_dir.display()));
    }

    // Classify each track file as picture, sound, or unsupported.
    let mut pictures = Vec::new();
    let mut sounds = Vec::new();
    for mxf in &mxf_files {
        match classify(mxf)? {
            Track::Picture(p) => pictures.push((mxf.clone(), p)),
            Track::Sound(s) => sounds.push((mxf.clone(), s)),
        }
    }

    if pictures.len() != 1 || sounds.len() > 1 {
        return Err(format!(
            "to-dcp implements a single-reel IMP (one picture, optional one sound); \
             found {} picture and {} sound track(s); multi-reel conversion is not implemented",
            pictures.len(),
            sounds.len()
        ));
    }

    let tmp = opts.output_dir.join(".imfwizard_todcp_tmp");
    std::fs::create_dir_all(&tmp)
        .map_err(|e| format!("cannot create temp dir {}: {e}", tmp.display()))?;
    let result = build_dcp(opts, &pictures, &sounds, &tmp);
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

fn build_dcp(
    opts: &ToDcpOptions,
    pictures: &[(PathBuf, PictureInfo)],
    sounds: &[(PathBuf, SoundInfo)],
    tmp: &Path,
) -> Result<String, String> {
    let mut assets: Vec<DcpAsset> = Vec::new();

    let (pic_path, pic) = &pictures[0];
    let edit_rate = check_dcp_edit_rate(pic.fps_num, pic.fps_den)?;
    let frame_dir = tmp.join("j2k");
    std::fs::create_dir_all(&frame_dir).map_err(|e| format!("temp: {e}"))?;
    let (frames, picture_report) = match picture_route(pic_path)? {
        PictureRoute::Rewrap => (
            extract_j2k_frames(pic_path, pic, &frame_dir)?,
            "picture: rewrapped, DCI cinema codestreams".to_string(),
        ),
        PictureRoute::Transcode => {
            let bitrate_mbps = opts
                .bitrate_mbps
                .unwrap_or(postkit::j2k::DCI_MAX_BITRATE_MBPS);
            let rsiz =
                transcode_picture_to_cinema(pic_path, pic, edit_rate, bitrate_mbps, &frame_dir)?;
            let container = match rsiz {
                postkit::j2k::CINEMA_4K_RSIZ => "4K",
                _ => "2K",
            };
            (
                cinema_frame_paths(&frame_dir, pic.frame_count)?,
                format!(
                    "picture: transcoded Rec.709 RGB to X'Y'Z' cinema {container} at \
                     {bitrate_mbps} Mbps, {} frames",
                    pic.frame_count
                ),
            )
        }
    };
    let out_mxf = opts.output_dir.join("picture.mxf");
    let wrapped = postkit::mxf_wrap::mxf_wrap(&postkit::mxf_wrap::MxfWrapOptions {
        input_files: frames,
        output: out_mxf.clone(),
        essence_type: postkit::mxf_wrap::EssenceType::J2k,
        standard: postkit::mxf_wrap::MxfStandard::AsDcp,
        fps_num: pic.fps_num,
        fps_den: pic.fps_den,
        partition_size: 0,
        encryption: None,
        mca_config: None,
        resource_ids: vec![],
        hdr: None,
        asset_uuid: None,
        timed_text_duration_frames: None,
    });
    if !wrapped.success {
        return Err(format!(
            "rewrapping picture as AS-DCP failed: {}",
            wrapped.error
        ));
    }
    assets.push(dcp_asset(
        &out_mxf,
        &wrapped.uuid,
        wrapped.duration,
        pic.fps_num,
        pic.fps_den,
        AssetKind::Picture {
            width: pic.width,
            height: pic.height,
        },
    )?);

    // Sound: unwrap AS-02 PCM to a WAV, rewrap as AS-DCP.
    if let Some((snd_path, snd)) = sounds.first() {
        let wav = tmp.join("audio.wav");
        extract_pcm_wav(snd_path, snd, pic.fps_num, pic.fps_den, &wav)?;
        let out_mxf = opts.output_dir.join("sound.mxf");
        let wrapped = postkit::mxf_wrap::mxf_wrap(&postkit::mxf_wrap::MxfWrapOptions {
            input_files: vec![wav],
            output: out_mxf.clone(),
            essence_type: postkit::mxf_wrap::EssenceType::Pcm,
            standard: postkit::mxf_wrap::MxfStandard::AsDcp,
            fps_num: pic.fps_num,
            fps_den: pic.fps_den,
            partition_size: 0,
            encryption: None,
            mca_config: None,
            resource_ids: vec![],
            hdr: None,
            asset_uuid: None,
            timed_text_duration_frames: None,
        });
        if !wrapped.success {
            return Err(format!(
                "rewrapping sound as AS-DCP failed: {}",
                wrapped.error
            ));
        }
        assets.push(dcp_asset(
            &out_mxf,
            &wrapped.uuid,
            wrapped.duration,
            pic.fps_num,
            pic.fps_den,
            AssetKind::Sound,
        )?);
    }

    let title = opts
        .title
        .clone()
        .or_else(|| {
            crate::info::inspect_imp(&opts.imp_dir)
                .ok()
                .map(|i| i.title)
                .filter(|t| !t.is_empty())
        })
        .unwrap_or_else(|| "IMF Wizard DCP".to_string());
    let content_kind = if opts.content_kind.is_empty() {
        "feature".to_string()
    } else {
        opts.content_kind.clone()
    };

    let cpl_uuid = uuid::Uuid::new_v4().to_string();
    let pkl_uuid = uuid::Uuid::new_v4().to_string();
    let cpl_path = opts.output_dir.join(format!("CPL_{cpl_uuid}.xml"));
    write_cpl(&cpl_path, &cpl_uuid, &title, &content_kind, &assets)?;

    let cpl_hash =
        dcpdoctor_core::hash::sha1_base64(&cpl_path).map_err(|e| format!("hashing CPL: {e}"))?;
    let cpl_size = std::fs::metadata(&cpl_path).map(|m| m.len()).unwrap_or(0);

    let pkl_path = opts.output_dir.join(format!("PKL_{pkl_uuid}.xml"));
    write_pkl(
        &pkl_path, &pkl_uuid, &cpl_uuid, &cpl_hash, cpl_size, &title, &assets,
    )?;

    write_assetmap(
        &opts.output_dir.join("ASSETMAP.xml"),
        &pkl_uuid,
        &format!("PKL_{pkl_uuid}.xml"),
        &cpl_uuid,
        &format!("CPL_{cpl_uuid}.xml"),
        &assets,
    )?;
    write_volindex(&opts.output_dir.join("VOLINDEX.xml"))?;
    Ok(picture_report)
}

fn check_dcp_edit_rate(fps_num: u32, fps_den: u32) -> Result<u32, String> {
    if fps_den == 1 && DCP_EDIT_RATES.contains(&fps_num) {
        return Ok(fps_num);
    }
    let allowed = DCP_EDIT_RATES
        .iter()
        .map(|rate| rate.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "the picture edit rate is {fps_num}/{fps_den}, and a DCP carries one of {allowed}; \
         retime the composition before converting it"
    ))
}

fn dcp_asset(
    path: &Path,
    uuid: &str,
    duration: u64,
    fps_num: u32,
    fps_den: u32,
    kind: AssetKind,
) -> Result<DcpAsset, String> {
    Ok(DcpAsset {
        uuid: uuid.to_string(),
        filename: path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string(),
        hash_b64: dcpdoctor_core::hash::sha1_base64(path)
            .map_err(|e| format!("hashing {}: {e}", path.display()))?,
        size: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        duration,
        fps_num,
        fps_den,
        kind,
    })
}

fn collect_mxf(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("mxf"))
        })
        .collect();
    files.sort();
    Ok(files)
}

struct PictureInfo {
    width: u32,
    height: u32,
    frame_count: u32,
    fps_num: u32,
    fps_den: u32,
}

struct SoundInfo {
    frame_count: u32,
    channels: u32,
    bits: u32,
    sample_rate: u32,
}

enum Track {
    Picture(PictureInfo),
    Sound(SoundInfo),
}

/// Identify an AS-02 MXF as picture or sound, or reject it. Needs a default edit
/// rate to probe PCM (which is clip-wrapped and edit-rate agnostic on read).
fn classify(mxf: &Path) -> Result<Track, String> {
    let name = mxf.to_string_lossy().to_string();

    // A reader may open a foreign essence yet fail to read its descriptor, so a
    // track is picture/sound only when both the open and the descriptor succeed.
    let mut jp = asdcplib::as02::jp2k::MxfReader::new();
    if jp.open_read(&name).is_ok()
        && let Ok(d) = jp.picture_descriptor()
    {
        return Ok(Track::Picture(PictureInfo {
            width: d.stored_width,
            height: d.stored_height,
            frame_count: d.container_duration,
            fps_num: d.edit_rate.numerator.max(1) as u32,
            fps_den: d.edit_rate.denominator.max(1) as u32,
        }));
    }

    let mut pc = asdcplib::as02::pcm::MxfReader::new();
    if pc.open_read(&name, Rational::new(24, 1)).is_ok()
        && let Ok(d) = pc.audio_descriptor()
    {
        return Ok(Track::Sound(SoundInfo {
            frame_count: d.container_duration,
            channels: d.channel_count,
            bits: d.quantization_bits,
            sample_rate: d.audio_sampling_rate.numerator.max(1) as u32,
        }));
    }

    Err(format!(
        "{} is neither AS-02 JPEG 2000 nor PCM; only those rewrap to a DCP (subtitle/IAB \
         conversion is not implemented)",
        mxf.display()
    ))
}

enum PictureRoute {
    Rewrap,
    Transcode,
}

fn picture_route(mxf: &Path) -> Result<PictureRoute, String> {
    let frame = read_j2k_frame(mxf, 0)?;
    let header = postkit::j2k::parse_j2k_header(&frame)
        .ok_or("picture essence is not a JPEG 2000 codestream")?;
    match postkit::j2k::J2kProfile::from(header.profile) {
        postkit::j2k::J2kProfile::Cinema2k | postkit::j2k::J2kProfile::Cinema4k => {
            Ok(PictureRoute::Rewrap)
        }
        postkit::j2k::J2kProfile::Imf => Ok(PictureRoute::Transcode),
        other => Err(format!(
            "J2K is a {other:?} profile, neither a DCI 2K/4K cinema profile to rewrap nor an \
             IMF profile to transcode"
        )),
    }
}

fn read_j2k_frame(mxf: &Path, index: u32) -> Result<Vec<u8>, String> {
    let mut reader = asdcplib::as02::jp2k::MxfReader::new();
    reader
        .open_read(&mxf.to_string_lossy())
        .map_err(|e| format!("opening picture {}: {e}", mxf.display()))?;
    let mut buf = vec![0u8; FRAME_READ_BUFFER_BYTES];
    let read = reader
        .read_frame(index, &mut buf, None, None)
        .map_err(|e| format!("reading J2K frame {index}: {e}"))?;
    buf.truncate(read);
    Ok(buf)
}

fn check_rec709_source_colour(mxf: &Path) -> Result<(), String> {
    use asdcplib::jp2k::{
        COLOR_PRIMARIES_BT709, COLOR_PRIMARIES_BT2020, COLOR_PRIMARIES_P3D65,
        TRANSFER_CHARACTERISTIC_BT709, TRANSFER_CHARACTERISTIC_BT2020,
        TRANSFER_CHARACTERISTIC_ST2084,
    };
    const REC709_ONLY: &str = "to-dcp converts Rec.709 picture only";

    let mut reader = asdcplib::as02::jp2k::MxfReader::new();
    reader
        .open_read(&mxf.to_string_lossy())
        .map_err(|e| format!("opening picture {}: {e}", mxf.display()))?;
    // a descriptor with no colour items is not an error, it reads as unsignalled
    let colour = reader.hdr_metadata().unwrap_or_default();
    let file = mxf.display();

    match colour.transfer_characteristic {
        None => tracing::warn!(
            "{file} signals no transfer characteristic, so the transcode assumes Rec.709"
        ),
        Some(ul) if ul == TRANSFER_CHARACTERISTIC_BT709 => {}
        Some(ul) if ul == TRANSFER_CHARACTERISTIC_ST2084 => {
            return Err(format!(
                "{file} signals the ST 2084 (PQ) transfer characteristic, and a DCP needs a \
                 tone map"
            ));
        }
        Some(ul) if ul == TRANSFER_CHARACTERISTIC_BT2020 => {
            return Err(format!(
                "{file} signals the BT.2020 transfer characteristic, and a DCP needs a tone map"
            ));
        }
        Some(ul) => {
            return Err(format!(
                "{file} signals the unrecognised transfer characteristic {ul:02x?}, and \
                 {REC709_ONLY}"
            ));
        }
    }

    match colour.color_primaries {
        None => {
            tracing::warn!("{file} signals no colour primaries, so the transcode assumes Rec.709")
        }
        Some(ul) if ul == COLOR_PRIMARIES_BT709 => {}
        Some(ul) if ul == COLOR_PRIMARIES_P3D65 => {
            return Err(format!(
                "{file} signals P3-D65 colour primaries, and a DCP needs a gamut conversion"
            ));
        }
        Some(ul) if ul == COLOR_PRIMARIES_BT2020 => {
            return Err(format!(
                "{file} signals BT.2020 colour primaries, and a DCP needs a gamut conversion"
            ));
        }
        Some(ul) => {
            return Err(format!(
                "{file} signals the unrecognised colour primaries {ul:02x?}, and {REC709_ONLY}"
            ));
        }
    }
    Ok(())
}

// the Rsiz it returns says whether the DCP is a 2K or a 4K one
fn transcode_picture_to_cinema(
    mxf: &Path,
    pic: &PictureInfo,
    edit_rate: u32,
    bitrate_mbps: f64,
    j2k_dir: &Path,
) -> Result<u16, String> {
    let dci_max = postkit::j2k::DCI_MAX_BITRATE_MBPS;
    if bitrate_mbps > dci_max {
        return Err(format!(
            "--bitrate {bitrate_mbps} Mbps is over the DCI limit of {dci_max:.0} Mbps"
        ));
    }
    check_rec709_source_colour(mxf)?;
    // rsiz_for_raster promotes the 2K cinema profile to the 4K one by raster
    let rsiz = postkit::j2k::rsiz_for_raster(postkit::j2k::CINEMA_2K_RSIZ, pic.width, pic.height)?;
    if pic.frame_count == 0 {
        return Err("picture track has no frames".into());
    }

    let fps = crate::encode::FrameRate::new(pic.fps_num, pic.fps_den);
    let options = postkit::encode::StreamEncodeOptions {
        output_dir: j2k_dir.to_path_buf(),
        target_codestream_bytes: Some(crate::encode::codestream_byte_cap_for_bitrate(
            fps.as_f64(),
            bitrate_mbps,
        )),
        codestream_byte_cap: Some(postkit::j2k::dci_codestream_byte_cap(edit_rate)),
        fps,
        source_colour: postkit::encode::SourceColour::DisplayRgb,
        rsiz,
        ..Default::default()
    };

    let open_loader = || -> Result<postkit::encode::FrameLoader<'_>, String> {
        let mut reader = asdcplib::as02::jp2k::MxfReader::new();
        reader
            .open_read(&mxf.to_string_lossy())
            .map_err(|e| format!("opening picture {}: {e}", mxf.display()))?;
        let mut buf = vec![0u8; FRAME_READ_BUFFER_BYTES];
        Ok(Box::new(move |index: u64| {
            let read = reader
                .read_frame(index as u32, &mut buf, None, None)
                .map_err(|e| format!("reading J2K frame {index}: {e}"))?;
            let decoded = postkit::grok_decoder::decode(buf[..read].to_vec(), 0)
                .map_err(|e| format!("decoding J2K frame {index}: {e}"))?;
            raw_frame(decoded, index)
        }))
    };

    let mut last_reported = 0u64;
    let on_progress = |progress: postkit::encode::StreamProgress| {
        let done = progress.frame >= progress.total_frames;
        if !done && progress.frame < last_reported + PROGRESS_FRAME_INTERVAL {
            return;
        }
        last_reported = progress.frame;
        eprintln!(
            "[to-dcp] transcoded {}/{} frames",
            progress.frame, progress.total_frames
        );
    };

    let result = postkit::encode::encode_loaded_frames(
        pic.frame_count as u64,
        open_loader,
        &options,
        &Arc::new(AtomicBool::new(false)),
        &Arc::new(AtomicBool::new(false)),
        None,
        on_progress,
    );
    if !result.success {
        return Err(format!(
            "transcoding picture to X'Y'Z' failed: {}",
            result.error
        ));
    }
    Ok(rsiz)
}

fn raw_frame(
    decoded: postkit::grok_decoder::DecodedFrame,
    index: u64,
) -> Result<postkit::grok_encoder::RawFrame, String> {
    let (width, height, precision) = (decoded.width, decoded.height, decoded.precision);
    let components: [Vec<i32>; 3] = decoded.components.try_into().map_err(|c: Vec<Vec<i32>>| {
        format!(
            "picture frame {index} decodes to {} components; a DCP picture has 3",
            c.len()
        )
    })?;
    Ok(postkit::grok_encoder::RawFrame::Planar {
        components,
        width,
        height,
        precision,
        index,
    })
}

fn cinema_frame_paths(j2k_dir: &Path, frame_count: u32) -> Result<Vec<PathBuf>, String> {
    (0..frame_count)
        .map(|index| {
            let path = j2k_dir.join(format!("frame_{index:08}.j2c"));
            if !path.is_file() {
                return Err(format!(
                    "the transcode wrote no codestream for frame {index} at {}",
                    path.display()
                ));
            }
            Ok(path)
        })
        .collect()
}

/// Read every J2K frame from an AS-02 picture MXF, DCI-check it, and write the
/// codestreams to `dir`. Returns the frame files in order.
fn extract_j2k_frames(mxf: &Path, pic: &PictureInfo, dir: &Path) -> Result<Vec<PathBuf>, String> {
    if pic.width > 4096 || pic.height > 2160 {
        return Err(format!(
            "picture is {}x{}, larger than the DCP 4K container (4096x2160); transcode required",
            pic.width, pic.height
        ));
    }

    let mut reader = asdcplib::as02::jp2k::MxfReader::new();
    reader
        .open_read(&mxf.to_string_lossy())
        .map_err(|e| format!("opening picture {}: {e}", mxf.display()))?;

    let dci_max = postkit::j2k::DCI_MAX_BITRATE_MBPS;
    let fps = pic.fps_num as f64 / pic.fps_den.max(1) as f64;
    let mut buf = vec![0u8; 16 * 1024 * 1024];
    let mut paths = Vec::with_capacity(pic.frame_count as usize);

    for i in 0..pic.frame_count {
        let n = reader
            .read_frame(i, &mut buf, None, None)
            .map_err(|e| format!("reading J2K frame {i}: {e}"))?;
        let frame = &buf[..n];

        if i == 0 {
            check_j2k_dci(frame)?;
        }
        let mbps = n as f64 * 8.0 * fps / 1_000_000.0;
        if mbps > dci_max {
            return Err(format!(
                "J2K frame {i} is {mbps:.1} Mbps, over the DCI limit of {dci_max:.0} Mbps; \
                 transcode required"
            ));
        }

        let path = dir.join(format!("frame_{i:08}.j2c"));
        std::fs::write(&path, frame).map_err(|e| format!("writing frame {i}: {e}"))?;
        paths.push(path);
    }
    if paths.is_empty() {
        return Err("picture track has no frames".into());
    }
    Ok(paths)
}

/// Reject J2K essence that would need a transcode for a DCP.
fn check_j2k_dci(frame: &[u8]) -> Result<(), String> {
    let h = postkit::j2k::parse_j2k_header(frame)
        .ok_or("picture essence is not a JPEG 2000 codestream")?;
    match postkit::j2k::J2kProfile::from(h.profile) {
        postkit::j2k::J2kProfile::Cinema2k | postkit::j2k::J2kProfile::Cinema4k => {}
        other => {
            return Err(format!(
                "J2K is a {other:?} profile, not a DCI 2K/4K cinema profile; transcode required"
            ));
        }
    }
    if h.bit_depth != 12 {
        return Err(format!(
            "J2K is {}-bit; a DCP requires 12-bit picture essence; transcode required",
            h.bit_depth
        ));
    }
    if h.num_components != 3 {
        return Err(format!(
            "J2K has {} components; a DCP requires 3; transcode required",
            h.num_components
        ));
    }
    Ok(())
}

/// Read every PCM frame from an AS-02 sound MXF and write a canonical WAV.
fn extract_pcm_wav(
    mxf: &Path,
    snd: &SoundInfo,
    fps_num: u32,
    fps_den: u32,
    wav: &Path,
) -> Result<(), String> {
    let mut reader = asdcplib::as02::pcm::MxfReader::new();
    reader
        .open_read(
            &mxf.to_string_lossy(),
            Rational::new(fps_num as i32, fps_den as i32),
        )
        .map_err(|e| format!("opening sound {}: {e}", mxf.display()))?;

    let block_align = (snd.bits / 8) * snd.channels;
    let samples_per_frame =
        (snd.sample_rate as f64 / (fps_num as f64 / fps_den.max(1) as f64)).ceil() as u32;
    let frame_size = (samples_per_frame * block_align) as usize;
    let mut buf = vec![0u8; frame_size.max(1)];
    let mut pcm = Vec::with_capacity(frame_size * snd.frame_count as usize);

    // Clip-wrapped PCM can report one more edit unit than it yields, so stop at
    // the first read that has no more data rather than failing.
    for i in 0..snd.frame_count {
        match reader.read_frame(i, &mut buf, None, None) {
            Ok(n) => pcm.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    if pcm.is_empty() {
        return Err(format!("sound track {} yielded no PCM", mxf.display()));
    }

    write_wav(wav, snd.channels, snd.sample_rate, snd.bits, &pcm)
}

fn write_wav(
    path: &Path,
    channels: u32,
    sample_rate: u32,
    bits: u32,
    pcm: &[u8],
) -> Result<(), String> {
    let block_align = (bits / 8) * channels;
    let byte_rate = sample_rate * block_align;
    let data_len = pcm.len() as u32;
    let mut w = Vec::with_capacity(pcm.len() + 44);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVE");
    w.extend_from_slice(b"fmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // WAVE_FORMAT_PCM
    w.extend_from_slice(&(channels as u16).to_le_bytes());
    w.extend_from_slice(&sample_rate.to_le_bytes());
    w.extend_from_slice(&byte_rate.to_le_bytes());
    w.extend_from_slice(&(block_align as u16).to_le_bytes());
    w.extend_from_slice(&(bits as u16).to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    w.extend_from_slice(pcm);
    std::fs::write(path, w).map_err(|e| format!("writing WAV {}: {e}", path.display()))
}

use postkit::packaging::{
    AssetMap, AssetMapAsset, DcpCpl, DcpCplReel, PackingList, PklAsset, ns, volindex_xml,
};

/// Write the DCP CPL (ST 429-7 SMPTE) via postkit. Folds the picture and optional
/// sound asset into a single reel; per-asset ScreenAspectRatio comes from the real
/// picture container dims read off the MXF.
fn write_cpl(
    path: &Path,
    cpl_uuid: &str,
    title: &str,
    content_kind: &str,
    assets: &[DcpAsset],
) -> Result<(), String> {
    let mut reel = DcpCplReel {
        reel_id: uuid::Uuid::new_v4().to_string(),
        ..Default::default()
    };
    for a in assets {
        match a.kind {
            AssetKind::Picture { width, height } => {
                reel.picture_id = a.uuid.clone();
                reel.picture_edit_rate_num = a.fps_num;
                reel.picture_edit_rate_den = a.fps_den;
                reel.picture_duration = a.duration;
                reel.picture_width = width;
                reel.picture_height = height;
                reel.picture_hash = Some(a.hash_b64.clone());
            }
            AssetKind::Sound => {
                reel.sound_id = Some(a.uuid.clone());
                reel.sound_edit_rate_num = a.fps_num;
                reel.sound_edit_rate_den = a.fps_den;
                reel.sound_duration = a.duration;
                reel.sound_hash = Some(a.hash_b64.clone());
            }
        }
    }
    let cpl = DcpCpl {
        uuid: cpl_uuid.to_string(),
        namespace: ns::CPL_SMPTE.to_string(),
        title: title.to_string(),
        content_kind: content_kind.to_string(),
        issuer: "IMF Wizard".to_string(),
        creator: "IMF Wizard".to_string(),
        issue_date: crate::issue_date(),
        // Bv2.1 8.1: present, and equal to the content title
        annotation_text: Some(title.to_string()),
        content_version_label: None,
        ratings: Vec::new(),
        reels: vec![reel],
    };
    std::fs::write(path, cpl.to_xml()).map_err(|e| format!("writing CPL {}: {e}", path.display()))
}

#[allow(clippy::too_many_arguments)]
fn write_pkl(
    path: &Path,
    pkl_uuid: &str,
    cpl_uuid: &str,
    cpl_hash: &str,
    cpl_size: u64,
    title: &str,
    assets: &[DcpAsset],
) -> Result<(), String> {
    let mut pkl_assets = vec![PklAsset {
        id: cpl_uuid.to_string(),
        hash: cpl_hash.to_string(),
        size: cpl_size,
        asset_type: "text/xml".to_string(),
    }];
    for a in assets {
        pkl_assets.push(PklAsset {
            id: a.uuid.clone(),
            hash: a.hash_b64.clone(),
            size: a.size,
            asset_type: "application/mxf".to_string(),
        });
    }
    let pkl = PackingList {
        uuid: pkl_uuid.to_string(),
        namespace: ns::PKL_SMPTE.to_string(),
        issuer: "IMF Wizard".to_string(),
        creator: "IMF Wizard".to_string(),
        issue_date: crate::issue_date(),
        assets: pkl_assets,
        // Bv2.1 8.1: the PKL repeats the CPL's content title
        annotation: Some(title.to_string()),
    };
    std::fs::write(path, pkl.to_xml()).map_err(|e| format!("writing PKL {}: {e}", path.display()))
}

fn write_assetmap(
    path: &Path,
    pkl_uuid: &str,
    pkl_file: &str,
    cpl_uuid: &str,
    cpl_file: &str,
    assets: &[DcpAsset],
) -> Result<(), String> {
    let mut am_assets = vec![
        AssetMapAsset {
            id: pkl_uuid.to_string(),
            path: pkl_file.to_string(),
            packing_list: true,
        },
        AssetMapAsset {
            id: cpl_uuid.to_string(),
            path: cpl_file.to_string(),
            packing_list: false,
        },
    ];
    for a in assets {
        am_assets.push(AssetMapAsset {
            id: a.uuid.clone(),
            path: a.filename.clone(),
            packing_list: false,
        });
    }
    let am = AssetMap {
        uuid: uuid::Uuid::new_v4().to_string(),
        namespace: ns::AM_SMPTE.to_string(),
        issuer: "IMF Wizard".to_string(),
        creator: "IMF Wizard".to_string(),
        issue_date: crate::issue_date(),
        assets: am_assets,
        annotation: None,
    };
    std::fs::write(path, am.to_xml())
        .map_err(|e| format!("writing ASSETMAP {}: {e}", path.display()))
}

fn write_volindex(path: &Path) -> Result<(), String> {
    std::fs::write(path, volindex_xml(ns::AM_SMPTE))
        .map_err(|e| format!("writing VOLINDEX {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(kind: AssetKind, uuid: &str, filename: &str) -> DcpAsset {
        DcpAsset {
            uuid: uuid.to_string(),
            filename: filename.to_string(),
            hash_b64: "kO0m3F3qX3qg3n3qg3n3qg3n3q0=".to_string(),
            size: 42,
            duration: 240,
            fps_num: 24,
            fps_den: 1,
            kind,
        }
    }

    /// The SMPTE 429-7/8/9 schemas dcpdoctor vendors, alongside local copies of
    /// xmldsig-core-schema.xsd and xml.xsd. POSTKIT_DCP_XSD_DIR overrides it.
    const VENDORED_DCP_XSD_DIR: &str = "../../../extern/dcpdoctor/schemas";

    /// Validate the DCP docs this module writes against the official SMPTE
    /// 429-7/8/9 XSDs. Full essence-bearing DCP validation is done separately
    /// with dcpdoctor once real MXF track files exist.
    #[test]
    fn generated_dcp_docs_pass_smpte_xsd() {
        let xsd_dir = match std::env::var("POSTKIT_DCP_XSD_DIR") {
            Ok(dir) => PathBuf::from(dir),
            Err(_) => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(VENDORED_DCP_XSD_DIR),
        };
        let xsd = xsd_dir.as_path();
        let dir = tempfile::tempdir().unwrap();

        // the 429-7 CPL imports xmldsig and xml.xsd by http URL; map them to the
        // local copies so xmllint resolves them offline.
        let catalog = dir.path().join("catalog.xml");
        std::fs::write(
            &catalog,
            format!(
                r#"<?xml version="1.0"?>
<catalog xmlns="urn:oasis:names:tc:entity:xmlns:xml:catalog">
  <system systemId="http://www.w3.org/TR/2002/REC-xmldsig-core-20020212/xmldsig-core-schema.xsd" uri="{dsig}"/>
  <system systemId="http://www.w3.org/2001/03/xml.xsd" uri="{xml_xsd}"/>
</catalog>"#,
                dsig = postkit::file_uri::file_uri(&xsd.join("xmldsig-core-schema.xsd")),
                xml_xsd = postkit::file_uri::file_uri(&xsd.join("xml.xsd")),
            ),
        )
        .unwrap();

        let assets = [
            asset(
                AssetKind::Picture {
                    width: 2048,
                    height: 858,
                },
                "77777777-7777-8888-9999-aaaaaaaaaaaa",
                "picture.mxf",
            ),
            asset(
                AssetKind::Sound,
                "88888888-7777-8888-9999-aaaaaaaaaaaa",
                "sound.mxf",
            ),
        ];

        let cpl_uuid = "11111111-2222-3333-4444-555555555555";
        let pkl_uuid = "bbbbbbbb-7777-8888-9999-aaaaaaaaaaaa";
        let cpl_path = dir.path().join(format!("CPL_{cpl_uuid}.xml"));
        let pkl_path = dir.path().join(format!("PKL_{pkl_uuid}.xml"));
        let am_path = dir.path().join("ASSETMAP.xml");
        write_cpl(&cpl_path, cpl_uuid, "Test", "feature", &assets).unwrap();
        write_pkl(
            &pkl_path,
            pkl_uuid,
            cpl_uuid,
            "kO0m3F3qX3qg3n3qg3n3qg3n3q0=",
            100,
            "Test Title",
            &assets,
        )
        .unwrap();
        write_assetmap(
            &am_path,
            pkl_uuid,
            &format!("PKL_{pkl_uuid}.xml"),
            cpl_uuid,
            &format!("CPL_{cpl_uuid}.xml"),
            &assets,
        )
        .unwrap();

        for (doc, schema) in [
            (&cpl_path, "SMPTE-429-7-2006-CPL.xsd"),
            (&pkl_path, "SMPTE-429-8-2006-PKL.xsd"),
            (&am_path, "SMPTE-429-9-2007-AM.xsd"),
        ] {
            let out = std::process::Command::new("xmllint")
                .args(["--nonet", "--noout", "--schema"])
                .arg(xsd.join(schema))
                .arg(doc)
                .env("XML_CATALOG_FILES", postkit::file_uri::file_uri(&catalog))
                .output()
                .expect("run xmllint");
            assert!(
                out.status.success(),
                "{} must pass {schema}:\n{}",
                doc.display(),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}
