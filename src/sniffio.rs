//! `asyncio`/`trio` compatible coroutine and async generator implementation, lazily specialized
//! using `sniffio`.
use pyo3::{exceptions::PyRuntimeError, prelude::*};

use crate::{asyncio, coroutine, trio, utils};

utils::module!(Sniffio, "sniffio", current_async_library);

enum Waker {
    Asyncio(asyncio::Waker),
    Trio(trio::Waker),
}

impl coroutine::CoroutineWaker for Waker {
    fn new(py: Python) -> PyResult<Self> {
        let sniffed = Sniffio::get(py)?.current_async_library.call0(py)?;
        match sniffed.extract(py)? {
            "asyncio" => Ok(Self::Asyncio(asyncio::Waker::new(py)?)),
            "trio" => Ok(Self::Trio(trio::Waker::new(py)?)),
            rt => Err(PyRuntimeError::new_err(format!("unsupported runtime {rt}"))),
        }
    }

    fn yield_(&self, py: Python) -> PyResult<PyObject> {
        match self {
            Self::Asyncio(w) => w.yield_(py),
            Self::Trio(w) => w.yield_(py),
        }
    }

    fn wake(&self, py: Python) {
        match self {
            Self::Asyncio(w) => w.wake(py),
            Self::Trio(w) => w.wake(py),
        }
    }

    fn update(&mut self, py: Python) -> PyResult<()> {
        match self {
            Self::Asyncio(w) => w.update(py),
            Self::Trio(w) => w.update(py),
        }
    }

    fn raise(&self, py: Python) -> PyResult<()> {
        match self {
            Self::Asyncio(w) => w.raise(py),
            Self::Trio(w) => w.raise(py),
        }
    }
}

utils::generate!(Waker);
