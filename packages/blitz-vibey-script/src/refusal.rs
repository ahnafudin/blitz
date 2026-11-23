//! Keep native refusals attached to their JavaScript exception objects across catches and rethrows.
use boa_engine::{Context, JsError, JsNativeError, JsString, object::builtins::JsWeakMap};

use crate::UncaughtScriptError;

// A page cannot inspect this context-owned map. Its weak keys let a caught error be collected.
#[derive(Clone)]
struct RefusalTags(JsWeakMap);

pub(crate) fn type_error(message: &'static str, context: &mut Context) -> JsError {
    let error = JsNativeError::typ()
        .with_message(message)
        .into_opaque(context);
    let tags = match context.get_data::<RefusalTags>() {
        Some(tags) => tags.clone(),
        None => {
            let tags = RefusalTags(JsWeakMap::new(context));
            context.insert_data(tags.clone());
            tags
        }
    };
    match tags.0.set(&error, JsString::from(message).into(), context) {
        Ok(_) => JsError::from_opaque(error.into()),
        Err(failure) => failure,
    }
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
