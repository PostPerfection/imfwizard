use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tempfile::TempDir;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FRAMES: u32 = 1;
const FRAMES_PER_SECOND: u32 = 24;

// what breaking the composition edit rate makes the validator say
const NO_EDIT_RATE: &str = "CPL has no EditRate";

fn cmd() -> Command {
    Command::cargo_bin("imfwizard").unwrap()
}

// one App 2E picture package written by the create subcommand, shared by every
// case here because each one only reads it or copies it
fn good_imp() -> &'static Path {
    static PACKAGE: OnceLock<(TempDir, PathBuf)> = OnceLock::new();
    let (_directory, imp) = PACKAGE.get_or_init(|| {
        let directory = TempDir::new().unwrap();
        let clip = directory.path().join("source.mkv");
        let made = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                &format!("color=c=red:s={WIDTH}x{HEIGHT}:r={FRAMES_PER_SECOND}"),
                "-frames:v",
                &FRAMES.to_string(),
                "-c:v",
                "ffv1",
                "-pix_fmt",
                "gbrp",
            ])
            .arg(&clip)
            .output()
            .expect("ffmpeg");
        assert!(
            made.status.success(),
            "{}",
            String::from_utf8_lossy(&made.stderr)
        );

        let imp = directory.path().join("imp");
        cmd()
            .args([
                "create",
                "-o",
                &imp.to_string_lossy(),
                "-t",
                "Validate",
                "--video",
                &clip.to_string_lossy(),
                "--fps-num",
                &FRAMES_PER_SECOND.to_string(),
                "--fps-den",
                "1",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("IMP created"));
        (directory, imp)
    });
    imp
}

fn only_file_starting_with(imp_dir: &Path, prefix: &str) -> PathBuf {
    let mut found: Vec<PathBuf> = std::fs::read_dir(imp_dir)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix))
        })
        .collect();
    assert_eq!(found.len(), 1, "expected one {prefix} file in {imp_dir:?}");
    found.pop().unwrap()
}

fn copy_of_the_good_imp(work_dir: &Path) -> PathBuf {
    let copy = work_dir.join("imp");
    std::fs::create_dir_all(&copy).unwrap();
    for entry in std::fs::read_dir(good_imp()).unwrap().flatten() {
        std::fs::copy(entry.path(), copy.join(entry.file_name())).unwrap();
    }
    copy
}

fn edit_cpl(imp_dir: &Path, from: &str, to: &str) {
    let cpl = only_file_starting_with(imp_dir, "CPL_");
    let xml = std::fs::read_to_string(&cpl).unwrap();
    assert!(
        xml.contains(from),
        "the written CPL no longer holds {from:?}"
    );
    std::fs::write(&cpl, xml.replacen(from, to, 1)).unwrap();
}

fn write_report(imp_dir: &Path, output: &Path, format: &str) -> String {
    cmd()
        .args([
            "report",
            "--imp",
            &imp_dir.to_string_lossy(),
            "--output",
            &output.to_string_lossy(),
            "--format",
            format,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Report written to"));
    std::fs::read_to_string(output).unwrap()
}

#[test]
fn every_report_format_carries_the_findings() {
    let work = TempDir::new().unwrap();
    let imp = copy_of_the_good_imp(work.path());
    // the composition edit rate comes first, the resource keeps its own
    edit_cpl(&imp, "<EditRate>24 1</EditRate>", "");

    let text = write_report(&imp, &work.path().join("report.txt"), "text");
    assert!(
        text.contains(&format!("[error] validation: {NO_EDIT_RATE}")),
        "the text report lost the finding: {text}"
    );

    let json = write_report(&imp, &work.path().join("report.json"), "json");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("the report is JSON");
    let entries = parsed["entries"].as_array().expect("an entries array");
    assert!(
        entries.iter().any(|entry| {
            entry["severity"] == "error"
                && entry["message"]
                    .as_str()
                    .is_some_and(|message| message.contains(NO_EDIT_RATE))
        }),
        "the JSON report lost the finding: {json}"
    );

    let html = write_report(&imp, &work.path().join("report.html"), "html");
    assert!(html.contains("<!DOCTYPE html>"), "not HTML: {html}");
    assert!(
        html.contains("<td class='error'>error</td>") && html.contains(NO_EDIT_RATE),
        "the HTML report lost the finding: {html}"
    );
}

#[test]
fn a_clean_report_says_the_package_passed() {
    let work = TempDir::new().unwrap();
    let json = write_report(good_imp(), &work.path().join("report.json"), "json");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["summary"], "IMP validation PASSED", "{json}");
}

// Photon reads the CPL against the ST 2067-3 schema, so this needs PHOTON_JAR
// and java, which scripts/fetch_photon.sh provides. Photon prints "no errors or
// warnings" for a composition it never parsed, so the clean half of this case
// only means something beside the half that catches a real CPL fault.
#[test]
fn photon_names_the_cpl_it_rejects() {
    let work = TempDir::new().unwrap();
    let imp = copy_of_the_good_imp(work.path());
    let cpl_name = only_file_starting_with(&imp, "CPL_")
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    edit_cpl(&imp, "<EssenceDescriptor>", "<EssenceDescriptorZZ>");
    edit_cpl(&imp, "</EssenceDescriptor>", "</EssenceDescriptorZZ>");

    cmd()
        .args(["validate", "--photon", &imp.to_string_lossy()])
        .assert()
        .failure()
        .stderr(predicate::str::contains(format!("Photon: {cpl_name}")))
        .stderr(predicate::str::contains("error(s)"));

    cmd()
        .args(["validate", "--photon", &good_imp().to_string_lossy()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Photon: PASS"));
}

// what a schema error out of dcpdoctor's Photon pass says, as against the
// "[Photon] deep IMF checks skipped" line a missing Photon produces
const PHOTON_SCHEMA_FINDING: &str = "[Photon] Line Number";

fn photon_jars() -> String {
    std::env::var("PHOTON_JAR").expect("PHOTON_JAR names the Photon jars fetch_photon.sh wrote")
}

// dcpdoctor searches its own cache when nothing names Photon, so an empty cache
// leaves the wizard's location as the only way its IMF pass can find the jars
#[test]
fn the_imf_pass_runs_photon_from_the_wizards_own_location() {
    let work = TempDir::new().unwrap();
    let imp = copy_of_the_good_imp(work.path());
    edit_cpl(&imp, "<EssenceDescriptor>", "<EssenceDescriptorZZ>");
    edit_cpl(&imp, "</EssenceDescriptor>", "</EssenceDescriptorZZ>");
    let empty_cache = TempDir::new().unwrap();
    let jars = photon_jars();

    cmd()
        .env_remove("PHOTON_DIR")
        .env("PHOTON_JAR", &jars)
        .env("XDG_CACHE_HOME", empty_cache.path())
        .args(["validate", &imp.to_string_lossy()])
        .assert()
        .failure()
        .stderr(predicate::str::contains(PHOTON_SCHEMA_FINDING));

    cmd()
        .env_remove("PHOTON_DIR")
        .env_remove("PHOTON_JAR")
        .env("XDG_CACHE_HOME", empty_cache.path())
        .args(["validate", "--photon-jar", &jars, &imp.to_string_lossy()])
        .assert()
        .failure()
        .stderr(predicate::str::contains(PHOTON_SCHEMA_FINDING));
}
