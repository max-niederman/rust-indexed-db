use std::ops::Deref;
use std::task::Poll;
use std::{
    cell::RefCell,
    rc::Rc,
    task::{Context, Waker},
};

use wasm_bindgen::{prelude::*, JsCast};

use crate::internal_utils::{create_lazy_ref_cell, wake};

use super::IdbTransactionResult;

type Cb = dyn Fn() + 'static;
type ErrCb = dyn Fn(web_sys::Event) + 'static;
type WakerRef = Rc<RefCell<Option<Waker>>>;
type ResultRef = Rc<RefCell<Option<IdbTransactionResult>>>;

/// IdbTransaction event listeners
#[derive(Debug)]
pub(crate) struct IdbTransactionListeners {
    waker: WakerRef,
    result: ResultRef,
    on_success: Closure<Cb>,
    on_abort: Closure<Cb>,
    on_error: Closure<ErrCb>,
}

impl IdbTransactionListeners {
    pub fn new(inner: &web_sys::IdbTransaction) -> Self {
        let waker = create_lazy_ref_cell();
        let result = create_lazy_ref_cell();

        let on_success =
            base_callback(waker.clone(), result.clone(), IdbTransactionResult::Success);
        let on_error = error_callback(waker.clone(), result.clone());
        let on_abort = base_callback(waker.clone(), result.clone(), IdbTransactionResult::Abort);

        inner.set_oncomplete(Some(on_success.as_ref().unchecked_ref()));
        inner.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        inner.set_onabort(Some(on_abort.as_ref().unchecked_ref()));

        Self {
            waker,
            result,
            on_error,
            on_success,
            on_abort,
        }
    }

    pub fn do_poll(&self, ctx: &Context<'_>) -> Poll<IdbTransactionResult> {
        if let Some(v) = self.result.borrow().deref() {
            Poll::Ready(v.clone())
        } else {
            self.waker.borrow_mut().replace(ctx.waker().clone());
            Poll::Pending
        }
    }
}

fn error_callback(waker: WakerRef, result: ResultRef) -> Closure<ErrCb> {
    /// Returns true if the waker should be called
    fn process(evt: web_sys::Event, result: &ResultRef) -> bool {
        let req: web_sys::IdbRequest = match evt.target() {
            Some(t) => t.unchecked_into(),
            None => {
                return false;
            }
        };
        let err = if let Some(err) = req
            .error()
            .expect("Error unreachable on an errored transaction")
        {
            err
        } else {
            return false;
        };
        let mut result_ref = if let Ok(result_ref) = result.try_borrow_mut() {
            result_ref
        } else {
            return false;
        };

        if result_ref.is_none() {
            result_ref.replace(IdbTransactionResult::Error(err));
            true
        } else {
            false
        }
    }
    let b = Box::new(move |e: web_sys::Event| {
        if process(e, &result) {
            wake(&waker);
        }
    });
    Closure::wrap(b)
}

fn base_callback(waker: WakerRef, result: ResultRef, kind: IdbTransactionResult) -> Closure<Cb> {
    /// Returns true if the waker should be called
    fn process(result: &ResultRef, kind: IdbTransactionResult) -> bool {
        let mut result_ref = if let Ok(v) = result.try_borrow_mut() {
            v
        } else {
            return false;
        };
        if result_ref.is_none() {
            result_ref.replace(kind.clone()); // Clone so this can be Fn and not FnOnce

            true
        } else {
            false
        }
    }

    let b = Box::new(move || {
        if process(&result, kind.clone()) {
            // Clone so this can be Fn and not FnOnce
            wake(&waker);
        }
    });
    Closure::wrap(b)
}