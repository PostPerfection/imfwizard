use std::path::{Path, PathBuf};

use imfwizard_core::imp::{Composition, CplEntry, ImpOptions};
use imfwizard_core::mxf_wrap::{
    MxfWrapOptions, picture_colour, synthetic_j2k_codestream, wrap_mxf,
};
use imfwizard_core::signature::{sign_document, verify_signature};

mod xmlsec_tool;

const CPL_ID: &str = "44444444-4444-4444-4444-444444444444";
const PKL_ID: &str = "55555555-5555-5555-5555-555555555555";
const TRACK_ID: &str = "66666666-6666-6666-6666-666666666666";
const TITLE: &str = "Signed Feature OV";

// TODO: no CPL here, postkit c14n rejects the xsi:type ST 2067-3 puts on every Resource
// TODO: a signed ASSETMAP fails ST 429-9, whose AssetMapType has no ds:Signature
struct SignableDocuments {
    pkl: PathBuf,
    assetmap: PathBuf,
}

impl SignableDocuments {
    fn each(&self) -> [&PathBuf; 2] {
        [&self.pkl, &self.assetmap]
    }
}

struct SignerFiles {
    chain_directory: PathBuf,
    certificate: PathBuf,
    private_key: PathBuf,
    chain: Vec<PathBuf>,
}

// the picture is a real track file, since the CPL reads its descriptor back
fn write_imp(directory: &Path) -> SignableDocuments {
    std::fs::create_dir_all(directory).unwrap();
    let frames = directory.join("j2k");
    std::fs::create_dir_all(&frames).unwrap();
    std::fs::write(
        frames.join("0001.j2c"),
        synthetic_j2k_codestream(1920, 1080, 12),
    )
    .unwrap();
    let wrap = wrap_mxf(&MxfWrapOptions {
        input_dir: frames,
        output_file: directory.join("VIDEO_track.mxf"),
        essence_type: imfwizard_core::EssenceType::J2k,
        edit_rate_num: 24,
        edit_rate_den: 1,
        duration: 1,
        hdr: Some(picture_colour(None)),
        mca: None,
        asset_uuid: Some(*uuid::Uuid::parse_str(TRACK_ID).unwrap().as_bytes()),
    });
    assert!(wrap.success, "picture wrap failed: {}", wrap.error);
    let tracks = [wrap.track_file];

    let options = ImpOptions {
        output_dir: directory.to_path_buf(),
        fps_num: 24,
        fps_den: 1,
        ..Default::default()
    };
    let composition = Composition {
        title: TITLE.into(),
        content_kind: "feature".into(),
        ..Default::default()
    };
    let cpl = directory.join(format!("CPL_{CPL_ID}.xml"));
    imfwizard_core::cpl::write_cpl(&cpl, CPL_ID, &options, &composition, &tracks).unwrap();

    let cpls = [CplEntry {
        uuid: CPL_ID.into(),
        path: cpl,
    }];
    let pkl = directory.join(format!("PKL_{PKL_ID}.xml"));
    imfwizard_core::pkl::write_pkl(&pkl, PKL_ID, &cpls, &tracks).unwrap();
    let assetmap = directory.join("ASSETMAP.xml");
    imfwizard_core::assetmap::write_assetmap(&assetmap, PKL_ID, &cpls, &tracks).unwrap();

    SignableDocuments { pkl, assetmap }
}

fn signer_files(directory: &Path) -> SignerFiles {
    std::fs::create_dir_all(directory).unwrap();
    assert_eq!(
        postkit::certificate::generate_chain("IMF Wizard Test", directory),
        0,
        "chain generation failed"
    );
    SignerFiles {
        chain_directory: directory.to_path_buf(),
        certificate: directory.join("signer.pem"),
        private_key: directory.join("signer.key"),
        chain: vec![
            directory.join("intermediate.pem"),
            directory.join("root.pem"),
        ],
    }
}

fn sign_in_place(document: &Path, signer: &SignerFiles) {
    sign_document(
        document,
        document,
        &signer.certificate,
        &signer.private_key,
        &signer.chain,
    )
    .unwrap_or_else(|error| panic!("signing {} failed: {error}", document.display()));
}

// one byte of the CPL hash the PKL records, what swapping an asset would change
fn tamper_with_the_recorded_hash(signed: &str) -> String {
    let start = signed.find("<Hash>").expect("the PKL records a hash") + "<Hash>".len();
    let replacement = if signed.as_bytes()[start] == b'A' {
        "B"
    } else {
        "A"
    };
    let mut tampered = signed.to_string();
    tampered.replace_range(start..start + 1, replacement);
    tampered
}

#[test]
fn the_pkl_and_assetmap_carry_a_signature_xmlsec1_accepts() {
    let directory = tempfile::tempdir().unwrap();
    let documents = write_imp(&directory.path().join("imp"));
    let signer = signer_files(&directory.path().join("certificates"));

    for document in documents.each() {
        sign_in_place(document, &signer);
        let xml = std::fs::read_to_string(document).unwrap();
        assert!(
            xml.contains("<ds:Signature"),
            "{} must carry a signature",
            document.display()
        );

        verify_signature(document, Some(&signer.certificate))
            .unwrap_or_else(|error| panic!("{} must verify: {error}", document.display()));

        // an independent verifier, chaining the embedded leaf to the test root
        xmlsec_tool::assert_verifies(document, &signer.chain_directory, &[]);
    }
}

#[test]
fn changing_one_byte_of_a_signed_pkl_breaks_its_signature() {
    let directory = tempfile::tempdir().unwrap();
    let imp = directory.path().join("imp");
    let documents = write_imp(&imp);
    let signer = signer_files(&directory.path().join("certificates"));
    sign_in_place(&documents.pkl, &signer);

    let signed = std::fs::read_to_string(&documents.pkl).unwrap();
    let tampered = tamper_with_the_recorded_hash(&signed);
    assert_ne!(signed, tampered, "the tamper must change the document");
    assert_eq!(
        signed.len(),
        tampered.len(),
        "the tamper must touch content only, not the signature block"
    );
    let tampered_path = imp.join("PKL_tampered.xml");
    std::fs::write(&tampered_path, &tampered).unwrap();

    let error = verify_signature(&tampered_path, Some(&signer.certificate))
        .expect_err("a tampered PKL must not verify");
    assert!(error.contains("digest mismatch"), "got: {error}");

    let rejected = xmlsec_tool::verify(&tampered_path, &signer.chain_directory, &[]);
    assert!(
        !rejected.status.success(),
        "xmlsec1 must reject the tampered {}",
        xmlsec_tool::report(&tampered_path, &rejected)
    );
}

#[test]
fn a_signature_fails_against_another_signers_certificate() {
    let directory = tempfile::tempdir().unwrap();
    let documents = write_imp(&directory.path().join("imp"));
    let signer = signer_files(&directory.path().join("certificates"));
    let other = signer_files(&directory.path().join("other_certificates"));
    sign_in_place(&documents.pkl, &signer);

    verify_signature(&documents.pkl, Some(&signer.certificate))
        .expect("the real signer must verify");

    let error = verify_signature(&documents.pkl, Some(&other.certificate))
        .expect_err("another signer's certificate must not verify");
    assert!(
        error.contains("does not match the trusted certificate"),
        "got: {error}"
    );

    let rejected = xmlsec_tool::verify(&documents.pkl, &other.chain_directory, &[]);
    assert!(
        !rejected.status.success(),
        "xmlsec1 must reject a foreign trust anchor {}",
        xmlsec_tool::report(&documents.pkl, &rejected)
    );
}
