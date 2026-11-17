//! With `ahcut`: refuse style sheet changes once the script document's style sheets are frozen.
use super::{NativeFnPtr, dom_ctx, node_id_of_value};
use blitz_dom::{BaseDocument, NodeId};
use boa_engine::{Context, JsNativeError, JsResult, JsValue, NativeFunction};

const REBUILD: &str =
    "STYLESHEET_REBUILD_REQUIRED: recreate ScriptDocument to change stylesheet definitions";
fn refusal() -> boa_engine::JsError {
    JsNativeError::typ().with_message(REBUILD).into()
}
fn valid(context: &mut Context) -> JsResult<()> {
    if !dom_ctx(context)?.doc.borrow().font_stylesheets_valid() {
        return Err(refusal());
    }
    Ok(())
}
/// A DOM method or setter that checks the freeze before and the document after it runs.
pub(super) fn guarded(name: &str, setter: bool, body: NativeFnPtr) -> NativeFunction {
    let guard = kind(name, setter);
    NativeFunction::from_copy_closure(move |this, args, context| {
        let args = preflight(guard, this, args, context)?;
        let result = body(this, &args, context)?;
        valid(context)?;
        Ok(result)
    })
}
/// A CSSOM native: the ones that change a sheet are refused while the style sheets are frozen.
pub(super) fn guarded_cssom(name: &str, body: NativeFnPtr) -> NativeFunction {
    let mutates = matches!(
        name,
        "__blitz_sheet_set_disabled"
            | "__blitz_sheet_insert_rule"
            | "__blitz_sheet_delete_rule"
            | "__blitz_sheet_set_selector_text"
            | "__blitz_sheet_style_set_css_text"
            | "__blitz_sheet_style_set"
            | "__blitz_sheet_style_remove"
    );
    NativeFunction::from_copy_closure(move |this, args, context| {
        if mutates && dom_ctx(context)?.doc.borrow().font_stylesheets_frozen() {
            return Err(refusal());
        }
        valid(context)?;
        body(this, args, context)
    })
}
fn kind(name: &str, setter: bool) -> u8 {
    match name {
        "innerHTML" | "outerHTML" if setter => 3,
        "insertAdjacentHTML" => 4,
        "textContent" | "nodeValue" | "data" | "innerText" | "outerText" if setter => 2,
        "appendChild"
        | "insertBefore"
        | "removeChild"
        | "replaceChild"
        | "remove"
        | "before"
        | "after"
        | "replaceWith"
        | "append"
        | "prepend"
        | "replaceChildren"
        | "insertAdjacentElement"
        | "insertAdjacentText" => 2,
        "setAttribute" | "setAttributeNS" | "removeAttribute" | "removeAttributeNS"
        | "toggleAttribute" => 1,
        _ if setter => 1,
        _ => 0,
    }
}
fn css_node(doc: &BaseDocument, id: NodeId) -> bool {
    doc.get_node(id)
        .and_then(|n| n.element_data())
        .is_some_and(|e| matches!(e.name.local.as_ref(), "style" | "link"))
}
fn css_ancestor(doc: &BaseDocument, mut id: NodeId) -> bool {
    loop {
        if css_node(doc, id) {
            return true;
        }
        let Some(parent) = doc.get_node(id).and_then(|n| n.parent) else {
            return false;
        };
        id = parent;
    }
}
fn css_subtree(doc: &BaseDocument, id: NodeId) -> bool {
    let mut stack = vec![id];
    while let Some(id) = stack.pop() {
        if css_node(doc, id) {
            return true;
        }
        if let Some(node) = doc.get_node(id) {
            stack.extend(node.children.iter().copied());
        }
    }
    false
}
fn preflight(
    kind: u8,
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<Vec<JsValue>> {
    valid(context)?;
    let mut owned = args.to_vec();
    if kind == 0 {
        return Ok(owned);
    }
    // Convert markup once so a stateful JS toString cannot evade the preflight.
    if kind >= 3 {
        let index = usize::from(kind == 4);
        let text = owned
            .get(index)
            .cloned()
            .unwrap_or_default()
            .to_string(context)?;
        let lower = text.to_std_string_lossy().to_ascii_lowercase();
        if lower.contains("<style") || lower.contains("<link") {
            return Err(refusal());
        }
        if owned.len() <= index {
            owned.resize(index + 1, JsValue::undefined());
        }
        owned[index] = text.into();
    }
    let ctx = dom_ctx(context)?;
    let doc = ctx.doc.borrow();
    if !doc.font_stylesheets_frozen() {
        return Ok(owned);
    }
    let target = node_id_of_value(this);
    let touches = target
        .is_some_and(|id| css_ancestor(&doc, id) || (kind >= 2 && css_subtree(&doc, id)))
        || (kind >= 2
            && owned
                .iter()
                .filter_map(node_id_of_value)
                .any(|id| css_ancestor(&doc, id) || css_subtree(&doc, id)));
    if touches {
        return Err(refusal());
    }
    Ok(owned)
}
