//! Two compositions in one IMP, the shape the GUI builds when it packages more
//! than one CPL tab: `create_imp` takes a list of compositions and writes one
//! PKL and one ASSETMAP over all of them.

use imfwizard_core::imp::{Composition, ImpOptions, create_imp};
use imfwizard_core::mxf_wrap::synthetic_j2k_codestream;
use std::path::Path;
use tempfile::TempDir;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FPS: u32 = 24;
const FEATURE_FRAMES: usize = 4;
const TRAILER_FRAMES: usize = 2;

fn codestreams(root: &Path, name: &str, frames: usize) -> std::path::PathBuf {
    let directory = root.join(name);
    std::fs::create_dir_all(&directory).unwrap();
    let codestream = synthetic_j2k_codestream(WIDTH, HEIGHT, 12);
    for frame in 0..frames {
        std::fs::write(directory.join(format!("{frame:04}.j2c")), &codestream).unwrap();
    }
    directory
}

/// Every text between `<tag>` and its close, in document order.
fn values_of(xml: &str, tag: &str) -> Vec<String> {
    xml.split(&format!("<{tag}>"))
        .skip(1)
        .filter_map(|rest| rest.split_once('<').map(|(value, _)| value.to_string()))
        .collect()
}

/// Two compositions leave two CPLs, one PKL and one ASSETMAP, and both
/// documents list every asset of both compositions.
#[test]
fn two_compositions_share_one_pkl_and_one_assetmap() {
    let directory = TempDir::new().unwrap();
    let output = directory.path().join("imp");

    let result = create_imp(&ImpOptions {
        output_dir: output.clone(),
        compositions: vec![
            Composition {
                title: "Feature Version".into(),
                content_kind: "feature".into(),
                j2k_dir: Some(codestreams(directory.path(), "feature", FEATURE_FRAMES)),
                ..Default::default()
            },
            Composition {
                title: "Trailer Version".into(),
                content_kind: "trailer".into(),
                j2k_dir: Some(codestreams(directory.path(), "trailer", TRAILER_FRAMES)),
                ..Default::default()
            },
        ],
        fps_num: FPS,
        fps_den: 1,
        ..Default::default()
    });
    assert!(result.success, "{}", result.error);

    assert_eq!(result.cpl_paths.len(), 2, "one CPL a composition");
    assert_eq!(
        std::fs::read_dir(&output)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("PKL_"))
            .count(),
        1,
        "the two compositions share one PKL"
    );
    assert!(result.assetmap_path.is_file());
    assert_eq!(
        result.assetmap_path.file_name().unwrap(),
        "ASSETMAP.xml",
        "one ASSETMAP for the volume"
    );
    assert_eq!(
        result.track_files.len(),
        2,
        "each composition wrapped its own picture"
    );

    // the two CPLs are different documents describing different content
    let cpls: Vec<String> = result
        .cpl_paths
        .iter()
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect();
    let titles: Vec<String> = cpls
        .iter()
        .flat_map(|cpl| values_of(cpl, "ContentTitle"))
        .collect();
    assert!(
        titles.contains(&"Feature Version".to_string())
            && titles.contains(&"Trailer Version".to_string()),
        "the CPLs carry {titles:?}"
    );
    let durations: Vec<u64> = cpls
        .iter()
        .flat_map(|cpl| values_of(cpl, "IntrinsicDuration"))
        .map(|value| value.parse().unwrap())
        .collect();
    assert!(
        durations.contains(&(FEATURE_FRAMES as u64))
            && durations.contains(&(TRAILER_FRAMES as u64)),
        "the CPLs run {durations:?} frames"
    );

    // the PKL covers everything, by the UUID each asset carries: both CPLs and
    // both track files
    let pkl = std::fs::read_to_string(&result.pkl_path).unwrap();
    let packed = values_of(&pkl, "Id");
    let cpl_ids: Vec<String> = cpls
        .iter()
        .map(|cpl| values_of(cpl, "Id").first().expect("the CPL's own Id").clone())
        .collect();
    assert_ne!(cpl_ids[0], cpl_ids[1], "the two CPLs share a UUID");

    let mut expected: Vec<String> = cpl_ids.clone();
    expected.extend(
        result
            .track_files
            .iter()
            .map(|track| format!("urn:uuid:{}", track.uuid)),
    );
    for id in &expected {
        assert!(packed.contains(id), "the PKL misses {id}: {packed:?}");
    }

    // and the ASSETMAP maps every one of them, plus the PKL itself, to a file
    // that is really in the package
    let assetmap = std::fs::read_to_string(&result.assetmap_path).unwrap();
    let mapped = values_of(&assetmap, "Id");
    let paths = values_of(&assetmap, "Path");
    let pkl_id = values_of(&pkl, "Id")
        .first()
        .expect("the PKL's own Id")
        .clone();
    for id in expected.iter().chain([&pkl_id]) {
        assert!(mapped.contains(id), "the ASSETMAP misses {id}: {mapped:?}");
    }
    for path in &paths {
        assert!(
            output.join(path).is_file(),
            "the ASSETMAP names {path}, which the package does not hold"
        );
    }
}

/// The package the two compositions share still validates as one IMP.
#[test]
fn a_two_composition_imp_validates() {
    let directory = TempDir::new().unwrap();
    let output = directory.path().join("imp");

    let result = create_imp(&ImpOptions {
        output_dir: output.clone(),
        compositions: vec![
            Composition {
                title: "One".into(),
                content_kind: "feature".into(),
                j2k_dir: Some(codestreams(directory.path(), "one", FEATURE_FRAMES)),
                ..Default::default()
            },
            Composition {
                title: "Two".into(),
                content_kind: "feature".into(),
                j2k_dir: Some(codestreams(directory.path(), "two", TRAILER_FRAMES)),
                ..Default::default()
            },
        ],
        fps_num: FPS,
        fps_den: 1,
        ..Default::default()
    });
    assert!(result.success, "{}", result.error);

    let report = imfwizard_core::validate::validate_imp(&output);
    assert!(report.valid, "{:?}", report.errors);

    // the timeline pass finds both compositions in the one package
    let cpls = imfwizard_core::timeline::list_cpls(&output);
    assert_eq!(cpls.len(), 2, "{cpls:?}");
    let mut titles: Vec<String> = cpls.iter().map(|cpl| cpl.title.clone()).collect();
    titles.sort();
    assert_eq!(titles, vec!["One".to_string(), "Two".to_string()]);
}
