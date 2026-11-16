//! Coherent requested font snapshots from one document-owned registry revision.
use crate::dom::{dom_ctx, to_rust_string};
use boa_engine::object::{ObjectInitializer, builtins::JsArray};
use boa_engine::property::Attribute;
use boa_engine::{Context, JsNativeError, JsResult, JsString, JsValue, js_string};

pub(super) fn snapshot(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let requested = args
        .first()
        .filter(|v| !v.is_undefined())
        .map(|v| to_rust_string(v, context))
        .transpose()?;
    let mut waiting = Vec::new();
    if let Some(array) = args.get(1).and_then(|value| value.as_object()) {
        let array = JsArray::from_object(array)?;
        for i in 0..array.length(context)? {
            waiting.push(to_rust_string(&array.get(i, context)?, context)?);
        }
    }
    let ctx = dom_ctx(context)?;
    let snapshot = {
        let mut doc = ctx.doc.borrow_mut();
        doc.resolve(0.0);
        doc.font_loading_snapshot(requested.as_deref(), &waiting)
            .map_err(|message| {
                if message.starts_with("STYLESHEET_REBUILD_REQUIRED") {
                    JsNativeError::typ().with_message(message)
                } else {
                    JsNativeError::syntax().with_message(message)
                }
            })?
    };
    let values: Vec<JsValue> = snapshot
        .faces
        .into_iter()
        .map(|face| {
            ObjectInitializer::new(context)
                .property(js_string!("id"), face.id as f64, Attribute::all())
                .property(
                    js_string!("family"),
                    JsString::from(face.family.as_str()),
                    Attribute::all(),
                )
                .property(js_string!("weight"), face.weight as f64, Attribute::all())
                .property(
                    js_string!("style"),
                    JsString::from(face.style.as_str()),
                    Attribute::all(),
                )
                .property(
                    js_string!("status"),
                    JsString::from(face.status.as_str()),
                    Attribute::all(),
                )
                .build()
                .into()
        })
        .collect();
    let faces = JsArray::from_iter(values, context);
    let ids = JsArray::from_iter(
        snapshot.ids.into_iter().map(|id| JsValue::from(id as f64)),
        context,
    );
    let waiter_values: Vec<JsValue> = snapshot
        .waiter_ids
        .into_iter()
        .map(|ids| {
            JsArray::from_iter(ids.into_iter().map(|id| JsValue::from(id as f64)), context).into()
        })
        .collect();
    let waiter_ids = JsArray::from_iter(waiter_values, context);
    Ok(ObjectInitializer::new(context)
        .property(js_string!("faces"), faces, Attribute::all())
        .property(js_string!("ids"), ids, Attribute::all())
        .property(js_string!("blocked"), snapshot.blocked, Attribute::all())
        .property(
            js_string!("revision"),
            snapshot.revision as f64,
            Attribute::all(),
        )
        .property(js_string!("waiterIds"), waiter_ids, Attribute::all())
        .build()
        .into())
}
