use assert_cmd::Command;
use imfwizard_core::imp::{Composition, CplEntry, ImpOptions};
use imfwizard_core::mxf_wrap::{
    MxfWrapOptions, picture_colour, synthetic_j2k_codestream, wrap_mxf,
};
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const CPL_ID: &str = "77777777-7777-7777-7777-777777777777";
const PKL_ID: &str = "88888888-8888-8888-8888-888888888888";
const TITLE: &str = "Signed Feature OV";

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

// TODO: no CPL here, postkit c14n rejects the xsi:type ST 2067-3 puts on every Resource
// TODO: a signed ASSETMAP fails ST 429-9, whose AssetMapType has no ds:Signature
struct SignableDocuments {
    pkl: PathBuf,
    assetmap: PathBuf,
}

struct SignerFiles {
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
        asset_uuid: None,
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
        certificate: directory.join("signer.pem"),
        private_key: directory.join("signer.key"),
        chain: vec![
            directory.join("intermediate.pem"),
            directory.join("root.pem"),
        ],
    }
}

fn sign_command(document: &Path, signer: &SignerFiles) -> Command {
    let mut command = cmd();
    command.args([
        "sign",
        "-i",
        document.to_str().unwrap(),
        "--cert",
        signer.certificate.to_str().unwrap(),
        "--key",
        signer.private_key.to_str().unwrap(),
    ]);
    for link in &signer.chain {
        command.args(["--chain", link.to_str().unwrap()]);
    }
    command
}

fn verify_command(document: &Path, trusted_certificate: Option<&Path>) -> Command {
    let mut command = cmd();
    command.args(["verify-sig", "-i", document.to_str().unwrap()]);
    if let Some(certificate) = trusted_certificate {
        command.args(["--trusted-cert", certificate.to_str().unwrap()]);
    }
    command
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
fn sign_then_verify_sig_covers_the_pkl_and_assetmap() {
    let directory = TempDir::new().unwrap();
    let documents = write_imp(&directory.path().join("imp"));
    let signer = signer_files(&directory.path().join("certificates"));

    for document in [&documents.pkl, &documents.assetmap] {
        sign_command(document, &signer)
            .assert()
            .success()
            .stdout(predicate::str::contains("Signed"));
        let xml = std::fs::read_to_string(document).unwrap();
        assert!(
            xml.contains("<ds:Signature"),
            "{} must carry a signature",
            document.display()
        );

        verify_command(document, Some(&signer.certificate))
            .assert()
            .success()
            .stdout(predicate::str::contains("Signature valid"));
    }
}

#[test]
fn verify_sig_rejects_a_tampered_pkl() {
    let directory = TempDir::new().unwrap();
    let imp = directory.path().join("imp");
    let documents = write_imp(&imp);
    let signer = signer_files(&directory.path().join("certificates"));
    sign_command(&documents.pkl, &signer).assert().success();

    let signed = std::fs::read_to_string(&documents.pkl).unwrap();
    let tampered = tamper_with_the_recorded_hash(&signed);
    assert_ne!(signed, tampered, "the tamper must change the document");
    let tampered_path = imp.join("PKL_tampered.xml");
    std::fs::write(&tampered_path, &tampered).unwrap();

    verify_command(&tampered_path, Some(&signer.certificate))
        .assert()
        .failure()
        .stderr(predicate::str::contains("digest mismatch"));
}

// --trusted-cert is the only thing that authenticates the signer: without it
// verify-sig only checks the maths against the certificate the document carries
#[test]
fn verify_sig_needs_a_trusted_cert_to_reject_a_foreign_signer() {
    let directory = TempDir::new().unwrap();
    let documents = write_imp(&directory.path().join("imp"));
    let expected = signer_files(&directory.path().join("expected_certificates"));
    let foreign = signer_files(&directory.path().join("foreign_certificates"));
    sign_command(&documents.pkl, &foreign).assert().success();

    verify_command(&documents.pkl, Some(&expected.certificate))
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "does not match the trusted certificate",
        ));

    verify_command(&documents.pkl, None)
        .assert()
        .success()
        .stdout(predicate::str::contains("Signature valid"));
}
