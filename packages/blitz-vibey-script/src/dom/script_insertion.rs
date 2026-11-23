//! Refuse script elements before a script-driven insertion changes either tree.

use blitz_dom::{BaseDocument, DocumentConfig, NodeId, ns};
use boa_engine::{JsNativeError, JsResult};
use std::collections::HashSet;

const REFUSAL: &str = "SCRIPT_INSERT_REFUSED:declare the script in the initial HTML";

/// Plan the temporary fragment from DOM's “convert nodes into a node” after Web IDL argument
/// conversion. Expanding a fragment consumes the children still in it. Inserting a node again
/// moves that node to the end. Keep the live tree intact until admission succeeds.
pub(super) fn argument_roots(doc: &BaseDocument, nodes: Vec<NodeId>) -> Vec<NodeId> {
    if nodes.len() < 2 {
        if let [id] = nodes.as_slice()
            && super::node::is_fragment(doc, *id)
        {
            return doc.get_node(*id).expect("fragment").children.to_vec();
        }
        return nodes;
    }
    let mut moved = HashSet::new();
    let mut roots = Vec::with_capacity(nodes.len());
    let mut repeated_node = false;
    for id in nodes {
        if super::node::is_fragment(doc, id) {
            if moved.insert(id) {
                for &child in &doc.get_node(id).expect("fragment").children {
                    if moved.insert(child) {
                        roots.push(child);
                    }
                }
            }
        } else {
            repeated_node |= !moved.insert(id);
            roots.push(id);
        }
    }
    if repeated_node {
        moved.clear();
        roots.reverse();
        roots.retain(|id| moved.insert(*id));
        roots.reverse();
    }
    roots
}

pub(super) fn check(doc: &BaseDocument, roots: &[NodeId]) -> JsResult<()> {
    let mut pending = roots.to_vec();
    while let Some(id) = pending.pop() {
        let Some(node) = doc.get_node(id) else {
            continue;
        };
        if node.element_data().is_some_and(|el| {
            el.name.local == blitz_dom::local_name!("script")
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
