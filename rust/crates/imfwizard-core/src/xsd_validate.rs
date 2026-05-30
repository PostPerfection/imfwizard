use std::path::Path;

/// Result of XSD validation for a single file.
#[derive(Debug, Clone)]
pub struct XsdValidationResult {
    pub file: String,
    pub valid: bool,
    pub errors: Vec<String>,
}

/// Validate IMP XML files (CPL, PKL, AssetMap) against SMPTE ST 2067 XSD schemas.
///
/// Requires xmllint to be installed with the schemas available.
pub fn validate_imp_schemas(
    imp_dir: &Path,
    schema_dir: Option<&Path>,
) -> Result<Vec<XsdValidationResult>, String> {
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

    let mut results = Vec::new();

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

        let schema_file = detect_schema(&content, &schema_base);
        if let Some(schema) = schema_file {
            let result = validate_with_xmllint(&path, &schema);
            results.push(result);
        }
    }

    Ok(results)
}

/// Detect which XSD schema to use based on the XML root element namespace.
fn detect_schema(content: &str, schema_dir: &Path) -> Option<String> {
    if content.contains("CompositionPlaylist") || content.contains("2067-3") {
        find_schema(schema_dir, "cpl")
    } else if content.contains("PackingList") || content.contains("2067-2") {
        find_schema(schema_dir, "pkl")
    } else if content.contains("AssetMap") {
        find_schema(schema_dir, "assetmap")
    } else if content.contains("OutputProfileList") || content.contains("2067-9") {
        find_schema(schema_dir, "opl")
    } else {
        None
    }
}

/// Find an XSD schema file by type.
fn find_schema(schema_dir: &Path, doc_type: &str) -> Option<String> {
    let candidates: Vec<String> = match doc_type {
        "cpl" => vec![
            "st2067-3-2020-CPL.xsd".into(),
            "st2067-3-CPL.xsd".into(),
            "imf-cpl.xsd".into(),
        ],
        "pkl" => vec![
            "st2067-2-2020-PKL.xsd".into(),
            "st2067-2-PKL.xsd".into(),
            "imf-pkl.xsd".into(),
        ],
        "assetmap" => vec![
            "st0429-9-2007-AM.xsd".into(),
            "st429-9-AM.xsd".into(),
            "imf-assetmap.xsd".into(),
        ],
        "opl" => vec!["st2067-9-OPL.xsd".into()],
        _ => return None,
    };

    for name in &candidates {
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
                name.to_lowercase().contains(doc_type) && name.ends_with(".xsd")
            }) {
                return Some(p.to_string_lossy().to_string());
            }
        }
    }

    None
}

/// Run xmllint --schema against a single file.
fn validate_with_xmllint(xml_path: &Path, schema_path: &str) -> XsdValidationResult {
    let output = std::process::Command::new("xmllint")
        .args(["--schema", schema_path, "--noout"])
        .arg(xml_path)
        .output();

    let filename = xml_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

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
