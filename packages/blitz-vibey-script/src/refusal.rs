//! Keep native refusals attached to their JavaScript exception objects across catches and rethrows.
use boa_engine::{
    Context, JsError, JsNativeError, JsResult, JsString, JsValue, NativeFunction, Source,
    object::{FunctionObjectBuilder, JsObject, builtins::JsWeakMap},
};

use crate::UncaughtScriptError;

// A page cannot inspect this context-owned map. Its weak keys let a caught error be collected.
#[derive(Clone)]
struct RefusalTags(JsWeakMap);

fn tag(error: JsObject, message: JsString, context: &mut Context) -> JsResult<JsValue> {
    let tags = match context.get_data::<RefusalTags>() {
        Some(tags) => tags.clone(),
        None => {
            let tags = RefusalTags(JsWeakMap::new(context));
            context.insert_data(tags.clone());
            tags
        }
    };
    tags.0.set(&error, message.into(), context)?;
    Ok(error.into())
}

pub(crate) fn type_error(message: &'static str, context: &mut Context) -> JsError {
    let error = JsNativeError::typ()
        .with_message(message)
        .into_opaque(context);
    tag(error, JsString::from(message), context).map_or_else(|error| error, JsError::from_opaque)
}

fn make_refusal(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let message = args.first().and_then(JsValue::as_string).ok_or_else(|| {
        JsNativeError::typ().with_message("the SDK refusal factory requires a string")
    })?;
    let error = JsNativeError::error()
        .with_message(message.to_std_string_lossy())
        .into_opaque(context);
    tag(error, message, context)
}

pub(crate) fn install_sdk(source: &str, context: &mut Context) -> JsResult<()> {
    let installer = context
        .eval(Source::from_bytes(source))?
        .as_callable()
        .ok_or_else(|| {
            JsNativeError::typ().with_message("the host SDK installer must be a function")
        })?;
    let factory =
        FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(make_refusal))
            .length(1)
            .build();
    installer.call(&JsValue::undefined(), &[factory.into()], context)?;
    Ok(())
}

pub(crate) fn uncaught(
    message: String,
    error: &JsError,
    context: &mut Context,
) -> UncaughtScriptError {
    let refusal = error
        .as_opaque()
        .and_then(|value| value.as_object())
        .and_then(|object| {
            let tags = context.get_data::<RefusalTags>()?.clone();
            tags.0
                .get(&object, context)
                .ok()?
                .as_string()
                .map(|s| s.to_std_string_lossy())
        });
    UncaughtScriptError { message, refusal }
}
