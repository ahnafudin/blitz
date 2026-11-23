//! Explicit virtual epoch regressions on the actual document/Boa/timer path.

use blitz_dom::{BaseDocument, Document, DocumentConfig};
use blitz_vibey_script::ScriptDocument;
use web_time::Duration;

const MAX_DATE_MILLIS: i64 = 8_640_000_000_000_000;
const HTML: &str = "<script>globalThis.firstDate=[Date.now(),new Date().getTime()];</script>";

fn document(epoch: i64) -> ScriptDocument {
    let mut doc =
        ScriptDocument::from_html_with_virtual_epoch(HTML, DocumentConfig::default(), epoch)
            .unwrap();
    doc.execute_scripts();
    assert!(doc.take_js_errors().is_empty());
    doc
}

fn observe(doc: &mut ScriptDocument, expression: &str) -> String {
    doc.eval(&format!(
        "__blitz_send_message(JSON.stringify(({expression})))"
    ));
    assert!(doc.take_js_errors().is_empty());
    let mut messages = doc.take_messages();
    assert_eq!(messages.len(), 1);
    messages.remove(0)
}

#[cfg(feature = "ahcut")]
#[test]
fn performance_and_seeded_random_are_ready_for_the_first_script() {
    let html = "<script>globalThis.start = [performance.timeOrigin, performance.now(), Date.now(), ...Array.from({length: 3}, () => Math.random() * 2 ** 53)];</script>";
    for _ in 0..2 {
        let mut doc =
            ScriptDocument::from_html_with_virtual_epoch(html, DocumentConfig::default(), 1024)
                .unwrap();
        let origin = doc.clock_now();
        doc.execute_scripts();
        assert!(doc.take_js_errors().is_empty());
        assert_eq!(
            observe(&mut doc, "start"),
            "[1024,0,1024,7956156453446585,3886858653415212,238094247788840]"
        );
        doc.try_advance_clock_to(origin + Duration::from_nanos(33_333_333))
            .unwrap();
        assert_eq!(
            observe(
                &mut doc,
                "[performance.timeOrigin, Math.round(performance.now() * 1e6), Date.now()]"
            ),
            "[1024,33333333,1057]"
        );
    }
}

#[cfg(feature = "ahcut")]
#[test]
fn a_timer_microtask_can_cancel_another_due_callback() {
    let mut doc = document(500);
    let origin = doc.clock_now();
    doc.eval(
        r#"
        globalThis.log = [];
        setTimeout(() => {
            log.push('first');
            Promise.resolve().then(() => {
                log.push('promise');
                clearTimeout(timer);
                cancelAnimationFrame(frame);
                clearInterval(interval);
            });
        }, 16);
        const timer = setTimeout(() => log.push('cancelled timer'), 16);
        const frame = requestAnimationFrame(() => log.push('cancelled frame'));
        const interval = setInterval(() => log.push('cancelled interval'), 16);
        setTimeout(() => log.push('last'), 16);
    "#,
    );
    doc.try_advance_clock_to(origin + Duration::from_millis(16))
        .unwrap();
    doc.poll(None);
    assert_eq!(observe(&mut doc, "log"), r#"["first","promise","last"]"#);
    assert!(doc.next_timer_deadline().is_none());
}

#[cfg(feature = "ahcut")]
#[test]
fn performance_instrumentation_is_inert_and_each_query_returns_a_fresh_list() {
    let mut doc = document(500);
    let observed = observe(
        &mut doc,
        r#"(()=>{
        const p=performance, returns=[p.mark("start"),p.measure("total","start"),p.clearMarks(),p.clearMeasures()];
        const first=p.getEntries(); first.push("local");
        return [returns.every(v=>v===undefined),p.getEntries(),p.getEntriesByName("total"),p.getEntriesByType("mark"),p.now()];
    })()"#,
    );
    assert_eq!(observed, "[true,[],[],[],0]");
}

#[test]
fn explicit_epoch_is_visible_to_the_first_project_script_and_new_date() {
    for epoch in [-1, 0, 1_700_000_000_000, MAX_DATE_MILLIS] {
        let mut first = document(epoch);
        let mut second = document(epoch);
        let expected = format!("[{epoch},{epoch}]");
        assert_eq!(observe(&mut first, "firstDate"), expected);
        assert_eq!(observe(&mut second, "firstDate"), expected);
    }
}

#[test]
fn parsed_document_constructor_also_sets_epoch_before_the_first_eval() {
    let base = BaseDocument::new(DocumentConfig::default());
    let mut doc = ScriptDocument::from_base_document_with_virtual_epoch(base, -25).unwrap();
    assert_eq!(
        observe(&mut doc, "[Date.now(),new Date().getTime()]"),
        "[-25,-25]"
    );
}

#[test]
fn forward_date_and_same_deadline_timer_and_raf_order_share_one_origin() {
    let mut doc = document(1_000);
    let origin = doc.clock_now();
    doc.eval("globalThis.order=[];globalThis.frames=[];for(const name of ['A','B','C'])setTimeout(()=>order.push([name,Date.now()]),12);for(const name of ['X','Y','Z'])requestAnimationFrame(t=>frames.push([name,t,Date.now()]));");
    doc.try_advance_clock_to(origin + Duration::from_millis(12))
        .unwrap();
    doc.poll(None);
    assert_eq!(
        observe(&mut doc, "[order,frames]"),
        r#"[[["A",1012],["B",1012],["C",1012]],[]]"#
    );
    doc.try_advance_clock_to(origin + Duration::from_millis(16))
        .unwrap();
    doc.poll(None);
    assert_eq!(
        observe(&mut doc, "frames"),
        r#"[["X",16,1016],["Y",16,1016],["Z",16,1016]]"#
    );
    doc.poll(None);
    assert_eq!(
        observe(&mut doc, "[Date.now(),order.length,frames.length]"),
        "[1016,3,3]"
    );
}

fn sampled_at(milliseconds: u64) -> String {
    let mut doc = document(500);
    doc.eval("globalThis.fired=0;setTimeout(()=>fired++,20);");
    let deadline = doc.clock_now() + Duration::from_millis(milliseconds);
    doc.try_advance_clock_to(deadline).unwrap();
    doc.poll(None);
    observe(&mut doc, "[firstDate,Date.now(),fired]")
}

#[test]
fn repeat_is_stable_and_backward_sampling_requires_a_fresh_document() {
    let mut doc = document(500);
    doc.eval("globalThis.fired=0;setTimeout(()=>fired++,20);");
    let origin = doc.clock_now();
    doc.try_advance_clock_to(origin + Duration::from_millis(40))
        .unwrap();
    doc.poll(None);
    let at40 = observe(&mut doc, "[firstDate,Date.now(),fired]");
    assert_eq!(at40, "[[500,500],540,1]");
    doc.try_advance_clock_to(origin + Duration::from_millis(40))
        .unwrap();
    doc.poll(None);
    assert_eq!(observe(&mut doc, "[firstDate,Date.now(),fired]"), at40);
    doc.try_advance_clock_to(origin).unwrap();
    doc.poll(None);
    assert_eq!(observe(&mut doc, "[firstDate,Date.now(),fired]"), at40);
    assert_eq!(sampled_at(0), "[[500,500],500,0]");
    assert_eq!(sampled_at(20), "[[500,500],520,1]");
    assert_eq!(sampled_at(20), sampled_at(20));
}

#[test]
fn invalid_epoch_and_out_of_range_advance_refuse_without_changing_the_clock() {
    for epoch in [
        i64::MIN,
        -MAX_DATE_MILLIS - 1,
        MAX_DATE_MILLIS + 1,
        i64::MAX,
    ] {
        assert!(
            ScriptDocument::from_html_with_virtual_epoch(HTML, DocumentConfig::default(), epoch)
                .is_err()
        );
    }
    let mut doc = document(MAX_DATE_MILLIS);
    let before = doc.clock_now();
    let beyond = before + Duration::from_millis(1);
    assert!(doc.try_advance_clock_to(beyond).is_err());
    assert_eq!(doc.clock_now(), before);
    assert_eq!(
        observe(&mut doc, "[Date.now(),new Date().getTime()]"),
        "[8640000000000000,8640000000000000]"
    );
    doc.advance_clock_to(beyond);
    let errors = doc.take_js_errors();
    assert_eq!(
        errors,
        vec!["virtual clock advance exceeds ECMAScript Date range".to_string()]
    );
    assert_eq!(doc.clock_now(), before);
    assert_eq!(observe(&mut doc, "Date.now()"), "8640000000000000");
}

#[test]
fn legacy_virtual_switch_does_not_reset_an_explicit_epoch_or_elapsed_time() {
    let mut doc = document(-1);
    doc.try_advance_clock_to(doc.clock_now() + Duration::from_millis(1))
        .unwrap();
    let mut doc = doc.with_virtual_time();
    assert_eq!(
        observe(&mut doc, "[firstDate,Date.now(),new Date().getTime()]"),
        "[[-1,-1],0,0]"
    );
}
