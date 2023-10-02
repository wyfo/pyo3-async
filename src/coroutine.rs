use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::task::ArcWake;
use pyo3::{exceptions::PyRuntimeError, iter::IterNextOutput, prelude::*};

use crate::PyFuture;

pub(crate) trait CoroutineWaker: Sized {
    fn new(py: Python) -> PyResult<Self>;
    fn yield_(&self, py: Python) -> PyResult<PyObject>;
    fn wake(&self, py: Python);
    fn update(&mut self, _py: Python) -> PyResult<()> {
        Ok(())
    }
    fn raise(&self, _py: Python) -> PyResult<()> {
        Ok(())
    }
}

pub(crate) struct Coroutine<W> {
    future: Option<Pin<Box<dyn PyFuture>>>,
    throw: Option<Box<dyn FnMut(Python, Option<PyErr>) + Send>>,
    waker: Option<Arc<W>>,
}

impl<W> Coroutine<W> {
    pub(crate) fn new(
        future: Pin<Box<dyn PyFuture>>,
        throw: Option<Box<dyn FnMut(Python, Option<PyErr>) + Send>>,
    ) -> Self {
        Self {
            future: Some(future),
            throw,
            waker: None,
        }
    }

    pub(crate) fn close(&mut self, py: Python) -> PyResult<()> {
        if let Some(mut future_rs) = self.future.take() {
            if let Some(ref mut throw) = self.throw {
                throw(py, None);
                let waker = futures::task::noop_waker();
                let res = future_rs
                    .as_mut()
                    .poll_py(py, &mut Context::from_waker(&waker));
                if let Poll::Ready(Err(err)) = res {
                    return Err(err);
                }
            }
        }
        Ok(())
    }
}

impl<W: CoroutineWaker + ArcWake + 'static> Coroutine<W> {
    pub(crate) fn poll(
        &mut self,
        py: Python,
        exc: Option<PyErr>,
    ) -> PyResult<IterNextOutput<PyObject, PyObject>> {
        let Some(ref mut future_rs) = self.future else {
            return Err(PyRuntimeError::new_err(
                "cannot reuse already awaited coroutine",
            ));
        };
        let exc = exc.or_else(|| self.waker.as_ref().and_then(|w| w.raise(py).err()));
        match (exc, &mut self.throw) {
            (Some(exc), Some(throw)) => throw(py, Some(exc)),
            (Some(exc), _) => {
                self.future.take();
                return Err(exc);
            }
            _ => {}
        }
        if let Some(waker) = self.waker.as_mut().and_then(Arc::get_mut) {
            waker.update(py)?;
        } else {
            self.waker = Some(Arc::new(W::new(py)?));
        }
        let waker = futures::task::waker(self.waker.clone().unwrap());
        let res = future_rs
            .as_mut()
            .poll_py(py, &mut Context::from_waker(&waker));
        Ok(match res {
            Poll::Ready(res) => {
                self.future.take();
                IterNextOutput::Return(res?)
            }
            Poll::Pending => IterNextOutput::Yield(self.waker.as_ref().unwrap().yield_(py)?),
        })
    }
}
