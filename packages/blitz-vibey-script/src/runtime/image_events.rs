//! Dispatch nonbubbling resource events and bind image content attributes to their target listeners.

use std::rc::Rc;

use blitz_dom::NodeId;
use boa_engine::object::{FunctionObjectBuilder, JsObject};
use boa_engine::{Context, JsError, JsResult, JsValue, NativeFunction, Source, js_string};

use super::{ScriptRuntime, events::report_js_error};
use crate::dom::event::{EventRef, create_event};
use crate::dom::{define_value, dom_ctx, js_str, node_wrapper};
use crate::state::{DomCtx, Listener, RuntimeState};

pub(super) fn install(ctx: &DomCtx, proto: &JsObject, context: &mut Context) {
    let report = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(report_handler_error),
    )
    .build();
    let install = context
        .eval(Source::from_bytes(include_bytes!(
            "bootstrap/image_handlers.js"
        )))
        .expect("image handler bootstrap")
        .as_callable()
        .expect("image handler installer");
    let update = install
        .call(
            &JsValue::undefined(),
            &[proto.clone().into(), report.into()],
            context,
        )
        .expect("install image handler accessors")
        .as_callable()
        .expect("image attribute update");
    ctx.state.borrow_mut().image_handler_attribute = Some(update);
}

fn report_handler_error(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let ctx = dom_ctx(context)?;
    let error = JsError::from_opaque(args.first().cloned().unwrap_or_default());
    report_js_error(&ctx, context, "image handler attribute", &error);
    Ok(JsValue::undefined())
}

pub(crate) fn initialize_attributes(
    ctx: &DomCtx,
    id: NodeId,
    image: &JsObject,
    context: &mut Context,
) {
    let attributes = {
        let doc = ctx.doc.borrow();
        let Some(el) = doc.get_node(id).and_then(|node| node.element_data()) else {
            return;
        };
        if el.name.ns != markup5ever::ns!(html) || el.name.local != blitz_dom::local_name!("img") {
            return;
        }
        [("load", "onload"), ("error", "onerror")].map(|(name, attribute)| {
            let value = el
                .attrs()
                .iter()
                .find(|attr| attr.name.ns == markup5ever::ns!() && &*attr.name.local == attribute);
            (name, value.map(|attr| attr.value.clone()))
        })
    };
    for (name, source) in attributes {
        if let Some(source) = source {
            update_attribute(ctx, image, name, Some(&source), context);
        }
    }
}

pub(crate) fn changed_attribute(
    ctx: &DomCtx,
    id: NodeId,
    this: &JsValue,
    name: &str,
    value: Option<&str>,
    context: &mut Context,
) {
    let name = match name {
        "onload" => "load",
        "onerror" => "error",
        _ => return,
    };
    if !ctx
        .doc
        .borrow()
        .get_node(id)
        .and_then(|node| node.element_data())
        .is_some_and(|el| {
            el.name.ns == markup5ever::ns!(html) && el.name.local == blitz_dom::local_name!("img")
        })
    {
        return;
    }
    if let Some(image) = this.as_object() {
        update_attribute(ctx, &image, name, value, context);
    }
}

fn update_attribute(
    ctx: &DomCtx,
    image: &JsObject,
    name: &str,
    value: Option<&str>,
    context: &mut Context,
) {
    let update = ctx.state.borrow().image_handler_attribute.clone();
    if let Some(update) = update
        && let Err(error) = update.call(
            &JsValue::undefined(),
            &[
                image.clone().into(),
                js_str(name),
                value.map_or(JsValue::null(), js_str),
            ],
            context,
        )
    {
        report_js_error(ctx, context, "image handler attribute", &error);
    }
}

fn listeners<'a>(
    state: &'a mut RuntimeState,
    id: Option<NodeId>,
    name: &str,
) -> Option<&'a mut Vec<Listener>> {
    match id {
        Some(id) => state.node_listeners.get_mut(&id)?.get_mut(name),
        None => state.window_listeners.get_mut(name),
    }
}

impl ScriptRuntime {
    pub(super) fn dispatch_image_event(&mut self, id: NodeId, name: &str) {
        let target = node_wrapper(&self.ctx, id, &mut self.context).into();
        let event = create_event(&self.ctx, name, false, false, &target, &mut self.context);
        let global = self.context.global_object();
        let previous = global
            .get(js_string!("event"), &mut self.context)
            .unwrap_or_default();
        define_value(&global, "event", event.clone().into(), &mut self.context);

        // Freeze the path before invoking any author code, including when a parent
        // moves the image. Document's event parent is null for load, Window for error.
        let (ancestors, window_parent) = {
            let doc = self.ctx.doc.borrow();
            let mut ancestors = Vec::new();
            let mut current = id;
            while let Some(parent) = doc.get_node(current).and_then(|node| node.parent) {
                ancestors.push(parent);
                current = parent;
            }
            (ancestors, name != "load" && current == doc.root_node().id)
        };
        define_value(&event, "eventPhase", 1.into(), &mut self.context);
        if window_parent {
            self.invoke_image_listeners(None, name, true, &event);
        }
        for parent in ancestors.into_iter().rev() {
            self.invoke_image_listeners(Some(parent), name, true, &event);
        }
        define_value(&event, "eventPhase", 2.into(), &mut self.context);
        self.invoke_image_listeners(Some(id), name, true, &event);
        self.invoke_image_listeners(Some(id), name, false, &event);
        define_value(&event, "currentTarget", JsValue::null(), &mut self.context);
        define_value(&event, "eventPhase", 0.into(), &mut self.context);
        define_value(&global, "event", previous, &mut self.context);
    }

    fn invoke_image_listeners(
        &mut self,
        id: Option<NodeId>,
        name: &str,
        capture: bool,
        event: &JsObject,
    ) {
        if event
            .downcast_ref::<EventRef>()
            .is_some_and(|e| e.stopped.get())
        {
            return;
        }
        let snapshot = listeners(&mut self.ctx.state.borrow_mut(), id, name)
            .map(|list| {
                list.iter()
                    .filter(|l| l.capture == capture)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if snapshot.is_empty() {
            return;
        }
        let target: JsValue = match id {
            Some(id) => node_wrapper(&self.ctx, id, &mut self.context).into(),
            None => self.context.global_object().into(),
        };
        define_value(event, "currentTarget", target.clone(), &mut self.context);
        for entry in snapshot {
            {
                let mut state = self.ctx.state.borrow_mut();
                let Some(list) = listeners(&mut state, id, name) else {
                    continue;
                };
                let Some(at) = list
                    .iter()
                    .position(|other| Rc::ptr_eq(&entry.registration, &other.registration))
                else {
                    continue;
                };
                if entry.once {
                    list.remove(at);
                }
            }
            if let Err(error) =
                entry
                    .callback
                    .call(&target, &[event.clone().into()], &mut self.context)
            {
                report_js_error(&self.ctx, &mut self.context, "image event listener", &error);
            }
            self.run_jobs("image event listener microtasks");
            if event
                .downcast_ref::<EventRef>()
                .is_some_and(|e| e.stopped_immediate.get())
            {
                break;
            }
        }
    }
}
