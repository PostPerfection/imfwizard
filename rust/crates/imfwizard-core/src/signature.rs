//! XML-DSIG signing/verification for IMF documents.
//!
//! postkit's XML-DSig implementation (certificate::build_signature/c14n) is private and
//! shaped for KDMs (two references over AuthenticatedPublic/AuthenticatedPrivate), not the
//! enveloped single-reference signature IMF CPL/PKL use. It is not exposed for arbitrary XML
//! and extern/ is frozen, so we cannot delegate. Rather than emit an empty <SignatureValue/>
//! (which no verifier accepts) or hand-roll a c14n that would not match xmlsec1, both entry
//! points fail loud until a reusable signer exists in postkit.

const UNAVAILABLE: &str = "IMF XML-DSIG signing/verification is not available: postkit's signer \
is KDM-specific and not exposed for arbitrary IMF XML. Use signed KDMs (the `kdm` command) or \
sign with an external tool (xmlsec1).";

/// Verify the XML digital signature of an IMF document.
pub fn verify_signature(_xml_path: &std::path::Path) -> Result<bool, String> {
    Err(UNAVAILABLE.to_string())
}

/// Sign an IMF XML document using the provided key and certificate.
pub fn sign_document(
    _xml_path: &std::path::Path,
    _key_path: &std::path::Path,
    _cert_path: &std::path::Path,
) -> Result<(), String> {
    Err(UNAVAILABLE.to_string())
}
