use std::path::{Path, PathBuf};

use crate::MxfTrackFile;
use crate::imp::CplEntry;

pub struct PartialVersion {
    pub cpl_path: PathBuf,
    pub track_files: Vec<PathBuf>,
}

/// Copy one CPL and the track files it references into a new IMP, with a PKL
/// and ASSETMAP covering only those assets.
pub fn create_partial_version(
    imp_dir: &Path,
    output_dir: &Path,
    cpl_uuid: &str,
) -> Result<PartialVersion, String> {
    let cpl = crate::timeline::list_cpls(imp_dir)
        .into_iter()
        .find(|entry| entry.id.eq_ignore_ascii_case(cpl_uuid))
        .ok_or_else(|| format!("no CPL with id {cpl_uuid} in {}", imp_dir.display()))?;
    let source_cpl = imp_dir.join(&cpl.file_path);
    let cpl_xml = std::fs::read_to_string(&source_cpl)
        .map_err(|e| format!("cannot read {}: {e}", source_cpl.display()))?;

    let assets = crate::timeline::parse_assetmap(imp_dir);
    let mut track_files = Vec::new();
    for uuid in referenced_track_files(&cpl_xml) {
        let relative = assets.get(&uuid).ok_or_else(|| {
            format!(
                "{} references {uuid}, which its ASSETMAP omits",
                cpl.file_path
            )
        })?;
        let path = imp_dir.join(relative);
        let hash = postkit::hash::hash_file(&path, postkit::hash::HashAlgorithm::Sha1)
            .map_err(|e| format!("cannot hash {}: {e}", path.display()))?
            .base64;
        let size = std::fs::metadata(&path)
            .map_err(|e| format!("cannot stat {}: {e}", path.display()))?
            .len();
        track_files.push(MxfTrackFile {
            path,
            uuid,
            hash,
            size,
            duration: 0,
        });
    }

    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("cannot create {}: {e}", output_dir.display()))?;
    let cpl_path = output_dir.join(file_name(&source_cpl));
    std::fs::copy(&source_cpl, &cpl_path)
        .map_err(|e| format!("cannot copy {}: {e}", source_cpl.display()))?;
    for track_file in &mut track_files {
        let destination = output_dir.join(file_name(&track_file.path));
        std::fs::copy(&track_file.path, &destination)
            .map_err(|e| format!("cannot copy {}: {e}", track_file.path.display()))?;
        track_file.path = destination;
    }

    let pkl_uuid = uuid::Uuid::new_v4().to_string();
    let cpls = [CplEntry {
        uuid: cpl.id.clone(),
        path: cpl_path.clone(),
    }];
    crate::pkl::write_pkl(
        &output_dir.join(format!("PKL_{pkl_uuid}.xml")),
        &pkl_uuid,
        &cpls,
        &track_files,
    )
    .map_err(|e| format!("cannot write the PKL: {e}"))?;
    crate::assetmap::write_assetmap(
        &output_dir.join("ASSETMAP.xml"),
        &pkl_uuid,
        &cpls,
        &track_files,
    )
    .map_err(|e| format!("cannot write the ASSETMAP: {e}"))?;

    Ok(PartialVersion {
        cpl_path,
        track_files: track_files.into_iter().map(|file| file.path).collect(),
    })
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

// every track file the CPL's resources name, in the order they appear and
// without the repeats a track split over several resources makes
fn referenced_track_files(cpl_xml: &str) -> Vec<String> {
    const OPEN: &str = "<TrackFileId>";
    const CLOSE: &str = "</TrackFileId>";
    let mut ids: Vec<String> = Vec::new();
    let mut rest = cpl_xml;
    while let Some(start) = rest.find(OPEN) {
        let after = &rest[start + OPEN.len()..];
        let Some(end) = after.find(CLOSE) else {
            break;
        };
        let id = after[..end]
            .trim()
            .trim_start_matches("urn:uuid:")
            .to_string();
        if !id.is_empty() && !ids.contains(&id) {
            ids.push(id);
        }
        rest = &after[end + CLOSE.len()..];
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::referenced_track_files;

    #[test]
    fn a_track_file_named_by_two_resources_is_listed_once() {
        let cpl = "<Resource><TrackFileId>urn:uuid:aaa</TrackFileId></Resource>\
                   <Resource><TrackFileId>urn:uuid:aaa</TrackFileId></Resource>\
                   <Resource><TrackFileId>urn:uuid:bbb</TrackFileId></Resource>";
        assert_eq!(referenced_track_files(cpl), ["aaa", "bbb"]);
    }
}
