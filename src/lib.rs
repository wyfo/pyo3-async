//! PyO3 bindings to various Python asynchronous frameworks.
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use futures::Stream;
use pyo3::prelude::*;

#[cfg(feature = "allow-threads")]
mod allow_threads;
mod async_generator;
pub mod asyncio;
mod coroutine;
pub mod sniffio;
pub mod trio;
mod utils;

#[cfg(feature = "allow-threads")]
pub use allow_threads::{AllowThreads, AllowThreadsExt};
#[cfg(feature = "macros")]
pub use pyo3_async_macros::{pyfunction, pymethods};

/// GIL-bound [`Future`].
///
/// Provided with a blanket implementation for [`Future`]. GIL is maintained during polling
/// operation. To release the GIL, see [`AllowThreads`].
pub trait PyFuture: Send {
    /// GIL-bound [`Future::poll`].
    fn poll_py(self: Pin<&mut Self>, py: Python, cx: &mut Context) -> Poll<PyResult<PyObject>>;
}

impl<F, T, E> PyFuture for F
where
    F: Future<Output = Result<T, E>> + Send,
    T: IntoPy<PyObject> + Send,
    E: Send,
    PyErr: From<E>,
{
    fn poll_py(self: Pin<&mut Self>, py: Python, cx: &mut Context) -> Poll<PyResult<PyObject>> {
        let poll = self.poll(cx);
        poll.map_ok(|ok| ok.into_py(py)).map_err(PyErr::from)
    }
}

/// GIL-bound [`Stream`].
///
/// Provided with a blanket implementation for [`Stream`]. GIL is maintained during polling
/// operation. To release the GIL, see [`AllowThreads`].
///
/// [`Stream`]: https://docs.rs/futures/latest/futures/stream/trait.Stream.html
pub trait PyStream: Send {
    /// GIL-bound [`Stream::poll_next`].
    fn poll_next_py(
        self: Pin<&mut Self>,
        py: Python,
        cx: &mut Context,
    ) -> Poll<Option<PyResult<PyObject>>>;
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
        cx: &mut Context,
    ) -> Poll<Option<PyResult<PyObject>>> {
        let poll = self.poll_next(cx);
        poll.map_ok(|ok| ok.into_py(py)).map_err(PyErr::from)
    }
}

/// Callback for Python coroutine `throw` method (see [`asyncio::Coroutine::new`]) and
/// async generator `athrow` method (see [`asyncio::AsyncGenerator::new`]).
pub type ThrowCallback = Box<dyn FnMut(Python, Option<PyErr>) + Send>;
