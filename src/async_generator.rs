use std::{
    marker::PhantomData,
    mem,
    pin::Pin,
    task::{ready, Context, Poll},
};

use pyo3::{exceptions::PyStopAsyncIteration, prelude::*};

use crate::{PyFuture, PyStream, ThrowCallback};

#[pyclass]
struct StreamNext(Option<Pin<Box<dyn PyStream>>>);

struct PyStreamNext {
    next: Py<StreamNext>,
    close: bool,
}

impl PyFuture for PyStreamNext {
    fn poll_py(self: Pin<&mut Self>, py: Python, cx: &mut Context) -> Poll<PyResult<PyObject>> {
        let err = || Err(PyStopAsyncIteration::new_err(py.None()));
        let this = Pin::into_inner(self);
        let mut ref_mut = this.next.borrow_mut(py);
        let Some(stream) = &mut ref_mut.0 else {
            return Poll::Ready(err());
        };
        let opt_res = ready!(stream.as_mut().poll_next_py(py, cx));
        if let Some(res) = opt_res {
            if this.close {
                ref_mut.0 = None;
            }
            return Poll::Ready(res);
        }
        ref_mut.0 = None;
        Poll::Ready(err())
    }
}

enum StreamOrNext {
    Stream(Pin<Box<dyn PyStream>>),
    Next(Py<StreamNext>),
    // temporary used to switch between variants
    Tmp,
}

impl StreamOrNext {
    fn next<'a>(&'a mut self, py: Python<'a>) -> PyResult<&'a mut Py<StreamNext>> {
        if matches!(self, Self::Stream(_)) {
            let Self::Stream(stream) = mem::replace(self, StreamOrNext::Tmp) else {
                unreachable!();
            };
            *self = Self::Next(Py::new(py, StreamNext(Some(stream)))?);
        }
        match self {
            Self::Next(next) => Ok(next),
            _ => unreachable!(),
        }
    }
}

pub(crate) trait CoroutineFactory {
    type Coroutine: IntoPy<PyObject>;
    fn coroutine(future: impl PyFuture + 'static) -> Self::Coroutine;
}

pub(crate) struct AsyncGenerator<C> {
    stream_or_next: StreamOrNext,
    throw: Option<ThrowCallback>,
    _phantom: PhantomData<C>,
}

impl<C> AsyncGenerator<C> {
    pub(crate) fn new(stream: Pin<Box<dyn PyStream>>, throw: Option<ThrowCallback>) -> Self {
        Self {
            stream_or_next: StreamOrNext::Stream(stream),
            throw,
            _phantom: PhantomData,
        }
    }
}

impl<C: CoroutineFactory> AsyncGenerator<C> {
    pub(crate) fn _next(&mut self, py: Python, close: bool) -> PyResult<PyObject> {
        let next = self.stream_or_next.next(py)?.clone();
        Ok(C::coroutine(PyStreamNext { next, close }).into_py(py))
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
