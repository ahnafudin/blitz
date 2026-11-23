//! Keep native refusals attached to their JavaScript exception objects across catches and rethrows.
use boa_engine::{
    Context, JsError, JsNativeError, JsResult, JsString, JsValue, NativeFunction, Source,
    object::{
        FunctionObjectBuilder, JsObject,
        builtins::{JsArray, JsWeakMap},
    },
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

pub(crate) fn install_sdk(source: &str, context: &mut Context) -> JsResult<Option<JsObject>> {
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
    let result = installer.call(&JsValue::undefined(), &[factory.into()], context)?;
    Ok(result.as_object())
}

pub(crate) fn call_sdk(
    host: Option<&JsObject>,
    method: &str,
    context: &mut Context,
) -> JsResult<JsValue> {
    let host =
        host.ok_or_else(|| JsNativeError::typ().with_message("the host SDK is unavailable"))?;
    let callback = host
        .get(JsString::from(method), context)?
        .as_callable()
        .ok_or_else(|| JsNativeError::typ().with_message("the host SDK method must be callable"))?;
    callback.call(&host.clone().into(), &[], context)
}

/// Read a dense, fixed-size array of native nonnegative integer counters without coercion.
pub(crate) fn counters<const N: usize>(
    host: Option<&JsObject>,
    method: &str,
    context: &mut Context,
) -> Option<[u32; N]> {
    let value = call_sdk(host, method, context).ok()?;
    let array = JsArray::from_object(value.as_object()?).ok()?;
    if array.length(context).ok()? != N as u64 {
        return None;
    }
    let mut counters = [0; N];
    for (index, out) in counters.iter_mut().enumerate() {
        let key = u32::try_from(index).ok()?;
        let property = array.borrow().properties().get(&key.into())?;
        let number = property.value()?.as_number()?;
        if !(0.0..=f64::from(u32::MAX)).contains(&number) || number.fract() != 0.0 {
            return None;
        }
        *out = number as u32;
    }
    Some(counters)
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
