use std::path::{Path, PathBuf};

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
    // imports ahead of the schema that declares the root element
    carried_schemas: &'static [CarriedSchema],
}

struct CarriedSchema {
    namespace: &'static str,
    path: &'static str,
    contents: &'static str,
}

macro_rules! carried_schema {
    ($namespace:literal, $path:literal) => {
        CarriedSchema {
            namespace: $namespace,
            path: $path,
            contents: include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/schemas/", $path)),
        }
    };
}

const XML_NAMESPACE_SCHEMA: CarriedSchema = carried_schema!(
    "http://www.w3.org/XML/1998/namespace",
    "w3c/XML/1998/namespace/xml.xsd"
);
const XMLDSIG_SCHEMA: CarriedSchema = carried_schema!(
    "http://www.w3.org/2000/09/xmldsig#",
    "w3c/2000/09/xmldsig/xmldsig-core-schema.xsd"
);
const DCML_TYPES_SCHEMA: CarriedSchema = carried_schema!(
    "http://www.smpte-ra.org/schemas/433/2008/dcmlTypes/",
    "433/2008/dcmlTypes/st433b-2008-am1-2011.xsd"
);
const CPL_SCHEMA: CarriedSchema = carried_schema!(
    "http://www.smpte-ra.org/schemas/2067-3/2016",
    "2067-3/2016/st2067-3a-2016.xsd"
);
const PKL_SCHEMA: CarriedSchema = carried_schema!(
    "http://www.smpte-ra.org/schemas/2067-2/2016/PKL",
    "2067-2/2016/PKL/st2067-2b-2016.xsd"
);
const ASSETMAP_SCHEMA: CarriedSchema = carried_schema!(
    "http://www.smpte-ra.org/schemas/429-9/2007/AM",
    "429-9/2007/AM/st-429-9-2014.xsd"
);
const OPL_SCHEMA: CarriedSchema = carried_schema!(
    "http://www.smpte-ra.org/schemas/2067-100/2014",
    "2067-100/2014/st2067-100a-2014.xsd"
);

const SCHEMA_DIR_VARIABLE: &str = "IMF_SCHEMA_DIR";

enum SchemaSource {
    Directory(PathBuf),
    Carried(tempfile::TempDir),
}

const IMF_DOCUMENTS: [ImfDocument; 4] = [
    ImfDocument {
        root_element: "CompositionPlaylist",
        name_fragment: "cpl",
        schema_names: &["st2067-3-2020-CPL.xsd", "st2067-3-CPL.xsd", "imf-cpl.xsd"],
        carried_schemas: &[
            XML_NAMESPACE_SCHEMA,
            XMLDSIG_SCHEMA,
            DCML_TYPES_SCHEMA,
            CPL_SCHEMA,
        ],
    },
    ImfDocument {
        root_element: "PackingList",
        name_fragment: "pkl",
        schema_names: &["st2067-2-2020-PKL.xsd", "st2067-2-PKL.xsd", "imf-pkl.xsd"],
        carried_schemas: &[XMLDSIG_SCHEMA, PKL_SCHEMA],
    },
    ImfDocument {
        root_element: "AssetMap",
        name_fragment: "assetmap",
        schema_names: &["st0429-9-2007-AM.xsd", "st429-9-AM.xsd", "imf-assetmap.xsd"],
        carried_schemas: &[ASSETMAP_SCHEMA],
    },
    ImfDocument {
        root_element: "OutputProfileList",
        name_fragment: "opl",
        schema_names: &["st2067-9-OPL.xsd"],
        carried_schemas: &[
            XML_NAMESPACE_SCHEMA,
            XMLDSIG_SCHEMA,
            DCML_TYPES_SCHEMA,
            OPL_SCHEMA,
        ],
    },
];

/// Validate IMP XML files (CPL, PKL, AssetMap, OPL) against their SMPTE XSD schemas.
///
/// Uses the schemas the crate carries unless `schema_dir` or `IMF_SCHEMA_DIR`
/// names a directory to search instead.
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

    let schema_override = schema_dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os(SCHEMA_DIR_VARIABLE).map(PathBuf::from));
    let schema_source = match schema_override {
        Some(directory) => SchemaSource::Directory(directory),
        None => SchemaSource::Carried(write_carried_schemas()?),
    };

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
        match &schema_source {
            SchemaSource::Carried(carried_dir) => report.checked.push(validate_with_xmllint(
                &path,
                &carried_driver_path(carried_dir.path(), document),
                &["--nonet"],
            )),
            SchemaSource::Directory(directory) => match find_schema(directory, document) {
                Some(schema) => report
                    .checked
                    .push(validate_with_xmllint(&path, &schema, &[])),
                None => report.skipped.push(SkippedDocument {
                    file: file_name(&path),
                    schema_names: document.schema_names,
                }),
            },
        }
    }

    Ok(report)
}

fn carried_driver_path(carried_dir: &Path, document: &ImfDocument) -> PathBuf {
    carried_dir.join(format!("{}.xsd", document.root_element))
}

// the registry schemas import by namespace with no schemaLocation
fn write_carried_schemas() -> Result<tempfile::TempDir, String> {
    let carried_dir = tempfile::tempdir()
        .map_err(|e| format!("Failed to create a directory for the carried schemas: {e}"))?;
    let write = |path: &Path, contents: &str| {
        let parent = path.parent().expect("a schema path has a parent");
        std::fs::create_dir_all(parent)
            .and_then(|()| std::fs::write(path, contents))
            .map_err(|e| format!("Failed to write {}: {e}", path.display()))
    };
    for document in &IMF_DOCUMENTS {
        let mut driver = String::from(
            "<?xml version=\"1.0\"?>\n<xs:schema xmlns:xs=\"http://www.w3.org/2001/XMLSchema\">\n",
        );
        for schema in document.carried_schemas {
            write(&carried_dir.path().join(schema.path), schema.contents)?;
            driver.push_str(&format!(
                "  <xs:import namespace=\"{}\" schemaLocation=\"{}\"/>\n",
                schema.namespace, schema.path
            ));
        }
        driver.push_str("</xs:schema>\n");
        write(&carried_driver_path(carried_dir.path(), document), &driver)?;
    }
    Ok(carried_dir)
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

fn find_schema(schema_dir: &Path, document: &ImfDocument) -> Option<PathBuf> {
    for name in document.schema_names {
        let path = schema_dir.join(name);
        if path.is_file() {
            return Some(path);
        }
    }

    // Search recursively
    if let Ok(entries) = std::fs::read_dir(schema_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.file_name().and_then(|n| n.to_str()).is_some_and(|name| {
                name.to_lowercase().contains(document.name_fragment) && name.ends_with(".xsd")
            }) {
                return Some(p);
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
fn validate_with_xmllint(
    xml_path: &Path,
    schema_path: &Path,
    extra_arguments: &[&str],
) -> XsdValidationResult {
    let output = std::process::Command::new("xmllint")
        .args(extra_arguments)
        .arg("--schema")
        .arg(schema_path)
        .arg("--noout")
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

#[cfg(test)]
mod tests {
    use super::*;

    // xmllint only reaches the root element check once the driver compiled
    const FOREIGN_DOCUMENT: &str = "<Unrelated/>";
    const SCHEMA_COMPILED: &str =
        "No matching global declaration available for the validation root";

    #[test]
    fn every_carried_driver_compiles_offline() {
        let carried_dir = write_carried_schemas().unwrap();
        let foreign = carried_dir.path().join("foreign.xml");
        std::fs::write(&foreign, FOREIGN_DOCUMENT).unwrap();
        for document in &IMF_DOCUMENTS {
            let driver = carried_driver_path(carried_dir.path(), document);
            let result = validate_with_xmllint(&foreign, &driver, &["--nonet"]);
            assert!(
                result
                    .errors
                    .iter()
                    .any(|error| error.contains(SCHEMA_COMPILED)),
                "the {} driver did not compile: {:?}",
                document.root_element,
                result.errors
            );
        }
    }
}
