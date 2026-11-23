//! With `ahcut`: `src` and `srcset` as properties of the HTML elements that reflect them. The setter writes the
//! content attribute, so `img.src = url` starts the load that `setAttribute('src', url)` starts.
use super::attributes::{read_attr, write_attr};
use crate::dom::{define_accessor, dom_ctx, js_str, this_node_id, to_rust_string};
use boa_engine::object::JsObject;
use boa_engine::{Context, JsResult, JsValue};
use url::Url;

pub(in crate::dom) fn prototypes(
    base: &JsObject,
    context: &mut Context,
) -> std::collections::HashMap<&'static str, JsObject> {
    let mut prototypes = std::collections::HashMap::new();
    for tag in [
        "img", "iframe", "script", "embed", "source", "video", "audio", "track", "input",
    ] {
        let proto = JsObject::with_object_proto(context.intrinsics());
        proto.set_prototype(Some(base.clone()));
        define_accessor(&proto, "src", Some(get_src), Some(set_src), context);
        if matches!(tag, "img" | "source") {
            define_accessor(
                &proto,
                "srcset",
                Some(get_srcset),
                Some(set_srcset),
                context,
            );
        }
        prototypes.insert(tag, proto);
    }
    prototypes
}

/// `src` reads as a URL resolved against the document's base URL; `srcset` reads as it is written.
fn get(name: &str, this: &JsValue, context: &mut Context) -> JsResult<JsValue> {
    let ctx = dom_ctx(context)?;
    let node_id = this_node_id(this)?;
    let Some(value) = read_attr(&ctx, node_id, name) else {
        return Ok(js_str(""));
    };
    if name != "src" {
        return Ok(js_str(&value));
    }
    let resolved = match &ctx.state.borrow().base_url {
        Some(base) => base.join(&value).ok(),
        None => Url::parse(&value).ok(),
    };
    Ok(js_str(
        resolved.as_ref().map_or(value.as_str(), Url::as_str),
    ))
}

fn set(name: &str, this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let ctx = dom_ctx(context)?;
    let node_id = this_node_id(this)?;
    let value = args.first().cloned().unwrap_or_default();
    let value = to_rust_string(&value, context)?;
    write_attr(&ctx, node_id, name, &value);
    Ok(JsValue::undefined())
}

fn get_src(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    get("src", this, context)
}

fn set_src(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    set("src", this, args, context)
}

fn get_srcset(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    get("srcset", this, context)
}

fn set_srcset(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    set("srcset", this, args, context)
}
