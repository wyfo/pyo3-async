use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use futures::{FutureExt, Stream};
use pyo3::prelude::*;

mod async_generator;
pub mod asyncio;
mod coroutine;
pub mod sniffio;
pub mod trio;
mod utils;

pub trait PyFuture: Send {
    fn poll_py(self: Pin<&mut Self>, py: Python, waker: &Waker) -> Poll<PyResult<PyObject>>;
}

impl<F, T, E> PyFuture for F
where
    F: Future<Output = Result<T, E>> + Send,
    T: IntoPy<PyObject> + Send,
    E: Send,
    PyErr: From<E>,
{
    fn poll_py(self: Pin<&mut Self>, py: Python, waker: &Waker) -> Poll<PyResult<PyObject>> {
        py.allow_threads(|| Future::poll(self, &mut Context::from_waker(waker)))
            .map_ok(|ok| ok.into_py(py))
            .map_err(PyErr::from)
    }
}

pub trait PyStream: Send {
    fn poll_next_py(
        self: Pin<&mut Self>,
        py: Python,
        waker: &Waker,
    ) -> Poll<Option<PyResult<PyObject>>>;

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, None)
    }
}

impl<S, T, E> PyStream for S
where
    S: Stream<Item = Result<T, E>> + Send,
    T: IntoPy<PyObject> + Send,
    E: Send,
    PyErr: From<E>,
{
    fn poll_next_py(
        self: Pin<&mut Self>,
        py: Python,
        waker: &Waker,
    ) -> Poll<Option<PyResult<PyObject>>> {
        py.allow_threads(|| Stream::poll_next(self, &mut Context::from_waker(waker)))
            .map_ok(|ok| ok.into_py(py))
            .map_err(PyErr::from)
    }
}

#[derive(Debug, Clone)]
pub struct FutureWrapper(PyObject);

impl FutureWrapper {
    pub fn new(future: impl Into<PyObject>) -> Self {
        Self(future.into())
    }

    pub fn as_ref<'a>(
        &'a mut self,
        py: Python<'a>,
    ) -> impl Future<Output = PyResult<PyObject>> + Unpin + 'a {
        utils::WithGil { inner: self, py }
    }
}

impl<'a> Future for utils::WithGil<'_, &'a mut FutureWrapper> {
    type Output = PyResult<PyObject>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self
            .inner
            .0
            .call_method0(self.py, "done")?
            .is_true(self.py)?
        {
            return Poll::Ready(self.inner.0.call_method0(self.py, "result"));
        }
        let callback = utils::WakeCallback(Some(cx.waker().clone()));
        self.inner
            .0
            .call_method1(self.py, "add_done_callback", (callback,))?;
        Poll::Pending
    }
}

impl Future for FutureWrapper {
    type Output = PyResult<PyObject>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Python::with_gil(|gil| Pin::into_inner(self).as_ref(gil).poll_unpin(cx))
    }
}
