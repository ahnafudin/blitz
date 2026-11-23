//! With `ahcut`: CSSOM View's `offsetParent` for an absolute box, the moves that frozen style sheets admit
//! and refuse, and the `src` property as the `src` attribute.
#![cfg(feature = "ahcut")]

use blitz_dom::{Document, DocumentConfig};
use blitz_traits::net::{NetHandler, NetProvider, Request};
use blitz_vibey_script::ScriptDocument;
use std::sync::{Arc, Mutex};

const HEAD_SHEET: &str =
    "#free,#held{position:absolute}#wrap{position:relative}#hidden{display:none}";

#[test]
fn checked_style_ingress_preserves_the_first_refusal_even_after_script_catches_it() {
    const MOTION: &str = "CSS_KEYFRAMES_REFUSED:write the animation on the timeline in __seek";
    const STROKE: &str = "STYLE_PROPERTY_REFUSED:use filled text or an outlined SVG path";
    for (sheet, inline, write, expected) in [
        (
            "@media print{@supports(display:grid){@\\6b eyframes unused{to{opacity:1}}}}",
            "",
            "",
            Some(MOTION),
        ),
        (
            "@-webkit-keyframes unused{from{opacity:0}}",
            "",
            "",
            Some(MOTION),
        ),
        (
            "a:hover{-WEBKIT-TEXT-STROKE-COLOR:blue}",
            "",
            "",
            Some(STROKE),
        ),
        ("", "-webkit-text-stroke:1px red", "", Some(STROKE)),
        (
            "",
            "",
            "el.style.webkitTextStrokeWidth='1px';el.style.cssText='color:red'",
            Some(STROKE),
        ),
        (
            "",
            "",
            "el.style.setProperty('-webkit-text-stroke','1px red')",
            Some(STROKE),
        ),
        (
            "",
            "",
            "el.style.cssText='-webkit-text-stroke-color:blue'",
            Some(STROKE),
        ),
        (
            "p{color:blue}",
            "",
            "document.styleSheets[0].cssRules[0].style.webkitTextStrokeWidth='2px'",
            Some(STROKE),
        ),
        (
            "p{color:blue}",
            "",
            "document.styleSheets[0].insertRule('@keyframes unused{}',0)",
            Some(MOTION),
        ),
        (
            "@keyframes unused{}",
            "",
            "el.style.webkitTextStroke='1px red'",
            Some(MOTION),
        ),
        (
            "/* @keyframes fake {} */ p{content:'-webkit-text-stroke:1px';--shape:{ @keyframes fake{} };color:blue}",
            "",
            "el.style.color='navy'",
            None,
        ),
    ] {
        let page =
            format!("<!doctype html><style>{sheet}</style><p id='el' style='{inline}'>text</p>");
        let mut doc = ScriptDocument::from_html(&page, DocumentConfig::default());
        doc.execute_scripts();
        doc.eval(&format!(
            "const el=document.getElementById('el');try{{{write}}}catch(e){{}}"
        ));
        assert_eq!(
            doc.inner().script_style_refusal(),
            expected,
            "sheet={sheet}, inline={inline}, write={write}"
        );
    }
}

fn document() -> ScriptDocument {
    let html = format!(
        r#"<!doctype html><html><head><style>{HEAD_SHEET}</style></head><body><div id="free"></div><div id="wrap"><div id="held"></div></div><div id="hidden"><div id="inside"></div></div><div id="holder"><style>#plain{{color:red}}</style></div><div id="plain"></div><svg width="10" height="10"><g id="g" style="position:absolute"></g></svg></body></html>"#
    );
    let mut doc = ScriptDocument::from_html(&html, DocumentConfig::default());
    doc.execute_scripts();
    assert!(doc.take_js_errors().is_empty());
    doc
}

#[test]
fn linked_and_imported_css_use_the_stylesheet_encoding_for_admission() {
    struct Sheets;
    impl NetProvider for Sheets {
        fn fetch(&self, _: usize, request: Request, handler: Box<dyn NetHandler>) {
            let bytes = match request.url.path() {
                "/root.css" => b"@import 'motion.css';".to_vec(),
                "/motion.css" => std::iter::once(0xfeffu16)
                    .chain("@keyframes unused{to{opacity:1}}".encode_utf16())
                    .flat_map(u16::to_le_bytes)
                    .collect(),
                path => panic!("unexpected URL {path}"),
            };
            handler.bytes(request.url.into(), bytes.into());
        }
    }
    let config = DocumentConfig {
        base_url: Some("https://assets.invalid/".into()),
        net_provider: Some(Arc::new(Sheets)),
        ..Default::default()
    };
    let mut doc = ScriptDocument::from_html(
        "<!doctype html><head><link rel='stylesheet' href='root.css'></head><body>text</body>",
        config,
    );
    doc.inner_mut().resolve(0.);
    assert_eq!(
        doc.inner().script_style_refusal(),
        Some("CSS_KEYFRAMES_REFUSED:write the animation on the timeline in __seek")
    );
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

/// Remembers every URL it is asked for and serves none.
struct Asked(Mutex<Vec<String>>);

impl NetProvider for Asked {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.to_string();
        self.0.lock().unwrap().push(url.clone());
        handler.error(url, "nothing is served".into());
    }
}

/// `img.src = url` writes the `src` attribute and asks for the image as `setAttribute('src', url)` does: at
/// once on an element of the document, at the insertion of a new one. It reads back resolved; `srcset` is
/// reflected as it is written. An element whose interface has no `src` keeps a plain property.
#[test]
fn the_src_property_is_the_src_attribute_and_starts_the_same_load() {
    let asked = Arc::new(Asked(Mutex::new(Vec::new())));
    let config = DocumentConfig {
        base_url: Some("https://ahcut.invalid/page/index.html".to_owned()),
        net_provider: Some(asked.clone()),
        ..DocumentConfig::default()
    };
    let mut doc = ScriptDocument::from_html(
        r#"<!doctype html><html><body><img id="a"><img id="b"><div id="d"></div></body></html>"#,
        config,
    );
    doc.execute_scripts();
    assert!(doc.take_js_errors().is_empty());
    assert_eq!(
        observe(
            &mut doc,
            "(()=>{const [a,b,d]=['a','b','d'].map(id=>document.getElementById(id));const before=[a.src,d.src];a.setAttribute('src','one.png');b.src='two.png';const late=document.createElement('img');late.src='three.png';document.body.appendChild(late);b.srcset='two.png 2x';d.src='four.png';return [before,a.src,b.src,b.getAttribute('src'),late.getAttribute('src'),b.srcset,b.getAttribute('srcset'),d.src,d.getAttribute('src')]})()"
        ),
        r#"[["",null],"https://ahcut.invalid/page/one.png","https://ahcut.invalid/page/two.png","two.png","three.png","two.png 2x","two.png 2x","four.png",null]"#
    );
    assert_eq!(
        *asked.0.lock().unwrap(),
        ["one.png", "two.png", "three.png"]
            .map(|file| format!("https://ahcut.invalid/page/{file}"))
    );
}
