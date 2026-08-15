use assert_cmd::Command;
use postkit::certificate::KdmFormulation;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tempfile::TempDir;

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

#[test]
fn version_flag() {
    cmd().arg("--version").assert().success().stdout(
        predicate::str::contains("imfwizard")
            .and(predicate::str::contains(env!("CARGO_PKG_VERSION"))),
    );
}

#[test]
fn help_flag() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("create"))
        .stdout(predicate::str::contains("encode"))
        .stdout(predicate::str::contains("analytics"))
        .stdout(predicate::str::contains("profiles"));
}

#[test]
fn create_subcommand_help() {
    cmd()
        .args(["create", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Create"));
}

#[test]
fn encode_subcommand_help() {
    cmd()
        .args(["encode", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Encode"));
}

#[test]
fn profiles_lists_output() {
    cmd().args(["profiles"]).assert().success();
}

#[test]
fn analyze_missing_directory() {
    let dir = TempDir::new().unwrap();
    let nonexistent = dir.path().join("does_not_exist");

    cmd()
        .args(["analyze", nonexistent.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn analyze_empty_directory() {
    let dir = TempDir::new().unwrap();

    cmd()
        .args(["analyze", dir.path().to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn hash_missing_file() {
    cmd()
        .args(["hash", "/nonexistent/file.mxf"])
        .assert()
        .failure();
}

#[test]
fn hash_existing_file() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("test.bin");
    std::fs::write(&file, b"hello world").unwrap();

    cmd()
        .args(["hash", file.to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn timecode_conversion() {
    cmd()
        .args(["timecode", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("timecode"));
}

#[test]
fn verbose_flag_accepted() {
    cmd().args(["-v", "profiles"]).assert().success();
}

#[test]
fn create_requires_input() {
    // create without arguments should fail
    cmd().args(["create"]).assert().failure();
}

#[test]
fn transcode_subcommand_help() {
    cmd()
        .args(["transcode", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Transcode"));
}

#[test]
fn subtitle_convert_help() {
    cmd()
        .args(["subtitle-convert", "--help"])
        .assert()
        .success();
}

// write a mono 16-bit 48k WAV of a 1 kHz sine at the given peak amplitude (0..1)
fn write_sine_wav(path: &std::path::Path, amplitude: f64) {
    let sample_rate = 48_000u32;
    let samples: Vec<i16> = (0..sample_rate)
        .map(|n| {
            let t = n as f64 / sample_rate as f64;
            let v = amplitude * (2.0 * std::f64::consts::PI * 1000.0 * t).sin();
            (v * i16::MAX as f64).round() as i16
        })
        .collect();
    let data_bytes = samples.len() * 2;
    let mut buf = Vec::with_capacity(44 + data_bytes);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&((36 + data_bytes) as u32).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // mono
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    buf.extend_from_slice(&2u16.to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    for s in samples {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, buf).unwrap();
}

#[test]
fn loudness_adjust_to_target_writes_output() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("in.wav");
    let output = dir.path().join("out.wav");
    write_sine_wav(&input, 0.25);
    cmd()
        .args([
            "loudness",
            input.to_str().unwrap(),
            "--adjust-to",
            "-24",
            "-o",
            output.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Gain applied"))
        .stdout(predicate::str::contains("Adjusted audio written"));
    assert!(output.exists(), "adjusted wav should exist");
}

#[test]
fn loudness_adjust_refuses_when_true_peak_would_clip() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("quiet.wav");
    let output = dir.path().join("loud.wav");
    // quiet source + loud target forces a large positive gain that breaches the ceiling
    write_sine_wav(&input, 0.05);
    cmd()
        .args([
            "loudness",
            input.to_str().unwrap(),
            "--adjust-to",
            "-3",
            "-o",
            output.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("true-peak ceiling exceeded"));
    assert!(!output.exists(), "clip-safe: nothing written on breach");
}

/// Any valid UUID identifies the CPL a test KDM targets: nothing reads the
/// composition itself.
const TEST_CPL_ID: &str = "1a2b3c4d-5e6f-4a8b-9c0d-1e2f3a4b5c6d";
const TEST_CONTENT_TITLE: &str = "Formulation Test";
/// The KDMRequiredExtensions element that only the dci- formulations emit.
const CONTENT_AUTHENTICATOR_ELEMENT: &str = "ContentAuthenticator";

/// A signer chain plus a separate device leaf, generated once per test binary
/// because each certificate costs an RSA key generation.
struct KdmCerts {
    _dir: TempDir,
    signer_cert: PathBuf,
    signer_key: PathBuf,
    device_cert: PathBuf,
}

fn kdm_certs() -> &'static KdmCerts {
    static CERTS: OnceLock<KdmCerts> = OnceLock::new();
    CERTS.get_or_init(|| {
        let dir = TempDir::new().unwrap();
        assert_eq!(
            postkit::certificate::generate_chain("IMF Wizard Test", dir.path()),
            0,
            "signer chain generation should succeed"
        );
        let device_cert = dir.path().join("device.pem");
        let device_opts = postkit::certificate::CertOptions {
            cert_type: postkit::certificate::CertType::Leaf,
            common_name: "IMF Wizard Test Device".to_string(),
            output_cert: device_cert.clone(),
            output_key: dir.path().join("device.key"),
            issuer_cert: dir.path().join("intermediate.pem"),
            issuer_key: dir.path().join("intermediate.key"),
            ..Default::default()
        };
        assert_eq!(
            postkit::certificate::generate_certificate(&device_opts),
            0,
            "device certificate generation should succeed"
        );
        KdmCerts {
            signer_cert: dir.path().join("signer.pem"),
            signer_key: dir.path().join("signer.key"),
            device_cert,
            _dir: dir,
        }
    })
}

/// A `kdm` invocation with every required argument filled in, so a test only
/// adds the formulation and device arguments it is about.
fn kdm_cmd(certs: &KdmCerts, output: &Path) -> Command {
    let mut command = cmd();
    command.args([
        "kdm",
        "--cpl-id",
        TEST_CPL_ID,
        "--content-title",
        TEST_CONTENT_TITLE,
        "--cert",
        certs.signer_cert.to_str().unwrap(),
        "--signer-cert",
        certs.signer_cert.to_str().unwrap(),
        "--signer-key",
        certs.signer_key.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);
    command
}

#[test]
fn every_formulation_reaches_the_kdm() {
    let certs = kdm_certs();
    let device_thumbprint = postkit::certificate::read_certificate(&certs.device_cert).thumbprint;
    assert!(
        !device_thumbprint.is_empty(),
        "device certificate should parse"
    );
    let dir = TempDir::new().unwrap();

    // (formulation, lists the supplied device, carries a ContentAuthenticator)
    let cases = [
        (KdmFormulation::ModifiedTransitional1, false, false),
        (KdmFormulation::MultipleModifiedTransitional1, true, false),
        (KdmFormulation::DciAny, false, true),
        (KdmFormulation::DciSpecific, true, true),
    ];

    for (formulation, lists_device, content_authenticator) in cases {
        let output = dir.path().join(format!("{formulation}.kdm.xml"));
        let mut command = kdm_cmd(certs, &output);
        command.args(["--formulation", &formulation.to_string()]);
        if lists_device {
            command.args(["--device-cert", certs.device_cert.to_str().unwrap()]);
        }
        command.assert().success();

        let xml = std::fs::read_to_string(&output).unwrap();
        assert_eq!(
            xml.contains(&device_thumbprint),
            lists_device,
            "{formulation} device list"
        );
        assert_eq!(
            xml.contains(CONTENT_AUTHENTICATOR_ELEMENT),
            content_authenticator,
            "{formulation} ContentAuthenticator"
        );
    }
}

#[test]
fn unknown_formulation_is_rejected() {
    let dir = TempDir::new().unwrap();
    let mut command = kdm_cmd(kdm_certs(), &dir.path().join("unused.kdm.xml"));
    let mut assertion = command
        .args(["--formulation", "no-such-formulation"])
        .assert()
        .failure();
    // the error has to name every spelling the user could have meant
    for formulation in [
        KdmFormulation::ModifiedTransitional1,
        KdmFormulation::MultipleModifiedTransitional1,
        KdmFormulation::DciAny,
        KdmFormulation::DciSpecific,
    ] {
        assertion = assertion.stderr(predicate::str::contains(formulation.to_string()));
    }
}

#[test]
fn device_certificates_must_agree_with_the_formulation() {
    let certs = kdm_certs();
    let dir = TempDir::new().unwrap();

    // a device-listing formulation with nothing to list
    let output = dir.path().join("no-devices.kdm.xml");
    kdm_cmd(certs, &output)
        .args(["--formulation", &KdmFormulation::DciSpecific.to_string()])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("device certificate")
                .and(predicate::str::contains(KdmFormulation::DciAny.to_string())),
        );
    assert!(
        !output.exists(),
        "nothing written on a rejected formulation"
    );

    // devices named under the default formulation, which lists none
    let output = dir.path().join("unlisted-devices.kdm.xml");
    kdm_cmd(certs, &output)
        .args(["--device-cert", certs.device_cert.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            KdmFormulation::MultipleModifiedTransitional1.to_string(),
        ));
    assert!(
        !output.exists(),
        "nothing written on a rejected formulation"
    );
}
