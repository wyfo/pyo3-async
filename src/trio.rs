//! `trio` compatible coroutine and async generator implementation.
use pyo3::{intern, prelude::*};

use crate::{coroutine, utils};

utils::module!(
    Trio,
    "trio.lowlevel",
    Abort,
    current_task,
    current_trio_token,
    reschedule,
    wait_task_rescheduled
);

pub(crate) struct Waker {
    task: PyObject,
    token: PyObject,
}

impl coroutine::CoroutineWaker for Waker {
    fn new(py: Python) -> PyResult<Self> {
        let trio = Trio::get(py)?;
        Ok(Waker {
            task: trio.current_task.call0(py)?,
            token: trio.current_trio_token.call0(py)?,
        })
    }

    fn yield_(&self, py: Python) -> PyResult<PyObject> {
        Trio::get(py)?
            .wait_task_rescheduled
            .call1(py, (Py::new(py, AbortFunc)?,))?
            .call_method0(py, intern!(py, "__await__"))?
            .call_method0(py, intern!(py, "__next__"))
    }

    fn wake(&self, py: Python) {
        let reschedule = &Trio::get(py).unwrap().reschedule;
        reschedule
            .call1(py, (&self.task,))
            .expect("unexpected error while calling trio.lowlevel.reschedule");
    }

    fn wake_threadsafe(&self, py: Python) {
        let reschedule = &Trio::get(py).unwrap().reschedule;
        self.token
            .call_method1(py, intern!(py, "run_sync_soon"), (reschedule, &self.task))
            .expect("unexpected error while scheduling TrioToken.run_sync_soon");
    }
}

#[pyclass]
struct AbortFunc;

#[pymethods]
impl AbortFunc {
    fn __call__(&self, py: Python, _arg: PyObject) -> PyResult<PyObject> {
        Trio::get(py)?.Abort.getattr(py, intern!(py, "SUCCEEDED"))
    }
}

utils::generate!(Waker);
