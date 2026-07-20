//! Enveloped XML-DSig signing/verification for IMF documents.
//!
//! postkit exposes a reusable enveloped signer (`sign_enveloped`) keyed on an id
//! attribute: it digests each referenced element and inserts a ds:Signature as a
//! sibling of them. IMF CPL/PKL/OPL elements carry no id attribute, so before
//! signing we give every direct child of the root an `Id` attribute and reference
//! all of them. The signature goes in as the root's last child, so it is a
//! sibling of every referenced element and covers the whole document body.
//!
//! This produces a cryptographically valid enveloped XML signature (xmlsec1
//! verifies it), not the SMPTE 2067-3 URI="" enveloped-transform profile, which
//! postkit's signer does not implement.

use std::path::{Path, PathBuf};

use postkit::xmldsig::{XmlSigner, sign_enveloped, verify_enveloped};
use quick_xml::events::Event;
use quick_xml::reader::Reader;

/// The attribute the signer references elements by, and injects when absent.
const ID_ATTR: &str = "Id";

/// Sign an IMF XML document, writing the signed XML to `output`.
///
/// `chain` lists the CA certificates above the leaf (intermediate(s) then root),
/// leaf to root; they are embedded in ds:KeyInfo so a verifier can build the
/// chain to a trust anchor.
pub fn sign_document(
    input: &Path,
    output: &Path,
    cert: &Path,
    key: &Path,
    chain: &[PathBuf],
) -> Result<(), String> {
    let xml = std::fs::read_to_string(input)
        .map_err(|e| format!("cannot read {}: {e}", input.display()))?;

    let (prepared, ids) = tag_root_children(&xml)?;
    if ids.is_empty() {
        return Err(format!(
            "{} has no signable child elements under its root",
            input.display()
        ));
    }
    let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();

    let signer = XmlSigner {
        cert_file: cert.to_path_buf(),
        key_file: key.to_path_buf(),
        chain_files: chain.to_vec(),
    };
    let signed = sign_enveloped(&prepared, &id_refs, ID_ATTR, None, &signer)?;

    std::fs::write(output, signed).map_err(|e| format!("cannot write {}: {e}", output.display()))
}

/// Verify an IMF XML document's enveloped signature. When `trusted_cert` is
/// given, the embedded signing certificate must equal it.
pub fn verify_signature(input: &Path, trusted_cert: Option<&Path>) -> Result<(), String> {
    let xml = std::fs::read_to_string(input)
        .map_err(|e| format!("cannot read {}: {e}", input.display()))?;
    verify_enveloped(&xml, ID_ATTR, trusted_cert)
}

/// A direct child of the root whose start tag needs an `Id` attribute.
struct Child {
    /// Byte offset in the source to splice ` Id="..."` at (before `>` / `/>`).
    insert_at: usize,
    /// The id to reference: an existing `Id` attribute, or a generated one.
    id: String,
    /// True when the id already exists and must not be spliced in.
    existing: bool,
}

/// Give every direct child of the root element an `Id` attribute (keeping any it
/// already has) and return the modified XML plus the ids to reference, in
/// document order.
fn tag_root_children(xml: &str) -> Result<(String, Vec<String>), String> {
    let mut reader = Reader::from_str(xml);
    let mut depth: i32 = 0;
    let mut children: Vec<Child> = Vec::new();

    loop {
        let before = reader.buffer_position() as usize;
        let event = reader
            .read_event()
            .map_err(|e| format!("document is not valid XML: {e}"))?;
        let after = reader.buffer_position() as usize;
        match event {
            Event::Start(e) => {
                if depth == 1 {
                    children.push(child_at(&e, xml, before, after, children.len())?);
                }
                depth += 1;
            }
            Event::Empty(e) => {
                if depth == 1 {
                    children.push(child_at(&e, xml, before, after, children.len())?);
                }
            }
            Event::End(_) => depth -= 1,
            Event::Eof => break,
            _ => {}
        }
    }

    let ids: Vec<String> = children.iter().map(|c| c.id.clone()).collect();

    // Splice from the back so earlier offsets stay valid.
    let mut out = xml.to_string();
    for child in children.iter().rev() {
        if child.existing {
            continue;
        }
        out.insert_str(child.insert_at, &format!(r#" {ID_ATTR}="{}""#, child.id));
    }
    Ok((out, ids))
}

/// Describe one direct-child element: where to inject its `Id` and what id to use.
fn child_at(
    e: &quick_xml::events::BytesStart,
    xml: &str,
    before: usize,
    after: usize,
    index: usize,
) -> Result<Child, String> {
    // Reuse an existing Id attribute rather than adding a second one.
    for attr in e.attributes() {
        let attr = attr.map_err(|err| format!("cannot read an attribute: {err}"))?;
        if attr.key.as_ref() == ID_ATTR.as_bytes() {
            let value = attr
                .unescape_value()
                .map_err(|err| format!("cannot unescape an attribute value: {err}"))?
                .into_owned();
            return Ok(Child {
                insert_at: after,
                id: value,
                existing: true,
            });
        }
    }

    // Insert before the tag's closing `>` (or `/>` for an empty element).
    let tag = &xml[before..after];
    let gt = tag.rfind('>').ok_or("a start tag has no closing '>'")?;
    let mut insert_rel = gt;
    if tag[..gt].ends_with('/') {
        insert_rel = gt - 1;
    }
    Ok(Child {
        insert_at: before + insert_rel,
        id: format!("imfsig_{index}"),
        existing: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpl() -> &'static str {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016">
  <Id>urn:uuid:11111111-2222-3333-4444-555555555555</Id>
  <ContentTitle>Example &amp; Co "IMF" CPL</ContentTitle>
  <SegmentList>
    <Segment>
      <Id>urn:uuid:66666666-7777-8888-9999-aaaaaaaaaaaa</Id>
    </Segment>
  </SegmentList>
</CompositionPlaylist>
"#
    }

    #[test]
    fn tag_root_children_ids_every_top_level_child() {
        let (tagged, ids) = tag_root_children(cpl()).unwrap();
        // Id, ContentTitle, SegmentList
        assert_eq!(ids, vec!["imfsig_0", "imfsig_1", "imfsig_2"]);
        assert!(tagged.contains(r#"<ContentTitle Id="imfsig_1">"#));
        assert!(tagged.contains(r#"<SegmentList Id="imfsig_2">"#));
        // The nested Segment/Id must not be tagged.
        assert!(!tagged.contains(r#"<Segment Id="#));
    }
}
