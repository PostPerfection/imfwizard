//! A qualifying create writes its picture MXF while the encode runs, and the IMP
//! that comes out is the one the encode-then-wrap path used to produce: same
//! codestreams in the essence, same CPL and PKL over it.

use imfwizard_core::imp::{Composition, ImpOptions, create_imp};
use imfwizard_core::overlapped_picture::{
    PictureJob, PictureWrapTarget, encode_and_wrap_picture, overlap_refusal,
};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// App 2E's smallest legal raster, so the picture the overlapped wrap writes is
/// one `create_imp` would have accepted from the sequential wrap too.
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FRAMES: u64 = 4;
const FPS: u32 = 24;

/// The IMF Rsiz this raster and rate compose, which is what `create` encodes at.
fn imf_rsiz() -> u16 {
    imfwizard_core::encode::imf_rsiz_for_encode(
        WIDTH,
        HEIGHT,
        FPS as f64,
        imfwizard_core::encode::bitrate_mbps_for_job(None, None, WIDTH, HEIGHT, FPS as f64),
    )
    .unwrap()
}

fn have_ffmpeg() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn make_clip(path: &Path) {
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc=s={WIDTH}x{HEIGHT}:r={FPS}"),
            "-frames:v",
            &FRAMES.to_string(),
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(path)
        .output()
        .expect("ffmpeg");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// One frame of a picture MXF, read back through asdcplib's AS-02 reader.
fn essence_frame(reader: &mut asdcplib::as02::jp2k::MxfReader, index: u32) -> Vec<u8> {
    let mut buf = vec![0u8; 16 << 20];
    let read = reader.read_frame(index, &mut buf, None, None).unwrap();
    buf.truncate(read);
    buf
}

fn between(haystack: &str, start: &str, end: &str) -> String {
    let from = haystack.find(start).expect(start) + start.len();
    let rest = &haystack[from..];
    rest[..rest.find(end).expect(end)].to_string()
}

#[test]
fn a_video_create_wraps_its_picture_during_the_encode() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("clip.mp4");
    make_clip(&video);
    let encode_dir = dir.path().join("enc");
    let imp_dir = dir.path().join("imp");

    // the same job the CLI and the GUI hand the predicate for a plain video
    assert_eq!(
        overlap_refusal(&PictureJob {
            input_type: postkit::encode::detect_input_type(&video),
            still_hold: false,
        }),
        None
    );

    let hdr = imfwizard_core::hdr_wcg::HdrWcg::from_flags(
        "pq-bt2020",
        Some("R(34000,16000)G(13250,34500)B(7500,3000)WP(15635,16450)L(40000000,50)"),
    )
    .unwrap()
    .with_content_light_levels(Some(993), Some(362));

    let (encode, track) = encode_and_wrap_picture(
        &video,
        &encode_dir,
        &postkit::pipeline::EncodeRunOptions {
            fps: postkit::encode::FrameRate::whole(FPS),
            source_colour: postkit::encode::SourceColour::KeepRgb,
            rsiz: imf_rsiz(),
            ..Default::default()
        },
        PictureWrapTarget {
            imp_dir: imp_dir.clone(),
            fps_num: FPS,
            fps_den: 1,
            colour: imfwizard_core::mxf_wrap::picture_colour(Some(&hdr)),
        },
        &Arc::new(AtomicBool::new(false)),
        &Arc::new(AtomicBool::new(false)),
        |_| {},
        |_| {},
    )
    .expect("overlapped encode and wrap");

    assert_eq!(encode.frames_encoded, FRAMES);
    assert_eq!(track.duration, FRAMES);
    assert!(
        track
            .path
            .file_name()
            .and_then(|f| f.to_str())
            .is_some_and(|n| n.starts_with(imfwizard_core::imp::PICTURE_PREFIX)),
        "the CPL reads the track kind off the file name: {}",
        track.path.display()
    );

    let result = create_imp(&ImpOptions {
        output_dir: imp_dir.clone(),
        compositions: vec![Composition {
            title: "Overlapped".into(),
            content_kind: "feature".into(),
            j2k_dir: Some(encode.j2k_dir.clone()),
            picture_mxf: Some(track.clone()),
            hdr: Some(hdr),
            ..Default::default()
        }],
        fps_num: FPS,
        fps_den: 1,
        ..Default::default()
    });
    assert!(result.success, "create_imp failed: {}", result.error);

    // the picture MXF was not re-wrapped: the package points at the one the
    // encode wrote, and nothing else was left in the IMP directory
    let pictures: Vec<_> = std::fs::read_dir(&imp_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(imfwizard_core::imp::PICTURE_PREFIX))
        .collect();
    assert_eq!(
        pictures,
        vec![track.path.file_name().unwrap().to_string_lossy()]
    );

    let mut reader = asdcplib::as02::jp2k::MxfReader::new();
    reader
        .open_read(&track.path.to_string_lossy())
        .expect("the picture MXF opens");
    assert_eq!(
        reader.picture_descriptor().unwrap().container_duration,
        FRAMES as u32,
        "asdcplib writes the real duration at finalize"
    );

    // the essence holds the codestreams the encoder wrote, in index order: the
    // wrap sees them in completion order, so a reordering slip shows up here
    for index in 0..FRAMES {
        let codestream = encode.j2k_dir.join(format!("frame_{index:08}.j2c"));
        assert_eq!(
            essence_frame(&mut reader, index as u32),
            std::fs::read(&codestream).unwrap(),
            "frame {index} of the MXF is not {}",
            codestream.display()
        );
    }

    // HDR/WCG reaches the incremental wrap the way it reaches the sequential one
    let md = reader.hdr_metadata().expect("read hdr metadata");
    assert_eq!(
        md.transfer_characteristic,
        Some(asdcplib::jp2k::TRANSFER_CHARACTERISTIC_ST2084)
    );
    assert_eq!(
        md.color_primaries,
        Some(asdcplib::jp2k::COLOR_PRIMARIES_BT2020)
    );
    assert_eq!(md.mastering_display_max_luminance, Some(40000000));

    let cpl = std::fs::read_to_string(&result.cpl_paths[0]).unwrap();
    assert!(cpl.contains("<r0:RGBADescriptor"));
    assert!(cpl.contains("<SourceEncoding>"));
    assert!(
        cpl.contains(&track.uuid),
        "the CPL does not name the picture track file"
    );
    assert_eq!(
        between(&cpl, "<IntrinsicDuration>", "</IntrinsicDuration>"),
        FRAMES.to_string()
    );

    // the PKL hash has to be of the file the overlapped wrap left on disk
    let pkl = std::fs::read_to_string(&result.pkl_path).unwrap();
    assert!(
        pkl.contains(&track.hash),
        "the PKL does not carry the picture hash"
    );
    assert_eq!(
        std::fs::metadata(&track.path).unwrap().len(),
        track.size,
        "the track file size is not the file on disk"
    );
}
