//! HTML image bindings and task delivery for the document's current image selections.

use std::collections::BTreeMap;

use blitz_dom::{BaseDocument, ImageSelectionStatus, NodeId};
use boa_engine::builtins::promise::ResolvingFunctions;
use boa_engine::job::PromiseJob;
use boa_engine::object::{FunctionObjectBuilder, builtins::JsPromise};
use boa_engine::property::{Attribute, PropertyDescriptor};
use boa_engine::{Context, JsNativeError, JsResult, JsValue, NativeFunction, js_string};

use super::{ScriptRuntime, events::report_js_error};
use crate::dom::{
    define_accessor, define_method, define_value, dom_ctx, dom_exception, element::write_attr,
    node_wrapper, qual_name, this_node_id,
};
use crate::state::DomCtx;

// The document allocates tokens in selection order. Within a selection, resolve callers
// in registration order. Never let a hash-table iteration order schedule promise reactions.
pub(crate) type ImageDecodes = BTreeMap<u64, (NodeId, Vec<ResolvingFunctions>)>;

pub(super) fn install(ctx: &DomCtx, context: &mut Context) {
    let proto = ctx.state.borrow().protos().html_elements["img"].clone();
    define_accessor(&proto, "naturalWidth", Some(natural_width), None, context);
    define_accessor(&proto, "naturalHeight", Some(natural_height), None, context);
    define_accessor(&proto, "complete", Some(complete), None, context);
    define_method(&proto, "decode", 0, decode, context);

    // Both constructor objects expose the same native wrapper prototype. Only the
    // legacy Image factory can construct an element.
    let global = context.global_object();
    let interface = global
        .get(js_string!("HTMLImageElement"), context)
        .expect("HTMLImageElement bootstrap")
        .as_object()
        .expect("HTMLImageElement constructor");
    let html = global
        .get(js_string!("HTMLElement"), context)
        .expect("HTMLElement bootstrap")
        .as_object();
    interface.set_prototype(html);
    let factory = FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(image))
        .name(js_string!("Image"))
        .constructor(true)
        .build();
    for constructor in [&interface, &*factory] {
        constructor
            .define_property_or_throw(
                js_string!("prototype"),
                PropertyDescriptor::builder()
                    .value(proto.clone())
                    .writable(false)
                    .enumerable(false)
                    .configurable(false),
                context,
            )
            .expect("image interface prototype");
    }
    define_value(&proto, "constructor", interface.into(), context);
    context
        .register_global_property(
            js_string!("Image"),
            factory,
            Attribute::WRITABLE.union(Attribute::CONFIGURABLE),
        )
        .expect("image constructor");
    super::image_events::install(ctx, &proto, context);
}

fn image(new_target: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if new_target.is_undefined() {
        return Err(JsNativeError::typ().with_message("use new Image()").into());
    }
    let ctx = dom_ctx(context)?;
    let id = ctx
        .doc
        .borrow_mut()
        .mutate()
        .create_element(qual_name("img"), Vec::new());
    for (name, value) in ["width", "height"].into_iter().zip(args) {
        if !value.is_undefined() {
            let dimension = value.to_u32(context)?;
            write_attr(&ctx, id, name, &dimension.to_string());
        }
    }
    Ok(node_wrapper(&ctx, id, context).into())
}

fn image_node(this: &JsValue, doc: &BaseDocument) -> JsResult<NodeId> {
    let id = this_node_id(this)?;
    if doc
        .get_node(id)
        .and_then(|node| node.element_data())
        .is_some_and(|el| {
            el.name.ns == markup5ever::ns!(html) && el.name.local == blitz_dom::local_name!("img")
        })
    {
        return Ok(id);
    }
    Err(JsNativeError::typ()
        .with_message("expected an HTMLImageElement")
        .into())
}

fn natural_size(this: &JsValue, context: &mut Context) -> JsResult<(u32, u32)> {
    let ctx = dom_ctx(context)?;
    let doc = ctx.doc.borrow();
    let id = image_node(this, &doc)?;
    Ok(doc
        .get_node(id)
        .and_then(|node| node.element_data())
        .and_then(|el| el.image_data())
        .map(|data| data.natural_size())
        .unwrap_or_default())
}

fn natural_width(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(natural_size(this, context)?.0.into())
}

fn natural_height(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(natural_size(this, context)?.1.into())
}

fn complete(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let ctx = dom_ctx(context)?;
    let doc = ctx.doc.borrow();
    let id = image_node(this, &doc)?;
    Ok(doc
        .image_selection(id)
        .is_none_or(|s| s.status != ImageSelectionStatus::Pending)
        .into())
}

fn decode(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let ctx = dom_ctx(context)?;
    let (id, token) = {
        let doc = ctx.doc.borrow();
        let id = match image_node(this, &doc) {
            Ok(id) => id,
            Err(error) => return Ok(JsPromise::reject(error, context)?.into()),
        };
        (id, doc.image_selection(id).map(|selection| selection.token))
    };
    let (promise, resolvers) = JsPromise::new_pending(context);
    let job = PromiseJob::new(move |context| {
        let loaded = {
            let doc = ctx.doc.borrow();
            match doc.image_selection(id).filter(|s| Some(s.token) == token) {
                Some(s) if s.status == ImageSelectionStatus::Pending => {
                    ctx.state
                        .borrow_mut()
                        .image_decodes
                        .entry(s.token)
                        .or_insert_with(|| (id, Vec::new()))
                        .1
                        .push(resolvers);
                    return Ok(JsValue::undefined());
                }
                Some(s) => s.status == ImageSelectionStatus::Loaded,
                None => false,
            }
        };
        settle_decode(resolvers, loaded, context)
    });
    context.enqueue_job(job.into());
    Ok(promise.into())
}

fn settle_decode(
    resolvers: ResolvingFunctions,
    loaded: bool,
    context: &mut Context,
) -> JsResult<JsValue> {
    if loaded {
        resolvers.resolve.call(&JsValue::undefined(), &[], context)
    } else {
        let reason = dom_exception(
            context,
            "EncodingError",
            "The image selection changed or failed to load.",
        )
        .into_opaque(context)?;
        resolvers
            .reject
            .call(&JsValue::undefined(), &[reason], context)
    }
}

impl ScriptRuntime {
    pub(crate) fn poll_images(&mut self) -> bool {
        self.ctx.doc.borrow_mut().handle_messages();
        // Reselection and removing src must reject a waiting decode even if its old
        // resource never answers. Inspect outstanding requests only.
        let mut stale = Vec::new();
        {
            let doc = self.ctx.doc.borrow();
            self.ctx
                .state
                .borrow_mut()
                .image_decodes
                .retain(|&token, (id, waiters)| {
                    if doc.image_selection(*id).is_some_and(|s| s.token == token) {
                        return true;
                    }
                    stale.append(waiters);
                    false
                });
        }
        let mut ran = !stale.is_empty();
        if ran {
            self.settle_image_decodes(stale, false);
            self.run_jobs("superseded image decode");
        }
        // Yield to the document's bounded settle loop when an onload handler
        // repeatedly selects a cached image at the same clock time.
        for _ in 0..32 {
            let next = {
                let mut doc = self.ctx.doc.borrow_mut();
                doc.pop_image_completion().map(|(id, token)| {
                    let selection = doc.image_selection(id).expect("current image completion");
                    (id, token, selection.status == ImageSelectionStatus::Loaded)
                })
            };
            let Some((id, token, loaded)) = next else {
                break;
            };
            ran = true;
            let waiters = self.ctx.state.borrow_mut().image_decodes.remove(&token);
            if let Some((_, waiters)) = waiters {
                self.settle_image_decodes(waiters, loaded);
            }
            self.dispatch_image_event(id, if loaded { "load" } else { "error" });
            self.run_jobs("image completion microtasks");
        }
        ran
    }

    fn settle_image_decodes(&mut self, waiters: Vec<ResolvingFunctions>, loaded: bool) {
        for resolvers in waiters {
            if let Err(error) = settle_decode(resolvers, loaded, &mut self.context) {
                report_js_error(&self.ctx, &mut self.context, "image decode", &error);
            }
        }
    }
}
