//! Standard enveloped XML-DSig signing/verification for IMF documents.
//!
//! Uses postkit's SMPTE 2067-3 / W3C enveloped-signature profile: one
//! `Reference URI=""` with the enveloped-signature transform, digesting the
//! whole document with the `ds:Signature` removed. No `Id` attributes are added,
//! and any conformant verifier (xmlsec1 with no `--id-attr` hints) accepts it.

use std::path::{Path, PathBuf};

use postkit::xmldsig::{XmlSigner, sign_document_enveloped, verify_document_enveloped};

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
    let xml = std::fs::read_to_string(input)
        .map_err(|e| format!("cannot read {}: {e}", input.display()))?;

    let signer = XmlSigner {
        cert_file: cert.to_path_buf(),
        key_file: key.to_path_buf(),
        chain_files: chain.to_vec(),
    };
    let signed = sign_document_enveloped(&xml, &signer)?;

    std::fs::write(output, signed).map_err(|e| format!("cannot write {}: {e}", output.display()))
}

/// Verify an IMF XML document's enveloped signature. When `trusted_cert` is
/// given, the embedded signing certificate must equal it.
pub fn verify_signature(input: &Path, trusted_cert: Option<&Path>) -> Result<(), String> {
    let xml = std::fs::read_to_string(input)
        .map_err(|e| format!("cannot read {}: {e}", input.display()))?;
    verify_document_enveloped(&xml, trusted_cert)
}
