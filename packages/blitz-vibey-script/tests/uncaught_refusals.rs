//! Native DOM refusals retain their identity when an embedder drains uncaught exceptions.
#![cfg(feature = "ahcut")]

use blitz_dom::{Document, DocumentConfig};
use blitz_vibey_script::ScriptDocument;

const INSERT: &str = "SCRIPT_INSERT_REFUSED:declare the script in the initial HTML";
const LINK: &str = "STYLESHEET_REBUILD_REQUIRED:set link attributes in the HTML, not from script";
const SHEET: &str =
    "STYLESHEET_REBUILD_REQUIRED:recreate ScriptDocument to change stylesheet definitions";
const STROKE: &str = "STYLE_PROPERTY_REFUSED:use filled text or an outlined SVG path";
const KEYFRAMES: &str = "CSS_KEYFRAMES_REFUSED:write the animation on the timeline in __seek";

fn document() -> ScriptDocument {
    let mut doc = ScriptDocument::from_html(
        "<!doctype html><style>p{color:red}</style><link rel=icon><p>text</p>",
        DocumentConfig::default(),
    );
    doc.execute_scripts();
    doc
}

#[test]
fn only_the_native_exception_has_a_refusal_even_after_a_catch_and_rethrow() {
    let mut doc = document();
    doc.eval("try { document.body.appendChild(document.createElement('script')) } catch (e) { globalThis.saved = e }");
    assert!(doc.take_uncaught_errors().is_empty());
    doc.eval("throw new TypeError(saved.message)");
    doc.eval("throw saved");
    doc.eval("saved.message='changed by the page'; throw saved");
    doc.eval("throw Error('ordinary')");
    let entries = doc.take_uncaught_errors();
    let refusals: Vec<_> = entries
        .iter()
        .map(|entry| entry.refusal.as_deref())
        .collect();
    assert_eq!(refusals, [None, Some(INSERT), Some(INSERT), None]);
    assert!(entries[0].message.contains(INSERT));
    assert!(entries[1].message.contains(INSERT));
    assert!(entries[3].message.contains("ordinary"));
    assert!(doc.take_js_errors().is_empty());
}

#[test]
fn the_host_sdk_issues_refusals_by_identity_before_the_first_page_script() {
    let mut doc = ScriptDocument::from_html(
        "<body><script>try { rejectInput() } catch (error) { globalThis.saved = error }</script>",
        DocumentConfig::default(),
    );
    doc.install_host_sdk(
        "(makeRefusal) => { globalThis.rejectInput = () => { throw makeRefusal('INPUT_REFUSED:use a number') } }",
    );
    doc.execute_scripts();
    assert!(doc.take_uncaught_errors().is_empty());
    doc.eval("__blitz_send_message(typeof makeRefusal)");
    assert_eq!(doc.take_messages(), ["undefined"]);
    doc.eval("throw saved.message");
    doc.eval("throw Error(saved.message)");
    doc.eval("saved.message = 'changed'; throw saved");
    let entries = doc.take_uncaught_errors();
    assert_eq!(
        entries
            .iter()
            .map(|e| e.refusal.as_deref())
            .collect::<Vec<_>>(),
        [None, None, Some("INPUT_REFUSED:use a number")]
    );
}

#[test]
fn the_embedder_calls_the_private_object_returned_by_its_installer() {
    let mut doc = document();
    doc.install_host_sdk("(makeRefusal) => Object.freeze({check() { throw makeRefusal('INPUT_REFUSED:use a number') }})");
    doc.call_host_sdk("check");
    let errors = doc.take_uncaught_errors();
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].refusal.as_deref(),
        Some("INPUT_REFUSED:use a number")
    );
    doc.install_host_sdk("() => {}");
    doc.call_host_sdk("check");
    let errors = doc.take_uncaught_errors();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].refusal.is_none());
}

#[test]
fn a_broken_trusted_installer_reports_an_ordinary_exception() {
    for source in ["(", "null", "() => { throw Error('setup failed') }"] {
        let mut doc = ScriptDocument::from_html("<body>", DocumentConfig::default());
        doc.install_host_sdk(source);
        let errors = doc.take_uncaught_errors();
        assert_eq!(errors.len(), 1, "{source}");
        assert!(errors[0].refusal.is_none());
    }
}

#[test]
fn each_coded_dom_exception_reports_its_canonical_message() {
    for (write, refusal) in [
        ("document.querySelector('link').rel='help'", LINK),
        ("document.styleSheets[0].deleteRule(0)", SHEET),
        ("document.body.style.webkitTextStroke='1px black'", STROKE),
        (
            "document.styleSheets[0].insertRule('@keyframes x{}',0)",
            KEYFRAMES,
        ),
        ("document.body.innerHTML='<script></script>'", INSERT),
    ] {
        let mut doc = document();
        doc.eval(write);
        let entries = doc.take_uncaught_errors();
        assert_eq!(entries.len(), 1, "{write}: {entries:?}");
        assert_eq!(entries[0].refusal.as_deref(), Some(refusal), "{write}");
        assert!(
            entries[0].message.contains(refusal),
            "{}",
            entries[0].message
        );
        doc.eval(write);
        let messages = doc.take_js_errors();
        assert_eq!(messages, [entries[0].message.clone()]);
        assert!(doc.take_uncaught_errors().is_empty());
    }
}

#[test]
fn an_invalidated_native_font_graph_preserves_its_own_refusal() {
    let mut doc = document();
    let sheet = doc.inner().query_selector("style").unwrap().unwrap();
    doc.inner_mut().upsert_stylesheet_for_node(sheet);
    assert!(!doc.inner().font_stylesheets_valid());
    doc.eval("__blitz_font_snapshot()");
    let entries = doc.take_uncaught_errors();
    let expected = "STYLESHEET_REBUILD_REQUIRED:invalidated font graph";
    assert!(!entries.is_empty());
    assert!(
        entries
            .iter()
            .all(|entry| entry.refusal.as_deref() == Some(expected)),
        "{entries:?}"
    );
}

#[test]
fn the_typed_and_text_drains_share_the_bound_and_the_overflow_marker() {
    let mut doc = document();
    for _ in 0..258 {
        doc.eval("throw Error('ordinary')");
    }
    let entries = doc.take_uncaught_errors();
    assert_eq!(entries.len(), 257);
    assert!(
        entries[..256]
            .iter()
            .all(|entry| entry.message.contains("ordinary"))
    );
    assert_eq!(entries[256].message, "(further errors suppressed)");
    assert!(entries.iter().all(|entry| entry.refusal.is_none()));
    assert!(doc.take_js_errors().is_empty());
}
