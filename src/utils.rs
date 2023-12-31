use std::sync::atomic::{AtomicUsize, Ordering};

use pyo3::{exceptions::PyStopIteration, prelude::*, pyclass::IterNextOutput, types::PyCFunction};

// Don't use `std::thread::current` because of unnecessary Arc clone + drop.
pub(crate) type ThreadId = usize;
pub(crate) fn current_thread_id() -> ThreadId {
    static THREAD_COUNTER: AtomicUsize = AtomicUsize::new(0);
    thread_local! {
        pub(crate) static THREAD_ID: ThreadId = THREAD_COUNTER.fetch_add(1, Ordering::Relaxed);
    }
    THREAD_ID.with(|id| *id)
}

pub(crate) struct WithGil<'py, T> {
    pub(crate) inner: T,
    pub(crate) py: Python<'py>,
}

pub(crate) fn wake_callback(py: Python, waker: std::task::Waker) -> PyResult<&PyAny> {
    let func = PyCFunction::new_closure(py, None, None, move |_, _| waker.wake_by_ref())?;
    Ok(func)
}

macro_rules! module {
    ($name:ident ,$path:literal, $($field:ident),* $(,)?) => {
        #[allow(non_upper_case_globals)]
        static $name: ::pyo3::sync::GILOnceCell<$name> = ::pyo3::sync::GILOnceCell::new();

        #[allow(non_snake_case)]
        struct $name {
            $($field: PyObject),*
        }

        impl $name {
            fn get(py: Python) -> PyResult<&Self> {
                $name.get_or_try_init(py, || {
                    let module = py.import($path)?;
                    Ok(Self {
                        $($field: module.getattr(stringify!($field))?.into(),)*
                    })
                })
            }
        }
    };
}

pub(crate) use module;

pub(crate) fn poll_result(result: IterNextOutput<PyObject, PyObject>) -> PyResult<PyObject> {
    match result {
        IterNextOutput::Yield(ob) => Ok(ob),
        IterNextOutput::Return(ob) => Err(PyStopIteration::new_err(ob)),
    }
}

macro_rules! generate {
    ($waker:ty) => {
        /// Python coroutine wrapping a [`PyFuture`](crate::PyFuture).
        #[pyclass]
        pub struct Coroutine($crate::coroutine::Coroutine<$waker>);

        impl Coroutine {
            /// Wrap a boxed future in to a Python coroutine.
            ///
            /// If `throw` callback is provided:
            /// - coroutine `throw` method will call it with the passed exception before polling;
            /// - coroutine `close` method will call it with `None` before polling and dropping
            ///   the future.
            /// If `throw` callback is not provided, the future will dropped without additional
            /// poll.
            pub fn new(
                future: ::std::pin::Pin<Box<dyn $crate::PyFuture>>,
                throw: Option<$crate::ThrowCallback>,
            ) -> Self {
                Self($crate::coroutine::Coroutine::new(future, throw))
            }

            /// Wrap a generic future into a Python coroutine.
            pub fn from_future(future: impl $crate::PyFuture + 'static) -> Self {
                Self::new(Box::pin(future), None)
            }
        }

        #[pymethods]
        impl Coroutine {
            fn send(&mut self, py: Python, _value: &PyAny) -> PyResult<PyObject> {
                $crate::utils::poll_result(self.0.poll(py, None)?)
            }

            fn throw(&mut self, py: Python, exc: &PyAny) -> PyResult<PyObject> {
                $crate::utils::poll_result(self.0.poll(py, Some(PyErr::from_value(exc)))?)
            }

            fn close(&mut self, py: Python) -> PyResult<()> {
                self.0.close(py)
            }

            fn __await__(self_: &PyCell<Self>) -> PyResult<&PyAny> {
                Ok(self_)
            }

            fn __iter__(self_: &PyCell<Self>) -> PyResult<&PyAny> {
                Ok(self_)
            }

            fn __next__(
                &mut self,
                py: Python,
            ) -> PyResult<::pyo3::pyclass::IterNextOutput<PyObject, PyObject>> {
                self.0.poll(py, None)
            }
        }

        impl $crate::async_generator::CoroutineFactory for Coroutine {
            type Coroutine = Self;
            fn coroutine(future: impl $crate::PyFuture + 'static) -> Self::Coroutine {
                Self::from_future(future)
            }
        }

        /// Python async generator wrapping a [`PyStream`](crate::PyStream).
        #[pyclass]
        pub struct AsyncGenerator($crate::async_generator::AsyncGenerator<Coroutine>);

        impl AsyncGenerator {
            /// Wrap a boxed stream in to a Python async generator.
            ///
            /// If `throw` callback is provided:
            /// - async generator `athrow` method will call it with the passed exception
            ///   before polling;
            /// - async generator `aclose` method will call it with `None` before polling and
            ///   dropping the stream.
            /// If `throw` callback is not provided, the stream will dropped without additional
            /// poll.
            pub fn new(
                stream: ::std::pin::Pin<Box<dyn $crate::PyStream>>,
                throw: Option<$crate::ThrowCallback>,
            ) -> Self {
                Self($crate::async_generator::AsyncGenerator::new(stream, throw))
            }

            /// Wrap a generic stream.
            pub fn from_stream(stream: impl $crate::PyStream + 'static) -> Self {
                Self::new(Box::pin(stream), None)
            }
        }

        #[pymethods]
        impl AsyncGenerator {
            fn asend(&mut self, py: Python, _value: &PyAny) -> PyResult<PyObject> {
                self.0.next(py)
            }

            fn athrow(&mut self, py: Python, exc: &PyAny) -> PyResult<PyObject> {
                self.0.throw(py, PyErr::from_value(exc))
            }

            fn aclose(&mut self, py: Python) -> PyResult<PyObject> {
                self.0.close(py)
            }

            fn __aiter__(self_: &PyCell<Self>) -> PyResult<&PyAny> {
                Ok(self_)
            }

            // `Option` because https://github.com/PyO3/pyo3/issues/3190
            fn __anext__(&mut self, py: Python) -> PyResult<Option<PyObject>> {
                self.0.next(py).map(Some)
            }
        }
    };
}
pub(crate) use generate;
