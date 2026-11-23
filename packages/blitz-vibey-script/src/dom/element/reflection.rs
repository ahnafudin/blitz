//! With `ahcut`, reflect HTML properties through their element interfaces and content attributes.

use std::collections::HashMap;

use super::attributes::{clear_attr, write_attr};
use crate::dom::{
    define_native_accessor, dom_ctx, font_mutations, js_str, this_node_id, to_rust_string,
};
use crate::state::DomCtx;
use blitz_dom::{NodeId, ns};
use boa_engine::{Context, JsNativeError, JsResult, JsValue, NativeFunction, object::JsObject};
use url::Url;

#[derive(Clone, Copy)]
enum Kind {
    String,
    Url,
    Direction,
    Boolean,
    Signed(i32),
    Clamped { min: u32, max: u32 },
}

struct Reflection {
    property: &'static str,
    attribute: &'static str,
    // An empty list means every HTMLElement, including custom elements.
    tags: &'static [&'static str],
    kind: Kind,
}

const fn row(
    property: &'static str,
    attribute: &'static str,
    tags: &'static [&'static str],
    kind: Kind,
) -> Reflection {
    Reflection {
        property,
        attribute,
        tags,
        kind,
    }
}

const REFLECTIONS: &[Reflection] = &[
    row("dir", "dir", &[], Kind::Direction),
    row("lang", "lang", &[], Kind::String),
    row("alt", "alt", &["img", "input", "area"], Kind::String),
    row("rel", "rel", &["a", "area", "form", "link"], Kind::String),
    row(
        "media",
        "media",
        &["link", "meta", "source", "style"],
        Kind::String,
    ),
    row("open", "open", &["details", "dialog"], Kind::Boolean),
    row(
        "colSpan",
        "colspan",
        &["td", "th"],
        Kind::Clamped { min: 1, max: 1000 },
    ),
    row(
        "rowSpan",
        "rowspan",
        &["td", "th"],
        Kind::Clamped { min: 0, max: 65534 },
    ),
    row("start", "start", &["ol"], Kind::Signed(1)),
    row("reversed", "reversed", &["ol"], Kind::Boolean),
    row("value", "value", &["li"], Kind::Signed(0)),
    row(
        "span",
        "span",
        &["col", "colgroup"],
        Kind::Clamped { min: 1, max: 1000 },
    ),
    row("href", "href", &["a", "area", "link", "base"], Kind::Url),
    row(
        "src",
        "src",
        &[
            "img", "iframe", "script", "embed", "source", "video", "audio", "track", "input",
        ],
        Kind::Url,
    ),
    row("srcset", "srcset", &["img", "source"], Kind::String),
];

pub(in crate::dom) fn prototypes(
    base: &JsObject,
    context: &mut Context,
) -> (JsObject, HashMap<&'static str, JsObject>) {
    let html = JsObject::with_object_proto(context.intrinsics());
    html.set_prototype(Some(base.clone()));
    let mut tags = HashMap::new();
    for row in REFLECTIONS {
        if row.tags.is_empty() {
            row.install(&html, context);
        }
        for &tag in row.tags {
            let proto = tags.entry(tag).or_insert_with(|| {
                let proto = JsObject::with_object_proto(context.intrinsics());
                proto.set_prototype(Some(html.clone()));
                proto
            });
            row.install(proto, context);
        }
    }
    (html, tags)
}

impl Reflection {
    fn install(&'static self, proto: &JsObject, context: &mut Context) {
        let get =
            NativeFunction::from_copy_closure(move |this, _, context| self.get(this, context));
        let set = NativeFunction::from_copy_closure(move |this, args, context| {
            self.set(this, args, context)
        });
        define_native_accessor(proto, self.property, Some(get), Some(set), context);
    }

    fn receiver(&self, this: &JsValue, ctx: &DomCtx) -> JsResult<NodeId> {
        let id = this_node_id(this)?;
        let doc = ctx.doc.borrow();
        if doc
            .get_node(id)
            .and_then(|node| node.element_data())
            .is_some_and(|el| {
                el.name.ns == ns!(html)
                    && (self.tags.is_empty() || self.tags.contains(&el.name.local.as_ref()))
            })
        {
            Ok(id)
        } else {
            Err(JsNativeError::typ()
                .with_message(format!(
                    "{} called on an incompatible element",
                    self.property
                ))
                .into())
        }
    }

    fn get(&self, this: &JsValue, context: &mut Context) -> JsResult<JsValue> {
        let ctx = dom_ctx(context)?;
        let id = self.receiver(this, &ctx)?;
        let doc = ctx.doc.borrow();
        let el = doc
            .get_node(id)
            .and_then(|node| node.element_data())
            .expect("checked HTML element");
        let attr = el
            .attrs()
            .iter()
            .find(|attr| attr.name.ns == ns!() && attr.name.local.as_ref() == self.attribute)
            .map(|attr| attr.value.as_str());
        let raw = attr.unwrap_or_default();
        Ok(match self.kind {
            Kind::String => js_str(raw),
            Kind::Boolean => attr.is_some().into(),
            Kind::Direction => js_str(
                ["ltr", "rtl", "auto"]
                    .into_iter()
                    .find(|dir| raw.eq_ignore_ascii_case(dir))
                    .unwrap_or(""),
            ),
            Kind::Url if attr.is_none() && el.name.local.as_ref() != "base" => js_str(""),
            Kind::Url => {
                let state = ctx.state.borrow();
                let url = Url::options()
                    .base_url(state.base_url.as_ref())
                    .parse(raw)
                    .ok();
                js_str(url.as_ref().map_or(raw, Url::as_str))
            }
            Kind::Signed(default) => style::servo::attr::parse_integer(raw.chars())
                .unwrap_or(default)
                .into(),
            Kind::Clamped { min, max } => clamped_span(raw, min, max).into(),
        })
    }

    fn set(&self, this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
        let ctx = dom_ctx(context)?;
        let id = self.receiver(this, &ctx)?;
        font_mutations::reflected_write(&ctx.doc.borrow(), id)?;
        let value = args.first().cloned().unwrap_or_default();
        let attr = match self.kind {
            Kind::String | Kind::Direction | Kind::Url => Some(to_rust_string(&value, context)?),
            Kind::Boolean => value.to_boolean().then(String::new),
            Kind::Signed(_) => Some(value.to_i32(context)?.to_string()),
            Kind::Clamped { .. } => {
                let value = value.to_u32(context)?;
                Some(if value > i32::MAX as u32 { 1 } else { value }.to_string())
            }
        };
        font_mutations::reflected_write(&ctx.doc.borrow(), id)?;
        match attr {
            Some(attr) => write_attr(&ctx, id, self.attribute, &attr),
            None => clear_attr(&ctx, id, self.attribute),
        }
        font_mutations::valid(context)?;
        Ok(JsValue::undefined())
    }
}

/// HTML non-negative integer parsing, saturating before clamping even for very long digit strings.
fn clamped_span(raw: &str, min: u32, max: u32) -> u32 {
    let raw = raw.trim_start_matches([' ', '\t', '\n', '\r', '\u{000c}']);
    let negative = raw.starts_with('-');
    let digits = raw.strip_prefix(['+', '-']).unwrap_or(raw);
    let mut digits = digits.bytes().take_while(u8::is_ascii_digit).peekable();
    if digits.peek().is_none() {
        return 1;
    }
    let value = digits.fold(0u32, |value, digit| {
        value
            .saturating_mul(10)
            .saturating_add(u32::from(digit - b'0'))
    });
    if negative && value != 0 {
        1
    } else {
        value.clamp(min, max)
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, REFLECTIONS};
    use crate::ScriptDocument;
    use blitz_dom::DocumentConfig;

    #[test]
    fn every_reflection_row_installs_a_working_accessor_on_each_declared_element() {
        let config = DocumentConfig {
            base_url: Some("https://assets.invalid/".into()),
            ..DocumentConfig::default()
        };
        let mut doc = ScriptDocument::from_html("<!doctype html><body></body>", config);
        doc.execute_scripts();
        for row in REFLECTIONS {
            let (input, attribute, readback) = match row.kind {
                Kind::Direction => ("'LtR'", "LtR", "'ltr'"),
                Kind::String => ("'sample'", "sample", "'sample'"),
                Kind::Url => ("'sample'", "sample", "'https://assets.invalid/sample'"),
                Kind::Boolean => ("true", "", "true"),
                Kind::Signed(_) | Kind::Clamped { .. } => ("7", "7", "7"),
            };
            let tags = if row.tags.is_empty() {
                &["div", "x-custom"][..]
            } else {
                row.tags
            };
            for tag in tags {
                let property = row.property;
                let name = row.attribute;
                doc.eval(&format!("(()=>{{const el=document.createElement('{tag}');el.{property}={input};__blitz_send_message(String(el.getAttribute('{name}')==='{attribute}' && el.{property}==={readback}))}})()"));
                assert_eq!(
                    doc.take_js_errors(),
                    Vec::<String>::new(),
                    "{tag}.{property}"
                );
                assert_eq!(doc.take_messages(), ["true"], "{tag}.{property}");
            }
        }
    }
}
