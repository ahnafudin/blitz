//! With `ahcut`: `src` and `srcset` as properties of the HTML elements that reflect them. The setter writes the
//! content attribute, so `img.src = url` starts the load that `setAttribute('src', url)` starts.
use super::attributes::{read_attr, write_attr};
use crate::dom::{define_accessor, dom_ctx, js_str, this_node_id, to_rust_string};
use crate::state::DomCtx;
use blitz_dom::NodeId;
use boa_engine::object::JsObject;
use boa_engine::{Context, JsResult, JsString, JsValue};
use url::Url;

pub(super) fn init(proto: &JsObject, context: &mut Context) {
    define_accessor(proto, "src", Some(get_src), Some(set_src), context);
    define_accessor(proto, "srcset", Some(get_srcset), Some(set_srcset), context);
}

/// Whether the element's interface reflects the content attribute `name`. Any other element keeps a plain
/// property of that name, as in a browser.
fn reflects(ctx: &DomCtx, node_id: NodeId, name: &str) -> bool {
    let doc = ctx.doc.borrow();
    let Some(element) = doc.get_node(node_id).and_then(|node| node.element_data()) else {
        return false;
    };
    let tag = element.name.local.as_ref();
    element.name.ns == markup5ever::ns!(html)
        && match name {
            "src" => matches!(
                tag,
                "img"
                    | "iframe"
                    | "script"
                    | "embed"
                    | "source"
                    | "video"
                    | "audio"
                    | "track"
                    | "input"
            ),
            _ => matches!(tag, "img" | "source"),
        }
}

/// `src` reads as a URL resolved against the document's base URL; `srcset` reads as it is written.
fn get(name: &str, this: &JsValue, context: &mut Context) -> JsResult<JsValue> {
    let ctx = dom_ctx(context)?;
    let node_id = this_node_id(this)?;
    if !reflects(&ctx, node_id, name) {
        return Ok(JsValue::undefined());
    }
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
    if reflects(&ctx, node_id, name) {
        let value = to_rust_string(&value, context)?;
        write_attr(&ctx, node_id, name, &value);
    } else if let Some(object) = this.as_object() {
        object.create_data_property_or_throw(JsString::from(name), value, context)?;
    }
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
