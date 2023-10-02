# pyo3-async

PyO3 bindings to various Python asynchronous frameworks.

*Disclaimer: This crate is at an early stage of development.*

## Documentation

https://docs.rs/pyo3-async/

## How it works

Asynchronous implementations are not so different in Rust and Python. Rust uses callbacks (through `std::task::Waker`) to wake up the related executor, while Python `Asyncio.Future` also has a callback registered to wake up the event loop.

So, why not use Rust callback to wake up Python event loop, and vice versa ? That's all.

## Difference with [PyO3 Asyncio](ashttps://github.com/awestlake87/pyo3-asyncio)

- PyO3 Asyncio requires a running asynchronous runtime on Rust side, while this crate doesn't;
- PyO3 Asyncio only focus on *asyncio*, while this crate obviously support *asyncio*, but also [*trio*](https://github.com/python-trio/trio) or [*anyio*](https://github.com/agronholm/anyio).

## Example

You can build this module with [Maturin](https://github.com/PyO3/maturin)

```rust
#[pymodule]
fn example(_py: Python<'_>, m: &PyModule) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(sleep_asyncio, m)?)?;
    m.add_function(wrap_pyfunction!(sleep_trio, m)?)?;
    m.add_function(wrap_pyfunction!(sleep_sniffio, m)?)?;
    m.add_function(wrap_pyfunction!(spawn_future, m)?)?;
    m.add_function(wrap_pyfunction!(count_asyncio, m)?)?;
    m.add_function(wrap_pyfunction!(count_trio, m)?)?;
    m.add_function(wrap_pyfunction!(count_sniffio, m)?)?;
    Ok(())
}

fn tokio() -> &'static tokio::runtime::Runtime {
    use std::sync::OnceLock;
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Runtime::new().unwrap())
}

fn sleep(seconds: u64) -> impl Future<Output = PyResult<()>> + Send {
    let sleep = async move { tokio::time::sleep(std::time::Duration::from_secs(seconds)).await };
    tokio().spawn(sleep).map(|_| PyResult::Ok(()))
}

fn count(until: i32, tick: u64) -> impl Stream<Item = PyResult<i32>> + Send {
    futures::stream::unfold(0, move |i| async move {
        if i == until {
            return None;
        }
        sleep(tick).await.ok();
        Some((PyResult::Ok(i), i + 1))
    })
}

#[pyfunction]
fn sleep_asyncio(seconds: u64) -> asyncio::Coroutine {
    asyncio::Coroutine::from_future(sleep(seconds))
}

#[pyfunction]
fn sleep_trio(seconds: u64) -> trio::Coroutine {
    trio::Coroutine::from_future(sleep(seconds))
}

#[pyfunction]
fn sleep_sniffio(seconds: u64) -> sniffio::Coroutine {
    sniffio::Coroutine::from_future(sleep(seconds))
}

#[pyfunction]
fn spawn_future(fut: PyObject) {
    tokio().spawn(async move {
        FutureWrapper::new(fut, None).await.unwrap();
        println!("task done")
    });
}

#[pyfunction]
fn count_asyncio(until: i32, tick: u64) -> asyncio::AsyncGenerator {
    asyncio::AsyncGenerator::from_stream(count(until, tick))
}

#[pyfunction]
fn count_trio(until: i32, tick: u64) -> trio::AsyncGenerator {
    trio::AsyncGenerator::from_stream(count(until, tick))
}

#[pyfunction]
fn count_sniffio(until: i32, tick: u64) -> sniffio::AsyncGenerator {
    sniffio::AsyncGenerator::from_stream(count(until, tick))
}
```

and execute this Python code

```python
import asyncio
import trio
import example  # built with maturin

async def asyncio_main():
    await example.sleep_asyncio(1)
    # sleep 1s
    await example.sleep_sniffio(1)
    # sleep 1s
    example.spawn_future(asyncio.create_task(asyncio.sleep(1)))
    await asyncio.sleep(2)
    # print "done" after 1s
    async for i in example.count_asyncio(2, 1):
        print(i)
        # sleep 1s, print 0
        # sleep 1s, print 1

async def trio_run():
    await example.sleep_trio(1)
    # sleep 1s
    await example.sleep_sniffio(1)
    # sleep 1s
    async for i in example.count_trio(2, 1):
        print(i)
        # sleep 1s, print 0
        # sleep 1s, print 1

asyncio.run(asyncio_main())
print("======================")
trio.run(trio_run)
```