use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use futures::Stream;
use pin_project::pin_project;
use pyo3::Python;

/// Wrapper for [`Future`]/[`Stream`] that releases GIL while polling in
/// [`PyFuture`](crate::PyFuture)/[`PyStream`](crate::PyStream).
///
/// Can be instantiated with [`AllowThreadsExt::allow_threads`].
///
/// [`Stream`]: https://docs.rs/futures/latest/futures/stream/trait.Stream.html
#[derive(Debug)]
#[repr(transparent)]
#[pin_project]
pub struct AllowThreads<T>(#[pin] pub T);

impl<F> Future for AllowThreads<F>
where
    F: Future + Send,
    F::Output: Send,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let waker = cx.waker();
        Python::with_gil(|gil| gil.allow_threads(|| this.0.poll(&mut Context::from_waker(waker))))
    }
}

impl<S> Stream for AllowThreads<S>
where
    S: Stream + Send,
    S::Item: Send,
{
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        let waker = cx.waker();
        Python::with_gil(|gil| {
            gil.allow_threads(|| this.0.poll_next(&mut Context::from_waker(waker)))
        })
    }
}

/// Extension trait to allow threads while polling [`Future`] or [`Stream`].
///
/// It is implemented for every types.
///
/// [`Stream`]: https://docs.rs/futures/latest/futures/stream/trait.Stream.html
pub trait AllowThreadsExt: Sized {
    fn allow_threads(self) -> AllowThreads<Self> {
        AllowThreads(self)
    }
}

impl<T> AllowThreadsExt for T {}
