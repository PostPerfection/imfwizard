//! Export a composition's picture track to a numbered image sequence.
//!
//! Selects a CPL, walks its timeline, opens each picture track's AS-02 J2K MXF via
//! asdcplib, extracts every frame's J2K codestream, and decodes it with grk_decompress
//! (the repo's J2K codec, which preserves the codestream's native precision).
//!
//! Output is the codestream's native colour encoding: no colour transform is applied,
//! so XYZ or any other essence colorimetry is written through unchanged. Frames are
//! numbered from 1 (frame_000001.tif) over the exported subrange.

use std::path::{Path, PathBuf};

use crate::timeline;

/// Output image format. grk_decompress emits these at the codestream's native precision.
///
/// DPX is intentionally absent: grk_decompress cannot write DPX, and routing it through
/// a second decoder (ffmpeg) would promote the essence to 16-bit, breaking the
/// native-bit-depth contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Tiff,
    Png,
}

impl ExportFormat {
    pub fn from_flag(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "tiff" | "tif" => Ok(Self::Tiff),
            "png" => Ok(Self::Png),
            "dpx" => Err(
                "dpx export is not supported: grk_decompress (the J2K decoder, \
                          chosen to preserve native bit depth) cannot emit DPX; use tiff or png"
                    .into(),
            ),
            other => Err(format!("unknown format '{other}'; use tiff or png")),
        }
    }

    fn ext(self) -> &'static str {
        match self {
            Self::Tiff => "tif",
            Self::Png => "png",
        }
    }
}

/// Options for image-sequence export.
pub struct ExportFramesOptions {
    pub imp_dir: PathBuf,
    pub output_dir: PathBuf,
    pub format: ExportFormat,
    /// CPL selector: a UUID (with or without urn:uuid:) or a 0-based index. None uses
    /// the sole CPL and errors if the IMP has several.
    pub cpl: Option<String>,
    /// First composition frame to export (0-based).
    pub start: u32,
    /// Number of frames to export; None exports through the end.
    pub count: Option<u32>,
}

/// Result of an export.
#[derive(Debug)]
pub struct ExportFramesResult {
    pub frames_written: u32,
    pub output_dir: PathBuf,
    pub width: u32,
    pub height: u32,
}

/// One timeline resource: a picture MXF and the source-frame window to read from it.
struct Segment {
    mxf: PathBuf,
    entry: u32,
    count: u32,
}

pub fn export_frames(opts: &ExportFramesOptions) -> Result<ExportFramesResult, String> {
    if !opts.imp_dir.is_dir() {
        return Err(format!("IMP not found: {}", opts.imp_dir.display()));
    }

    let cpl = select_cpl(&opts.imp_dir, opts.cpl.as_deref())?;
    let cpl_path = opts.imp_dir.join(&cpl.file_path);

    let (segments, width, height) = plan_segments(&cpl_path)?;
    let total: usize = segments.iter().map(|s| s.count as usize).sum();
    if total == 0 {
        return Err("composition has no picture frames".into());
    }

    let start = opts.start as usize;
    if start >= total {
        return Err(format!(
            "--start {start} is past the composition's {total} picture frame(s)"
        ));
    }
    let end = match opts.count {
        Some(c) => (start + c as usize).min(total),
        None => total,
    };

    std::fs::create_dir_all(&opts.output_dir)
        .map_err(|e| format!("cannot create output {}: {e}", opts.output_dir.display()))?;

    let grk = postkit::grok::find_grk_decompress()
        .ok_or("grk_decompress not found (looked in $HOME/bin/grok/bin and PATH)")?;
    let lib_path = postkit::grok::grok_lib_path();

    let tmp = opts.output_dir.join(".imfwizard_export_tmp");
    std::fs::create_dir_all(&tmp).map_err(|e| format!("cannot create temp dir: {e}"))?;
    let result = decode_range(opts, &segments, start, end, &grk, &lib_path, &tmp);
    let _ = std::fs::remove_dir_all(&tmp);
    let frames_written = result?;

    Ok(ExportFramesResult {
        frames_written,
        output_dir: opts.output_dir.clone(),
        width,
        height,
    })
}

/// Decode composition frames [start, end) into numbered files. Returns the count written.
fn decode_range(
    opts: &ExportFramesOptions,
    segments: &[Segment],
    start: usize,
    end: usize,
    grk: &Path,
    lib_path: &str,
    tmp: &Path,
) -> Result<u32, String> {
    let mut buf = vec![0u8; 16 * 1024 * 1024];
    let mut global = 0usize;
    let mut out_no = 1u32;

    for seg in segments {
        let seg_start = global;
        let seg_end = global + seg.count as usize;
        let lo = start.max(seg_start);
        let hi = end.min(seg_end);
        if lo < hi {
            let mut reader = asdcplib::as02::jp2k::MxfReader::new();
            reader
                .open_read(&seg.mxf.to_string_lossy())
                .map_err(|e| format!("opening picture {}: {e}", seg.mxf.display()))?;

            for g in lo..hi {
                let src_index = seg.entry + (g - seg_start) as u32;
                let n = reader
                    .read_frame(src_index, &mut buf, None, None)
                    .map_err(|e| format!("reading J2K frame {src_index}: {e}"))?;

                let out = opts
                    .output_dir
                    .join(format!("frame_{out_no:06}.{}", opts.format.ext()));
                decode_frame(grk, lib_path, &buf[..n], tmp, &out)?;
                out_no += 1;
            }
        }
        global = seg_end;
    }

    Ok(out_no - 1)
}

/// Decode one J2K codestream to an image file via grk_decompress.
fn decode_frame(
    grk: &Path,
    lib_path: &str,
    j2c: &[u8],
    tmp: &Path,
    out: &Path,
) -> Result<(), String> {
    let src = tmp.join("frame.j2c");
    std::fs::write(&src, j2c).map_err(|e| format!("writing temp codestream: {e}"))?;

    let output = std::process::Command::new(grk)
        .env("LD_LIBRARY_PATH", lib_path)
        .args(["-i", &src.to_string_lossy(), "-o", &out.to_string_lossy()])
        .output()
        .map_err(|e| format!("spawning grk_decompress: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "grk_decompress could not decode a frame to {}: {}",
            out.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// Resolve the timeline into picture segments, rejecting encrypted essence and
/// dimension changes. Returns the segments plus the common picture dimensions.
fn plan_segments(cpl_path: &Path) -> Result<(Vec<Segment>, u32, u32), String> {
    let entries = timeline::get_timeline(cpl_path);
    let mut segments = Vec::new();
    let mut dims: Option<(u32, u32)> = None;

    for e in entries {
        if e.video_file.is_empty() {
            continue;
        }
        let mxf = PathBuf::from(&e.video_file);
        let probe = probe_picture(&mxf)?;
        if probe.encrypted {
            return Err(format!(
                "{} is encrypted; imfwizard has no KDM support, cannot decode",
                mxf.display()
            ));
        }
        match dims {
            Some((w, h)) if (w, h) != (probe.width, probe.height) => {
                return Err(format!(
                    "picture dimensions change across segments ({w}x{h} then {}x{}); \
                     not supported",
                    probe.width, probe.height
                ));
            }
            _ => dims = Some((probe.width, probe.height)),
        }

        let entry = e.entry_point as u32;
        let avail = probe.duration.saturating_sub(entry);
        let count = if e.duration_frames > 0 {
            (e.duration_frames as u32).min(avail)
        } else {
            avail
        };
        if count == 0 {
            continue;
        }
        segments.push(Segment { mxf, entry, count });
    }

    let (width, height) = dims.ok_or("composition has no picture track")?;
    Ok((segments, width, height))
}

struct PictureProbe {
    width: u32,
    height: u32,
    duration: u32,
    encrypted: bool,
}

/// Open an AS-02 J2K MXF and read its descriptor and encryption flag.
fn probe_picture(mxf: &Path) -> Result<PictureProbe, String> {
    let mut reader = asdcplib::as02::jp2k::MxfReader::new();
    reader
        .open_read(&mxf.to_string_lossy())
        .map_err(|e| format!("opening picture {}: {e}", mxf.display()))?;
    let encrypted = reader
        .writer_info()
        .map(|i| i.encrypted_essence)
        .unwrap_or(false);
    let d = reader
        .picture_descriptor()
        .map_err(|e| format!("reading picture descriptor for {}: {e}", mxf.display()))?;
    Ok(PictureProbe {
        width: d.stored_width,
        height: d.stored_height,
        duration: d.container_duration,
        encrypted,
    })
}

/// Select a CPL from the IMP by UUID or 0-based index, or the sole CPL when unspecified.
fn select_cpl(imp_dir: &Path, selector: Option<&str>) -> Result<timeline::CplInfo, String> {
    let cpls = timeline::list_cpls(imp_dir);
    if cpls.is_empty() {
        return Err(format!("no CPL found in {}", imp_dir.display()));
    }

    match selector {
        None => {
            if cpls.len() == 1 {
                Ok(cpls.into_iter().next().expect("len checked"))
            } else {
                let list = cpls
                    .iter()
                    .enumerate()
                    .map(|(i, c)| format!("  [{i}] {} {}", c.id, c.title))
                    .collect::<Vec<_>>()
                    .join("\n");
                Err(format!(
                    "IMP has {} CPLs; select one with --cpl <uuid-or-index>:\n{list}",
                    cpls.len()
                ))
            }
        }
        Some(sel) => {
            let want = sel.trim().trim_start_matches("urn:uuid:");
            if let Some(c) = cpls.iter().find(|c| c.id.eq_ignore_ascii_case(want)) {
                return Ok(c.clone());
            }
            if let Ok(idx) = sel.trim().parse::<usize>() {
                return cpls
                    .get(idx)
                    .cloned()
                    .ok_or_else(|| format!("--cpl index {idx} out of range (0..{})", cpls.len()));
            }
            Err(format!("no CPL matches --cpl {sel}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imp::{Composition, ImpOptions, create_imp};

    const W: u32 = 2048;
    const H: u32 = 1080;

    /// grk_compress/grk_decompress paths and lib dir.
    fn grok_tools() -> (PathBuf, PathBuf, String) {
        let compress = postkit::grok::find_grk_compress().expect("grk_compress on PATH");
        let decompress = postkit::grok::find_grk_decompress().expect("grk_decompress on PATH");
        (compress, decompress, postkit::grok::grok_lib_path())
    }

    /// Write an 8-bit RGB PPM whose pixels vary with a per-frame seed.
    fn write_ppm(path: &Path, seed: u8) {
        let mut data = format!("P6\n{W} {H}\n255\n").into_bytes();
        data.reserve((W * H * 3) as usize);
        for y in 0..H {
            for x in 0..W {
                data.push((x as u8).wrapping_add(seed));
                data.push((y as u8).wrapping_mul(2));
                data.push(seed);
            }
        }
        std::fs::write(path, data).unwrap();
    }

    /// The IMF profile and levels grk_compress is asked for, which is what an
    /// App 2E track file at 2048x1080 and 24 fps declares.
    const IMF_PROFILE_FLAG: &str = "2K,mainlevel=4,sublevel=2";

    /// Build `n` distinct J2K codestreams in `dir` via grk_compress. They carry
    /// an IMF Rsiz, since the AS-02 wrap takes no other family.
    fn make_j2c_frames(compress: &Path, lib: &str, dir: &Path, n: u8) {
        std::fs::create_dir_all(dir).unwrap();
        for i in 0..n {
            let ppm = dir.join(format!("src_{i}.ppm"));
            write_ppm(&ppm, i.wrapping_mul(37).wrapping_add(1));
            let j2c = dir.join(format!("frame_{i:04}.j2c"));
            let out = std::process::Command::new(compress)
                .env("LD_LIBRARY_PATH", lib)
                .args([
                    "-i",
                    &ppm.to_string_lossy(),
                    "-o",
                    &j2c.to_string_lossy(),
                    "-z",
                    IMF_PROFILE_FLAG,
                ])
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "grk_compress failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            std::fs::remove_file(&ppm).unwrap();
        }
    }

    /// create_imp over a J2K frame dir, returning the IMP directory.
    fn build_imp(j2k_dir: &Path, out_dir: &Path) {
        let opts = ImpOptions {
            output_dir: out_dir.to_path_buf(),
            compositions: vec![Composition {
                title: "Export Test".to_string(),
                content_kind: "feature".to_string(),
                j2k_dir: Some(j2k_dir.to_path_buf()),
                ..Default::default()
            }],
            fps_num: 24,
            fps_den: 1,
            edit_rate: "24 1".to_string(),
            duration: 0,
        };
        let r = create_imp(&opts);
        assert!(r.success, "create_imp failed: {}", r.error);
    }

    #[test]
    fn exports_full_sequence_and_subrange() {
        let (compress, _decompress, lib) = grok_tools();

        let tmp = tempfile::tempdir().unwrap();
        let frames = tmp.path().join("j2k");
        make_j2c_frames(&compress, &lib, &frames, 5);
        let imp = tmp.path().join("imp");
        build_imp(&frames, &imp);

        // full export
        let out = tmp.path().join("out_all");
        let r = export_frames(&ExportFramesOptions {
            imp_dir: imp.clone(),
            output_dir: out.clone(),
            format: ExportFormat::Tiff,
            cpl: None,
            start: 0,
            count: None,
        })
        .expect("export all");
        assert_eq!(r.frames_written, 5);
        assert_eq!((r.width, r.height), (W, H));
        for i in 1..=5 {
            let f = out.join(format!("frame_{i:06}.tif"));
            assert!(f.exists(), "missing {}", f.display());
            let img = postkit::grok::load_tiff(&f).expect("decode exported tiff");
            assert_eq!((img.width, img.height), (W, H));
        }
        assert!(!out.join("frame_000006.tif").exists());

        // subrange: composition frames [1, 3) -> two files, renumbered from 1
        let sub = tmp.path().join("out_sub");
        let r = export_frames(&ExportFramesOptions {
            imp_dir: imp,
            output_dir: sub.clone(),
            format: ExportFormat::Tiff,
            cpl: None,
            start: 1,
            count: Some(2),
        })
        .expect("export subrange");
        assert_eq!(r.frames_written, 2);
        assert!(sub.join("frame_000001.tif").exists());
        assert!(sub.join("frame_000002.tif").exists());
        assert!(!sub.join("frame_000003.tif").exists());
        let img = postkit::grok::load_tiff(&sub.join("frame_000002.tif")).unwrap();
        assert_eq!((img.width, img.height), (W, H));
    }

    #[test]
    fn rejects_encrypted_essence() {
        let (compress, _decompress, lib) = grok_tools();

        let tmp = tempfile::tempdir().unwrap();
        let frames = tmp.path().join("j2k");
        make_j2c_frames(&compress, &lib, &frames, 2);
        let imp = tmp.path().join("imp");
        build_imp(&frames, &imp);

        // overwrite the wrapped picture MXF with an encrypted AS-02 J2K MXF
        let video = std::fs::read_dir(&imp)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("VIDEO_"))
            })
            .expect("wrapped VIDEO mxf");
        let j2c = std::fs::read(frames.join("frame_0000.j2c")).unwrap();
        write_encrypted_mxf(&j2c, &video, 2);

        let err = export_frames(&ExportFramesOptions {
            imp_dir: imp,
            output_dir: tmp.path().join("out"),
            format: ExportFormat::Tiff,
            cpl: None,
            start: 0,
            count: None,
        })
        .expect_err("encrypted essence must be rejected");
        assert!(
            err.contains("encrypted") && err.contains("KDM"),
            "unexpected error: {err}"
        );
    }

    /// Write an encrypted AS-02 J2K MXF carrying `frames` copies of one codestream.
    ///
    /// The descriptor's sub-descriptor is read off `j2c` itself, so the MXF
    /// describes the essence it holds rather than a size and depth chosen here.
    fn write_encrypted_mxf(j2c: &[u8], out: &Path, frames: u32) {
        use asdcplib::crypto::{AesEncContext, HmacContext};
        use asdcplib::jp2k::{CodestreamHeader, PictureDescriptor};
        use asdcplib::{LabelSet, Rational, WriterInfo};

        let desc = PictureDescriptor {
            edit_rate: Rational::new(24, 1),
            sample_rate: Rational::new(24, 1),
            stored_width: W,
            stored_height: H,
            aspect_ratio: Rational::new(W as i32, H as i32),
            container_duration: frames,
            codestream: CodestreamHeader::parse(j2c).expect("read the codestream header"),
        };
        let info = WriterInfo {
            asset_uuid: [0x11; 16],
            context_id: [0x22; 16],
            cryptographic_key_id: [0x33; 16],
            encrypted_essence: true,
            uses_hmac: true,
            ..Default::default()
        };
        let key = [0x44u8; 16];
        let mut enc = AesEncContext::new();
        enc.init_key(&key).unwrap();
        let mut hmac = HmacContext::new();
        hmac.init_key(&key, LabelSet::Smpte).unwrap();

        let mut w = asdcplib::as02::jp2k::MxfWriter::new();
        w.open_write(&out.to_string_lossy(), &info, &desc, 16384)
            .unwrap();
        for _ in 0..frames {
            w.write_frame(j2c, Some(&mut enc), Some(&mut hmac)).unwrap();
        }
        w.finalize().unwrap();
    }

    #[test]
    fn format_flag_parsing() {
        assert_eq!(ExportFormat::from_flag("tiff").unwrap(), ExportFormat::Tiff);
        assert_eq!(ExportFormat::from_flag("TIF").unwrap(), ExportFormat::Tiff);
        assert_eq!(ExportFormat::from_flag("png").unwrap(), ExportFormat::Png);
        assert!(ExportFormat::from_flag("dpx").unwrap_err().contains("dpx"));
        assert!(ExportFormat::from_flag("gif").is_err());
    }
}
