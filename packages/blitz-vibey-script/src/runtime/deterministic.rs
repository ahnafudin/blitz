//! High-resolution page time and a repeatable random stream for a driven document.

use boa_engine::object::{FunctionObjectBuilder, ObjectInitializer};
use boa_engine::property::Attribute;
use boa_engine::{Context, JsResult, JsValue, NativeFunction, js_string};

use crate::dom::dom_ctx;

pub(super) fn install(epoch: i64, context: &mut Context) {
    let mut performance = ObjectInitializer::new(context);
    performance
        .property(
            js_string!("timeOrigin"),
            epoch as f64,
            Attribute::CONFIGURABLE,
        )
        .function(NativeFunction::from_fn_ptr(now), js_string!("now"), 0);
    for (method, length) in [
        ("mark", 1),
        ("measure", 1),
        ("clearMarks", 0),
        ("clearMeasures", 0),
    ] {
        performance.function(
            NativeFunction::from_fn_ptr(ignore_measurement),
            boa_engine::JsString::from(method),
            length,
        );
    }
    for (method, length) in [
        ("getEntries", 0),
        ("getEntriesByName", 1),
        ("getEntriesByType", 1),
    ] {
        performance.function(
            NativeFunction::from_fn_ptr(empty_entries),
            boa_engine::JsString::from(method),
            length,
        );
    }
    let performance = performance.build();
    context
        .register_global_property(
            js_string!("performance"),
            performance,
            Attribute::CONFIGURABLE,
        )
        .expect("install performance");
    let math = context
        .global_object()
        .get(js_string!("Math"), context)
        .expect("initial Math object")
        .as_object()
        .expect("Math is an object");
    let function = FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(random))
        .name(js_string!("random"))
        .length(0)
        .build();
    math.set(js_string!("random"), function, true, context)
        .expect("install Math.random");
}

fn now(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(dom_ctx(context)?.state.borrow().elapsed_millis().into())
}

fn ignore_measurement(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

fn empty_entries(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(boa_engine::object::builtins::JsArray::new(context)?.into())
}

fn random(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let ctx = dom_ctx(context)?;
    let mut state = ctx.state.borrow_mut();
    state.sample_hook.record(crate::SampleEffect::Random);
    state.random_state = state.random_state.wrapping_add(0x9e3779b97f4a7c15);
    let mut value = state.random_state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    let high_53 = (value ^ (value >> 31)) >> 11;
    Ok(((high_53 as f64) / 9_007_199_254_740_992.0).into())
}
