//! A single image held for a given duration, in place of a video file or a
//! directory of frames.
//!
//! The image is encoded once and the codestream is then repeated, so holding a
//! still for an hour costs one encode rather than 86400.

use std::path::{Path, PathBuf};

/// Where the lone image is staged for the encoder, under the job's work directory.
const STILL_SOURCE_DIR: &str = "still_source";

/// Where the still's own encode runs, kept apart from a video encode's `j2k`
/// directory so a re-run cannot find a previous still's codestream beside the
/// new one.
const STILL_ENCODE_DIR: &str = "still_encode";

/// Where the repeated codestreams are written, under the job's output directory.
pub const HELD_PICTURE_DIR: &str = "j2k_still";

/// Scratch paths for one still: the directory the encoder reads and the one it
/// writes into. Both start empty on every run.
pub struct StillScratch {
    pub source_dir: PathBuf,
    pub encode_dir: PathBuf,
}

/// Is this a lone image file the encoder can read, rather than a video or a
/// directory of frames?
pub fn is_still_image(path: &Path) -> bool {
    path.is_file()
        && crate::encode::detect_image_format(path) != crate::encode::ImageFormat::Unknown
}

/// Stage the still in a directory of its own, so the image-sequence encoder sees
/// one frame rather than everything else sitting beside it in its folder.
pub fn prepare_still_source(image: &Path, work_dir: &Path) -> Result<StillScratch, String> {
    let Some(name) = image.file_name() else {
        return Err(format!("still input has no file name: {}", image.display()));
    };
    let source_dir = crate::source_edits::fresh_dir(&work_dir.join(STILL_SOURCE_DIR))?;
    crate::source_edits::link_or_copy(image, &source_dir.join(name))?;
    Ok(StillScratch {
        encode_dir: crate::source_edits::fresh_dir(&work_dir.join(STILL_ENCODE_DIR))?,
        source_dir,
    })
}

/// Repeat the encoded still so the picture track runs for `frame_count` frames.
pub fn hold_frames(encoded: &Path, frame_count: u64, dest: &Path) -> Result<PathBuf, String> {
    if frame_count == 0 {
        return Err("a still has to be held for at least one frame".to_string());
    }
    let mut codestreams: Vec<PathBuf> = std::fs::read_dir(encoded)
        .map_err(|e| format!("cannot read {}: {e}", encoded.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("j2c" | "j2k")
                )
        })
        .collect();
    codestreams.sort();

    let [codestream] = codestreams.as_slice() else {
        return Err(format!(
            "expected one encoded still in {}, found {}",
            encoded.display(),
            codestreams.len()
        ));
    };

    crate::source_edits::write_frame_dir(&vec![codestream.clone(); frame_count as usize], dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_files_are_stills_and_videos_are_not() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["frame.tif", "frame.dpx", "frame.png", "frame.exr"] {
            let path = dir.path().join(name);
            std::fs::write(&path, b"x").unwrap();
            assert!(is_still_image(&path), "{name} should be a still");
        }
        for name in ["movie.mov", "movie.mp4", "frame.j2c"] {
            let path = dir.path().join(name);
            std::fs::write(&path, b"x").unwrap();
            assert!(!is_still_image(&path), "{name} should not be a still");
        }
        assert!(!is_still_image(dir.path()), "a directory is not a still");
    }

    #[test]
    fn staging_leaves_the_encoder_exactly_one_frame() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("slate.tif");
        std::fs::write(&image, b"tiff").unwrap();
        // a neighbour the encoder must not pick up
        std::fs::write(dir.path().join("other.tif"), b"tiff").unwrap();

        let scratch = prepare_still_source(&image, &dir.path().join("work")).unwrap();
        let frames = crate::encode::find_source_frames(&scratch.source_dir).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].file_name().unwrap(), "slate.tif");
        assert!(scratch.encode_dir.is_dir());
    }

    /// A second run with a different image must not leave the first one staged,
    /// or the encoder would find two frames where a still has one.
    #[test]
    fn staging_a_second_still_clears_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("work");
        for name in ["first.tif", "second.dpx"] {
            let image = dir.path().join(name);
            std::fs::write(&image, b"image").unwrap();
            let scratch = prepare_still_source(&image, &work).unwrap();
            let frames = crate::encode::find_source_frames(&scratch.source_dir).unwrap();
            assert_eq!(frames.len(), 1, "{name} staged beside an earlier still");
            assert_eq!(frames[0].file_name().unwrap(), name);
        }
    }

    #[test]
    fn the_still_is_repeated_for_the_whole_duration() {
        let dir = tempfile::tempdir().unwrap();
        let encoded = dir.path().join("j2k");
        std::fs::create_dir_all(&encoded).unwrap();
        std::fs::write(encoded.join("slate.j2k"), b"codestream").unwrap();

        let held = hold_frames(&encoded, 48, &dir.path().join("held")).unwrap();
        let frames = crate::source_edits::picture_frames(&held).unwrap();
        assert_eq!(frames.len(), 48);
        assert!(
            frames
                .iter()
                .all(|f| std::fs::read(f).unwrap() == b"codestream")
        );
    }

    #[test]
    fn a_zero_length_hold_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let encoded = dir.path().join("j2k");
        std::fs::create_dir_all(&encoded).unwrap();
        std::fs::write(encoded.join("slate.j2k"), b"codestream").unwrap();
        assert!(hold_frames(&encoded, 0, &dir.path().join("held")).is_err());
    }
}
