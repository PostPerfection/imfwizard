use std::path::PathBuf;

/// Options for creating a supplemental IMP.
pub struct SupplementOptions {
    pub ov_dir: PathBuf,
    pub title: String,
    pub output_dir: PathBuf,
    pub video: Option<PathBuf>,
    pub entry_point: u64,
    pub duration: Option<String>,
}

/// Result of supplemental IMP creation.
pub struct SupplementResult {
    pub success: bool,
    pub error: String,
    pub output_dir: PathBuf,
}

/// Create a supplemental IMP referencing an Original Version (OV).
///
/// Not implemented. A true supplemental IMP packages only the new or changed
/// track files plus a CPL that references the OV's originals (with optional
/// segment replacement via entry point/duration). The previous version silently
/// built a standalone IMP that did not reference the OV at all, so this fails
/// loud instead of producing a mislabelled package. Use `create` for a full IMP.
pub fn create_supplement(opts: &SupplementOptions) -> SupplementResult {
    // read the segment-replacement inputs so they aren't dead fields
    let _ = (&opts.video, opts.entry_point, &opts.duration, &opts.title);
    SupplementResult {
        success: false,
        error: format!(
            "supplemental IMP creation is not implemented; build a full IMP with `create` (OV: {})",
            opts.ov_dir.display()
        ),
        output_dir: opts.output_dir.clone(),
    }
}
