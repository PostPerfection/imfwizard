pub use postkit::report::{Report, ReportEntry, ReportFormat};

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const PICTURE_CATEGORY: &str = "picture";

pub fn picture_not_scanned_entry() -> ReportEntry {
    ReportEntry {
        severity: "info".into(),
        category: PICTURE_CATEGORY.into(),
        message: "Picture was not scanned for black or frozen runs".into(),
        details: "Use report --scan-picture to decode every frame".into(),
    }
}

pub fn scan_picture_entries(imp_dir: &Path) -> Vec<ReportEntry> {
    let mut tracks: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    let mut entries = Vec::new();

    for cpl in crate::timeline::list_cpls(imp_dir) {
        let composition = if cpl.title.is_empty() {
            cpl.file_path.clone()
        } else {
            cpl.title
        };
        for segment in crate::timeline::get_timeline(&imp_dir.join(cpl.file_path)) {
            if segment.video_track_file_id.is_empty() {
                continue;
            }
            if segment.video_file.is_empty() {
                entries.push(scan_skipped_entry(
                    &composition,
                    &segment.video_track_file_id,
                    "the ASSETMAP has no local path for this track file",
                ));
                continue;
            }
            tracks
                .entry(PathBuf::from(segment.video_file))
                .or_default()
                .insert(composition.clone());
        }
    }

    for (track, compositions) in tracks {
        let composition = compositions.into_iter().collect::<Vec<_>>().join(", ");
        let track_name = track
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let resolved = match postkit::preview::resolve_picture(&track) {
            Ok(resolved) => resolved,
            Err(error) => {
                entries.push(scan_skipped_entry(
                    &composition,
                    &track_name,
                    &error.to_string(),
                ));
                continue;
            }
        };
        if resolved.encrypted {
            entries.push(scan_skipped_entry(
                &composition,
                &track_name,
                "the picture essence is encrypted",
            ));
            continue;
        }
        match postkit::picture_findings::detect_in_essence(
            &resolved.mxf,
            resolved.fps,
            u64::from(resolved.frame_count),
        ) {
            Ok(findings) => {
                let findings = findings.describe(resolved.fps);
                if findings.is_empty() {
                    entries.push(ReportEntry {
                        severity: "pass".into(),
                        category: PICTURE_CATEGORY.into(),
                        message: format!("{composition}: {track_name} has no black or frozen runs"),
                        details: String::new(),
                    });
                } else {
                    entries.extend(findings.into_iter().map(|finding| ReportEntry {
                        severity: "warning".into(),
                        category: PICTURE_CATEGORY.into(),
                        message: format!("{composition}: {track_name}: {finding}"),
                        details: String::new(),
                    }));
                }
            }
            Err(error) => entries.push(scan_skipped_entry(&composition, &track_name, &error)),
        }
    }

    if entries.is_empty() {
        entries.push(ReportEntry {
            severity: "info".into(),
            category: PICTURE_CATEGORY.into(),
            message: "No picture track was available to scan".into(),
            details: String::new(),
        });
    }
    entries
}

fn scan_skipped_entry(composition: &str, track: &str, reason: &str) -> ReportEntry {
    ReportEntry {
        severity: "info".into(),
        category: PICTURE_CATEGORY.into(),
        message: format!("{composition}: {track} was not scanned"),
        details: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_without_the_scan_names_the_flag() {
        let entry = picture_not_scanned_entry();
        assert_eq!(entry.severity, "info");
        assert!(entry.details.contains("--scan-picture"));
    }

    #[test]
    fn a_missing_picture_path_is_reported_as_unscanned() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("CPL.xml"),
            r#"<CompositionPlaylist>
<ContentTitle>My Film</ContentTitle>
<SegmentList><Segment><SequenceList><MainImageSequence><ResourceList><Resource>
<TrackFileId>urn:uuid:picture-1</TrackFileId><SourceDuration>72</SourceDuration>
</Resource></ResourceList></MainImageSequence></SequenceList></Segment></SegmentList>
</CompositionPlaylist>"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("ASSETMAP.xml"),
            r#"<AssetMap><AssetList><Asset><Id>urn:uuid:cpl</Id><ChunkList><Chunk><Path>CPL.xml</Path></Chunk></ChunkList></Asset></AssetList></AssetMap>"#,
        )
        .unwrap();

        let entries = scan_picture_entries(dir.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].severity, "info");
        assert!(entries[0].message.contains("picture-1 was not scanned"));
        assert!(entries[0].details.contains("ASSETMAP"));
    }

    #[test]
    fn a_black_finished_track_is_reported() {
        const WIDTH: u32 = 1920;
        const HEIGHT: u32 = 1080;
        const FRAME_COUNT: u64 = 72;

        let dir = tempfile::tempdir().unwrap();
        let frames = dir.path().join("frames");
        let profile = postkit::j2k::ImfProfile::for_raster(WIDTH, HEIGHT).unwrap();
        let levels = postkit::j2k::imf_levels(WIDTH, HEIGHT, 24.0, 200_000_000).unwrap();
        let params = postkit::grok_encoder::CompressParams {
            compression_ratio: 10.0,
            num_resolutions: 6,
            profile: postkit::j2k::imf_rsiz(profile, levels),
            apply_xyz_transform: false,
            ..Default::default()
        };
        postkit::grok_encoder::initialize(0);
        let mut frame = Some(postkit::grok_encoder::RawFrame::Packed {
            data: vec![0; (WIDTH * HEIGHT * 6) as usize],
            width: WIDTH,
            height: HEIGHT,
            precision: 16,
            index: 0,
            order: postkit::grok_encoder::SampleOrder::Big,
        });
        let encoded = postkit::grok_encoder::encode_pipeline(
            &frames,
            &params,
            1,
            &std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            &std::sync::Arc::new(postkit::grok_encoder::PhaseClocks::default()),
            || frame.take(),
            |_| {},
        );
        assert!(encoded.success, "{}", encoded.error);
        let first = frames.join("frame_00000000.j2c");
        for index in 1..FRAME_COUNT {
            std::fs::hard_link(&first, frames.join(format!("frame_{index:08}.j2c"))).unwrap();
        }

        let package = dir.path().join("imp");
        let result = crate::imp::create_imp(&crate::imp::ImpOptions {
            output_dir: package.clone(),
            compositions: vec![crate::imp::Composition {
                title: "Black Film".into(),
                content_kind: "feature".into(),
                j2k_dir: Some(frames),
                ..Default::default()
            }],
            fps_num: 24,
            fps_den: 1,
            ..Default::default()
        });
        assert!(result.success, "{}", result.error);

        let entries = scan_picture_entries(&package);
        assert!(
            entries.iter().any(|entry| {
                entry.severity == "warning" && entry.message.contains("black picture")
            }),
            "{entries:?}"
        );
    }
}
