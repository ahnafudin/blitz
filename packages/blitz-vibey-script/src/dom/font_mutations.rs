//! With `ahcut`: refuse style sheet changes once the script document's style sheets are frozen.
use super::{NativeFnPtr, dom_ctx, node_id_of_value};
use blitz_dom::{BaseDocument, NodeId};
use boa_engine::{Context, JsNativeError, JsResult, JsValue, NativeFunction};

const REBUILD: &str =
    "STYLESHEET_REBUILD_REQUIRED: recreate ScriptDocument to change stylesheet definitions";
const LINK_WRITE: &str =
    "STYLESHEET_REBUILD_REQUIRED: set link attributes in the HTML, not from script";
fn refusal() -> boa_engine::JsError {
    JsNativeError::typ().with_message(REBUILD).into()
}
pub(super) fn valid(context: &mut Context) -> JsResult<()> {
    if !dom_ctx(context)?.doc.borrow().font_stylesheets_valid() {
        return Err(refusal());
    }
    Ok(())
}
/// A DOM method or setter that checks the freeze before and the document after it runs.
pub(super) fn guarded(name: &str, setter: bool, body: NativeFnPtr) -> NativeFunction {
    let guard = kind(name, setter);
    // `insertBefore(node, reference)` moves its first argument only.
    let moved = if name == "insertBefore" {
        1
    } else {
        usize::MAX
    };
    NativeFunction::from_copy_closure(move |this, args, context| {
        let args = preflight(guard, moved, this, args, context)?;
        let result = body(this, &args, context)?;
        valid(context)?;
        Ok(result)
    })
}
/// A CSSOM native: the ones that change a sheet are refused while the style sheets are frozen.
pub(super) fn guarded_cssom(name: &str, body: NativeFnPtr) -> NativeFunction {
    use blitz_dom::StyleInput;
    let style_input = match name {
        "__blitz_sheet_insert_rule" => Some(StyleInput::Sheet),
        "__blitz_sheet_style_set_css_text" => Some(StyleInput::Declarations),
        "__blitz_sheet_style_set" => Some(StyleInput::Property),
        _ => None,
    };
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
            if let (Some(input), Some(text)) = (style_input, args.get(2)) {
                let text = text.to_string(context)?.to_std_string_lossy();
                let removal = matches!(input, StyleInput::Property)
                    && args
                        .get(3)
                        .and_then(JsValue::as_string)
                        .is_some_and(|s| s.is_empty());
                if !removal {
                    dom_ctx(context)?
                        .doc
                        .borrow()
                        .admit_script_style(input, &text)
                        .map_err(|message| JsNativeError::typ().with_message(message))?;
                }
            }
            return Err(refusal());
        }
        valid(context)?;
        body(this, args, context)
    })
}
/// What a guarded call can reach. 0: nothing. 1: an attribute or a property of the node. 2: only the nodes it
/// is given, which it moves. 3: the node itself or its children, which it removes. 4: the same, by parsed
/// markup. 5: `insertAdjacentHTML`, held to the check of 4.
fn kind(name: &str, setter: bool) -> u8 {
    match name {
        "innerHTML" | "outerHTML" if setter => 4,
        "insertAdjacentHTML" => 5,
        "textContent" | "nodeValue" | "data" | "innerText" | "outerText" if setter => 3,
        "remove" | "replaceWith" | "replaceChildren" => 3,
        "appendChild"
        | "insertBefore"
        | "removeChild"
        | "replaceChild"
        | "before"
        | "after"
        | "append"
        | "prepend"
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
    moved: usize,
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
    if kind >= 4 {
        let index = usize::from(kind == 5);
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
    if kind == 1
        && let Some(id) = target
    {
        connected_link_write(&doc, id)?;
    }
    // A move is refused when the subtree that moves holds a style sheet, or when it would land inside one:
    // that is what could change the set, the order or the text of style sheets. A subtree without a style
    // sheet may land anywhere else.
    let touches = target
        .is_some_and(|id| css_ancestor(&doc, id) || (kind >= 3 && css_subtree(&doc, id)))
        || (kind >= 2
            && owned
                .iter()
                .take(moved)
                .filter_map(node_id_of_value)
                .any(|id| css_ancestor(&doc, id) || css_subtree(&doc, id)));
    // `insertBefore` lands its node in the parent of the reference, whatever node it is called on.
    let lands_in_sheet = owned
        .iter()
        .skip(moved)
        .filter_map(node_id_of_value)
        .filter_map(|id| doc.get_node(id).and_then(|node| node.parent))
        .any(|parent| css_ancestor(&doc, parent));
    if touches || lands_in_sheet {
        return Err(refusal());
    }
    Ok(owned)
}

/// A detached element has no active style sheet. Check after Web IDL conversion, since the
/// script supplying the value can itself mutate the tree during conversion.
pub(super) fn reflected_write(doc: &BaseDocument, id: NodeId) -> JsResult<()> {
    connected_link_write(doc, id)?;
    if !doc.font_stylesheets_valid()
        || (doc.font_stylesheets_frozen()
            && doc
                .get_node(id)
                .is_some_and(|node| node.flags.is_in_document())
            && css_ancestor(doc, id))
    {
        return Err(refusal());
    }
    Ok(())
}

/// Link relations are frozen at load, including links for icons and preloads.
pub(super) fn connected_link_write(doc: &BaseDocument, id: NodeId) -> JsResult<()> {
    if doc.font_stylesheets_frozen()
        && doc.get_node(id).is_some_and(|node| {
            node.flags.is_in_document()
                && node.element_data().is_some_and(|el| {
                    el.name.ns == blitz_dom::ns!(html)
                        && el.name.local == blitz_dom::local_name!("link")
                })
        })
    {
        return Err(JsNativeError::typ().with_message(LINK_WRITE).into());
    }
    Ok(())
}
