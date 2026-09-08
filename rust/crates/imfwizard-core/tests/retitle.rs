//! postkit's package retitle has to work on an IMP written the way imfwizard
//! writes one, so the CPL/PKL/ASSETMAP writers here and the rewriter there
//! cannot drift apart.

use std::path::Path;

use imfwizard_core::MxfTrackFile;
use imfwizard_core::imp::{Composition, CplEntry, ImpOptions};
use imfwizard_core::mxf_wrap::{
    MxfWrapOptions, picture_colour, synthetic_j2k_codestream, wrap_mxf,
};
use postkit::package_edit::{PackageEdit, edit_package};

const CPL_ID: &str = "11111111-1111-1111-1111-111111111111";
const PKL_ID: &str = "33333333-3333-3333-3333-333333333333";
const TRACK_ID: &str = "22222222-2222-2222-2222-222222222222";
const OLD_TITLE: &str = "Feature OV";

/// An IMP holding one composition, built through imfwizard's own writers. The
/// picture is a real track file, since the CPL reads its descriptor back.
fn write_imp(dir: &Path) -> MxfTrackFile {
    let frames = dir.join("j2k");
    std::fs::create_dir_all(&frames).unwrap();
    std::fs::write(
        frames.join("0001.j2c"),
        synthetic_j2k_codestream(1920, 1080, 12),
    )
    .unwrap();
    let wrap = wrap_mxf(&MxfWrapOptions {
        input_dir: frames,
        output_file: dir.join("VIDEO_track.mxf"),
        essence_type: imfwizard_core::EssenceType::J2k,
        edit_rate_num: 24,
        edit_rate_den: 1,
        duration: 1,
        hdr: Some(picture_colour(None)),
        mca: None,
        asset_uuid: Some(*uuid::Uuid::parse_str(TRACK_ID).unwrap().as_bytes()),
    });
    assert!(wrap.success, "picture wrap failed: {}", wrap.error);
    assert_eq!(wrap.track_file.uuid, TRACK_ID);
    let tracks = [wrap.track_file.clone()];

    let opts = ImpOptions {
        output_dir: dir.to_path_buf(),
        fps_num: 24,
        fps_den: 1,
        ..Default::default()
    };
    let comp = Composition {
        title: OLD_TITLE.into(),
        content_kind: "feature".into(),
        ..Default::default()
    };
    let cpl_path = dir.join(format!("CPL_{CPL_ID}.xml"));
    imfwizard_core::cpl::write_cpl(&cpl_path, CPL_ID, &opts, &comp, &tracks).unwrap();

    let cpls = [CplEntry {
        uuid: CPL_ID.into(),
        path: cpl_path,
    }];
    imfwizard_core::pkl::write_pkl(
        &dir.join(format!("PKL_{PKL_ID}.xml")),
        PKL_ID,
        &cpls,
        &tracks,
    )
    .unwrap();
    imfwizard_core::assetmap::write_assetmap(&dir.join("ASSETMAP.xml"), PKL_ID, &cpls, &tracks)
        .unwrap();
    wrap.track_file
}

#[test]
fn a_retitle_lands_in_the_cpl_and_repoints_the_pkl_and_assetmap() {
    let dir = tempfile::tempdir().unwrap();
    let track = write_imp(dir.path());
    let essence_before = std::fs::read(&track.path).unwrap();

    let edited = edit_package(&PackageEdit {
        input: dir.path().to_path_buf(),
        title: Some("Feature OV, reconformed".into()),
        ..Default::default()
    })
    .unwrap();

    let cpl = std::fs::read_to_string(&edited.cpl_path).unwrap();
    assert!(cpl.contains("<ContentTitle>Feature OV, reconformed</ContentTitle>"));
    assert!(!cpl.contains(CPL_ID), "the composition id must change");
    assert!(cpl.contains(TRACK_ID), "the track file keeps its asset id");

    // imfwizard's list_cpls reads the ASSETMAP, so it only reports the new title
    // if the ASSETMAP now names the rewritten CPL
    let listed = imfwizard_core::timeline::list_cpls(dir.path());
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].title, "Feature OV, reconformed");
    assert_eq!(listed[0].id, edited.composition_id);

    let expected_hash =
        postkit::hash::hash_file(&edited.cpl_path, postkit::hash::HashAlgorithm::Sha1)
            .unwrap()
            .base64;
    let expected_size = std::fs::metadata(&edited.cpl_path).unwrap().len();
    let pkl = std::fs::read_to_string(dir.path().join(format!("PKL_{PKL_ID}.xml"))).unwrap();
    assert!(pkl.contains(&format!("<Hash>{expected_hash}</Hash>")));
    assert!(pkl.contains(&format!("<Size>{expected_size}</Size>")));
    assert!(!pkl.contains(CPL_ID));
    assert!(
        pkl.contains(&format!("<Hash>{}</Hash>", track.hash)),
        "the track file's own PKL entry is untouched"
    );

    assert_eq!(
        std::fs::read(&track.path).unwrap(),
        essence_before,
        "essence must be untouched"
    );
}
