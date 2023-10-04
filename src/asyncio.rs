//! `asyncio` compatible coroutine and async generator implementation.
use std::{
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::{FutureExt, Stream, StreamExt};
use pyo3::{
    exceptions::{PyStopAsyncIteration, PyStopIteration},
    prelude::*,
};

use crate::{coroutine, utils};

utils::module!(Asyncio, "asyncio", Future);

fn asyncio_future(py: Python) -> PyResult<PyObject> {
    Asyncio::get(py)?.Future.call0(py)
}

pub(crate) struct Waker {
    call_soon_threadsafe: PyObject,
    future: PyObject,
}

impl coroutine::CoroutineWaker for Waker {
    fn new(py: Python) -> PyResult<Self> {
        let future = asyncio_future(py)?;
        let call_soon_threadsafe = future
            .call_method0(py, "get_loop")?
            .getattr(py, "call_soon_threadsafe")?;
        Ok(Waker {
            call_soon_threadsafe,
            future,
        })
    }

    fn yield_(&self, py: Python) -> PyResult<PyObject> {
        self.future
            .call_method0(py, "__await__")?
            .call_method0(py, "__next__")
    }

    fn wake(&self, py: Python) {
        let set_result = self
            .future
            .getattr(py, "set_result")
            .expect("error while calling Future.set_result");
        self.call_soon_threadsafe
            .call1(py, (set_result, py.None()))
            .expect("error while calling EventLoop.call_soon_threadsafe");
    }

    fn update(&mut self, py: Python) -> PyResult<()> {
        self.future = Asyncio::get(py)?.Future.call0(py)?;
        Ok(())
    }

    fn raise(&self, py: Python) -> PyResult<()> {
        self.future.call_method0(py, "result")?;
        Ok(())
    }
}

utils::generate!(Waker);

/// [`Future`] wrapper for a Python awaitable (in `asyncio` context).
///
/// The future should be polled in the thread where the event loop is running.
pub struct AwaitableWrapper {
    future_iter: PyObject,
    future: Option<PyObject>,
}

impl AwaitableWrapper {
    /// Wrap a Python awaitable.
    pub fn new(awaitable: &PyAny) -> PyResult<Self> {
        Ok(Self {
            future_iter: awaitable.call_method0("__await__")?.extract()?,
            future: None,
        })
    }

    /// GIL-bound [`Future`] reference.
    pub fn as_mut<'a>(
        &'a mut self,
        py: Python<'a>,
    ) -> impl Future<Output = PyResult<PyObject>> + Unpin + 'a {
        utils::WithGil { inner: self, py }
    }
}

impl<'a> Future for utils::WithGil<'_, &'a mut AwaitableWrapper> {
    type Output = PyResult<PyObject>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(fut) = self.inner.future.as_ref() {
            fut.call_method0(self.py, "result")?;
        }
        match self.inner.future_iter.call_method0(self.py, "__next__") {
            Ok(future) => {
                let callback = utils::WakeCallback(Some(cx.waker().clone()));
                future.call_method1(self.py, "add_done_callback", (callback,))?;
                self.inner.future = Some(future);
                Poll::Pending
            }
            Err(err) if err.is_instance_of::<PyStopIteration>(self.py) => {
                Poll::Ready(Ok(err.value(self.py).getattr("value")?.into()))
            }
            Err(err) => Poll::Ready(Err(err)),
        }
    }
}

impl Future for AwaitableWrapper {
    type Output = PyResult<PyObject>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Python::with_gil(|gil| Pin::into_inner(self).as_mut(gil).poll_unpin(cx))
    }
}

/// [`Stream`] wrapper for a Python async generator (in `asyncio` context).
///
/// The stream should be polled in the thread where the event loop is running.
pub struct AsyncGeneratorWrapper {
    async_generator: PyObject,
    next: Option<AwaitableWrapper>,
}

impl AsyncGeneratorWrapper {
    /// Wrap a Python async generator.
    pub fn new(async_generator: &PyAny) -> Self {
        Self {
            async_generator: async_generator.into(),
            next: None,
        }
    }

    /// GIL-bound [`Stream`] reference.
    pub fn as_mut<'a>(
        &'a mut self,
        py: Python<'a>,
    ) -> impl Stream<Item = PyResult<PyObject>> + Unpin + 'a {
        utils::WithGil { inner: self, py }
    }
}

impl<'a> Stream for utils::WithGil<'_, &'a mut AsyncGeneratorWrapper> {
    type Item = PyResult<PyObject>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.inner.next.is_none() {
            let next = self
                .inner
                .async_generator
                .as_ref(self.py)
                .call_method0("__anext__")?;
            self.inner.next = Some(AwaitableWrapper::new(next)?);
        }
        let res = ready!(self.inner.next.as_mut().unwrap().poll_unpin(cx));
        self.inner.next = None;
        Poll::Ready(match res {
            Ok(obj) => Some(Ok(obj)),
            Err(err) if err.is_instance_of::<PyStopAsyncIteration>(self.py) => None,
            Err(err) => Some(Err(err)),
        })
    }
}

impl Stream for AsyncGeneratorWrapper {
    type Item = PyResult<PyObject>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Python::with_gil(|gil| Pin::into_inner(self).as_mut(gil).poll_next_unpin(cx))
    }
}
