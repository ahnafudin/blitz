//! Resource event listeners and document load ordering with host-validated and held HTML images.
#![cfg(feature = "ahcut")]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use blitz_dom::{Document, DocumentConfig};
use blitz_traits::net::{ImageFacts, NetHandler, NetProvider, Request, Url};
use blitz_vibey_script::ScriptDocument;

#[derive(Default)]
struct Images(Mutex<HashMap<String, Box<dyn NetHandler>>>);

impl NetProvider for Images {
    fn image_facts(&self, _: usize, url: &Url) -> Result<Option<ImageFacts>, String> {
        match url.path() {
            "/ok" => Ok(Some(ImageFacts {
                asset: 1,
                width: 6,
                height: 9,
                has_alpha: true,
            })),
            "/bad" | "/bad-again" => Err("image missing".into()),
            _ => Ok(None),
        }
    }

    fn fetch(&self, _: usize, request: Request, handler: Box<dyn NetHandler>) {
        assert!(
            self.0
                .lock()
                .unwrap()
                .insert(request.url.to_string(), handler)
                .is_none()
        );
    }
}

impl Images {
    fn fail(&self, path: &str) {
        let url = format!("https://assets.invalid/{path}");
        let handler = self.0.lock().unwrap().remove(&url).expect("held image");
        handler.error(url, "image missing".into());
    }
}

fn document(html: &str) -> (ScriptDocument, Arc<Images>) {
    let net = Arc::new(Images::default());
    let config = DocumentConfig {
        base_url: Some("https://assets.invalid/".into()),
        net_provider: Some(net.clone()),
        ..Default::default()
    };
    let mut doc = ScriptDocument::from_html_with_virtual_epoch(html, config, 0).unwrap();
    doc.execute_scripts();
    (doc, net)
}

fn check(doc: &mut ScriptDocument, expected: &str) {
    for _ in 0..8 {
        if !doc.poll(None) {
            break;
        }
    }
    doc.eval("__blitz_send_message(log.join(','))");
    assert_eq!(doc.take_js_errors(), Vec::<String>::new());
    assert_eq!(doc.take_messages(), [expected]);
}

#[test]
fn markup_and_property_share_one_handler_slot_on_the_image() {
    let (mut doc, _) = document(
        r#"<!doctype html><img id="i" src="ok"
        onload="log.push(id + ':' + event.type);" onerror="log.push('old');"><script>
        const el = document.getElementById('i'), log = [];
        el.addEventListener('load', () => log.push('registered'));
        el.addEventListener('error', () => log.push('error listener'));
        el.onerror = e => log.push(e.currentTarget.id + ':property');
    </script>"#,
    );
    check(&mut doc, "i:load,registered");
    doc.eval("log.length=0; el.src='bad'");
    check(&mut doc, "i:property,error listener");
    doc.eval("log.length=0; el.removeAttribute('onerror'); el.setAttribute('onerror', \"log.push(id + ':' + event.type)\"); el.src='bad-again'");
    check(&mut doc, "error listener,i:error");
}

#[test]
fn image_capture_uses_the_event_path_and_honours_removal_and_stop() {
    let (mut doc, _) = document(
        r#"<!doctype html><section id="p"><img id="i" src="ok"></section><script>
        const el = document.getElementById('i'), p = document.getElementById('p'), log = [];
        function removed() { log.push('removed'); }
        for (const type of ['load','error']) {
            window.addEventListener(type, e => { if (e.target === el) log.push('window'); }, true);
            document.addEventListener(type, e => { if (e.target === el) log.push('doc' + e.eventPhase); }, true);
            p.addEventListener(type, () => log.push('parent bubble'));
            el.addEventListener(type, () => log.push('target'));
            el.addEventListener(type, e => log.push('target' + e.eventPhase), true);
        }
        p.addEventListener('load', e => {
            log.push('parent' + e.eventPhase); p.removeEventListener('load', removed, true);
            e.stopPropagation();
        }, {capture:true, once:true});
        p.addEventListener('load', removed, true);
    </script>"#,
    );
    check(&mut doc, "doc1,parent1");
    doc.eval("log.length=0; el.src='bad'");
    check(&mut doc, "window,doc1,target2,target");
    doc.eval("log.length=0; el.src='ok'");
    check(&mut doc, "doc1,target2,target");
    doc.eval("log.length=0; el.remove(); el.src='bad'");
    check(&mut doc, "target2,target");
}

#[test]
fn document_load_waits_through_the_last_current_image_task() {
    let (mut doc, net) = document(
        r#"<!doctype html><img id="i" src="old"><script>
        const el = document.getElementById('i'), log = [];
        el.src = 'first';
        document.addEventListener('DOMContentLoaded', () => log.push(document.readyState));
        el.onerror = () => {
            log.push('error:' + document.readyState);
            if (el.getAttribute('src') === 'first') queueMicrotask(() => { el.src = 'last'; });
        };
        window.onload = e => log.push('window:' + document.readyState + ':' +
            (e.target === document && e.currentTarget === window));
    </script>"#,
    );
    check(&mut doc, "interactive");
    net.fail("first");
    check(&mut doc, "interactive,error:interactive");
    net.fail("last");
    check(
        &mut doc,
        "interactive,error:interactive,error:interactive,window:complete:true",
    );
    net.fail("old");
    check(
        &mut doc,
        "interactive,error:interactive,error:interactive,window:complete:true",
    );
}
