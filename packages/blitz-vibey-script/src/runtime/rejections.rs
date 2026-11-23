//! Track rejected promises by identity until the current microtask checkpoint finishes.

use super::{
    ScriptRuntime,
    events::{report_js_error, report_unhandled_rejection},
};
use boa_engine::{
    Context, JsError, JsString, JsValue,
    builtins::promise::{OperationType, Promise, PromiseState},
    context::HostHooks,
    object::{JsObject, builtins::JsPromise},
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

/// A private evaluation promise belongs to its module or sample for its entire lifetime.
pub(super) enum PromiseOwner<'a> {
    Module(&'a str),
    Sample,
}

#[derive(Default)]
pub(super) struct Rejections {
    pending: RefCell<HashMap<JsObject<Promise>, u64>>,
    sequence: Cell<u64>,
}

impl HostHooks for Rejections {
    fn promise_rejection_tracker(
        &self,
        promise: &JsObject<Promise>,
        operation: OperationType,
        _: &mut Context,
    ) {
        match operation {
            OperationType::Reject => {
                let order = self.sequence.get();
                self.sequence.set(order.saturating_add(1));
                self.pending.borrow_mut().insert(promise.clone(), order);
            }
            OperationType::Handle => {
                self.pending.borrow_mut().remove(promise);
            }
        }
    }
}

impl ScriptRuntime {
    /// Weak keys retain the origin across turns without keeping an abandoned computation alive.
    pub(super) fn own_promise(&mut self, promise: &JsPromise, owner: PromiseOwner<'_>) {
        let origin = match owner {
            PromiseOwner::Module(description) => JsString::from(description).into(),
            PromiseOwner::Sample => JsValue::null(),
        };
        self.promise_owners
            .set(&promise.clone().into(), origin, &mut self.context)
            .expect("native weak map and promise key");
    }

    pub(super) fn finish_rejections(&mut self) {
        // Release the per-turn promise roots and the backing allocation at the checkpoint.
        self.rejections.sequence.set(0);
        let mut remaining: Vec<_> = self.rejections.pending.take().into_iter().collect();
        remaining.sort_unstable_by_key(|(_, order)| *order);
        for (promise, _) in remaining {
            let object = promise.upcast();
            let origin = self
                .promise_owners
                .get(&object, &mut self.context)
                .expect("native weak map and promise key");
            // An active sample has its own reporter. An abandoned sample already has its answer.
            if origin.is_null() {
                continue;
            }
            let promise = JsPromise::from_object(object).expect("tracked intrinsic promise");
            if let PromiseState::Rejected(reason) = promise.state() {
                let error = JsError::from_opaque(reason);
                if let Some(module) = origin.as_string() {
                    report_js_error(
                        &self.ctx,
                        &mut self.context,
                        &module.to_std_string_lossy(),
                        &error,
                    );
                } else {
                    report_unhandled_rejection(&self.ctx, &mut self.context, &error);
                }
            }
        }
    }
}
