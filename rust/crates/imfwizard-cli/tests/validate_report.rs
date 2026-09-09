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

/// The vendored ST 2067-3 2016 CPL schema, which is the namespace `create`
/// writes. The other IMP documents have no schema beside it, so the XSD pass
/// reports on the CPL alone.
const IMF_CPL_XSD_DIR: &str =
    "../../../extern/postkit/tests/fixtures/xsd/imf/org/smpte_ra/schemas/st2067_3_2016";

fn schema_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(IMF_CPL_XSD_DIR);
    assert!(dir.is_dir(), "no ST 2067-3 schema under {}", dir.display());
    dir
}

/// A frame count worth measuring a bitrate over, at a raster whose codestreams
/// are large enough that rounding cannot carry the comparison.
const REPORT_FRAMES: u32 = 6;

/// An IMP built from moving detail, with its codestreams kept beside it.
fn measurable_imp(work: &Path) -> PathBuf {
    let clip = work.join("moving.mkv");
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "lavfi"])
        .args([
            "-i",
            &format!("testsrc2=s={WIDTH}x{HEIGHT}:r={FRAMES_PER_SECOND}"),
        ])
        .args(["-frames:v", &REPORT_FRAMES.to_string()])
        .args(["-c:v", "ffv1", "-pix_fmt", "yuv444p10le"])
        .arg(&clip)
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );

    let imp = work.join("measurable");
    cmd()
        .args([
            "create",
            "--keep-intermediates",
            "-o",
            &imp.to_string_lossy(),
            "-t",
            "Measured",
            "--video",
            &clip.to_string_lossy(),
            "--fps-num",
            &FRAMES_PER_SECOND.to_string(),
            "--fps-den",
            "1",
        ])
        .assert()
        .success();
    imp
}

/// Every codestream `--keep-intermediates` left, in name order.
fn kept_codestream_sizes(imp: &Path) -> Vec<u64> {
    let mut frames: Vec<PathBuf> = std::fs::read_dir(imp.join("j2k"))
        .expect("the j2k directory")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "j2c" || extension == "j2k")
        })
        .collect();
    frames.sort();
    assert_eq!(frames.len(), REPORT_FRAMES as usize);
    frames
        .iter()
        .map(|path| std::fs::metadata(path).expect("a codestream").len())
        .collect()
}

/// The number after `label` in `line`, up to the next space.
fn figure_after(line: &str, label: &str) -> f64 {
    let (_, rest) = line
        .split_once(label)
        .unwrap_or_else(|| panic!("no {label:?} in {line:?}"));
    rest.split_whitespace()
        .next()
        .expect("a figure")
        .parse()
        .unwrap_or_else(|e| panic!("{e} parsing the figure after {label:?} in {line:?}"))
}

fn line_containing<'a>(report: &'a str, needle: &str) -> &'a str {
    report
        .lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no line holding {needle:?} in the report:\n{report}"))
}

/// The report reads the picture essence: it prints the codestream's own coding
/// parameters and a bitrate measured off the frames, and both describe the
/// package that was built rather than what the encode was asked for.
#[test]
fn the_report_carries_the_forensics_and_a_bitrate_the_codestreams_agree_with() {
    let work = TempDir::new().unwrap();
    let imp = measurable_imp(work.path());
    let report = write_report(&imp, &work.path().join("report.txt"), "text");

    let forensics = line_containing(&report, "JPEG 2000 codestream");
    assert!(
        forensics.contains(&format!("{WIDTH}x{HEIGHT}")),
        "the forensics name no raster: {forensics}"
    );
    for parameter in [
        "decomposition levels",
        "code-blocks",
        "tile(s)",
        "tile-parts",
        "TLM",
        "MCT",
        "RSIZ=",
    ] {
        assert!(
            forensics.contains(parameter),
            "the forensics carry no {parameter}: {forensics}"
        );
    }
    assert_eq!(
        figure_after(forensics, "identical across"),
        f64::from(REPORT_FRAMES),
        "the forensics were read off every frame: {forensics}"
    );

    let sizes = kept_codestream_sizes(&imp);
    let worst = *sizes.iter().max().expect("a largest codestream");
    assert_eq!(
        figure_after(forensics, "worst frame") as u64,
        worst,
        "the worst frame the report names is not the largest codestream: {forensics}"
    );

    // bits a second: the essence bytes over the running time at the edit rate
    let total: u64 = sizes.iter().sum();
    let seconds = f64::from(REPORT_FRAMES) / f64::from(FRAMES_PER_SECOND);
    let measured_mbps = total as f64 * 8.0 / seconds / 1_000_000.0;
    let peak_mbps = worst as f64 * 8.0 * f64::from(FRAMES_PER_SECOND) / 1_000_000.0;

    let bitrate = line_containing(&report, "picture bitrate");
    let reported_average = figure_after(bitrate, "average");
    let reported_peak = figure_after(bitrate, "Peak picture bitrate");
    assert!(
        (reported_average - measured_mbps).abs() < BITRATE_TOLERANCE_MBPS,
        "the report says {reported_average} Mbps average and the codestreams total \
         {total} bytes over {seconds} s, which is {measured_mbps:.1} Mbps"
    );
    assert!(
        (reported_peak - peak_mbps).abs() < BITRATE_TOLERANCE_MBPS,
        "the report says {reported_peak} Mbps peak and the largest codestream is \
         {worst} bytes, which is {peak_mbps:.1} Mbps"
    );
    assert!(
        reported_peak >= reported_average,
        "a peak below the average: {bitrate}"
    );
}

// the MXF frame carries the codestream plus its KLV, so the two figures differ
// by a few bytes a frame
const BITRATE_TOLERANCE_MBPS: f64 = 0.5;

/// `--scan-picture` decodes every frame, and the report says which track it
/// scanned and what it found. Without it the report says the scan was skipped
/// and names the flag.
#[test]
fn the_picture_scan_reports_on_the_track_it_decoded() {
    let work = TempDir::new().unwrap();
    let imp = measurable_imp(work.path());
    let picture = only_file_starting_with(&imp, imfwizard_core::imp::PICTURE_PREFIX)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();

    let unscanned = write_report(&imp, &work.path().join("plain.txt"), "text");
    assert!(
        unscanned.contains("--scan-picture"),
        "a report without the scan must name the flag:\n{unscanned}"
    );

    let scanned = cmd()
        .args([
            "report",
            "--imp",
            &imp.to_string_lossy(),
            "--output",
            &work.path().join("scanned.txt").to_string_lossy(),
            "--format",
            "text",
            "--scan-picture",
        ])
        .assert()
        .success();
    scanned.success();
    let scanned = std::fs::read_to_string(work.path().join("scanned.txt")).unwrap();
    assert!(
        scanned.contains(&picture),
        "the scan must name the track file it decoded:\n{scanned}"
    );
    assert!(
        scanned.contains("no black or frozen runs"),
        "testsrc2 is neither black nor frozen:\n{scanned}"
    );
}

/// `validate --xsd` runs the CPL through xmllint against the ST 2067-3 schema.
#[test]
fn xsd_validation_passes_a_created_cpl() {
    let imp = good_imp();
    let cpl = only_file_starting_with(imp, "CPL_")
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();

    cmd()
        .args([
            "validate",
            &imp.to_string_lossy(),
            "--xsd",
            "--schema-dir",
            &schema_dir().to_string_lossy(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!("XSD {cpl}: PASS")));
}

/// A CPL the schema forbids fails the pass, and the message names the element
/// the schema tripped on rather than saying the file is bad.
#[test]
fn xsd_validation_names_the_element_the_schema_rejects() {
    let work = TempDir::new().unwrap();
    let imp = copy_of_the_good_imp(work.path());
    let cpl = only_file_starting_with(&imp, "CPL_")
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    // ST 2067-3 fixes the order and names of a CompositionPlaylist's children
    edit_cpl(&imp, "<ContentTitle>", "<ContentTitleTypo>");
    edit_cpl(&imp, "</ContentTitle>", "</ContentTitleTypo>");

    cmd()
        .args([
            "validate",
            &imp.to_string_lossy(),
            "--xsd",
            "--schema-dir",
            &schema_dir().to_string_lossy(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(format!("XSD {cpl}: FAIL")))
        .stderr(predicate::str::contains("ContentTitleTypo"));
}
