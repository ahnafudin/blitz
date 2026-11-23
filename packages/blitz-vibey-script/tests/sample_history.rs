//! Runtime history observations use the native operation even through aliases and promise callbacks.
#![cfg(feature = "ahcut")]

use blitz_dom::DocumentConfig;
use blitz_vibey_script::{SampleEffect, ScriptDocument};

#[test]
fn a_second_host_call_cannot_replace_the_native_pending_promise() {
    let mut doc=ScriptDocument::from_html_with_virtual_epoch(
        "<script>let calls=0,finish; __seek=()=>{calls++}; __settle=()=>new Promise(r=>finish=r);</script>",
        DocumentConfig::default(), 0).unwrap();
    doc.execute_scripts();
    let before = doc.clock_now();
    doc.begin_sample_hook();
    doc.start_sample(0.25).unwrap();
    assert!(!doc.sample_settled().unwrap());
    assert!(doc.start_sample(1.0).is_err());
    doc.eval("if(calls!==1) throw Error('duplicate seek'); globalThis.__settle=()=>123;finish()");
    assert!(doc.sample_settled().unwrap());
    assert_eq!(doc.finish_sample_hook(), None);
    assert_eq!(doc.clock_now(), before);
    assert!(doc.take_js_errors().is_empty());
}

#[test]
fn the_observation_includes_the_settle_hook_and_its_promise_jobs() {
    let mut doc = ScriptDocument::from_html_with_virtual_epoch(
        "<script>__seek=()=>{};__settle=()=>Promise.resolve().then(()=>Math.random());</script>",
        DocumentConfig::default(),
        0,
    )
    .unwrap();
    doc.execute_scripts();
    doc.begin_sample_hook();
    doc.start_sample(0.5).unwrap();
    assert!(doc.sample_settled().unwrap());
    assert_eq!(doc.finish_sample_hook(), Some(SampleEffect::Random));
    doc.eval("if(typeof __blitz_poll_fonts!=='undefined') throw Error('font settlement driver should be private')");
    assert!(doc.take_js_errors().is_empty());
}

#[test]
fn setup_is_outside_the_observation_and_a_caught_exception_cannot_clear_the_first_effect() {
    let mut doc = ScriptDocument::from_html_with_virtual_epoch(
        "<script>const draw = Math.random, sleep = setTimeout, frame = requestAnimationFrame;
        draw(); sleep(()=>{}, 10000); frame(()=>{});</script>",
        DocumentConfig::default(),
        0,
    )
    .unwrap();
    doc.execute_scripts();
    assert_eq!(doc.finish_sample_hook(), None);
    doc.eval("if (typeof globalThis.__blitz_note_dom_toggle !== 'undefined' || Object.hasOwn(globalThis, '__blitz_note_dom_toggle')) throw Error('the DOM toggle observer survived bootstrap')");
    assert!(doc.take_js_errors().is_empty());
    for (script, expected) in [
        ("void draw;", None),
        (
            "try { draw(); throw Error('caught'); } catch {} sleep(()=>{},10);",
            Some(SampleEffect::Random),
        ),
        ("sleep(()=>{},10);", Some(SampleEffect::Timer)),
        (
            "clearInterval(setInterval(()=>{},5));",
            Some(SampleEffect::Timer),
        ),
        (
            "Promise.resolve().then(()=>frame(()=>{}));",
            Some(SampleEffect::AnimationFrame),
        ),
        ("sleep(null); frame(null);", None),
        (
            "document.documentElement.classList.toggle('active', true);",
            None,
        ),
        (
            "document.documentElement.classList.toggle('active');",
            Some(SampleEffect::DomToggle),
        ),
    ] {
        doc.begin_sample_hook();
        doc.eval(script);
        doc.begin_sample_hook();
        assert_eq!(doc.finish_sample_hook(), expected, "{script}");
        assert!(doc.take_js_errors().is_empty());
    }
}
