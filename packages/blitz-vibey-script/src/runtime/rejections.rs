//! Track rejected promises by identity until the current microtask checkpoint finishes.

use super::{ScriptRuntime, events::report_unhandled_rejection};
use boa_engine::{
    Context, JsError,
    builtins::promise::{OperationType, Promise, PromiseState},
    context::HostHooks,
    object::{JsObject, builtins::JsPromise},
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

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
    /// Module and sample promises are observed by the embedder at their own settlement boundary.
    pub(super) fn finish_rejections(&mut self, module: Option<&JsPromise>) {
        // Release the per-turn promise roots and the backing allocation at the checkpoint.
        let mut pending = self.rejections.pending.borrow_mut();
        self.rejections.sequence.set(0);
        for observed in [module, self.sample_promise.as_ref()].into_iter().flatten() {
            pending.remove(&**observed);
        }
        let mut remaining: Vec<_> = std::mem::take(&mut *pending).into_iter().collect();
        drop(pending);
        remaining.sort_unstable_by_key(|(_, order)| *order);
        for (promise, _) in remaining {
            let promise =
                JsPromise::from_object(promise.upcast()).expect("tracked intrinsic promise");
            if let PromiseState::Rejected(reason) = promise.state() {
                report_unhandled_rejection(
                    &self.ctx,
                    &mut self.context,
                    &JsError::from_opaque(reason),
                );
            }
        }
    }
}
