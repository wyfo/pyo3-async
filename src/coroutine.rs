use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::task::ArcWake;
use pyo3::{exceptions::PyRuntimeError, iter::IterNextOutput, prelude::*};

use crate::{
    utils::{current_thread_id, ThreadId},
    PyFuture, ThrowCallback,
};

pub(crate) trait CoroutineWaker: Sized {
    fn new(py: Python) -> PyResult<Self>;
    fn yield_(&self, py: Python) -> PyResult<PyObject>;
    fn wake(&self, py: Python);
    fn wake_threadsafe(&self, py: Python);
    fn update(&mut self, _py: Python) -> PyResult<()> {
        Ok(())
    }
    fn raise(&self, _py: Python) -> PyResult<()> {
        Ok(())
    }
}

pub(crate) struct Waker<W> {
    inner: W,
    thread_id: ThreadId,
}

impl<W: CoroutineWaker + Send + Sync> ArcWake for Waker<W> {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        if current_thread_id() == arc_self.thread_id {
            Python::with_gil(|gil| CoroutineWaker::wake(&arc_self.inner, gil))
        } else {
            Python::with_gil(|gil| CoroutineWaker::wake_threadsafe(&arc_self.inner, gil))
        }
    }
}

pub(crate) struct Coroutine<W> {
    future: Option<Pin<Box<dyn PyFuture>>>,
    throw: Option<ThrowCallback>,
    waker: Option<Arc<Waker<W>>>,
}

impl<W> Coroutine<W> {
    pub(crate) fn new(future: Pin<Box<dyn PyFuture>>, throw: Option<ThrowCallback>) -> Self {
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

impl<W: CoroutineWaker + Send + Sync + 'static> Coroutine<W> {
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
        let exc = exc.or_else(|| self.waker.as_ref().and_then(|w| w.inner.raise(py).err()));
        match (exc, &mut self.throw) {
            (Some(exc), Some(throw)) => throw(py, Some(exc)),
            (Some(exc), _) => {
                self.future.take();
                return Err(exc);
            }
            _ => {}
        }
        if let Some(waker) = self.waker.as_mut().and_then(Arc::get_mut) {
            waker.inner.update(py)?;
        } else {
            self.waker = Some(Arc::new(Waker {
                inner: W::new(py)?,
                thread_id: current_thread_id(),
            }));
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
            Poll::Pending => IterNextOutput::Yield(self.waker.as_ref().unwrap().inner.yield_(py)?),
        })
    }
}
