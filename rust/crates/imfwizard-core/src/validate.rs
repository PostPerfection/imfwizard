use serde::{Deserialize, Serialize};
use std::path::Path;

/// IMP validation result.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    /// What the checks measured rather than judged, such as the picture bitrate
    /// the deeper pass reads off the essence. The QC report lists these.
    pub infos: Vec<String>,
}

/// Validate an IMP directory for structural correctness.
///
/// Uses dcpdoctor-core for ASSETMAP/PKL/hash verification (shared structure
/// between DCP and IMP), plus IMF-specific checks.
pub fn validate_imp(imp_dir: &Path) -> ValidationResult {
    validate_imp_with_photon(imp_dir, None)
}

// dcpdoctor reads PHOTON_DIR and its own cache, neither of which is where the wizard puts Photon
pub fn validate_imp_with_photon(imp_dir: &Path, photon: Option<&Path>) -> ValidationResult {
    let options = dcpdoctor_core::VerifyOptions {
        photon: crate::photon::photon_path(photon),
        ..dcpdoctor_core::VerifyOptions::standard()
    };
    validate_imp_with_options(imp_dir, options)
}

/// Validate an IMP for the QC report, which reads the picture essence itself.
///
/// The frame-by-frame checks are the expensive part of a verify, so the report
/// pays for them and the plain validate stays fast.
pub fn validate_imp_for_report(imp_dir: &Path) -> ValidationResult {
    let options = dcpdoctor_core::VerifyOptions {
        check_picture_details: true,
        scan_every_frame: true,
        photon: crate::photon::photon_path(None),
        ..dcpdoctor_core::VerifyOptions::standard()
    };
    validate_imp_with_options(imp_dir, options)
}

fn validate_imp_with_options(
    imp_dir: &Path,
    options: dcpdoctor_core::VerifyOptions,
) -> ValidationResult {
    let mut result = ValidationResult {
        valid: true,
        ..Default::default()
    };

    if !imp_dir.is_dir() {
        result.valid = false;
        result
            .errors
            .push(format!("Not a directory: {}", imp_dir.display()));
        return result;
    }

    // Use dcpdoctor-core for structural validation (ASSETMAP, PKL, hashes)
    let verify_result = dcpdoctor_core::verify(imp_dir, &options);

    for note in &verify_result.notes {
        match note.severity {
            dcpdoctor_core::Severity::Error => {
                result.valid = false;
                result.errors.push(note.message.clone());
            }
            dcpdoctor_core::Severity::Warning => {
                result.warnings.push(note.message.clone());
            }
            dcpdoctor_core::Severity::Info => {
                result.infos.push(note.message.clone());
            }
        }
    }

    // IMF-specific checks: ensure at least one CPL exists
    let cpls = crate::timeline::list_cpls(imp_dir);
    if cpls.is_empty() && !result.errors.iter().any(|e| e.contains("CPL")) {
        result.valid = false;
        result
            .errors
            .push("No Composition Playlist (CPL) found".to_string());
    }

    // Check MXF files are present
    let mxf_count = std::fs::read_dir(imp_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("mxf"))
                })
                .count()
        })
        .unwrap_or(0);
    if mxf_count == 0 {
        result
            .warnings
            .push("No MXF essence files found".to_string());
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const WIDTH: u32 = 1920;
    const HEIGHT: u32 = 1080;
    const BIT_DEPTH: u8 = 12;

    const CPL_PREFIX: &str = "CPL_";
    const PKL_PREFIX: &str = "PKL_";

    // only reached once the CPL has been read as an App 2E composition, so a
    // clean result without it means the IMF pass never ran
    const APP2E_PASS_RAN: &str = "App 2E: no MainAudioSequence found";

    fn create_good_imp(work_dir: &Path) -> PathBuf {
        let codestream_dir = work_dir.join("codestreams");
        std::fs::create_dir_all(&codestream_dir).unwrap();
        std::fs::write(
            codestream_dir.join("0001.j2c"),
            crate::mxf_wrap::synthetic_j2k_codestream(WIDTH, HEIGHT, BIT_DEPTH),
        )
        .unwrap();

        let output_dir = work_dir.join("imp");
        let options = crate::imp::ImpOptions {
            output_dir: output_dir.clone(),
            compositions: vec![crate::imp::Composition {
                title: "Validate".to_string(),
                content_kind: "feature".to_string(),
                j2k_dir: Some(codestream_dir),
                ..Default::default()
            }],
            fps_num: 24,
            fps_den: 1,
            ..Default::default()
        };
        let result = crate::imp::create_imp(&options);
        assert!(result.success, "create_imp failed: {}", result.error);
        output_dir
    }

    fn only_file_starting_with(imp_dir: &Path, prefix: &str) -> PathBuf {
        let mut found: Vec<PathBuf> = std::fs::read_dir(imp_dir)
            .unwrap()
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(prefix))
            })
            .collect();
        assert_eq!(found.len(), 1, "expected one {prefix} file in {imp_dir:?}");
        found.pop().unwrap()
    }

    fn replace_once(xml: &str, from: &str, to: &str) -> String {
        assert!(
            xml.contains(from),
            "the written XML no longer holds {from:?}"
        );
        xml.replacen(from, to, 1)
    }

    fn cut_between(xml: &str, open: &str, close: &str) -> String {
        let start = xml.find(open).unwrap_or_else(|| panic!("no {open}"));
        let end = xml.find(close).unwrap_or_else(|| panic!("no {close}")) + close.len();
        format!("{}{}", &xml[..start], &xml[end..])
    }

    fn edited_imp(work_dir: &Path, prefix: &str, edit: impl FnOnce(&str) -> String) -> PathBuf {
        let imp_dir = create_good_imp(work_dir);
        let target = only_file_starting_with(&imp_dir, prefix);
        let edited = edit(&std::fs::read_to_string(&target).unwrap());
        std::fs::write(&target, edited).unwrap();
        imp_dir
    }

    fn assert_error(imp_dir: &Path, needle: &str) {
        let result = validate_imp(imp_dir);
        assert!(!result.valid, "the package validated clean: {result:?}");
        assert!(
            result.errors.iter().any(|error| error.contains(needle)),
            "no error mentions {needle:?}, errors: {:?}",
            result.errors
        );
    }

    fn assert_warning(imp_dir: &Path, needle: &str) {
        let result = validate_imp(imp_dir);
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains(needle)),
            "no warning mentions {needle:?}, warnings: {:?}",
            result.warnings
        );
    }

    #[test]
    fn a_created_imp_validates_clean() {
        let work = tempfile::tempdir().unwrap();
        let imp = create_good_imp(work.path());
        let result = validate_imp(&imp);

        assert!(result.valid, "errors: {:?}", result.errors);
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        assert!(
            result.warnings.iter().any(|w| w.contains(APP2E_PASS_RAN)),
            "the App 2E pass left no trace, so a clean result proves nothing: {result:?}"
        );
        assert!(
            !result
                .warnings
                .iter()
                .any(|w| w.contains("No MXF essence files")),
            "the picture track file is missing: {result:?}"
        );
    }

    #[test]
    fn a_path_that_is_not_a_directory_is_rejected() {
        let work = tempfile::tempdir().unwrap();
        let missing = work.path().join("nowhere");
        assert_error(&missing, "Not a directory");
    }

    #[test]
    fn an_empty_directory_is_rejected_for_having_no_composition() {
        let work = tempfile::tempdir().unwrap();
        assert_error(work.path(), "No Composition Playlist (CPL) found");
    }

    #[test]
    fn a_package_whose_cpl_was_deleted_is_rejected() {
        let work = tempfile::tempdir().unwrap();
        let imp = create_good_imp(work.path());
        std::fs::remove_file(only_file_starting_with(&imp, CPL_PREFIX)).unwrap();
        assert_error(&imp, "No valid CPL found");
    }

    #[test]
    fn a_cpl_with_no_edit_rate_is_rejected() {
        let work = tempfile::tempdir().unwrap();
        // the composition edit rate comes first, the resource keeps its own
        let imp = edited_imp(work.path(), CPL_PREFIX, |xml| {
            replace_once(xml, "<EditRate>24 1</EditRate>", "")
        });
        assert_error(&imp, "CPL has no EditRate");
    }

    #[test]
    fn a_cpl_with_no_picture_track_is_rejected() {
        let work = tempfile::tempdir().unwrap();
        let imp = edited_imp(work.path(), CPL_PREFIX, |xml| {
            cut_between(xml, "<SequenceList>", "</SequenceList>")
        });
        assert_error(&imp, "CPL has no MainImageSequence virtual track");
    }

    #[test]
    fn a_cpl_with_no_segments_is_rejected() {
        let work = tempfile::tempdir().unwrap();
        let imp = edited_imp(work.path(), CPL_PREFIX, |xml| {
            cut_between(xml, "<SegmentList>", "</SegmentList>")
        });
        assert_error(&imp, "CPL has no Segment elements");
    }

    #[test]
    fn a_malformed_uuid_in_the_cpl_is_rejected() {
        let work = tempfile::tempdir().unwrap();
        let imp = edited_imp(work.path(), CPL_PREFIX, |xml| {
            let segment_id = between(xml, "<Segment>\n      <Id>urn:uuid:", "</Id>");
            replace_once(xml, &segment_id, "not-a-uuid")
        });
        assert_error(&imp, "Malformed UUID: 'not-a-uuid'");
    }

    #[test]
    fn a_resource_longer_than_its_intrinsic_duration_is_rejected() {
        let work = tempfile::tempdir().unwrap();
        let imp = edited_imp(work.path(), CPL_PREFIX, |xml| {
            replace_once(
                xml,
                "<SourceDuration>1</SourceDuration>",
                "<SourceDuration>9</SourceDuration>",
            )
        });
        assert_error(&imp, "source range exceeds intrinsic duration");
    }

    #[test]
    fn a_cpl_the_packing_list_does_not_name_is_rejected() {
        let work = tempfile::tempdir().unwrap();
        let imp = edited_imp(work.path(), CPL_PREFIX, |xml| {
            let cpl_id = between(xml, "<Id>urn:uuid:", "</Id>");
            replace_once(xml, &cpl_id, "3fd0f2a0-0000-4000-8000-000000000000")
        });
        assert_error(&imp, "is not listed in any PKL");
    }

    #[test]
    fn a_packing_list_asset_with_no_hash_algorithm_is_rejected() {
        let work = tempfile::tempdir().unwrap();
        let imp = edited_imp(work.path(), PKL_PREFIX, |xml| {
            xml.lines()
                .filter(|line| !line.contains("HashAlgorithm"))
                .collect::<Vec<_>>()
                .join("\n")
        });
        assert_error(&imp, "has no HashAlgorithm");
    }

    #[test]
    fn a_packing_list_mime_type_that_contradicts_the_file_warns() {
        let work = tempfile::tempdir().unwrap();
        let imp = edited_imp(work.path(), PKL_PREFIX, |xml| {
            replace_once(xml, "application/mxf", "text/xml")
        });
        assert_warning(&imp, "but file extension suggests 'application/mxf'");
    }

    #[test]
    fn a_package_with_no_track_files_warns() {
        let work = tempfile::tempdir().unwrap();
        let imp = create_good_imp(work.path());
        std::fs::remove_file(only_file_starting_with(&imp, crate::imp::PICTURE_PREFIX)).unwrap();
        assert_warning(&imp, "No MXF essence files found");
    }

    fn between(text: &str, open: &str, close: &str) -> String {
        let start = text.find(open).unwrap_or_else(|| panic!("no {open}")) + open.len();
        let rest = &text[start..];
        rest[..rest.find(close).unwrap_or_else(|| panic!("no {close}"))].to_string()
    }
}
