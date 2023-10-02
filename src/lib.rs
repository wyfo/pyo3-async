//! PyO3 bindings to various Python asynchronous frameworks.
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{FutureExt, Stream};
use pyo3::prelude::*;

mod async_generator;
pub mod asyncio;
mod coroutine;
pub mod sniffio;
pub mod trio;
mod utils;

/// GIL-bound [`Future`].
///
/// Provided with a blanket implementation for [`Future`] which polls inside [`Python::allow_threads`].
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
        let waker = cx.waker();
        py.allow_threads(|| Future::poll(self, &mut Context::from_waker(waker)))
            .map_ok(|ok| ok.into_py(py))
            .map_err(PyErr::from)
    }
}

/// GIL-bound [`Stream`].
///
/// Provided with a blanket implementation for [`Stream`] which polls inside [`Python::allow_threads`].
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
        let waker = cx.waker();
        py.allow_threads(|| Stream::poll_next(self, &mut Context::from_waker(waker)))
            .map_ok(|ok| ok.into_py(py))
            .map_err(PyErr::from)
    }
}

/// [`Future`] wrapper for Python future.
///
/// Duck-typed to work either with [`asyncio.Future`](https://docs.python.org/3/library/asyncio-future.html#asyncio.Future) or [`concurrent.futures.Future`](https://docs.python.org/3/library/concurrent.futures.html#concurrent.futures.Future).
#[derive(Debug)]
pub struct FutureWrapper {
    future: PyObject,
    cancel_on_drop: Option<CancelOnDrop>,
}

/// Cancel-on-drop error handling policy (see [`FutureWrapper::new`]).
#[derive(Debug, Copy, Clone)]
pub enum CancelOnDrop {
    IgnoreError,
    PanicOnError,
}

impl FutureWrapper {
    /// Wrap a Python future.
    ///
    /// If `cancel_on_drop` is not `None`, the Python future will be cancelled, and error may be
    /// handled following the provided policy.
    pub fn new(future: impl Into<PyObject>, cancel_on_drop: Option<CancelOnDrop>) -> Self {
        Self {
            future: future.into(),
            cancel_on_drop,
        }
    }

    /// GIL-bound [`Future`] reference.
    pub fn as_mut<'a>(
        &'a mut self,
        py: Python<'a>,
    ) -> impl Future<Output = PyResult<PyObject>> + Unpin + 'a {
        utils::WithGil { inner: self, py }
    }
}

impl<'a> Future for utils::WithGil<'_, &'a mut FutureWrapper> {
    type Output = PyResult<PyObject>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self
            .inner
            .future
            .call_method0(self.py, "done")?
            .is_true(self.py)?
        {
            self.inner.cancel_on_drop = None;
            return Poll::Ready(self.inner.future.call_method0(self.py, "result"));
        }
        let callback = utils::WakeCallback(Some(cx.waker().clone()));
        self.inner
            .future
            .call_method1(self.py, "add_done_callback", (callback,))?;
        Poll::Pending
    }
}

impl Future for FutureWrapper {
    type Output = PyResult<PyObject>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Python::with_gil(|gil| Pin::into_inner(self).as_mut(gil).poll_unpin(cx))
    }
}

impl Drop for FutureWrapper {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel_on_drop {
            let res = Python::with_gil(|gil| self.future.call_method0(gil, "cancel"));
            if let (Err(err), CancelOnDrop::PanicOnError) = (res, cancel) {
                panic!("Cancel error while dropping FutureWrapper: {err:?}");
            }
        }
    }
}

fn tokio() -> &'static tokio::runtime::Runtime {
    use std::sync::OnceLock;
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Runtime::new().unwrap())
}

#[pyfunction]
fn sleep_asyncio() -> asyncio::Coroutine {
    let sleep = async move { tokio::time::sleep(std::time::Duration::from_secs(1)).await };
    let future = tokio().spawn(sleep).map(|_| PyResult::Ok(()));
    asyncio::Coroutine::from_future(future)
}

#[pyfunction]
fn sleep_trio() -> trio::Coroutine {
    let sleep = async move { tokio::time::sleep(std::time::Duration::from_secs(1)).await };
    let future = tokio().spawn(sleep).map(|_| PyResult::Ok(()));
    trio::Coroutine::from_future(future)
}

#[pyfunction]
fn sleep_sniffio() -> sniffio::Coroutine {
    let sleep = async move { tokio::time::sleep(std::time::Duration::from_secs(1)).await };
    let future = tokio().spawn(sleep).map(|_| PyResult::Ok(()));
    sniffio::Coroutine::from_future(future)
}

#[pyfunction]
fn spawn_future(fut: PyObject) {
    tokio().spawn(async move {
        FutureWrapper::new(fut, None).await.unwrap();
        println!("task done")
    });
}

#[pyfunction]
fn async_gen() -> asyncio::AsyncGenerator {
    asyncio::AsyncGenerator::from_stream(futures::stream::unfold(0, |i| async move {
        let sleep = async move { tokio::time::sleep(std::time::Duration::from_secs(1)).await };
        tokio().spawn(sleep).await.ok();
        Some((PyResult::Ok(i), i + 1))
    }))
}

/// A Python module implemented in Rust. The name of this function must match
/// the `lib.name` setting in the `Cargo.toml`, else Python will not be able to
/// import the module.
#[pymodule]
fn pyo3_async(_py: Python<'_>, m: &PyModule) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(sleep_asyncio, m)?)?;
    m.add_function(wrap_pyfunction!(sleep_trio, m)?)?;
    m.add_function(wrap_pyfunction!(sleep_sniffio, m)?)?;
    m.add_function(wrap_pyfunction!(spawn_future, m)?)?;
    m.add_function(wrap_pyfunction!(async_gen, m)?)?;
    Ok(())
}
