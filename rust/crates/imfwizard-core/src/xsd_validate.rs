use std::path::Path;

/// Result of XSD validation for a single file.
#[derive(Debug, Clone)]
pub struct XsdValidationResult {
    pub file: String,
    pub valid: bool,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SkippedDocument {
    pub file: String,
    pub schema_names: &'static [&'static str],
}

#[derive(Debug, Clone, Default)]
pub struct XsdReport {
    pub checked: Vec<XsdValidationResult>,
    pub skipped: Vec<SkippedDocument>,
}

struct ImfDocument {
    root_element: &'static str,
    name_fragment: &'static str,
    schema_names: &'static [&'static str],
}

const IMF_DOCUMENTS: [ImfDocument; 4] = [
    ImfDocument {
        root_element: "CompositionPlaylist",
        name_fragment: "cpl",
        schema_names: &["st2067-3-2020-CPL.xsd", "st2067-3-CPL.xsd", "imf-cpl.xsd"],
    },
    ImfDocument {
        root_element: "PackingList",
        name_fragment: "pkl",
        schema_names: &["st2067-2-2020-PKL.xsd", "st2067-2-PKL.xsd", "imf-pkl.xsd"],
    },
    ImfDocument {
        root_element: "AssetMap",
        name_fragment: "assetmap",
        schema_names: &["st0429-9-2007-AM.xsd", "st429-9-AM.xsd", "imf-assetmap.xsd"],
    },
    ImfDocument {
        root_element: "OutputProfileList",
        name_fragment: "opl",
        schema_names: &["st2067-9-OPL.xsd"],
    },
];

/// Validate IMP XML files (CPL, PKL, AssetMap) against SMPTE ST 2067 XSD schemas.
///
/// Requires xmllint to be installed with the schemas available.
pub fn validate_imp_schemas(
    imp_dir: &Path,
    schema_dir: Option<&Path>,
) -> Result<XsdReport, String> {
    if !crate::tools::has_xmllint() {
        return Err(
            "xmllint is not installed. Install libxml2-utils for XSD schema validation."
                .to_string(),
        );
    }

    let schema_base = schema_dir
        .map(|p| p.to_path_buf())
        .or_else(find_schema_dir)
        .ok_or_else(|| {
            "SMPTE ST 2067 XSD schemas not found. Use --schema-dir to specify location.".to_string()
        })?;

    let mut report = XsdReport::default();

    // Find XML files in the IMP directory
    let entries =
        std::fs::read_dir(imp_dir).map_err(|e| format!("Failed to read IMP directory: {e}"))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "xml" {
            continue;
        }

        // Detect document type from content
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let Some(document) = detect_document(&content) else {
            continue;
        };
        match find_schema(&schema_base, document) {
            Some(schema) => report.checked.push(validate_with_xmllint(&path, &schema)),
            None => report.skipped.push(SkippedDocument {
                file: file_name(&path),
                schema_names: document.schema_names,
            }),
        }
    }

    Ok(report)
}

fn detect_document(content: &str) -> Option<&'static ImfDocument> {
    let root = root_element_name(content)?;
    IMF_DOCUMENTS
        .iter()
        .find(|document| document.root_element == root)
}

fn root_element_name(content: &str) -> Option<&str> {
    let mut rest = content;
    loop {
        let start = rest.find('<')?;
        rest = &rest[start + 1..];
        if rest.starts_with('?') || rest.starts_with('!') {
            continue;
        }
        let end = rest.find(|character: char| {
            character.is_whitespace() || character == '>' || character == '/'
        })?;
        let qualified = &rest[..end];
        return Some(
            qualified
                .rsplit_once(':')
                .map_or(qualified, |(_, local)| local),
        );
    }
}

fn find_schema(schema_dir: &Path, document: &ImfDocument) -> Option<String> {
    for name in document.schema_names {
        let path = schema_dir.join(name);
        if path.is_file() {
            return Some(path.to_string_lossy().to_string());
        }
    }

    // Search recursively
    if let Ok(entries) = std::fs::read_dir(schema_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.file_name().and_then(|n| n.to_str()).is_some_and(|name| {
                name.to_lowercase().contains(document.name_fragment) && name.ends_with(".xsd")
            }) {
                return Some(p.to_string_lossy().to_string());
            }
        }
    }

    None
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Run xmllint --schema against a single file.
fn validate_with_xmllint(xml_path: &Path, schema_path: &str) -> XsdValidationResult {
    let output = std::process::Command::new("xmllint")
        .args(["--schema", schema_path, "--noout"])
        .arg(xml_path)
        .output();

    let filename = file_name(xml_path);

    match output {
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let valid = out.status.success();
            let errors: Vec<String> = if valid {
                Vec::new()
            } else {
                stderr
                    .lines()
                    .filter(|l| !l.trim().is_empty() && !l.contains("validates"))
                    .map(|l| l.to_string())
                    .collect()
            };
            XsdValidationResult {
                file: filename,
                valid,
                errors,
            }
        }
        Err(e) => XsdValidationResult {
            file: filename,
            valid: false,
            errors: vec![format!("Failed to run xmllint: {e}")],
        },
    }
}

/// Find the SMPTE schema directory.
fn find_schema_dir() -> Option<std::path::PathBuf> {
    let candidates = [
        "/usr/share/xml/smpte",
        "/usr/local/share/xml/smpte",
        "/usr/share/imf/schemas",
        "/usr/local/share/imf/schemas",
    ];
    for path in &candidates {
        let p = std::path::PathBuf::from(path);
        if p.is_dir() {
            return Some(p);
        }
    }
    std::env::var("IMF_SCHEMA_DIR")
        .ok()
        .map(std::path::PathBuf::from)
}
