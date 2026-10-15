//! Standard enveloped XML-DSig signing/verification for IMF documents.
//!
//! Uses postkit's SMPTE 2067-3 / W3C enveloped-signature profile: one
//! `Reference URI=""` with the enveloped-signature transform, digesting the
//! whole document with the `ds:Signature` removed. No `Id` attributes are added,
//! and any conformant verifier (xmlsec1 with no `--id-attr` hints) accepts it.

use std::path::{Path, PathBuf};

use postkit::xmldsig::{XmlSigner, sign_document_enveloped, verify_document_enveloped};

use crate::pkl::{file_size, patch_pkl_asset, replace_element, rewrite_asset};

/// Sign an IMF XML document, writing the signed XML to `output`.
///
/// `chain` lists the CA certificates above the leaf (intermediate(s) then root);
/// they are embedded in ds:KeyInfo so a verifier can build the chain to a trust
/// anchor.
pub fn sign_document(
    input: &Path,
    output: &Path,
    cert: &Path,
    key: &Path,
    chain: &[PathBuf],
) -> Result<(), String> {
    let xml = read(input)?;
    let listing = if same_file(input, output) {
        package_listing(input)?
    } else {
        None
    };

    let signer = XmlSigner {
        cert_file: cert.to_path_buf(),
        key_file: key.to_path_buf(),
        chain_files: chain.to_vec(),
    };
    let signed = sign_document_enveloped(&xml, &signer)?;
    write(output, &signed)?;

    match listing {
        Some(listing) => listing.refresh(input),
        None => Ok(()),
    }
}

struct PackageListing {
    assetmap: PathBuf,
    document_id: String,
    // asset id and path of every PKL that lists the document
    packing_lists: Vec<(String, PathBuf)>,
}

impl PackageListing {
    fn refresh(&self, document: &Path) -> Result<(), String> {
        for (_, pkl) in &self.packing_lists {
            let patched = patch_pkl_asset(&read(pkl)?, &self.document_id, document)?;
            write(pkl, &patched)?;
        }

        // ST 429-9 makes the chunk Length optional
        let original = read(&self.assetmap)?;
        let mut assetmap = original.clone();
        let rewritten = std::iter::once((&self.document_id, document)).chain(
            self.packing_lists
                .iter()
                .map(|(id, pkl)| (id, pkl.as_path())),
        );
        for (asset_id, file) in rewritten {
            let length = file_size(file)?.to_string();
            if let Some(updated) = rewrite_asset(&assetmap, asset_id, |block| {
                replace_element(block, "Length", &length)
            }) {
                assetmap = updated;
            }
        }
        if assetmap != original {
            write(&self.assetmap, &assetmap)?;
        }
        Ok(())
    }
}

// None when the document is not an asset of a package in its own directory
fn package_listing(document: &Path) -> Result<Option<PackageListing>, String> {
    let Some(package) = document.parent() else {
        return Ok(None);
    };
    let Some(assetmap) = postkit::assetmap::find(package) else {
        return Ok(None);
    };
    let assets = postkit::assetmap::parse_ordered(&assetmap);
    let Some((document_id, _)) = assets
        .iter()
        .find(|(_, path)| same_file(&package.join(path), document))
    else {
        return Ok(None);
    };

    let mut packing_lists = Vec::new();
    for (asset_id, path) in &assets {
        let candidate = package.join(path);
        let is_xml = candidate
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"));
        if asset_id == document_id || !is_xml {
            continue;
        }
        let xml = read(&candidate)?;
        if !is_packing_list(&xml) || !xml.to_ascii_lowercase().contains(document_id) {
            continue;
        }
        if postkit::xmldsig::strip_signature(&mut xml.clone()) {
            return Err(format!(
                "{} is already signed and lists {}, sign the CPL before the PKL so the PKL can \
                 record the signed CPL's hash",
                candidate.display(),
                document.display()
            ));
        }
        packing_lists.push((asset_id.clone(), candidate));
    }

    Ok(Some(PackageListing {
        assetmap,
        document_id: document_id.clone(),
        packing_lists,
    }))
}

fn is_packing_list(xml: &str) -> bool {
    regex_lite::Regex::new(&format!(
        r"<{}PackingList[\s>]",
        postkit::cpl_xml::ELEMENT_PREFIX_PATTERN
    ))
    .is_ok_and(|root| root.is_match(xml))
}

fn same_file(first: &Path, second: &Path) -> bool {
    match (first.canonicalize(), second.canonicalize()) {
        (Ok(first), Ok(second)) => first == second,
        _ => false,
    }
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn write(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Verify an IMF XML document's enveloped signature. When `trusted_cert` is
/// given, the embedded signing certificate must equal it.
pub fn verify_signature(input: &Path, trusted_cert: Option<&Path>) -> Result<(), String> {
    let xml = std::fs::read_to_string(input)
        .map_err(|e| format!("cannot read {}: {e}", input.display()))?;
    verify_document_enveloped(&xml, trusted_cert)
}
