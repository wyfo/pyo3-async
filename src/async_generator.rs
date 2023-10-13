use std::{
    marker::PhantomData,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{ready, Context, Poll},
};

use pyo3::{exceptions::PyStopAsyncIteration, prelude::*};

use crate::{PyFuture, PyStream, ThrowCallback};

type SharedStream = Arc<Mutex<Option<Pin<Box<dyn PyStream>>>>>;

struct PyStreamNext {
    stream: SharedStream,
    close: bool,
}

impl PyFuture for PyStreamNext {
    fn poll_py(self: Pin<&mut Self>, py: Python, cx: &mut Context) -> Poll<PyResult<PyObject>> {
        let err = || Err(PyStopAsyncIteration::new_err(py.None()));
        let this = Pin::into_inner(self);
        let mut guard = this.stream.lock().unwrap();
        let Some(ref mut stream) = *guard else {
            return Poll::Ready(err());
        };
        let opt_res = ready!(stream.as_mut().poll_next_py(py, cx));
        if let Some(res) = opt_res {
            if this.close {
                *guard = None;
            }
            return Poll::Ready(res);
        }
        *guard = None;
        Poll::Ready(err())
    }
}

pub(crate) trait CoroutineFactory {
    type Coroutine: IntoPy<PyObject>;
    fn coroutine(future: impl PyFuture + 'static) -> Self::Coroutine;
}

pub(crate) struct AsyncGenerator<C> {
    stream: SharedStream,
    throw: Option<ThrowCallback>,
    _phantom: PhantomData<C>,
}

impl<C> AsyncGenerator<C> {
    pub(crate) fn new(stream: Pin<Box<dyn PyStream>>, throw: Option<ThrowCallback>) -> Self {
        Self {
            stream: Arc::new(Mutex::new(Some(stream))),
            throw,
            _phantom: PhantomData,
        }
    }
}

impl<C: CoroutineFactory> AsyncGenerator<C> {
    pub(crate) fn _next(&mut self, py: Python, close: bool) -> PyResult<PyObject> {
        let stream = self.stream.clone();
        Ok(C::coroutine(PyStreamNext { stream, close }).into_py(py))
    }

    pub(crate) fn next(&mut self, py: Python) -> PyResult<PyObject> {
        self._next(py, false)
    }

    pub(crate) fn throw(&mut self, py: Python, exc: PyErr) -> PyResult<PyObject> {
        let Some(throw) = &mut self.throw else {
            return Ok(C::coroutine(async move { Err::<(), _>(exc) }).into_py(py));
        };
        throw(py, Some(exc));
        self._next(py, false)
    }

    pub(crate) fn close(&mut self, py: Python) -> PyResult<PyObject> {
        if let Some(throw) = &mut self.throw {
            throw(py, None);
        }
        self._next(py, true)
    }
}
