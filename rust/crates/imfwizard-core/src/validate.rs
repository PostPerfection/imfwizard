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
    validate_imp_with_options(imp_dir, dcpdoctor_core::VerifyOptions::standard())
}

/// Validate an IMP for the QC report, which reads the picture essence itself.
///
/// The frame-by-frame checks are the expensive part of a verify, so the report
/// pays for them and the plain validate stays fast.
pub fn validate_imp_for_report(imp_dir: &Path) -> ValidationResult {
    let options = dcpdoctor_core::VerifyOptions {
        check_picture_details: true,
        scan_every_frame: true,
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
