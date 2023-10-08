use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use futures::Stream;
use pin_project::pin_project;
use pyo3::prelude::*;

use crate::{PyFuture, PyStream};

/// Wrapper for [`Future`]/[`Stream`] that releases GIL while polling in [`PyFuture`]/[`PyStream`].
///
/// Can be instantiated with [`UnbindGIL::unbind_gil`].
#[derive(Debug)]
#[repr(transparent)]
#[pin_project]
pub struct GilUnbound<T>(#[pin] pub T);

impl<F, T, E> PyFuture for GilUnbound<F>
where
    F: Future<Output = Result<T, E>> + Send,
    T: IntoPy<PyObject> + Send,
    E: Send,
    PyErr: From<E>,
{
    fn poll_py(self: Pin<&mut Self>, py: Python, cx: &mut Context) -> Poll<PyResult<PyObject>> {
        let this = self.project();
        let waker = cx.waker();
        let poll = py.allow_threads(|| this.0.poll(&mut Context::from_waker(waker)));
        poll.map_ok(|ok| ok.into_py(py)).map_err(PyErr::from)
    }
}

impl<S, T, E> PyStream for GilUnbound<S>
where
    S: Stream<Item = Result<T, E>> + Send,
    T: IntoPy<PyObject> + Send,
    E: Send,
    PyErr: From<E>,
{
    fn poll_next_py(
        self: Pin<&mut Self>,
        py: Python,
        cx: &mut Context,
    ) -> Poll<Option<PyResult<PyObject>>> {
        let this = self.project();
        let waker = cx.waker();
        let poll = py.allow_threads(|| this.0.poll_next(&mut Context::from_waker(waker)));
        poll.map_ok(|ok| ok.into_py(py)).map_err(PyErr::from)
    }
}

/// Extension trait to unbind GIL while polling [`Future`] or [`Stream`].
///
/// It is implemented for every types.
pub trait UnbindGil: Sized {
    fn unbind_gil(self) -> GilUnbound<Self> {
        GilUnbound(self)
    }
}

impl<T> UnbindGil for T {}
