use std::path::Path;

use postkit::packaging::{PackingList, PklAsset, ns};

use crate::MxfTrackFile;
use crate::imp::CplEntry;

/// Write an IMF PKL (ST 2067-2) covering one or more CPLs plus track files.
pub fn write_pkl(
    path: &Path,
    pkl_uuid: &str,
    cpls: &[CplEntry],
    track_files: &[MxfTrackFile],
) -> std::io::Result<()> {
    let mut assets = Vec::new();
    for cpl in cpls {
        let hash = postkit::hash::hash_file(&cpl.path, postkit::hash::HashAlgorithm::Sha1)
            .map(|h| h.base64)
            .unwrap_or_default();
        let size = std::fs::metadata(&cpl.path).map(|m| m.len()).unwrap_or(0);
        assets.push(PklAsset {
            id: cpl.uuid.clone(),
            hash,
            size,
            asset_type: "text/xml".to_string(),
        });
    }
    for tf in track_files {
        assets.push(PklAsset {
            id: tf.uuid.clone(),
            hash: tf.hash.clone(),
            size: tf.size,
            asset_type: "application/mxf".to_string(),
        });
    }

    let pkl = PackingList {
        uuid: pkl_uuid.to_string(),
        namespace: ns::PKL_IMF.to_string(),
        issuer: "IMF Wizard".to_string(),
        creator: "IMF Wizard".to_string(),
        issue_date: crate::issue_date(),
        assets,
        annotation: None,
    };
    std::fs::write(path, pkl.to_xml())
}

// the Hash and Size the PKL carries for one asset, taken off the file again
pub(crate) fn patch_pkl_asset(pkl: &str, asset_id: &str, file: &Path) -> Result<String, String> {
    let hash = postkit::hash::hash_file(file, postkit::hash::HashAlgorithm::Sha1)
        .map_err(|error| format!("cannot hash {}: {error}", file.display()))?
        .base64;
    let size = file_size(file)?.to_string();
    rewrite_asset(pkl, asset_id, |block| {
        replace_element(&replace_element(block, "Hash", &hash), "Size", &size)
    })
    .ok_or_else(|| format!("the packing list lists no asset {asset_id}"))
}

pub(crate) fn file_size(file: &Path) -> Result<u64, String> {
    Ok(std::fs::metadata(file)
        .map_err(|error| format!("cannot stat {}: {error}", file.display()))?
        .len())
}

// every <Asset> block that names `asset_id` passed through `edit`, None when no block does
pub(crate) fn rewrite_asset(
    xml: &str,
    asset_id: &str,
    edit: impl Fn(&str) -> String,
) -> Option<String> {
    const ASSET: &str = "<Asset>";
    const ASSET_END: &str = "</Asset>";
    let asset_id = asset_id.to_ascii_lowercase();
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    let mut rewritten = false;
    while let Some(start) = rest.find(ASSET) {
        let end = rest[start..]
            .find(ASSET_END)
            .map_or(rest.len(), |at| start + at + ASSET_END.len());
        out.push_str(&rest[..start]);
        let block = &rest[start..end];
        if block.to_ascii_lowercase().contains(&asset_id) {
            out.push_str(&edit(block));
            rewritten = true;
        } else {
            out.push_str(block);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    rewritten.then_some(out)
}

pub(crate) fn replace_element(xml: &str, element: &str, value: &str) -> String {
    let (open, close) = (format!("<{element}>"), format!("</{element}>"));
    match (xml.find(&open), xml.find(&close)) {
        (Some(start), Some(end)) if start < end => {
            format!("{}{open}{value}{}", &xml[..start], &xml[end..])
        }
        _ => xml.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_pkl_includes_track_files() {
        let dir = tempfile::tempdir().unwrap();
        let cpl_path = dir.path().join("CPL_cpl.xml");
        std::fs::write(&cpl_path, "cpl").unwrap();
        let pkl_path = dir.path().join("PKL_pkl.xml");
        let cpls = [CplEntry {
            uuid: "cpl".into(),
            path: cpl_path,
        }];
        let tracks = [MxfTrackFile {
            path: dir.path().join("VIDEO_track.mxf"),
            uuid: "track".into(),
            hash: "base64-hash".into(),
            size: 42,
            duration: 24,
        }];

        write_pkl(&pkl_path, "pkl", &cpls, &tracks).unwrap();
        let xml = std::fs::read_to_string(pkl_path).unwrap();
        assert!(xml.contains("http://www.smpte-ra.org/schemas/2067-2/2016/PKL"));
        assert!(xml.contains("<IssueDate>"));
        assert!(xml.contains("<Id>urn:uuid:track</Id>"));
        assert!(xml.contains("<Hash>base64-hash</Hash>"));
        assert!(xml.contains("<Size>42</Size>"));
    }

    #[test]
    fn write_pkl_covers_multiple_cpls() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("CPL_a.xml");
        let b = dir.path().join("CPL_b.xml");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let pkl_path = dir.path().join("PKL_pkl.xml");
        let cpls = [
            CplEntry {
                uuid: "aaa".into(),
                path: a,
            },
            CplEntry {
                uuid: "bbb".into(),
                path: b,
            },
        ];

        write_pkl(&pkl_path, "pkl", &cpls, &[]).unwrap();
        let xml = std::fs::read_to_string(pkl_path).unwrap();
        assert!(xml.contains("<Id>urn:uuid:aaa</Id>"));
        assert!(xml.contains("<Id>urn:uuid:bbb</Id>"));
    }
}
