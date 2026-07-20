//! IMP to DCP conversion by rewrapping DCI-compatible essence.
//!
//! The tractable path: IMF essence that already meets DCI constraints (JPEG 2000
//! in a DCI 2K/4K profile, 12-bit, within the DCI bitrate; and linear PCM) is
//! unwrapped from its AS-02 (ST 2067) MXF and rewrapped as AS-DCP (ST 429) MXF
//! via postkit, then a DCP (ST 429-7 CPL, 429-8 PKL, 429-9 ASSETMAP/VOLINDEX) is
//! written around it. Anything needing a real transcode (wrong J2K profile or bit
//! depth, over the DCI bitrate, non-J2K video, or a colour/transfer change) is a
//! hard error naming what is unsupported, never a silent essence copy.
//!
//! Colour space and transfer characteristics cannot be read from a J2K codestream
//! or from the AS-02 descriptors asdcplib exposes, so this tool cannot gate on
//! them; it assumes the picture essence is already DCI (XYZ) colorimetry, as a
//! rewrap contract. It never converts colour.

use std::path::{Path, PathBuf};

use asdcplib::Rational;

/// Options for IMP to DCP conversion.
pub struct ToDcpOptions {
    pub imp_dir: PathBuf,
    pub output_dir: PathBuf,
    pub title: Option<String>,
    pub content_kind: String,
}

/// Result of IMP to DCP conversion.
pub struct ToDcpResult {
    pub success: bool,
    pub error: String,
    pub output_dir: PathBuf,
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
        Ok(()) => ToDcpResult {
            success: true,
            error: String::new(),
            output_dir: opts.output_dir.clone(),
        },
        Err(e) => ToDcpResult {
            success: false,
            error: e,
            output_dir: opts.output_dir.clone(),
        },
    }
}

fn convert(opts: &ToDcpOptions) -> Result<(), String> {
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
) -> Result<(), String> {
    let mut assets: Vec<DcpAsset> = Vec::new();

    // Picture: unwrap AS-02 J2K frames, DCI-check them, rewrap as AS-DCP.
    let (pic_path, pic) = &pictures[0];
    let frame_dir = tmp.join("j2k");
    std::fs::create_dir_all(&frame_dir).map_err(|e| format!("temp: {e}"))?;
    let frames = extract_j2k_frames(pic_path, pic, &frame_dir)?;
    let out_mxf = opts.output_dir.join("picture.mxf");
    let wrapped = postkit::mxf_wrap::mxf_wrap(&postkit::mxf_wrap::MxfWrapOptions {
        input_files: frames,
        output: out_mxf.clone(),
        essence_type: postkit::mxf_wrap::EssenceType::J2k,
        standard: postkit::mxf_wrap::MxfStandard::AsDcp,
        fps_num: pic.fps_num,
        fps_den: pic.fps_den,
        partition_size: 0,
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
    Ok(())
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

    let dci_max = postkit::j2k::dci_max_bitrate_mbps(pic.width);
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
        postkit::j2k::J2kProfile::CinemaS2k | postkit::j2k::J2kProfile::CinemaS4k => {}
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

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn write_cpl(
    path: &Path,
    cpl_uuid: &str,
    title: &str,
    content_kind: &str,
    assets: &[DcpAsset],
) -> Result<(), String> {
    use std::fmt::Write;
    let mut x = String::new();
    let _ = writeln!(x, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(
        x,
        r#"<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">"#
    );
    let _ = writeln!(x, "  <Id>urn:uuid:{cpl_uuid}</Id>");
    let _ = writeln!(x, "  <IssueDate>{}</IssueDate>", crate::issue_date());
    let _ = writeln!(x, "  <Issuer>IMF Wizard</Issuer>");
    let _ = writeln!(x, "  <Creator>IMF Wizard</Creator>");
    let _ = writeln!(
        x,
        "  <ContentTitleText>{}</ContentTitleText>",
        xml_escape(title)
    );
    let _ = writeln!(
        x,
        "  <ContentKind>{}</ContentKind>",
        xml_escape(content_kind)
    );
    let _ = writeln!(x, "  <ReelList>");
    let _ = writeln!(x, "    <Reel>");
    let _ = writeln!(x, "      <Id>urn:uuid:{}</Id>", uuid::Uuid::new_v4());
    let _ = writeln!(x, "      <AssetList>");
    for a in assets {
        let er = format!("{} {}", a.fps_num, a.fps_den);
        match a.kind {
            AssetKind::Picture { width, height } => {
                let _ = writeln!(x, "        <MainPicture>");
                let _ = writeln!(x, "          <Id>urn:uuid:{}</Id>", a.uuid);
                let _ = writeln!(x, "          <EditRate>{er}</EditRate>");
                let _ = writeln!(
                    x,
                    "          <IntrinsicDuration>{}</IntrinsicDuration>",
                    a.duration
                );
                let _ = writeln!(x, "          <EntryPoint>0</EntryPoint>");
                let _ = writeln!(x, "          <Duration>{}</Duration>", a.duration);
                let _ = writeln!(x, "          <FrameRate>{er}</FrameRate>");
                let _ = writeln!(
                    x,
                    "          <ScreenAspectRatio>{width} {height}</ScreenAspectRatio>"
                );
                let _ = writeln!(x, "        </MainPicture>");
            }
            AssetKind::Sound => {
                let _ = writeln!(x, "        <MainSound>");
                let _ = writeln!(x, "          <Id>urn:uuid:{}</Id>", a.uuid);
                let _ = writeln!(x, "          <EditRate>{er}</EditRate>");
                let _ = writeln!(
                    x,
                    "          <IntrinsicDuration>{}</IntrinsicDuration>",
                    a.duration
                );
                let _ = writeln!(x, "          <EntryPoint>0</EntryPoint>");
                let _ = writeln!(x, "          <Duration>{}</Duration>", a.duration);
                let _ = writeln!(x, "        </MainSound>");
            }
        }
    }
    let _ = writeln!(x, "      </AssetList>");
    let _ = writeln!(x, "    </Reel>");
    let _ = writeln!(x, "  </ReelList>");
    let _ = writeln!(x, "</CompositionPlaylist>");
    std::fs::write(path, x).map_err(|e| format!("writing CPL {}: {e}", path.display()))
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
    use std::fmt::Write;
    let mut x = String::new();
    let _ = writeln!(x, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(
        x,
        r#"<PackingList xmlns="http://www.smpte-ra.org/schemas/429-8/2007/PKL">"#
    );
    let _ = writeln!(x, "  <Id>urn:uuid:{pkl_uuid}</Id>");
    let _ = writeln!(
        x,
        "  <AnnotationText>{}</AnnotationText>",
        xml_escape(title)
    );
    let _ = writeln!(x, "  <IssueDate>{}</IssueDate>", crate::issue_date());
    let _ = writeln!(x, "  <Issuer>IMF Wizard</Issuer>");
    let _ = writeln!(x, "  <Creator>IMF Wizard</Creator>");
    let _ = writeln!(x, "  <AssetList>");
    let _ = writeln!(x, "    <Asset>");
    let _ = writeln!(x, "      <Id>urn:uuid:{cpl_uuid}</Id>");
    let _ = writeln!(x, "      <Hash>{cpl_hash}</Hash>");
    let _ = writeln!(x, "      <Size>{cpl_size}</Size>");
    let _ = writeln!(x, "      <Type>text/xml</Type>");
    let _ = writeln!(x, "    </Asset>");
    for a in assets {
        let _ = writeln!(x, "    <Asset>");
        let _ = writeln!(x, "      <Id>urn:uuid:{}</Id>", a.uuid);
        let _ = writeln!(x, "      <Hash>{}</Hash>", a.hash_b64);
        let _ = writeln!(x, "      <Size>{}</Size>", a.size);
        let _ = writeln!(x, "      <Type>application/mxf</Type>");
        let _ = writeln!(x, "    </Asset>");
    }
    let _ = writeln!(x, "  </AssetList>");
    let _ = writeln!(x, "</PackingList>");
    std::fs::write(path, x).map_err(|e| format!("writing PKL {}: {e}", path.display()))
}

fn write_assetmap(
    path: &Path,
    pkl_uuid: &str,
    pkl_file: &str,
    cpl_uuid: &str,
    cpl_file: &str,
    assets: &[DcpAsset],
) -> Result<(), String> {
    use std::fmt::Write;
    let mut x = String::new();
    let _ = writeln!(x, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(
        x,
        r#"<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">"#
    );
    let _ = writeln!(x, "  <Id>urn:uuid:{}</Id>", uuid::Uuid::new_v4());
    let _ = writeln!(x, "  <Creator>IMF Wizard</Creator>");
    let _ = writeln!(x, "  <VolumeCount>1</VolumeCount>");
    let _ = writeln!(x, "  <IssueDate>{}</IssueDate>", crate::issue_date());
    let _ = writeln!(x, "  <Issuer>IMF Wizard</Issuer>");
    let _ = writeln!(x, "  <AssetList>");
    let _ = writeln!(x, "    <Asset>");
    let _ = writeln!(x, "      <Id>urn:uuid:{pkl_uuid}</Id>");
    let _ = writeln!(x, "      <PackingList>true</PackingList>");
    let _ = writeln!(
        x,
        "      <ChunkList><Chunk><Path>{pkl_file}</Path></Chunk></ChunkList>"
    );
    let _ = writeln!(x, "    </Asset>");
    let _ = writeln!(x, "    <Asset>");
    let _ = writeln!(x, "      <Id>urn:uuid:{cpl_uuid}</Id>");
    let _ = writeln!(
        x,
        "      <ChunkList><Chunk><Path>{cpl_file}</Path></Chunk></ChunkList>"
    );
    let _ = writeln!(x, "    </Asset>");
    for a in assets {
        let _ = writeln!(x, "    <Asset>");
        let _ = writeln!(x, "      <Id>urn:uuid:{}</Id>", a.uuid);
        let _ = writeln!(
            x,
            "      <ChunkList><Chunk><Path>{}</Path></Chunk></ChunkList>",
            a.filename
        );
        let _ = writeln!(x, "    </Asset>");
    }
    let _ = writeln!(x, "  </AssetList>");
    let _ = writeln!(x, "</AssetMap>");
    std::fs::write(path, x).map_err(|e| format!("writing ASSETMAP {}: {e}", path.display()))
}

fn write_volindex(path: &Path) -> Result<(), String> {
    let x = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<VolumeIndex xmlns=\"http://www.smpte-ra.org/schemas/429-9/2007/AM\">\n",
        "  <Index>1</Index>\n",
        "</VolumeIndex>\n"
    );
    std::fs::write(path, x).map_err(|e| format!("writing VOLINDEX {}: {e}", path.display()))
}
