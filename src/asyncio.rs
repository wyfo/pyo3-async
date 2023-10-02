use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use futures::FutureExt;
use pyo3::{exceptions::PyStopIteration, prelude::*};

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

pub struct AwaitableWrapper {
    future_iter: PyObject,
    future: Option<PyObject>,
}

impl AwaitableWrapper {
    pub fn new(awaitable: &PyAny) -> PyResult<Self> {
        Ok(Self {
            future_iter: awaitable.call_method0("__await__")?.extract()?,
            future: None,
        })
    }

    pub fn as_ref<'a>(
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
        Python::with_gil(|gil| Pin::into_inner(self).as_ref(gil).poll_unpin(cx))
    }
}
