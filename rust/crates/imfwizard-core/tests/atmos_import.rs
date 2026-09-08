//! An ADM master, plain RIFF and BW64, imported and read back out of the MXF.

use imfwizard_core::atmos::{
    BwfForm, SYNTHETIC_ADM_BITS, SYNTHETIC_ADM_CHANNELS, SYNTHETIC_ADM_SAMPLE_RATE,
    extract_axml_chunk, import_atmos, synthetic_adm_bwf, synthetic_adm_sample, synthetic_adm_xml,
};

/// Just under a second, and no whole number of frames at either rate, so the samples the
/// wrap keeps say which edit rate it used.
const SAMPLE_FRAMES: u32 = 47_000;

fn write_master(dir: &std::path::Path, form: BwfForm) -> std::path::PathBuf {
    let name = match form {
        BwfForm::Riff => "riff_master.wav",
        BwfForm::Bw64 => "bw64_master.wav",
    };
    let path = dir.join(name);
    std::fs::write(&path, synthetic_adm_bwf(form, SAMPLE_FRAMES)).unwrap();
    path
}

#[test]
fn axml_comes_back_whole_from_both_riff_forms() {
    let dir = tempfile::tempdir().unwrap();
    let xml = synthetic_adm_xml();
    for form in [BwfForm::Riff, BwfForm::Bw64] {
        let master = write_master(dir.path(), form);
        assert_eq!(extract_axml_chunk(&master).unwrap(), xml, "{form:?}");
    }
}

/// A BW64 master keeps its data and axml sizes in the ds64 chunk, so a reader that
/// takes the 32-bit fields at face value sees 4 GiB chunks and finds no axml at all.
#[test]
fn a_bw64_master_routes_its_sizes_through_ds64() {
    let bw64 = synthetic_adm_bwf(BwfForm::Bw64, SAMPLE_FRAMES);
    assert_eq!(&bw64[..4], b"BW64");
    assert_eq!(&bw64[4..8], &0xFFFF_FFFFu32.to_le_bytes());
    assert_eq!(&bw64[12..16], b"ds64");
    let axml_size_fields = bw64
        .windows(8)
        .filter(|w| {
            (&w[..4] == b"axml" || &w[..4] == b"data") && w[4..] == 0xFFFF_FFFFu32.to_le_bytes()
        })
        .count();
    assert_eq!(axml_size_fields, 2, "data and axml both defer to ds64");
}

#[test]
fn a_file_without_an_axml_chunk_names_the_chunk() {
    let dir = tempfile::tempdir().unwrap();
    let master = write_master(dir.path(), BwfForm::Riff);
    let mut bytes = std::fs::read(&master).unwrap();
    let axml_at = bytes
        .windows(4)
        .position(|w| w == b"axml")
        .expect("fixture carries an axml chunk");
    bytes[axml_at..axml_at + 4].copy_from_slice(b"junk");
    let stripped = dir.path().join("no_axml.wav");
    std::fs::write(&stripped, &bytes).unwrap();

    let error = extract_axml_chunk(&stripped).unwrap_err();
    assert!(error.contains("axml"), "{error}");
}

/// A chunk claiming more bytes than the file holds is refused, not read past.
#[test]
fn a_chunk_running_past_the_end_of_the_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let master = write_master(dir.path(), BwfForm::Riff);
    let mut bytes = std::fs::read(&master).unwrap();
    let data_at = bytes.windows(4).position(|w| w == b"data").unwrap();
    let overlong = bytes.len() as u32;
    bytes[data_at + 4..data_at + 8].copy_from_slice(&overlong.to_le_bytes());
    let truncated = dir.path().join("overlong.wav");
    std::fs::write(&truncated, &bytes).unwrap();

    let error = extract_axml_chunk(&truncated).unwrap_err();
    assert!(error.contains("data"), "{error}");
    assert!(error.contains("past the end"), "{error}");
}

/// The whole import, on both forms: the sidecar is the chunk, and the MXF carries the
/// fixture's channels, sample rate, frame count and samples.
#[test]
fn an_imported_master_wraps_its_channels_and_samples() {
    let dir = tempfile::tempdir().unwrap();
    let xml = synthetic_adm_xml();
    for (form, fps_num, fps_den) in [(BwfForm::Riff, 24, 1), (BwfForm::Bw64, 25, 1)] {
        let master = write_master(dir.path(), form);
        let output = dir.path().join(format!("out_{fps_num}"));
        let result = import_atmos(&master, &output, fps_num, fps_den);
        assert!(result.success, "{form:?}: {}", result.error);
        assert_eq!(result.bed_count, 10);
        assert_eq!(result.object_count, 2);
        assert_eq!(result.total_channels, SYNTHETIC_ADM_CHANNELS as u32);
        assert_eq!(std::fs::read_to_string(&result.adm_sidecar).unwrap(), xml);

        let mut reader = asdcplib::as02::pcm::MxfReader::new();
        reader
            .open_read(
                &result.mxf_output.to_string_lossy(),
                asdcplib::Rational::new(fps_num as i32, fps_den as i32),
            )
            .expect("open the wrapped MXF");
        let wave = reader.wave_audio_descriptor().expect("wave descriptor");
        assert_eq!(wave.channel_count, SYNTHETIC_ADM_CHANNELS as u32);
        assert_eq!(wave.quantization_bits, SYNTHETIC_ADM_BITS as u32);
        assert_eq!(
            wave.audio_sampling_rate,
            asdcplib::Rational::new(SYNTHETIC_ADM_SAMPLE_RATE as i32, 1)
        );

        // PCM is clip-wrapped, so ContainerDuration counts samples: the whole frames the
        // edit rate makes of the master, 23 at 24 fps and 24 at 25 fps
        let samples_per_frame = (SYNTHETIC_ADM_SAMPLE_RATE * fps_den).div_ceil(fps_num);
        let frames = SAMPLE_FRAMES / samples_per_frame;
        assert_eq!(
            wave.container_duration,
            Some((frames * samples_per_frame) as u64),
            "{form:?} at {fps_num}/{fps_den}"
        );

        let block_align = wave.block_align as usize;
        let mut essence = vec![0u8; samples_per_frame as usize * block_align];
        assert_eq!(
            reader.read_frame(frames - 1, &mut essence, None, None).ok(),
            Some(essence.len()),
            "the last frame the edit rate implies"
        );
        assert!(
            reader.read_frame(frames, &mut essence, None, None).is_err(),
            "one frame past the end"
        );

        let read = reader.read_frame(0, &mut essence, None, None).unwrap();
        assert_eq!(read, essence.len(), "a whole frame of PCM");
        for channel in 0..SYNTHETIC_ADM_CHANNELS {
            let at = channel as usize * (SYNTHETIC_ADM_BITS / 8) as usize;
            let sample =
                i32::from_le_bytes([0, essence[at], essence[at + 1], essence[at + 2]]) >> 8;
            assert_eq!(sample, synthetic_adm_sample(channel), "channel {channel}");
        }
    }
}
