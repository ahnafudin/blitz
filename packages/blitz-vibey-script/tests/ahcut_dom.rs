//! With `ahcut`: CSSOM View's `offsetParent` for an absolute box, and the moves that frozen style sheets admit
//! and refuse.
#![cfg(feature = "ahcut")]

use blitz_dom::DocumentConfig;
use blitz_vibey_script::ScriptDocument;

const HEAD_SHEET: &str =
    "#free,#held{position:absolute}#wrap{position:relative}#hidden{display:none}";

fn document() -> ScriptDocument {
    let html = format!(
        r#"<!doctype html><html><head><style>{HEAD_SHEET}</style></head><body><div id="free"></div><div id="wrap"><div id="held"></div></div><div id="hidden"><div id="inside"></div></div><div id="holder"><style>#plain{{color:red}}</style></div><div id="plain"></div><svg width="10" height="10"><g id="g" style="position:absolute"></g></svg></body></html>"#
    );
    let mut doc = ScriptDocument::from_html(&html, DocumentConfig::default());
    doc.execute_scripts();
    assert!(doc.take_js_errors().is_empty());
    doc
}

/// What an expression gives, as JSON; what it throws, as `ERROR <text>`.
fn observe(doc: &mut ScriptDocument, expression: &str) -> String {
    doc.eval(&format!(
        "try {{ __blitz_send_message(String(JSON.stringify(({expression})))); }} catch (e) {{ __blitz_send_message('ERROR ' + String(e)); }}"
    ));
    assert!(doc.take_js_errors().is_empty());
    let mut messages = doc.take_messages();
    assert_eq!(messages.len(), 1);
    messages.remove(0)
}

/// An absolute box that no positioned ancestor holds answers `body`; one in a positioned wrapper, the wrapper;
/// an absolute element that was never laid out (a child of an inline `<svg>`), nothing.
#[test]
fn offset_parent_of_an_absolute_box_is_the_nearest_positioned_ancestor_or_body() {
    let mut doc = document();
    assert_eq!(
        observe(
            &mut doc,
            "['free','held','g'].map(id=>{const o=document.getElementById(id).offsetParent;return o===document.body?'body':o&&o.id})"
        ),
        r#"["body","wrap",null]"#
    );
}

/// A move is refused only when the subtree that moves holds a `<style>` or a `<link>`, or when it would land
/// inside one. A plain subtree lands anywhere else, under `html` and beside an element that holds a style
/// sheet; every refusal comes before anything changed.
#[test]
fn a_move_is_refused_only_when_it_touches_a_style_sheet() {
    let mut doc = document();
    assert_eq!(
        observe(
            &mut doc,
            "(()=>{const p=document.getElementById('plain'),h=document.getElementById('holder');document.documentElement.appendChild(p);const under=p.parentNode===document.documentElement;document.body.insertBefore(p,h);return [under,p.parentNode===document.body,p.nextElementSibling===h]})()"
        ),
        "[true,true,true]"
    );
    for moving in [
        "document.documentElement.appendChild(h)",
        "document.body.insertBefore(h,document.body.firstChild)",
        "document.body.removeChild(h)",
        "document.getElementById('plain').before(h)",
        "h.remove()",
        "h.replaceWith(document.createElement('p'))",
        "document.querySelector('style').appendChild(document.createTextNode('x'))",
        "document.body.insertBefore(document.createTextNode('x'),document.querySelector('style').firstChild)",
    ] {
        let thrown = observe(
            &mut doc,
            &format!("(()=>{{const h=document.getElementById('holder');{moving}}})()"),
        );
        assert!(
            thrown.starts_with("ERROR TypeError: STYLESHEET_REBUILD_REQUIRED"),
            "{moving}: {thrown}"
        );
    }
    assert_eq!(
        observe(
            &mut doc,
            "(h=>[h.parentNode===document.body,h.previousElementSibling.id,document.querySelectorAll('style').length,document.querySelector('style').textContent])(document.getElementById('holder'))"
        ),
        format!(r#"[true,"plain",2,"{HEAD_SHEET}"]"#)
    );
}
