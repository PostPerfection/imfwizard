//! Optional Netflix Photon IMF validation, gated behind `validate --photon`.
//!
//! Shell-out only (no build dep): runs `java -cp <classpath>
//! com.netflix.imflibrary.app.IMPAnalyzer <imp>` and maps its per-file summary
//! lines ("<file> has N errors and M warnings") into findings. Photon logs via
//! slf4j, so counts are read from the message text regardless of log backend.
//!
//! Photon ships no fat jar, so its own jar cannot run alone: slf4j, regxmllib
//! and jaxb-runtime must be on the classpath too. The configured path may
//! therefore be a directory of jars, which becomes a `dir/*` classpath entry.

use std::path::{Path, PathBuf};

/// Photon findings mapped from IMPAnalyzer output.
#[derive(Debug, Clone, Default)]
pub struct PhotonResult {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

const MAIN_CLASS: &str = "com.netflix.imflibrary.app.IMPAnalyzer";

/// Run Photon over an IMP directory. `explicit_jar` is `--photon-jar`; otherwise
/// the `PHOTON_JAR` env var is used. Either may name the Photon jar itself or a
/// directory holding it alongside its dependencies. Errors with a one-line hint
/// when java or the jar is missing.
pub fn run_photon(imp_dir: &Path, explicit_jar: Option<&Path>) -> Result<PhotonResult, String> {
    let java = find_java()
        .ok_or("java not found; install a JRE (e.g. apt install default-jre) to use --photon")?;
    let classpath = find_classpath(explicit_jar).ok_or(
        "Photon jar not found; pass --photon-jar <path> or set PHOTON_JAR to Photon's jar or to a directory holding it with its dependencies (see scripts/fetch_photon.sh)",
    )?;

    let out = std::process::Command::new(&java)
        .arg("-cp")
        .arg(&classpath)
        .arg(MAIN_CLASS)
        .arg(imp_dir)
        .output()
        .map_err(|e| format!("Failed to run Photon: {e}"))?;

    // Photon logs to stderr or stdout depending on backend; parse both.
    let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
    combined.push('\n');
    combined.push_str(&String::from_utf8_lossy(&out.stderr));

    // A launch failure (wrong jar / ClassNotFound) emits no summary lines; don't
    // let that masquerade as a clean pass.
    let saw_summary = combined.lines().any(|l| {
        l.contains(" has ") && (l.contains(" errors and ") || l.contains("no errors or warnings"))
    });
    if !saw_summary && !out.status.success() {
        let last = combined.lines().rev().find(|l| !l.trim().is_empty());
        return Err(format!(
            "Photon produced no analysis (is the jar correct?): {}",
            last.unwrap_or("no output")
        ));
    }

    Ok(parse_photon_output(&combined))
}

/// Parse IMPAnalyzer output into errors/warnings from its per-file summary lines.
pub fn parse_photon_output(text: &str) -> PhotonResult {
    let mut result = PhotonResult::default();
    for line in text.lines() {
        let Some(idx) = line.find(" has ") else {
            continue;
        };
        let rest = line[idx + 5..].trim();
        // "N errors and M warnings" or "no errors or warnings"
        let toks: Vec<&str> = rest.split_whitespace().collect();
        if toks.len() < 5 || toks[1] != "errors" {
            continue;
        }
        let (Ok(errs), Ok(warns)) = (toks[0].parse::<u64>(), toks[3].parse::<u64>()) else {
            continue;
        };
        if errs == 0 && warns == 0 {
            continue;
        }
        // strip any slf4j/logback prefix: keep the text after the last " - "
        let label = line[..idx]
            .rsplit(" - ")
            .next()
            .unwrap_or(&line[..idx])
            .trim();
        if errs > 0 {
            result
                .errors
                .push(format!("Photon: {label}: {errs} error(s)"));
        }
        if warns > 0 {
            result
                .warnings
                .push(format!("Photon: {label}: {warns} warning(s)"));
        }
    }
    result
}

fn find_java() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("JAVA_HOME") {
        let p = PathBuf::from(home).join("bin").join("java");
        if p.is_file() {
            return Some(p);
        }
    }
    // fall back to PATH; verify it actually runs
    let ok = std::process::Command::new("java")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    ok.then(|| PathBuf::from("java"))
}

/// Java expands a trailing `*` classpath entry to every jar in that directory.
const CLASSPATH_WILDCARD: &str = "*";

fn find_classpath(explicit: Option<&Path>) -> Option<String> {
    let candidate = explicit
        .map(|p| p.to_path_buf())
        .or_else(|| std::env::var("PHOTON_JAR").ok().map(PathBuf::from))?;
    if candidate.is_dir() {
        return Some(candidate.join(CLASSPATH_WILDCARD).to_string_lossy().into());
    }
    candidate
        .is_file()
        .then(|| candidate.to_string_lossy().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    // representative IMPAnalyzer output (logback console pattern prefixes the
    // slf4j message with timestamp/level/logger and a " - " separator)
    const SAMPLE: &str = "\
12:00:01.100 [main] INFO  c.n.imflibrary.app.IMPAnalyzer - CPL_8b8c.xml has 0 errors and 0 warnings
12:00:01.200 [main] INFO  c.n.imflibrary.app.IMPAnalyzer - PKL_1f2a.xml has 0 errors and 0 warnings
12:00:01.300 [main] INFO  c.n.imflibrary.app.IMPAnalyzer - ASSETMAP.xml has no errors or warnings
12:00:02.000 [main] INFO  c.n.imflibrary.app.IMPAnalyzer - VIDEO_c918.mxf has 2 errors and 1 warnings
12:00:02.100 [main] INFO  c.n.imflibrary.app.IMPAnalyzer - CPL_8b8c.xml Virtual Track Conformance has 1 errors and 0 warnings
12:00:02.200 [main] ERROR c.n.imflibrary.app.IMPAnalyzer - \t\tERROR-IMF_CORE_CONSTRAINTS: something is wrong";

    #[test]
    fn parses_error_and_warning_counts() {
        let r = parse_photon_output(SAMPLE);
        // VIDEO has 2 errors + 1 warning, plus CPL conformance 1 error
        assert_eq!(r.errors.len(), 2);
        assert_eq!(r.warnings.len(), 1);
        assert!(
            r.errors
                .iter()
                .any(|e| e.contains("VIDEO_c918.mxf") && e.contains("2 error"))
        );
        assert!(
            r.errors
                .iter()
                .any(|e| e.contains("Virtual Track Conformance"))
        );
        assert!(r.warnings[0].contains("VIDEO_c918.mxf"));
    }

    #[test]
    fn clean_package_yields_no_findings() {
        let clean = "10:00:00.000 [main] INFO x - CPL_a.xml has 0 errors and 0 warnings\n\
                     10:00:00.001 [main] INFO x - ASSETMAP.xml has no errors or warnings";
        let r = parse_photon_output(clean);
        assert!(r.errors.is_empty());
        assert!(r.warnings.is_empty());
    }
}
