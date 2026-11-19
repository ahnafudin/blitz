//! SVG geometry bindings backed by native usvg bounds and transforms.
use crate::dom::{dom_ctx, this_node_id};
use boa_engine::object::{ObjectInitializer, builtins::JsArray};
use boa_engine::property::Attribute;
use boa_engine::{Context, JsNativeError, JsResult, JsValue, js_string};

pub(super) fn bbox(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if let Some(value) = args.first().filter(|value| !value.is_null_or_undefined()) {
        let options = value.as_object().ok_or_else(|| {
            JsNativeError::typ().with_message("SVG getBBox options must be a dictionary")
        })?;
        for (key, default) in [
            ("fill", true),
            ("stroke", false),
            ("markers", false),
            ("clipped", false),
        ] {
            let value = options.get(boa_engine::JsString::from(key), context)?;
            if !value.is_undefined() && value.to_boolean() != default {
                return Err(JsNativeError::typ()
                    .with_message("SVG getBBox nondefault options are not supported")
                    .into());
            }
        }
    }
    let ctx = dom_ctx(context)?;
    let id = this_node_id(this)?;
    let bounds = ctx
        .doc
        .borrow_mut()
        .svg_bbox(id)
        .map_err(|message| JsNativeError::typ().with_message(message))?;
    Ok(ObjectInitializer::new(context)
        .property(js_string!("x"), bounds[0], Attribute::all())
        .property(js_string!("y"), bounds[1], Attribute::all())
        .property(js_string!("width"), bounds[2], Attribute::all())
        .property(js_string!("height"), bounds[3], Attribute::all())
        .build()
        .into())
}

pub(super) fn ctm(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let ctx = dom_ctx(context)?;
    let id = this_node_id(this)?;
    let transform = ctx
        .doc
        .borrow_mut()
        .svg_ctm(id)
        .map_err(|message| JsNativeError::typ().with_message(message))?;
    Ok(JsArray::from_iter(transform.into_iter().map(JsValue::from), context).into())
}
