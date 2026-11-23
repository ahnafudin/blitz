//! Refuse script elements before a script-driven insertion changes either tree.

use blitz_dom::{BaseDocument, DocumentConfig, NodeId, ns};
use boa_engine::{JsNativeError, JsResult};

const REFUSAL: &str = "SCRIPT_INSERT_REFUSED:declare the script in the initial HTML";

pub(super) fn check(doc: &BaseDocument, roots: &[NodeId]) -> JsResult<()> {
    let mut pending = roots.to_vec();
    while let Some(id) = pending.pop() {
        let Some(node) = doc.get_node(id) else {
            continue;
        };
        if node.element_data().is_some_and(|el| {
            el.name.local.as_ref() == "script"
                && (el.name.ns == ns!(html) || el.name.ns == ns!(svg))
        }) {
            return Err(JsNativeError::typ().with_message(REFUSAL).into());
        }
        pending.extend_from_slice(&node.children);
    }
    Ok(())
}

pub(super) fn check_markup(doc: &BaseDocument, id: NodeId, html: &str) -> JsResult<()> {
    if !html
        .as_bytes()
        .windows(7)
        .any(|part| part.eq_ignore_ascii_case(b"<script"))
    {
        return Ok(());
    }
    let Some(el) = doc.get_node(id).and_then(|node| node.element_data()) else {
        return Ok(());
    };
    // The fragment parser distinguishes actual scripts from comments, attribute values, raw text
    // and foreign namespaces. Run it in an isolated document with no external resource providers.
    let mut probe = BaseDocument::new(DocumentConfig::default());
    let mut parser = probe.mutate();
    let context = parser.create_element(el.name.clone(), el.attrs().to_vec());
    blitz_html::DocumentHtmlParser::parse_inner_html_into_mutator(&mut parser, context, html);
    let roots = parser.child_ids(context);
    check(parser.doc, &roots)
}
