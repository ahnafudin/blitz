//! Invoke the two sample hooks in one turn and hold their settlement in the embedding runtime.

use super::{ScriptRuntime, report_js_error};
use boa_engine::builtins::promise::PromiseState;
use boa_engine::object::builtins::JsPromise;
use boa_engine::{JsError, JsNativeError, JsResult, JsValue, Source, js_string};

impl ScriptRuntime {
    pub(crate) fn start_sample(&mut self, seconds: f64) -> Result<(), &'static str> {
        if self.sample_promise.is_some() {
            return Err("a sample is already active");
        }
        self.sync_named_element_globals();
        // Use a fresh intrinsic promise, including for a returned thenable. Neither this
        // promise nor its resolving functions are installed on an author-visible object.
        let (promise, resolvers) = JsPromise::new_pending(&mut self.context);
        self.sample_promise = Some(promise);
        let adopted = self.call_sample_hooks(seconds).and_then(|value| {
            resolvers
                .resolve
                .call(&JsValue::undefined(), &[value], &mut self.context)
        });
        if let Err(error) = adopted {
            report_js_error(&self.ctx, &mut self.context, "sample hooks", &error);
            return Err("the sample hook failed");
        }
        self.run_jobs("sample microtasks");
        Ok(())
    }

    fn call_sample_hooks(&mut self, seconds: f64) -> JsResult<JsValue> {
        let context = &mut self.context;
        let window = context.global_object();
        let receiver: JsValue = window.clone().into();
        let time = [JsValue::from(seconds)];
        // Preserve the existing seek lookup, including a global lexical declaration.
        let seek = context.eval(Source::from_bytes("__seek"))?;
        let seek = seek
            .as_callable()
            .ok_or_else(|| JsNativeError::typ().with_message("__seek must be a function"))?;
        seek.call(&receiver, &time, context)?;
        let hook = window.get(js_string!("__settle"), context)?;
        match hook.as_callable() {
            Some(settle) => settle.call(&receiver, &time, context),
            None => Ok(JsValue::undefined()),
        }
    }

    pub(crate) fn sample_settled(&mut self) -> Result<bool, &'static str> {
        let promise = self.sample_promise.as_ref().ok_or("no active sample")?;
        match promise.state() {
            PromiseState::Pending => Ok(false),
            PromiseState::Fulfilled(_) => Ok(true),
            PromiseState::Rejected(reason) => {
                self.sample_promise = None;
                report_js_error(
                    &self.ctx,
                    &mut self.context,
                    "sample hooks",
                    &JsError::from_opaque(reason),
                );
                Ok(true)
            }
        }
    }
}
