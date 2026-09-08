use std::path::{Path, PathBuf};

// where the sound demuxed from the picture source is written
pub const DEMUXED_AUDIO_NAME: &str = "audio_demux.wav";

// each composition a GUI job builds gets its own scratch directory under this
pub const ENCODE_SCRATCH_PREFIX: &str = "enc_";

// every scratch directory a create run writes inside the output directory
const INTERMEDIATE_DIRECTORIES: [&str; 3] = [
    "j2k",
    postkit::still::HELD_PICTURE_DIR,
    crate::source_edits::TRIMMED_PICTURE_DIR,
];

// every scratch file a create run writes inside the output directory
const INTERMEDIATE_FILES: [&str; 2] = [DEMUXED_AUDIO_NAME, crate::audio_map::MAPPED_AUDIO_NAME];

// scratch named one per composition, per sound file or per subtitle file
const INTERMEDIATE_PREFIXES: [&str; 5] = [
    ENCODE_SCRATCH_PREFIX,
    crate::source_edits::DELAYED_AUDIO_PREFIX,
    crate::source_edits::TRIMMED_AUDIO_PREFIX,
    crate::source_edits::FITTED_AUDIO_PREFIX,
    crate::source_edits::TRIMMED_SUBTITLE_PREFIX,
];

pub fn remove_intermediates(output_dir: &Path, keep: &[&Path]) {
    let kept: Vec<PathBuf> = keep.iter().map(|path| resolve(path)).collect();
    for name in INTERMEDIATE_DIRECTORIES {
        remove(&output_dir.join(name), &kept);
    }
    for name in INTERMEDIATE_FILES {
        remove(&output_dir.join(name), &kept);
    }
    let Ok(entries) = std::fs::read_dir(output_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if INTERMEDIATE_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            remove(&entry.path(), &kept);
        }
    }
}

fn remove(path: &Path, kept: &[PathBuf]) {
    if kept.contains(&resolve(path)) {
        return;
    }
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

fn resolve(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scratch_goes_and_the_package_stays() {
        let dir = tempfile::TempDir::new().unwrap();
        let output = dir.path();
        for name in INTERMEDIATE_DIRECTORIES {
            std::fs::create_dir_all(output.join(name)).unwrap();
            std::fs::write(output.join(name).join("frame_00000000.j2c"), [0u8]).unwrap();
        }
        for name in INTERMEDIATE_FILES {
            std::fs::write(output.join(name), [0u8]).unwrap();
        }
        let scratch = output.join(format!("{ENCODE_SCRATCH_PREFIX}0"));
        std::fs::create_dir_all(scratch.join("j2k")).unwrap();
        let indexed = [
            format!("{}1.wav", crate::source_edits::DELAYED_AUDIO_PREFIX),
            format!("{}1.wav", crate::source_edits::TRIMMED_AUDIO_PREFIX),
            format!("{}1.xml", crate::source_edits::TRIMMED_SUBTITLE_PREFIX),
        ];
        for name in &indexed {
            std::fs::write(output.join(name), [0u8]).unwrap();
        }
        let package = ["ASSETMAP.xml", "CPL_1.xml", "PKL_1.xml", "VIDEO_1.mxf"];
        for name in package {
            std::fs::write(output.join(name), [0u8]).unwrap();
        }

        remove_intermediates(output, &[]);

        for name in INTERMEDIATE_DIRECTORIES {
            assert!(!output.join(name).exists(), "{name} survived the cleanup");
        }
        for name in INTERMEDIATE_FILES {
            assert!(!output.join(name).exists(), "{name} survived the cleanup");
        }
        assert!(!scratch.exists(), "a composition's scratch survived");
        for name in &indexed {
            assert!(!output.join(name).exists(), "{name} survived the cleanup");
        }
        for name in package {
            assert!(output.join(name).exists(), "{name} was not part of the IMP");
        }
    }

    #[test]
    fn a_codestream_directory_handed_in_is_never_deleted() {
        let dir = tempfile::TempDir::new().unwrap();
        let output = dir.path();
        let source = output.join("j2k");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("frame_00000000.j2c"), [0u8]).unwrap();
        std::fs::create_dir_all(output.join(crate::source_edits::TRIMMED_PICTURE_DIR)).unwrap();

        remove_intermediates(output, &[&source]);

        assert!(source.join("frame_00000000.j2c").exists());
        assert!(
            !output
                .join(crate::source_edits::TRIMMED_PICTURE_DIR)
                .exists()
        );
    }
}
